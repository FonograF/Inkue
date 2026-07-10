//! Video slot pool — one mpv context per simultaneously-visible visual cue.
//!
//! QLab-style layering: every Video / Image / Camera cue gets its own
//! [`VideoSlot`] (mpv context + event thread + FBO on the render thread), and
//! the compositor in `render.rs` stacks the slot textures in layer order with
//! per-slot opacity and blend mode.  Slots are created lazily up to
//! [`MAX_VIDEO_SLOTS`] and never destroyed — an idle slot costs one idle mpv
//! context.  When the pool is exhausted the oldest content is stolen (hard
//! stop, `Completed` status so the owning cue resets).
//!
//! GL output path only; the legacy Win32 path keeps the single-context
//! behaviour in `mod.rs`.

use std::ffi::{c_char, c_void, CString};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};

use crate::engine::mpv_sys::{
    MpvEventEndFile, MpvEventLogMessage, MpvLib,
    MPV_END_FILE_REASON_EOF, MPV_END_FILE_REASON_ERROR,
    MPV_EVENT_END_FILE, MPV_EVENT_FILE_LOADED, MPV_EVENT_LOG_MESSAGE,
    MPV_EVENT_PLAYBACK_RESTART, MPV_EVENT_SHUTDOWN, MPV_EVENT_VIDEO_RECONFIG,
    MPV_FORMAT_DOUBLE,
};
use crate::engine::AudioEngine;

use super::blend::BlendMode;
use super::types::{LayerStyle, MpvCtx, OutputStatus, VideoGeometry, VoiceId};
use super::{cs, opt_str, try_apply_crop, OUTPUT_STATUS_TX};

/// Maximum simultaneously-open video slots (excluding the overlay context).
/// Each active slot is a full decode pipeline; 8 covers any realistic stage.
pub(super) const MAX_VIDEO_SLOTS: usize = 8;

/// Stacking sequence for automatic layering (newest on top) and steal order.
static LAYER_SEQ: AtomicU64 = AtomicU64::new(1);

/// The slot registry.  Grow-only; the render thread iterates it every frame.
static SLOTS: OnceLock<RwLock<Vec<Arc<VideoSlot>>>> = OnceLock::new();

fn registry() -> &'static RwLock<Vec<Arc<VideoSlot>>> {
    SLOTS.get_or_init(|| RwLock::new(Vec::new()))
}

/// Snapshot of all slots (cheap Arc clones) for the render thread.
pub(super) fn all_slots() -> Vec<Arc<VideoSlot>> {
    registry().read().map(|v| v.clone()).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Opacity animation
// ---------------------------------------------------------------------------

/// Per-slot opacity animation (fade in/out, EOF fade, Fade Cue).
#[derive(Debug, Clone)]
pub(super) struct OpacityAnim {
    pub current: f32,
    pub start: f32,
    pub target: f32,
    pub duration_ms: u32,
    pub started_at: Instant,
}

impl OpacityAnim {
    fn resting(v: f32) -> Self {
        Self { current: v, start: v, target: v, duration_ms: 0, started_at: Instant::now() }
    }

    fn animate_to(&mut self, target: f32, duration_ms: u32) {
        self.start = self.current;
        self.target = target.clamp(0.0, 1.0);
        self.duration_ms = duration_ms;
        self.started_at = Instant::now();
    }

    fn set(&mut self, v: f32) {
        let v = v.clamp(0.0, 1.0);
        self.current = v;
        self.start = v;
        self.target = v;
        self.duration_ms = 0;
    }

    /// Advance one frame.  Returns `(current, just_completed)`.
    fn tick(&mut self) -> (f32, bool) {
        if (self.current - self.target).abs() < f32::EPSILON {
            return (self.current, false);
        }
        let t = if self.duration_ms == 0 {
            1.0
        } else {
            (self.started_at.elapsed().as_millis() as f32 / self.duration_ms as f32).min(1.0)
        };
        self.current = self.start + (self.target - self.start) * t;
        let done = t >= 1.0;
        if done {
            self.current = self.target;
        }
        (self.current, done)
    }

    pub(super) fn is_animating(&self) -> bool {
        (self.current - self.target).abs() > f32::EPSILON
    }
}

// ---------------------------------------------------------------------------
// SlotState
// ---------------------------------------------------------------------------

/// Everything about the content currently in a slot.  Locked briefly by the
/// engine (GO/stop/live edits), the slot's event thread, and the render
/// thread's per-frame tick — never held across blocking calls.
pub(super) struct SlotState {
    /// The output voice occupying this slot (`None` = idle).
    pub voice_id: Option<VoiceId>,
    /// The AudioEngine voice carrying this content's audio track, if any.
    pub audio_voice_id: Option<VoiceId>,
    /// Cue geometry, kept for the pixel-crop at `VIDEO_RECONFIG`.
    pub geometry: VideoGeometry,
    /// `true` once the crop has been resolved for the current load.
    pub crop_applied: bool,
    /// Compositor sort key: explicit layers order below automatic ones,
    /// sequence breaks ties (newest on top).  See [`resolve_layer_key`].
    pub layer_key: u64,
    pub blend_mode: BlendMode,
    /// The cue's base opacity; the animation multiplies against it.
    pub base_opacity: f32,
    /// Runtime opacity animation (0 → base on reveal, → 0 on stop).
    pub anim: OpacityAnim,
    /// Set while a paused video load waits for its first frame
    /// (`PLAYBACK_RESTART`); carries the fade-in duration.
    pub pending_reveal: Option<u32>,
    /// Failsafe deadline for the reveal (mpv event missing/late).
    pub reveal_deadline: Option<Instant>,
    /// `true` when the fade-out completes and mpv should be stopped + the
    /// slot released.  Drained by the render thread tick.
    pub pending_unload: bool,
    /// Freeze on last frame at EOF (`keep-open=yes` was set for this load).
    pub hold_last_frame: bool,
    /// Monotonic per-slot generation guard (a slow event for load N must not
    /// touch load N+1).
    pub generation: u64,
}

impl SlotState {
    fn idle() -> Self {
        Self {
            voice_id: None,
            audio_voice_id: None,
            geometry: VideoGeometry::default(),
            crop_applied: true,
            layer_key: 0,
            blend_mode: BlendMode::Normal,
            base_opacity: 1.0,
            anim: OpacityAnim::resting(0.0),
            pending_reveal: None,
            reveal_deadline: None,
            pending_unload: false,
            hold_last_frame: false,
            generation: 0,
        }
    }
}

/// Sort key so explicit layers (1–1000) stack below automatic ones, newest
/// automatic content on top, ties broken by GO order.
pub(super) fn resolve_layer_key(layer: Option<u32>, seq: u64) -> u64 {
    match layer {
        // Explicit layer L → band L, ordered by seq inside the band.
        Some(l) => (l.clamp(1, 1000) as u64) << 40 | (seq & 0xFF_FFFF_FFFF),
        // Automatic → above every explicit band.
        None => (1001u64 << 40) | (seq & 0xFF_FFFF_FFFF),
    }
}

// ---------------------------------------------------------------------------
// VideoSlot
// ---------------------------------------------------------------------------

/// One mpv context of the video pool.
pub(super) struct VideoSlot {
    pub index: usize,
    pub lib: Arc<MpvLib>,
    pub mpv_ctx: Arc<MpvCtx>,
    pub audio_engine: Arc<AudioEngine>,
    pub state: Mutex<SlotState>,
    /// mpv_render_context, created by the render thread (GL context needed);
    /// null until then.
    pub render_ctx: AtomicPtr<c_void>,
    /// Set at creation; cleared by the render thread once `render_ctx` is up.
    pub needs_render_init: AtomicBool,
}

// SAFETY: the raw mpv pointers are only used through the thread-safe libmpv
// client API; render_ctx is only touched by the render thread.
unsafe impl Send for VideoSlot {}
unsafe impl Sync for VideoSlot {}

impl VideoSlot {
    /// Send an OutputStatus to the show event loop.
    fn send_status(&self, status: OutputStatus) {
        if let Some(tx) = OUTPUT_STATUS_TX.get() {
            let _ = tx.send(status);
        }
    }
}

// ---------------------------------------------------------------------------
// Pool operations
// ---------------------------------------------------------------------------

/// Find the slot currently owning `voice`.
pub(super) fn slot_for_voice(voice: VoiceId) -> Option<Arc<VideoSlot>> {
    all_slots().into_iter().find(|s| {
        s.state.lock().map(|st| st.voice_id == Some(voice)).unwrap_or(false)
    })
}

/// Acquire a slot for new content: reuse an idle one, create one below the
/// cap, or steal the oldest occupied slot (hard stop + `Completed`).
pub(super) fn acquire_slot(
    lib: &Arc<MpvLib>,
    audio_engine: &Arc<AudioEngine>,
) -> Result<Arc<VideoSlot>> {
    // 1. Reuse an idle slot.
    for slot in all_slots() {
        if let Ok(st) = slot.state.lock() {
            if st.voice_id.is_none() && !st.pending_unload {
                return Ok(Arc::clone(&slot));
            }
        }
    }

    // 2. Create a new one below the cap.
    let count = registry().read().map(|v| v.len()).unwrap_or(0);
    if count < MAX_VIDEO_SLOTS {
        return create_slot(lib, audio_engine);
    }

    // 3. Steal the oldest (lowest sequence) occupied slot.
    let victim = all_slots()
        .into_iter()
        .min_by_key(|s| s.state.lock().map(|st| st.layer_key & 0xFF_FFFF_FFFF).unwrap_or(u64::MAX))
        .ok_or_else(|| anyhow!("video slot pool empty and at cap — cannot allocate"))?;
    log::warn!(
        "[slot] pool exhausted ({MAX_VIDEO_SLOTS} slots) — stealing slot {}",
        victim.index
    );
    hard_unload(&victim, true);
    Ok(victim)
}

/// Create a new mpv context + event thread and register the slot.
fn create_slot(lib: &Arc<MpvLib>, audio_engine: &Arc<AudioEngine>) -> Result<Arc<VideoSlot>> {
    let ctx = unsafe { (lib.mpv_create)() };
    if ctx.is_null() {
        return Err(anyhow!("mpv_create() returned null for video slot"));
    }

    unsafe {
        opt_str(lib, ctx, "vo", "libmpv");
        opt_str(lib, ctx, "hwdec", "auto-copy");
        opt_str(lib, ctx, "osc", "no");
        opt_str(lib, ctx, "osd-level", "0");
        opt_str(lib, ctx, "input-default-bindings", "no");
        opt_str(lib, ctx, "input-vo-keyboard", "no");
        opt_str(lib, ctx, "input-cursor", "no");
        opt_str(lib, ctx, "keep-open", "no");
        opt_str(lib, ctx, "idle", "yes");
        opt_str(lib, ctx, "ao", "null");
        opt_str(lib, ctx, "audio", "no");
        opt_str(lib, ctx, "video-sync", "desync");
        // Transparent background: where the slot has no pixels (letterbox,
        // idle) the FBO alpha is 0 so lower layers show through.
        // `background=none` is mpv ≥ 0.38; `alpha=yes` covers older libmpv
        // (Ubuntu 22.04 ships 0.34) — setting both is harmless.
        opt_str(lib, ctx, "background", "none");
        opt_str(lib, ctx, "alpha", "yes");

        let ret = (lib.mpv_initialize)(ctx);
        if ret < 0 {
            (lib.mpv_terminate_destroy)(ctx);
            return Err(anyhow!("mpv_initialize() failed for video slot: {ret}"));
        }
    }

    let index = registry().read().map(|v| v.len()).unwrap_or(0);
    let slot = Arc::new(VideoSlot {
        index,
        lib: Arc::clone(lib),
        mpv_ctx: Arc::new(MpvCtx(ctx)),
        audio_engine: Arc::clone(audio_engine),
        state: Mutex::new(SlotState::idle()),
        render_ctx: AtomicPtr::new(std::ptr::null_mut()),
        needs_render_init: AtomicBool::new(true),
    });

    {
        let event_slot = Arc::clone(&slot);
        std::thread::Builder::new()
            .name(format!("inkue-slot-{index}-events"))
            .spawn(move || slot_event_loop(event_slot))
            .map_err(|e| anyhow!("spawn slot event thread: {e}"))?;
    }

    registry().write().map_err(|_| anyhow!("slot registry poisoned"))?.push(Arc::clone(&slot));
    // The render thread creates the mpv_render_context + FBO on next wake.
    super::render::wake();
    log::info!("[slot] created video slot {index}");
    Ok(slot)
}

/// Immediately cut a slot's content: stop mpv, stop its audio voice, clear
/// the state.  `report_completed` sends `Completed` so the owning cue resets
/// (steal / panic paths).
pub(super) fn hard_unload(slot: &Arc<VideoSlot>, report_completed: bool) {
    let (voice, audio) = {
        let Ok(mut st) = slot.state.lock() else { return };
        st.generation = st.generation.wrapping_add(1);
        let voice = st.voice_id.take();
        let audio = st.audio_voice_id.take();
        st.pending_reveal = None;
        st.reveal_deadline = None;
        st.pending_unload = false;
        st.anim.set(0.0);
        (voice, audio)
    };
    unsafe {
        let stop = cs("stop");
        let args: [*const c_char; 2] = [stop.as_ptr(), std::ptr::null()];
        (slot.lib.mpv_command)(slot.mpv_ctx.0, args.as_ptr());
    }
    if let Some(aid) = audio {
        let _ = slot.audio_engine.stop_voice(aid, 0, crate::engine::ring_command::FadeCurve::Linear);
    }
    if report_completed {
        if let Some(vid) = voice {
            slot.send_status(OutputStatus::Completed { voice_id: vid });
        }
    }
    super::render::wake();
}

/// Parameters for [`load_into_slot`].
pub(super) struct SlotLoad {
    pub voice_id: VoiceId,
    pub audio_voice_id: Option<VoiceId>,
    pub url: String,
    pub is_image: bool,
    pub fade_in_ms: u32,
    pub loop_count: u32,
    pub start_ms: Option<u64>,
    pub end_ms: Option<u64>,
    pub display_duration_ms: Option<u64>,
    pub hold_last_frame: bool,
    pub live_source: bool,
    pub geometry: VideoGeometry,
    pub layer_style: LayerStyle,
}

/// Load content into an (idle) slot.
pub(super) fn load_into_slot(slot: &Arc<VideoSlot>, load: SlotLoad) {
    let lib = &slot.lib;
    let ctx = slot.mpv_ctx.0;
    let seq = LAYER_SEQ.fetch_add(1, Ordering::Relaxed);

    {
        let Ok(mut st) = slot.state.lock() else { return };
        st.generation = st.generation.wrapping_add(1);
        st.voice_id = Some(load.voice_id);
        st.audio_voice_id = load.audio_voice_id;
        st.geometry = load.geometry;
        st.crop_applied = !load.geometry.has_crop();
        st.layer_key = resolve_layer_key(load.layer_style.layer, seq);
        st.blend_mode = load.layer_style.blend_mode;
        st.base_opacity = load.layer_style.opacity.clamp(0.0, 1.0) as f32;
        st.pending_unload = false;
        st.hold_last_frame = load.hold_last_frame;
        if load.is_image {
            // Images decode near-instantly: start the reveal fade right away.
            st.pending_reveal = None;
            st.reveal_deadline = None;
            let base = st.base_opacity;
            if load.fade_in_ms > 0 {
                st.anim.set(0.0);
                st.anim.animate_to(base, load.fade_in_ms);
            } else {
                st.anim.set(base);
            }
        } else {
            // Videos load paused; PLAYBACK_RESTART reveals + unpauses.
            st.anim.set(0.0);
            st.pending_reveal = Some(load.fade_in_ms);
            st.reveal_deadline = None; // armed at FILE_LOADED
        }
    }

    // Per-slot scalar geometry (crop resolves at VIDEO_RECONFIG).
    super::apply_scalar_geometry(lib, ctx, &load.geometry);
    let _ = try_apply_crop(lib, ctx, &load.geometry);

    unsafe {
        let keep_open = if load.hold_last_frame && !load.is_image { "yes" } else { "no" };
        (lib.mpv_set_property_string)(ctx, cs("keep-open").as_ptr(), cs(keep_open).as_ptr());

        let mut opts: Vec<String> = vec!["audio=no".to_string()];
        if load.is_image {
            let duration_val = load
                .display_duration_ms
                .map(|ms| format!("{:.3}", ms as f64 / 1000.0))
                .unwrap_or_else(|| "inf".to_string());
            opts.push(format!("image-display-duration={duration_val}"));
            (lib.mpv_set_property_string)(ctx, cs("pause").as_ptr(), cs("no").as_ptr());
        } else {
            if let Some(start) = load.start_ms {
                opts.push(format!("start={:.3}", start as f64 / 1000.0));
            }
            if let Some(end) = load.end_ms {
                opts.push(format!("end={:.3}", end as f64 / 1000.0));
            }
            let loop_val = if load.loop_count == u32::MAX {
                "inf".to_string()
            } else if load.loop_count == 0 {
                "no".to_string()
            } else {
                load.loop_count.to_string()
            };
            opts.push(format!("loop-file={loop_val}"));
            if load.live_source {
                opts.push("cache=no".to_string());
                opts.push("demuxer-lavf-analyzeduration=0.1".to_string());
                opts.push("video-latency-hacks=yes".to_string());
            }
            // Paused load: frame 0 decoded → PLAYBACK_RESTART → reveal.
            (lib.mpv_set_property_string)(ctx, cs("pause").as_ptr(), cs("yes").as_ptr());
        }

        let Ok(path_cstr) = CString::new(load.url.as_str()) else {
            log::warn!("[slot] load path contains NUL byte");
            return;
        };
        let opts_cstr = cs(&opts.join(","));
        let cmd = cs("loadfile");
        let flags = cs("replace");
        let idx = cs("0");
        // loadfile signature: <url> <flags> <index> <options> (see fade.rs).
        let args: [*const c_char; 6] = [
            cmd.as_ptr(), path_cstr.as_ptr(), flags.as_ptr(),
            idx.as_ptr(), opts_cstr.as_ptr(), std::ptr::null(),
        ];
        let ret = (lib.mpv_command)(ctx, args.as_ptr());
        if ret < 0 {
            log::warn!("[slot {}] loadfile failed: {ret} ({})", slot.index, load.url);
        } else {
            log::info!("[slot {}] loadfile: {} opts=[{}]", slot.index, load.url, opts.join(","));
        }
    }
    super::render::wake();
}

/// Begin the stop fade for a voice.  The render thread finishes the unload
/// once the opacity reaches 0.  Audio is stopped by the caller (engine).
pub(super) fn begin_stop(slot: &Arc<VideoSlot>, visual_fade_ms: u32) {
    if let Ok(mut st) = slot.state.lock() {
        st.pending_reveal = None;
        st.reveal_deadline = None;
        st.pending_unload = true;
        st.audio_voice_id = None; // caller owns the audio stop
        if visual_fade_ms == 0 {
            st.anim.set(0.0);
        } else {
            st.anim.animate_to(0.0, visual_fade_ms);
        }
    }
    super::render::wake();
}

/// Per-frame slot maintenance, called by the render thread: advance the
/// opacity animation and finish pending unloads.  Returns the current
/// opacity and whether the slot still needs animation frames.
pub(super) fn tick_slot(slot: &Arc<VideoSlot>) -> (f32, bool) {
    // Reveal watchdog (mpv never signalled the first frame).
    let force_reveal = {
        let Ok(mut st) = slot.state.lock() else { return (0.0, false) };
        match (st.pending_reveal, st.reveal_deadline) {
            (Some(_), Some(deadline)) if Instant::now() >= deadline => {
                st.reveal_deadline = None;
                true
            }
            _ => false,
        }
    };
    if force_reveal {
        log::warn!("[slot {}] reveal watchdog fired — forcing unpause", slot.index);
        reveal(slot);
    }

    let Ok(mut st) = slot.state.lock() else { return (0.0, false) };
    let (opacity, completed) = st.anim.tick();
    let unload_now = completed && st.pending_unload && opacity <= 0.0;
    let animating = st.anim.is_animating();
    if unload_now {
        st.pending_unload = false;
        st.voice_id = None;
        st.generation = st.generation.wrapping_add(1);
        drop(st);
        unsafe {
            let stop = cs("stop");
            let args: [*const c_char; 2] = [stop.as_ptr(), std::ptr::null()];
            (slot.lib.mpv_command)(slot.mpv_ctx.0, args.as_ptr());
        }
        return (0.0, false);
    }
    (opacity, animating)
}

/// Reveal a paused video load: resume its audio voice, unpause mpv, start the
/// opacity fade-in.  Called on `PLAYBACK_RESTART` (or the watchdog).
fn reveal(slot: &Arc<VideoSlot>) {
    let (fade_in_ms, base, audio) = {
        let Ok(mut st) = slot.state.lock() else { return };
        let Some(fade) = st.pending_reveal.take() else { return };
        st.reveal_deadline = None;
        (fade, st.base_opacity, st.audio_voice_id)
    };

    if let Some(aid) = audio {
        let _ = slot.audio_engine.resume_voice(aid);
    }
    unsafe {
        (slot.lib.mpv_set_property_string)(
            slot.mpv_ctx.0, cs("pause").as_ptr(), cs("no").as_ptr(),
        );
    }
    if let Ok(mut st) = slot.state.lock() {
        if fade_in_ms > 0 {
            st.anim.set(0.0);
            st.anim.animate_to(base, fade_in_ms);
        } else {
            st.anim.set(base);
        }
    }
    super::render::wake();
}

// ---------------------------------------------------------------------------
// Live property updates
// ---------------------------------------------------------------------------

/// Live-apply a cue's LayerStyle edit (opacity edits retarget the animation
/// so a running fade is not fought).
pub(super) fn set_layer_style(slot: &Arc<VideoSlot>, style: &LayerStyle) {
    if let Ok(mut st) = slot.state.lock() {
        st.blend_mode = style.blend_mode;
        let new_base = style.opacity.clamp(0.0, 1.0) as f32;
        // Re-key only the explicit band; the sequence part is kept so the
        // stacking among same-layer content is stable.
        let seq = st.layer_key & 0xFF_FFFF_FFFF;
        st.layer_key = resolve_layer_key(style.layer, seq);
        if !st.anim.is_animating() && !st.pending_unload && st.pending_reveal.is_none() {
            st.anim.set(new_base);
        }
        st.base_opacity = new_base;
    }
    super::render::wake();
}

/// Directly drive a slot's opacity (Fade Cue tick, ~30 fps).
pub(super) fn set_opacity_direct(slot: &Arc<VideoSlot>, opacity: f32) {
    if let Ok(mut st) = slot.state.lock() {
        st.anim.set(opacity);
    }
    super::render::wake();
}

/// Current animated opacity of a voice's slot.
pub(super) fn opacity_of(slot: &Arc<VideoSlot>) -> f32 {
    slot.state.lock().map(|st| st.anim.current).unwrap_or(0.0)
}

/// Animate a slot's opacity to a target (EOF fade-out).
pub(super) fn animate_opacity(slot: &Arc<VideoSlot>, target: f32, duration_ms: u32) {
    if let Ok(mut st) = slot.state.lock() {
        st.anim.animate_to(target, duration_ms.max(1));
    }
    super::render::wake();
}

// ---------------------------------------------------------------------------
// Slot event loop
// ---------------------------------------------------------------------------

fn slot_event_loop(slot: Arc<VideoSlot>) {
    let lib = Arc::clone(&slot.lib);
    let ctx = slot.mpv_ctx.0 as usize; // Send-safe copy for this thread only.

    loop {
        let event = unsafe { (lib.mpv_wait_event)(ctx as *mut c_void, 1.0) };
        if event.is_null() {
            continue;
        }
        let event_id = unsafe { (*event).event_id };

        match event_id {
            MPV_EVENT_SHUTDOWN => break,

            MPV_EVENT_PLAYBACK_RESTART => {
                let has_pending = slot
                    .state
                    .lock()
                    .map(|st| st.pending_reveal.is_some())
                    .unwrap_or(false);
                if has_pending {
                    reveal(&slot);
                    log::info!("[slot {}] first frame — revealed", slot.index);
                }
            }

            MPV_EVENT_FILE_LOADED => {
                // Report the media duration to the show event loop.
                let mut duration_secs: f64 = 0.0;
                let ret = unsafe {
                    let name = cs("duration");
                    (lib.mpv_get_property)(
                        ctx as *mut c_void, name.as_ptr(), MPV_FORMAT_DOUBLE,
                        &mut duration_secs as *mut f64 as *mut c_void,
                    )
                };
                if let Ok(mut st) = slot.state.lock() {
                    if let Some(vid) = st.voice_id {
                        if ret == 0 {
                            slot.send_status(OutputStatus::Duration {
                                voice_id: vid,
                                duration_ms: (duration_secs * 1000.0) as u64,
                            });
                        }
                        // Arm the reveal watchdog for paused video loads.
                        if st.pending_reveal.is_some() {
                            st.reveal_deadline =
                                Some(Instant::now() + Duration::from_millis(2500));
                        }
                    }
                }
                super::render::wake();
            }

            MPV_EVENT_VIDEO_RECONFIG => {
                // Source dimensions are now known — resolve the pixel crop.
                let geometry = slot.state.lock().ok().and_then(|mut st| {
                    if st.crop_applied {
                        None
                    } else {
                        st.crop_applied = true;
                        Some(st.geometry)
                    }
                });
                if let Some(g) = geometry {
                    let _ = try_apply_crop(&lib, ctx as *mut c_void, &g);
                }
                super::render::wake();
            }

            MPV_EVENT_END_FILE => {
                let data_ptr = unsafe { (*event).data };
                let Some(end_data) = (unsafe { (data_ptr as *mut MpvEventEndFile).as_ref() })
                else {
                    continue;
                };
                match end_data.reason {
                    MPV_END_FILE_REASON_EOF => {
                        let (voice, audio) = {
                            let Ok(mut st) = slot.state.lock() else { continue };
                            let voice = st.voice_id.take();
                            let audio = st.audio_voice_id.take();
                            st.pending_reveal = None;
                            st.reveal_deadline = None;
                            st.pending_unload = false;
                            st.anim.set(0.0);
                            (voice, audio)
                        };
                        if let Some(aid) = audio {
                            let _ = slot.audio_engine.stop_voice(
                                aid, 0, crate::engine::ring_command::FadeCurve::Linear,
                            );
                        }
                        if let Some(vid) = voice {
                            slot.send_status(OutputStatus::Completed { voice_id: vid });
                        }
                        super::render::wake();
                    }
                    MPV_END_FILE_REASON_ERROR => {
                        let (voice, audio) = {
                            let Ok(mut st) = slot.state.lock() else { continue };
                            let voice = st.voice_id.take();
                            let audio = st.audio_voice_id.take();
                            st.pending_reveal = None;
                            st.reveal_deadline = None;
                            st.anim.set(0.0);
                            (voice, audio)
                        };
                        if let Some(aid) = audio {
                            let _ = slot.audio_engine.stop_voice(
                                aid, 0, crate::engine::ring_command::FadeCurve::Linear,
                            );
                        }
                        if let Some(vid) = voice {
                            slot.send_status(OutputStatus::Error {
                                voice_id: vid,
                                message: format!("mpv error (code {})", end_data.error),
                            });
                        }
                        super::render::wake();
                    }
                    _ => {}
                }
            }

            MPV_EVENT_LOG_MESSAGE => {
                let data = unsafe { (*event).data as *const MpvEventLogMessage };
                if !data.is_null() {
                    let level = unsafe { std::ffi::CStr::from_ptr((*data).level) }.to_string_lossy();
                    let text = unsafe { std::ffi::CStr::from_ptr((*data).text) }.to_string_lossy();
                    let trimmed = text.trim_end_matches('\n');
                    if !trimmed.is_empty() && matches!(level.as_ref(), "fatal" | "error") {
                        log::error!("[slot {}] [mpv] {trimmed}", slot.index);
                    }
                }
            }

            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Pool-wide operations
// ---------------------------------------------------------------------------

/// Panic: hard-unload every slot (double-Escape backstop).
pub(super) fn panic_all() {
    for slot in all_slots() {
        hard_unload(&slot, false);
    }
}

/// Current playback position of a voice's video, in ms.
pub(super) fn position_ms(slot: &Arc<VideoSlot>) -> Option<u64> {
    let mut secs: f64 = 0.0;
    let name = cs("time-pos");
    let ret = unsafe {
        (slot.lib.mpv_get_property)(
            slot.mpv_ctx.0, name.as_ptr(), MPV_FORMAT_DOUBLE,
            &mut secs as *mut f64 as *mut c_void,
        )
    };
    (ret == 0 && secs >= 0.0).then_some((secs * 1000.0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_key_explicit_bands_order_below_automatic() {
        let explicit_low = resolve_layer_key(Some(1), 100);
        let explicit_high = resolve_layer_key(Some(1000), 1);
        let auto_old = resolve_layer_key(None, 2);
        let auto_new = resolve_layer_key(None, 3);
        assert!(explicit_low < explicit_high);
        assert!(explicit_high < auto_old, "automatic stacks above every explicit layer");
        assert!(auto_old < auto_new, "newer automatic content stacks on top");
    }

    #[test]
    fn layer_key_same_band_ordered_by_sequence() {
        assert!(resolve_layer_key(Some(500), 1) < resolve_layer_key(Some(500), 2));
    }

    #[test]
    fn layer_key_clamps_out_of_range_layers() {
        assert_eq!(resolve_layer_key(Some(0), 7), resolve_layer_key(Some(1), 7));
        assert_eq!(resolve_layer_key(Some(5000), 7), resolve_layer_key(Some(1000), 7));
    }

    #[test]
    fn opacity_anim_snaps_and_animates() {
        let mut anim = OpacityAnim::resting(0.0);
        assert!(!anim.is_animating());
        anim.set(0.7);
        assert_eq!(anim.current, 0.7);
        anim.animate_to(0.0, 200);
        assert!(anim.is_animating());
        // Zero-duration animation completes on the first tick.
        anim.animate_to(1.0, 0);
        let (v, done) = anim.tick();
        assert_eq!(v, 1.0);
        assert!(done);
        assert!(!anim.is_animating());
    }

    #[test]
    fn opacity_anim_clamps_targets() {
        let mut anim = OpacityAnim::resting(0.5);
        anim.animate_to(7.0, 0);
        let (v, _) = anim.tick();
        assert_eq!(v, 1.0);
    }
}

