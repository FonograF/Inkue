//! [`OutputEngine`] — unified output for both video and image cues.
//!
//! A native window hosts libmpv via the OpenGL Render API (`vo=libmpv`, see
//! `render.rs`); the dip-to-black fade is a GL quad drawn in the same surface.
//! The render loop and fade are identical on every OS — only native window
//! creation differs (winit on Windows/Linux, AppKit/objc2 on macOS, see
//! `macos_window.rs`).
//!
//! On every OS the floating cue timer is a Tauri WebView window (`float-timer`),
//! and the on-output timer is mpv's OSD (`osd-msg1`).

mod blend;
mod fade;
mod mpv_events;
mod render;
mod slot;
mod warp;
/// macOS-only: AppKit NSWindow creation + control for the GL output path.
#[cfg(target_os = "macos")]
mod macos_window;
mod types;

pub use blend::BlendMode;
pub use types::{
    ContentRequest, FitMode, LayerStyle, OutputStatus, OutputSurface, OutputTransform, ScreenInfo,
    SurfaceId, TestPattern, VideoGeometry, VoiceId,
};
use types::{compose_display_props, FadeAnimState, MpvCtx, OutputVoice, PendingVideoStart};

use std::collections::HashMap;
use std::ffi::{c_char, c_void, CString};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use crossbeam_channel::{Receiver, Sender};
use uuid::Uuid;

use crate::cue::types::{db_to_linear, FadeSpec};
use crate::engine::AudioEngine;

use super::mpv_sys::{
    MpvLib, MpvNode, MpvNodeList, MpvNodeUnion,
    MPV_FORMAT_INT64, MPV_FORMAT_NODE_MAP, MPV_FORMAT_NONE, MPV_FORMAT_STRING,
};

// ---------------------------------------------------------------------------
// Global mpv state (cross-platform)
// ---------------------------------------------------------------------------

pub(super) static FADE_STATE: OnceLock<Mutex<FadeAnimState>> = OnceLock::new();
pub(super) static OUTPUT_MPV_CTX: OnceLock<Arc<MpvCtx>> = OnceLock::new();
pub(super) static OUTPUT_MPV_LIB: OnceLock<Arc<MpvLib>> = OnceLock::new();
pub(super) static OUTPUT_STATUS_TX: OnceLock<Sender<OutputStatus>> = OnceLock::new();
pub(super) static OUTPUT_CURRENT_VOICE: OnceLock<Mutex<Option<Uuid>>> = OnceLock::new();
pub(super) static OUTPUT_CURRENT_FADE_OUT_MS: OnceLock<Mutex<u32>> = OnceLock::new();
/// Set when a video `loadfile` is issued paused; consumed by the first
/// `MPV_EVENT_PLAYBACK_RESTART` to reveal + unpause once frame 0 is ready.
pub(super) static OUTPUT_PENDING_VIDEO_START: OnceLock<Mutex<Option<PendingVideoStart>>> =
    OnceLock::new();
/// The AudioEngine voice carrying the current video's audio track, if any.
pub(super) static OUTPUT_CURRENT_AUDIO_VOICE: OnceLock<Mutex<Option<Uuid>>> = OnceLock::new();
/// Geometry whose crop is waiting for the source dimensions.  mpv's
/// `video-crop` is pixel-based, so a fractional per-cue crop can only be
/// resolved once `video-params/w|h` are known — the `VIDEO_RECONFIG` event
/// handler drains this.
pub(super) static PENDING_CROP: OnceLock<Mutex<Option<VideoGeometry>>> = OnceLock::new();
/// Global projector-alignment transform, composed on top of every cue's
/// geometry.  Single source of truth is the workspace display preferences;
/// this mirror is what the (workspace-agnostic) load path reads.
pub(super) static OUTPUT_TRANSFORM: OnceLock<Mutex<OutputTransform>> = OnceLock::new();
/// The cue geometry most recently pushed to mpv — re-composed against the new
/// transform when the operator edits the alignment in Preferences.
pub(super) static LAST_CUE_GEOMETRY: OnceLock<Mutex<VideoGeometry>> = OnceLock::new();
/// When `Some`, the timer refresh loop shows this text instead of live cue time.
pub(crate) static TIMER_PREVIEW: OnceLock<Mutex<Option<String>>> = OnceLock::new();
/// Deduplication cache for the floating timer text (avoids redundant Tauri events).
pub(super) static FLOAT_TIMER_TEXT: OnceLock<Mutex<String>> = OnceLock::new();
/// Font family mirrored from OSD settings → emitted to the float-timer window.
pub(super) static FLOAT_TIMER_FONT: OnceLock<Mutex<String>> = OnceLock::new();
/// `true` while the overlay context has the transparent lavfi dummy loaded.
///
/// mpv needs a decoded video surface to composite OSD/text at all — **idle**
/// renders ignore the OSD *and* clear the target to opaque black on some
/// libmpv builds (0.41-dev on Windows honours neither `background=none` nor
/// OSD in idle; measured 2026-07-11).  With a fully transparent RGBA source
/// loaded, mpv honours the source alpha and composites the OSD with correct
/// per-pixel alpha, so timer/text float over the video layers below.
static OVERLAY_HAS_DUMMY: AtomicBool = AtomicBool::new(false);
/// `true` while the on-output timer (`osd-msg1`) shows text.
static TIMER_OSD_ACTIVE: AtomicBool = AtomicBool::new(false);
/// `true` while a test pattern occupies the overlay context.
static TEST_PATTERN_ACTIVE: AtomicBool = AtomicBool::new(false);

/// `true` when the overlay context currently shows something (timer OSD, Text
/// Cue, test pattern) and must be composited on top of the video layers.
///
/// **Load-bearing for the compositor**: the overlay context's idle render is
/// opaque black on some libmpv builds, so compositing it unconditionally
/// blacks out every video layer below — it must only be composited while one
/// of these is actually active.
pub(super) fn overlay_active() -> bool {
    TIMER_OSD_ACTIVE.load(Ordering::Relaxed)
        || render::TEXT_OVERLAY_ACTIVE.load(Ordering::Relaxed)
        || TEST_PATTERN_ACTIVE.load(Ordering::Relaxed)
}

/// mpv `osd-overlay` ID reserved for the Text Cue surface.  Distinct from the
/// timer (which uses `osd-msg1`, a separate OSD channel).
const TEXT_OSD_OVERLAY_ID: i64 = 47;

/// Load the transparent lavfi dummy into the overlay context (idempotent).
///
/// Called whenever timer/text OSD content appears.  No-op while a test
/// pattern is showing — the pattern is the overlay surface then.
fn ensure_overlay_surface() {
    if TEST_PATTERN_ACTIVE.load(Ordering::Relaxed)
        || OVERLAY_HAS_DUMMY.swap(true, Ordering::Relaxed)
    {
        return;
    }
    if let (Some(lib), Some(ctx)) = (OUTPUT_MPV_LIB.get(), OUTPUT_MPV_CTX.get()) {
        unsafe {
            let cmd = cs("loadfile");
            // Tiny + fully transparent: `format=rgba` keeps the alpha plane,
            // 10 fps keeps the OSD recomposited without measurable cost.
            let url = cs("av://lavfi:color=c=black@0.0:s=64x64:r=10,format=rgba");
            let flags = cs("replace");
            let idx = cs("0");
            let opts = cs("audio=no,loop-file=inf");
            let args: [*const c_char; 6] = [
                cmd.as_ptr(), url.as_ptr(), flags.as_ptr(),
                idx.as_ptr(), opts.as_ptr(), std::ptr::null(),
            ];
            let ret = (lib.mpv_command)(ctx.0, args.as_ptr());
            if ret < 0 {
                log::warn!("[output] overlay dummy loadfile failed: {ret}");
            }
        }
    }
    render::wake();
}

/// Unload the overlay dummy once neither the timer nor a Text Cue needs it.
fn release_overlay_surface_if_idle() {
    if overlay_active() || !OVERLAY_HAS_DUMMY.swap(false, Ordering::Relaxed) {
        return;
    }
    if let (Some(lib), Some(ctx)) = (OUTPUT_MPV_LIB.get(), OUTPUT_MPV_CTX.get()) {
        unsafe {
            let stop = cs("stop");
            let args: [*const c_char; 2] = [stop.as_ptr(), std::ptr::null()];
            (lib.mpv_command)(ctx.0, args.as_ptr());
        }
    }
    render::wake();
}

// ---------------------------------------------------------------------------
// OutputEngine
// ---------------------------------------------------------------------------

/// Manages the output window + libmpv context for all video and image output.
pub struct OutputEngine {
    mpv_lib: Arc<MpvLib>,
    mpv_ctx: Arc<MpvCtx>,
    current_voice: Arc<Mutex<Option<VoiceId>>>,
    voices: Mutex<HashMap<VoiceId, OutputVoice>>,
    #[allow(dead_code)]
    status_tx: Sender<OutputStatus>,
    status_rx: Mutex<Receiver<OutputStatus>>,
    default_surface_id: SurfaceId,
    audio_engine: Arc<AudioEngine>,
    go_sent_at: Arc<Mutex<Option<Instant>>>,
    /// Whether the output window is currently user-visible.
    visible: Arc<AtomicBool>,
    /// Tauri app handle — used to show/hide and emit events to the float-timer window.
    app_handle: tauri::AppHandle,
}

impl OutputEngine {
    /// Construct the engine.
    ///
    /// Creates the native GL output window (hidden) and blocks until mpv's
    /// render context is live.
    pub fn new(audio_engine: Arc<AudioEngine>, app_handle: tauri::AppHandle) -> Result<Self> {
        let lib = Arc::new(MpvLib::load()?);

        // mpv requires LC_NUMERIC=C; set it before mpv_create() on non-Windows.
        #[cfg(not(target_os = "windows"))]
        unsafe {
            libc::setlocale(libc::LC_NUMERIC, c"C".as_ptr());
        }

        let ctx = unsafe { (lib.mpv_create)() };
        if ctx.is_null() {
            return Err(anyhow!("mpv_create() returned null"));
        }

        unsafe {
            // mpv renders into our own native window via mpv_render_context_render()
            // instead of creating its own window.
            {
                opt_str(&lib, ctx, "vo", "libmpv");
                // This context is the layer compositor's OVERLAY (timer OSD,
                // Text Cue, test patterns): idle frames must be transparent
                // so it never masks the video slots below.  `background=none`
                // is mpv ≥ 0.38; `alpha=yes` covers older libmpv.
                opt_str(&lib, ctx, "background", "none");
                opt_str(&lib, ctx, "alpha", "yes");
            }

            opt_str(&lib, ctx, "hwdec", "auto");

            opt_str(&lib, ctx, "osc", "no");
            opt_str(&lib, ctx, "osd-level", "1");
            opt_str(&lib, ctx, "input-default-bindings", "no");
            opt_str(&lib, ctx, "input-vo-keyboard", "no");

            // Under vo=libmpv mpv has no window of its own on any OS — our host window
            // (winit on Windows/Linux, AppKit NSWindow on macOS) owns all mouse input
            // (dragging, double-click fullscreen). mpv's cursor handling would only get
            // in the way, so it stays off everywhere.
            opt_str(&lib, ctx, "input-cursor", "no");

            opt_str(&lib, ctx, "keep-open", "no");
            opt_str(&lib, ctx, "idle", "yes");

            // mpv plays VIDEO ONLY.  Each video's audio track is decoded separately
            // as a normal AudioEngine voice (Output Patch routing, VU, fades).
            opt_str(&lib, ctx, "ao", "null");
            opt_str(&lib, ctx, "audio", "no");
            opt_str(&lib, ctx, "video-sync", "desync");

            let v = cs("v");
            (lib.mpv_request_log_messages)(ctx, v.as_ptr());

            let ret = (lib.mpv_initialize)(ctx);
            if ret < 0 {
                (lib.mpv_terminate_destroy)(ctx);
                return Err(anyhow!("mpv_initialize() failed with code {ret}"));
            }

            // OSD style for the cue timer overlay (applied after init as properties).
            prop_str(&lib, ctx, "osd-font-size",     "120");
            prop_str(&lib, ctx, "osd-color",         "#FFFFFF");
            prop_str(&lib, ctx, "osd-border-color",  "#000000");
            prop_str(&lib, ctx, "osd-border-size",   "3");
            prop_str(&lib, ctx, "osd-align-x",       "center");
            prop_str(&lib, ctx, "osd-align-y",       "center");
            prop_str(&lib, ctx, "osd-margin-x",      "0");
            prop_str(&lib, ctx, "osd-margin-y",      "0");
        }

        let (status_tx, status_rx) = crossbeam_channel::unbounded();
        let current_voice: Arc<Mutex<Option<VoiceId>>> = Arc::new(Mutex::new(None));
        let mpv_ctx = Arc::new(MpvCtx(ctx));
        let go_sent_at: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));

        OUTPUT_MPV_CTX.get_or_init(|| Arc::clone(&mpv_ctx));
        OUTPUT_MPV_LIB.get_or_init(|| Arc::clone(&lib));
        OUTPUT_STATUS_TX.get_or_init(|| status_tx.clone());
        OUTPUT_CURRENT_VOICE.get_or_init(|| Mutex::new(None));
        OUTPUT_CURRENT_FADE_OUT_MS.get_or_init(|| Mutex::new(0));
        OUTPUT_PENDING_VIDEO_START.get_or_init(|| Mutex::new(None));
        OUTPUT_CURRENT_AUDIO_VOICE.get_or_init(|| Mutex::new(None));
        PENDING_CROP.get_or_init(|| Mutex::new(None));
        OUTPUT_TRANSFORM.get_or_init(|| Mutex::new(OutputTransform::default()));
        LAST_CUE_GEOMETRY.get_or_init(|| Mutex::new(VideoGeometry::default()));
        FADE_STATE.get_or_init(|| Mutex::new(FadeAnimState::idle()));
        TIMER_PREVIEW.get_or_init(|| Mutex::new(None));
        FLOAT_TIMER_TEXT.get_or_init(|| Mutex::new(String::new()));
        // Empty sentinel (never a real font name) so the first set_timer_style()
        // call always emits float-timer-font, regardless of what the persisted
        // preference happens to be — the float-timer window's own React state
        // has no other way to learn the current font.
        FLOAT_TIMER_FONT.get_or_init(|| Mutex::new(String::new()));

        // Create the winit/GL output window and block until mpv's render
        // context is live, so no `loadfile` can race ahead of it.
        render::init(&app_handle, Arc::clone(&lib), Arc::clone(&mpv_ctx))?;

        {
            let lib2   = Arc::clone(&lib);
            let ctx2   = Arc::clone(&mpv_ctx);
            let voice2 = Arc::clone(&current_voice);
            let tx2    = status_tx.clone();
            let gsa2   = Arc::clone(&go_sent_at);
            let ae     = Arc::clone(&audio_engine);
            std::thread::Builder::new()
                .name("inkue-output-mpv-events".into())
                .spawn(move || {
                    mpv_events::mpv_event_loop(lib2, ctx2, voice2, tx2, gsa2, ae)
                })
                .map_err(|e| anyhow!("Failed to spawn mpv event thread: {e}"))?;
        }

        Ok(Self {
            mpv_lib: lib,
            mpv_ctx,
            current_voice,
            voices: Mutex::new(HashMap::new()),
            status_tx,
            status_rx: Mutex::new(status_rx),
            default_surface_id: Uuid::new_v4(),
            audio_engine,
            go_sent_at,
            visible: Arc::new(AtomicBool::new(false)),
            app_handle,
        })
    }

    /// Expose the loaded `MpvLib` so callers can use it for probing.
    pub fn mpv_lib(&self) -> &MpvLib {
        &self.mpv_lib
    }

    /// Owned handle to the loaded `MpvLib` for background work
    /// (e.g. thumbnail generation on a blocking task).
    pub fn mpv_lib_arc(&self) -> Arc<MpvLib> {
        Arc::clone(&self.mpv_lib)
    }

    /// Probe the duration of a video file without displaying it.
    pub fn probe_duration(lib: &MpvLib, path: &Path) -> Option<Duration> {
        unsafe {
            let ctx = (lib.mpv_create)();
            if ctx.is_null() {
                return None;
            }

            opt_str(lib, ctx, "vo", "null");
            opt_str(lib, ctx, "ao", "null");
            opt_str(lib, ctx, "pause", "yes");
            opt_str(lib, ctx, "hwdec", "no");

            if (lib.mpv_initialize)(ctx) < 0 {
                (lib.mpv_terminate_destroy)(ctx);
                return None;
            }

            let path_str = path.to_string_lossy().replace('\\', "/");
            let path_cstr = match CString::new(path_str.as_str()) {
                Ok(c) => c,
                Err(_) => {
                    (lib.mpv_terminate_destroy)(ctx);
                    return None;
                }
            };
            let cmd_cstr     = cs("loadfile");
            let replace_cstr = cs("replace");
            let args: [*const std::ffi::c_char; 4] = [
                cmd_cstr.as_ptr(),
                path_cstr.as_ptr(),
                replace_cstr.as_ptr(),
                std::ptr::null(),
            ];
            (lib.mpv_command)(ctx, args.as_ptr());

            use super::mpv_sys::{MPV_EVENT_FILE_LOADED, MPV_EVENT_SHUTDOWN, MPV_FORMAT_DOUBLE};
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut duration_secs: Option<f64> = None;
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                let timeout = remaining.as_secs_f64().max(0.01);
                let event = (lib.mpv_wait_event)(ctx, timeout);
                if event.is_null() { break; }
                let event_id = (*event).event_id;
                if event_id == MPV_EVENT_FILE_LOADED {
                    let mut val: f64 = 0.0;
                    let name = cs("duration");
                    let ret = (lib.mpv_get_property)(
                        ctx, name.as_ptr(), MPV_FORMAT_DOUBLE,
                        &mut val as *mut f64 as *mut c_void,
                    );
                    if ret == 0 && val > 0.0 {
                        duration_secs = Some(val);
                    }
                    break;
                }
                if event_id == MPV_EVENT_SHUTDOWN { break; }
                if Instant::now() >= deadline { break; }
            }

            (lib.mpv_terminate_destroy)(ctx);
            duration_secs.map(|s| Duration::from_millis((s * 1000.0) as u64))
        }
    }

    /// Enumerate all connected monitors.  Index 0 is always the primary.
    pub fn list_screens(&self) -> Vec<ScreenInfo> {
        #[cfg(target_os = "windows")]
        {
            let mut screens: Vec<ScreenInfo> = Vec::new();
            unsafe {
                use windows_sys::Win32::Graphics::Gdi::{
                    EnumDisplayMonitors, GetMonitorInfoW, MONITORINFO,
                };
                extern "system" fn cb(
                    hmon: windows_sys::Win32::Graphics::Gdi::HMONITOR,
                    _hdc: windows_sys::Win32::Graphics::Gdi::HDC,
                    _rect: *mut windows_sys::Win32::Foundation::RECT,
                    data: windows_sys::Win32::Foundation::LPARAM,
                ) -> windows_sys::Win32::Foundation::BOOL {
                    unsafe {
                        let list = &mut *(data as *mut Vec<ScreenInfo>);
                        let mut mi: MONITORINFO = std::mem::zeroed();
                        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
                        if GetMonitorInfoW(hmon, &mut mi) != 0 {
                            let r = mi.rcMonitor;
                            let is_primary = (mi.dwFlags & 1) != 0;
                            list.push(ScreenInfo {
                                index: list.len() as u32,
                                width: (r.right - r.left) as u32,
                                height: (r.bottom - r.top) as u32,
                                x: r.left,
                                y: r.top,
                                is_primary,
                            });
                        }
                        1
                    }
                }
                EnumDisplayMonitors(
                    0,
                    std::ptr::null(),
                    Some(cb),
                    &mut screens as *mut Vec<ScreenInfo> as isize,
                );
            }
            screens.sort_by(|a, b| b.is_primary.cmp(&a.is_primary).then(a.x.cmp(&b.x)));
            for (i, s) in screens.iter_mut().enumerate() {
                s.index = i as u32;
            }
            screens
        }

        #[cfg(not(target_os = "windows"))]
        {
            use tauri::Manager;
            // Enumerate via the main Tauri window — available on the calling thread.
            let win = self.app_handle.get_webview_window("main");
            let Some(win) = win else { return Vec::new(); };

            let all = win.available_monitors().unwrap_or_default();
            let primary_pos = win.primary_monitor().ok().flatten().map(|p| *p.position());

            let mut screens: Vec<ScreenInfo> = all
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    let pos = m.position();
                    let sz  = m.size();
                    let is_primary = primary_pos
                        .map(|pp| pp.x == pos.x && pp.y == pos.y)
                        .unwrap_or(i == 0);
                    ScreenInfo {
                        index: i as u32,
                        width: sz.width,
                        height: sz.height,
                        x: pos.x,
                        y: pos.y,
                        is_primary,
                    }
                })
                .collect();
            screens.sort_by(|a, b| b.is_primary.cmp(&a.is_primary).then(a.x.cmp(&b.x)));
            for (i, s) in screens.iter_mut().enumerate() {
                s.index = i as u32;
            }
            screens
        }
    }

    /// The ID of the default "Screen 1" surface.
    pub fn default_surface_id(&self) -> SurfaceId {
        self.default_surface_id
    }

    /// Snapshot of all registered output surfaces.
    pub fn surfaces(&self) -> Vec<OutputSurface> {
        vec![OutputSurface {
            id: self.default_surface_id,
            name: "Screen 1".into(),
            label: String::new(),
        }]
    }

    // ── Unified content display ──────────────────────────────────────────────

    /// Display content (video, image or live feed) on the output window.
    ///
    /// The content gets its own video slot and is **composited** with
    /// whatever else is on stage (layer / opacity / blend from
    /// `req.layer_style`) — nothing is replaced; stopping other cues is the
    /// transport's policy, not the engine's.
    pub fn show_content(&self, req: ContentRequest<'_>) -> Result<VoiceId> {
        let voice_id = Uuid::new_v4();

        self.voices.lock().unwrap().insert(
            voice_id,
            OutputVoice { id: voice_id, started_at: Instant::now(), duration: None },
        );

        self.position_window(req.screen_index);
        // The master fade quad is only a blackout curtain now (startup idle,
        // panic, cleared test pattern) — any content GO lifts it; per-slot
        // opacity handles the actual reveal fade.
        fade::set_overlay_alpha(0);

        let slot = slot::acquire_slot(&self.mpv_lib, &self.audio_engine)?;
        slot::load_into_slot(&slot, slot::SlotLoad {
            voice_id,
            audio_voice_id: req.audio_voice_id,
            url: req.file_path.to_string_lossy().replace('\\', "/"),
            is_image: req.is_image,
            fade_in_ms: req.fade_in_ms,
            loop_count: req.loop_count,
            start_ms: req.start_ms,
            end_ms: req.end_ms,
            display_duration_ms: req.display_duration_ms,
            hold_last_frame: req.hold_last_frame,
            live_source: req.live_source,
            geometry: req.geometry,
            layer_style: req.layer_style,
            slices: req.slices,
        });

        Ok(voice_id)
    }

    /// Devamp: release the visual voice's current slice loop (see
    /// [`slot::devamp_slot`]) — the pass in progress finishes, then playback
    /// continues into the next slice, or stops at the boundary when
    /// `stop_at_end` is set.  No-op for unsliced content.
    pub fn devamp_voice(&self, voice_id: VoiceId, stop_at_end: bool) {
        if let Some(slot) = slot::slot_for_voice(voice_id) {
            slot::devamp_slot(&slot, stop_at_end);
        }
    }

    /// Current playback position of a visual voice in **file time** (ms) —
    /// mpv's `time-pos`, which reflects ab-loop jumps.  `None` when the voice
    /// is not on a slot (or mpv has no position yet).
    pub fn voice_position_ms(&self, voice_id: VoiceId) -> Option<u64> {
        let slot = slot::slot_for_voice(voice_id)?;
        slot::position_ms(&slot)
    }

    /// Stop the content identified by `voice_id`: fade its layer's opacity to
    /// zero over `visual_fade_ms` (then unload the slot) and fade its audio
    /// voice out over `audio_fade_ms`.  Other layers are untouched.
    pub fn stop_content(&self, voice_id: VoiceId, visual_fade_ms: u32, audio_fade_ms: u32) {
        self.voices.lock().unwrap().remove(&voice_id);
        let Some(slot) = slot::slot_for_voice(voice_id) else { return };

        // Take the audio voice out of the slot (the engine owns its fade-out;
        // the slot must not hard-cut it again at unload).
        let audio_id = slot.state.lock().ok().and_then(|st| st.audio_voice_id);
        slot::begin_stop(&slot, visual_fade_ms);
        if let Some(aid) = audio_id {
            let _ = self.audio_engine.stop_voice(
                aid,
                audio_fade_ms,
                crate::engine::ring_command::FadeCurve::SCurve,
            );
        }
    }

    /// Hard-stop all content immediately (no fade).
    pub fn hard_stop_current(&self) {
        self.voices.lock().unwrap().clear();
        slot::panic_all();
    }

    /// Panic: unconditionally cut whatever the output surface is doing.
    ///
    /// Unlike [`Self::stop_content`] / [`Self::hard_stop_current`], this does
    /// not consult the voice bookkeeping at all — it always issues `mpv stop`
    /// and paints the black quad, so it silences the surface even when a cue
    /// lost track of its voice (double-Escape backstop).
    pub fn panic_stop(&self) {
        *self.current_voice.lock().unwrap() = None;
        if let Some(cv) = OUTPUT_CURRENT_VOICE.get() {
            *cv.lock().unwrap() = None;
        }
        self.voices.lock().unwrap().clear();

        // Silence every video slot without needing voice ids.
        slot::panic_all();

        unsafe {
            let stop = cs("stop");
            let args: [*const std::ffi::c_char; 2] = [stop.as_ptr(), std::ptr::null()];
            (self.mpv_lib.mpv_command)(self.mpv_ctx.0, args.as_ptr());
        }
        fade::set_overlay_alpha(255);

        if let Some(m) = OUTPUT_CURRENT_FADE_OUT_MS.get() {
            *m.lock().unwrap() = 0;
        }
        if let Some(m) = OUTPUT_PENDING_VIDEO_START.get() {
            *m.lock().unwrap() = None;
        }
        *self.go_sent_at.lock().unwrap() = None;

        if let Some(av) = OUTPUT_CURRENT_AUDIO_VOICE.get() {
            let audio_id = av.lock().unwrap().take();
            if let Some(aid) = audio_id {
                let _ = self.audio_engine.stop_voice(
                    aid,
                    0,
                    crate::engine::ring_command::FadeCurve::Linear,
                );
            }
        }
    }

    /// `true` when `voice_id` is content currently on the output window.
    pub fn is_current_voice(&self, voice_id: VoiceId) -> bool {
        slot::slot_for_voice(voice_id).is_some()
    }

    /// Apply per-cue visual geometry to a voice's content, live.
    ///
    /// Called from `update_cue` when the operator edits the Geometry tab of a
    /// cue that is on screen; the load path applies geometry itself.
    pub fn apply_geometry(&self, voice_id: VoiceId, geometry: &VideoGeometry) {
        let Some(slot) = slot::slot_for_voice(voice_id) else { return };
        apply_scalar_geometry(&slot.lib, slot.mpv_ctx.0, geometry);
        let applied = try_apply_crop(&slot.lib, slot.mpv_ctx.0, geometry);
        if let Ok(mut st) = slot.state.lock() {
            st.geometry = *geometry;
            st.crop_applied = applied || !geometry.has_crop();
        }
        render::wake();
    }

    /// Live-apply a cue's compositing properties (layer / opacity / blend).
    pub fn set_layer_props(&self, voice_id: VoiceId, style: &LayerStyle) {
        if let Some(slot) = slot::slot_for_voice(voice_id) {
            slot::set_layer_style(&slot, style);
        }
    }

    /// Current animated opacity (0.0–1.0) of a voice's layer.
    pub fn get_voice_opacity(&self, voice_id: VoiceId) -> f32 {
        slot::slot_for_voice(voice_id).map(|s| slot::opacity_of(&s)).unwrap_or(0.0)
    }

    /// Directly drive a voice's layer opacity — Fade Cue tick at ~30 fps.
    pub fn set_voice_opacity(&self, voice_id: VoiceId, opacity: f32) {
        if let Some(slot) = slot::slot_for_voice(voice_id) {
            slot::set_opacity_direct(&slot, opacity);
        }
    }

    /// Begin the visual fade-out that lands exactly on a cue's natural end
    /// (EOF), so the content fades out instead of hard-cutting.
    ///
    /// Called from `VideoCue::tick` / `ImageCue::tick` once the remaining
    /// action time drops inside the configured fade-out window.  Returns
    /// `false` (and does nothing) when `voice_id` is no longer on screen.
    pub fn begin_eof_fade_out(&self, voice_id: VoiceId, fade_ms: u32) -> bool {
        let Some(slot) = slot::slot_for_voice(voice_id) else { return false };
        slot::animate_opacity(&slot, 0.0, fade_ms);
        true
    }

    /// Return the current overlay alpha (0 = transparent, 255 = black).
    pub fn get_overlay_alpha(&self) -> u8 {
        FADE_STATE.get()
            .and_then(|fs| fs.lock().ok())
            .map(|s| s.current_alpha)
            .unwrap_or(0)
    }

    /// Directly set the overlay alpha — called from FadeCue.tick() at ~30 fps.
    pub fn set_overlay_alpha_direct(&self, alpha: u8) {
        fade::set_overlay_alpha(alpha);
    }

    /// Return the AudioEngine voice carrying a video voice's audio track.
    pub fn video_audio_voice(&self, voice_id: VoiceId) -> Option<VoiceId> {
        let slot = slot::slot_for_voice(voice_id)?;
        let audio = slot.state.lock().ok().and_then(|st| st.audio_voice_id);
        audio
    }

    /// Current playback position of a voice's video (mpv `time-pos`), in ms.
    pub fn current_video_position_ms(&self, voice_id: VoiceId) -> Option<u64> {
        let slot = slot::slot_for_voice(voice_id)?;
        slot::position_ms(&slot)
    }

    /// Re-anchor the paired audio voice to the video's **actual** position
    /// (mpv `time-pos`), without moving mpv.  Corrects the A/V drift that builds
    /// up when the picture keeps advancing while the audio voice is frozen
    /// during an output-device outage.
    pub fn resync_audio_to_video(&self, voice_id: VoiceId) {
        if let (Some(ms), Some(av)) = (
            self.current_video_position_ms(voice_id),
            self.video_audio_voice(voice_id),
        ) {
            let _ = self.audio_engine.seek_voice_ms(av, ms);
        }
    }

    // ── Legacy API kept for VideoCue ─────────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    pub fn play_voice(
        &self,
        file_path: &Path,
        _surface_id: Option<SurfaceId>,
        _volume_db: f64,
        loop_count: u32,
        start_ms: Option<u64>,
        end_ms: Option<u64>,
        _fade_in: Option<&FadeSpec>,
        screen_index: Option<u32>,
    ) -> Result<VoiceId> {
        self.show_content(ContentRequest {
            file_path,
            is_image: false,
            fade_in_ms: 0,
            loop_count,
            start_ms,
            end_ms,
            screen_index,
            audio_voice_id: None,
            display_duration_ms: None,
            hold_last_frame: false,
            geometry: VideoGeometry::default(),
            live_source: false,
            layer_style: LayerStyle::default(),
            slices: Vec::new(),
        })
    }

    pub fn stop_voice(&self, voice_id: VoiceId, fade_ms: u32) -> Result<()> {
        self.stop_content(voice_id, fade_ms, fade_ms);
        Ok(())
    }

    pub fn stop_current_voice(&self, _fade_ms: u32) {
        self.hard_stop_current();
    }

    /// The mpv context owning `voice_id` (the voice's slot).
    fn voice_mpv_ctx(&self, voice_id: VoiceId) -> Option<*mut c_void> {
        slot::slot_for_voice(voice_id).map(|s| s.mpv_ctx.0)
    }

    pub fn pause_voice(&self, voice_id: VoiceId) -> Result<()> {
        if let Some(ctx) = self.voice_mpv_ctx(voice_id) {
            unsafe {
                (self.mpv_lib.mpv_set_property_string)(
                    ctx, cs("pause").as_ptr(), cs("yes").as_ptr(),
                );
            }
        }
        if let Some(aid) = self.video_audio_voice(voice_id) {
            let _ = self.audio_engine.pause_voice(aid);
        }
        Ok(())
    }

    pub fn resume_voice(&self, voice_id: VoiceId) -> Result<()> {
        if let Some(ctx) = self.voice_mpv_ctx(voice_id) {
            unsafe {
                (self.mpv_lib.mpv_set_property_string)(
                    ctx, cs("pause").as_ptr(), cs("no").as_ptr(),
                );
            }
        }
        if let Some(aid) = self.video_audio_voice(voice_id) {
            let _ = self.audio_engine.resume_voice(aid);
        }
        Ok(())
    }

    pub fn set_voice_volume(&self, voice_id: VoiceId, volume_db: f64) -> Result<()> {
        if let Some(aid) = self.video_audio_voice(voice_id) {
            let _ = self.audio_engine.set_voice_gain(aid, db_to_linear(volume_db) as f32);
        }
        Ok(())
    }

    /// Seek a voice's video (and re-anchor its paired audio voice).
    pub fn seek_voice_ms(&self, voice_id: VoiceId, position_ms: u64) {
        let Some(ctx) = self.voice_mpv_ctx(voice_id) else { return };
        let pos_str = format!("{:.3}", position_ms as f64 / 1000.0);
        let cmd_cstr = cs("seek");
        let pos_cstr = cs(&pos_str);
        let mode_cstr = cs("absolute");
        unsafe {
            let args = [
                cmd_cstr.as_ptr(),
                pos_cstr.as_ptr(),
                mode_cstr.as_ptr(),
                std::ptr::null(),
            ];
            (self.mpv_lib.mpv_command)(ctx, args.as_ptr());
        }
        if let Some(aid) = self.video_audio_voice(voice_id) {
            let _ = self.audio_engine.seek_voice_ms(aid, position_ms);
        }
    }

    // ── Window visibility ─────────────────────────────────────────────────────

    /// Toggle the output window visibility (F9 / View menu).
    pub fn toggle_visibility(&self) {
        if self.visible.load(Ordering::Relaxed) {
            self.hide_output();
        } else {
            self.show_output();
        }
    }

    /// Make the output window visible.
    pub fn show_output(&self) {
        self.visible.store(true, Ordering::Relaxed);
        // render.rs owns the native window. On Windows/Linux this calls winit
        // inline; on macOS it dispatches the AppKit show onto the main thread
        // (NSWindow methods are main-thread-only).
        render::show();
        use tauri::Emitter;
        let _ = self.app_handle.emit("output-window-visible", true);
    }

    /// Hide the output window.
    pub fn hide_output(&self) {
        self.visible.store(false, Ordering::Relaxed);
        render::hide();
        use tauri::Emitter;
        let _ = self.app_handle.emit("output-window-visible", false);
    }

    /// Return whether the output window is currently visible.
    pub fn is_visible(&self) -> bool {
        self.visible.load(Ordering::Relaxed)
    }

    // ── OSD / timer ──────────────────────────────────────────────────────────

    /// Update the countdown text shown on the output window timer (mpv OSD).
    ///
    /// Pass `None` (or an empty string) to hide the timer.
    pub fn set_output_timer(&self, text: Option<&str>) {
        let text = text.unwrap_or("");
        // The OSD only renders over a decoded surface (see OVERLAY_HAS_DUMMY),
        // and the overlay is only composited while flagged active.
        if text.is_empty() {
            TIMER_OSD_ACTIVE.store(false, Ordering::Relaxed);
        } else {
            TIMER_OSD_ACTIVE.store(true, Ordering::Relaxed);
            ensure_overlay_surface();
        }
        if let (Some(lib), Some(ctx)) = (OUTPUT_MPV_LIB.get(), OUTPUT_MPV_CTX.get()) {
            unsafe {
                prop_str(lib, ctx.0, "osd-msg1", text);
            }
        }
        if text.is_empty() {
            release_overlay_surface_if_idle();
            render::mark_overlay_dirty();
        }
    }

    /// Apply font, size, position and margin settings for the OSD timer overlay.
    pub fn set_timer_style(
        &self,
        font: &str,
        font_size: u32,
        position: crate::preferences::TimerPosition,
        margin: u32,
    ) {
        use crate::preferences::TimerPosition;
        let font_changed = FLOAT_TIMER_FONT.get().and_then(|m| m.lock().ok()).map(|mut g| {
            if *g != font { *g = font.to_owned(); true } else { false }
        }).unwrap_or(false);
        if font_changed {
            use tauri::Emitter;
            let _ = self.app_handle.emit("float-timer-font", font);
        }
        if let (Some(lib), Some(ctx)) = (OUTPUT_MPV_LIB.get(), OUTPUT_MPV_CTX.get()) {
            unsafe {
                prop_str(lib, ctx.0, "osd-font",      font);
                prop_str(lib, ctx.0, "osd-font-size", &font_size.to_string());
                let (align_x, align_y) = match position {
                    TimerPosition::Center      => ("center", "center"),
                    TimerPosition::TopLeft     => ("left",   "top"),
                    TimerPosition::TopRight    => ("right",  "top"),
                    TimerPosition::BottomLeft  => ("left",   "bottom"),
                    TimerPosition::BottomRight => ("right",  "bottom"),
                };
                let margin_str = match position {
                    TimerPosition::Center => "0".to_string(),
                    _                    => margin.to_string(),
                };
                prop_str(lib, ctx.0, "osd-align-x",  align_x);
                prop_str(lib, ctx.0, "osd-align-y",  align_y);
                prop_str(lib, ctx.0, "osd-margin-x", &margin_str);
                prop_str(lib, ctx.0, "osd-margin-y", &margin_str);
            }
        }
    }

    // ── Floating timer (Tauri WebView window) ─────────────────────────────────

    /// Show or hide the standalone floating timer window (Tauri WebView).
    ///
    /// GTK (Linux) and AppKit (macOS) require window show/hide on the main
    /// thread, but Tauri command handlers run on a worker thread.  Marshalling
    /// onto the main thread makes this safe on all three OS — the same
    /// cross-platform discipline the winit output window follows.
    pub fn set_floating_timer_visible(&self, visible: bool) {
        let app = self.app_handle.clone();
        let _ = self.app_handle.run_on_main_thread(move || {
            use tauri::Manager;
            if let Some(win) = app.get_webview_window("float-timer") {
                let _ = if visible { win.show() } else { win.hide() };
            }
        });
    }

    /// Write the current timer text to the floating window.
    /// Only emits a Tauri event when the text actually changed.
    pub fn update_floating_timer(&self, text: Option<&str>) {
        let new_text = text.unwrap_or("");
        let changed = FLOAT_TIMER_TEXT.get().and_then(|m| m.lock().ok()).map(|mut g| {
            if *g != new_text { *g = new_text.to_owned(); true } else { false }
        }).unwrap_or(false);
        if changed {
            use tauri::Emitter;
            let _ = self.app_handle.emit("float-timer-text", new_text);
        }
    }

    /// Set or clear the preview text shown on the OSD timer.
    pub fn set_timer_preview(&self, text: Option<String>) {
        if let Some(m) = TIMER_PREVIEW.get() {
            if let Ok(mut g) = m.lock() {
                *g = text;
            }
        }
    }

    /// Return the current preview text, if any.
    pub fn get_timer_preview(&self) -> Option<String> {
        TIMER_PREVIEW.get()?.lock().ok()?.clone()
    }

    // ── Text overlay (sub-text / ASS) ────────────────────────────────────────

    /// Display an ASS-tagged text string on the output surface.
    ///
    /// Uses mpv's `osd-overlay` command (`format=ass-events`), the API-supported
    /// way to draw client-supplied ASS: it honours full override tags (`\an`,
    /// `\fn`, `\fs`, `\c` …), is independent of `osd-level`, persists across file
    /// loads, and composites over whatever the VO shows.  (`sub-text` is read-only
    /// and `osd-msg2`/`osd-msg3` only render at `osd-level >= 2`, which is reserved
    /// for the cue timer on `osd-msg1`.)
    ///
    /// When nothing is playing, a black lavfi source is loaded so the OSD has a
    /// surface to composite onto and the output shows black rather than the desktop.
    pub fn show_text_overlay(&self, ass_text: &str, screen_index: Option<u32>) {
        self.position_window(screen_index);
        // Hint the render loop to keep compositing OSD-only changes.
        render::TEXT_OVERLAY_ACTIVE.store(true, Ordering::Relaxed);
        // The ASS overlay only renders over a decoded surface.
        ensure_overlay_surface();

        if let (Some(lib), Some(ctx)) = (OUTPUT_MPV_LIB.get(), OUTPUT_MPV_CTX.get()) {
            // osd-overlay persists across file loads, so it can be set right
            // away — the render loop composites it as soon as it renders.
            unsafe { osd_overlay_set(lib, ctx.0, ass_text); }
        }

        fade::set_overlay_alpha(0);
    }

    /// Clear the text set via [`show_text_overlay`].
    ///
    /// Restores the opaque-black idle state (alpha=255) when no video or image
    /// content is currently playing.
    pub fn clear_text_overlay(&self) {
        render::TEXT_OVERLAY_ACTIVE.store(false, Ordering::Relaxed);

        if let (Some(lib), Some(ctx)) = (OUTPUT_MPV_LIB.get(), OUTPUT_MPV_CTX.get()) {
            unsafe { osd_overlay_remove(lib, ctx.0); }
        }
        release_overlay_surface_if_idle();
        render::mark_overlay_dirty();

        // Blackout only when the whole stage is empty (no slot occupied).
        let has_content = !self.voices.lock().unwrap().is_empty();
        if !has_content {
            fade::set_overlay_alpha(255);
        }
    }

    /// Whether the output window is currently user-visible.
    pub fn is_output_visible(&self) -> bool {
        self.visible.load(Ordering::Relaxed)
    }

    /// Update the global projector-alignment transform and apply it
    /// immediately, so the operator sees the effect live while dragging in
    /// the alignment editor.  No-op when the transform is unchanged (the
    /// event loop re-asserts it every tick to stay in sync with the loaded
    /// workspace).
    ///
    /// The transform (incl. fractional rotation + corner pin) is a dedicated
    /// warp render pass — mpv properties are not touched.
    pub fn set_output_transform(&self, transform: OutputTransform) {
        let changed = OUTPUT_TRANSFORM
            .get()
            .and_then(|m| m.lock().ok())
            .map(|mut t| {
                if *t == transform {
                    false
                } else {
                    *t = transform;
                    true
                }
            })
            .unwrap_or(false);
        if !changed {
            return;
        }

        render::set_output_warp(warp::warp_matrix(&transform));
    }

    // ── Test patterns (projector calibration) ────────────────────────────────

    /// Show a calibration pattern (grid, colour bars, custom image, …) on the
    /// output window, replacing whatever is playing.
    ///
    /// The current content is hard-stopped first (its owning cue completes
    /// through the normal `OutputStatus::Completed` path), the window is
    /// positioned like a GO would (fallback + banner included), and the
    /// pattern is shown with **neutral cue geometry** — only the global
    /// [`OutputTransform`] applies, which is exactly what alignment and
    /// colorimetry need.
    pub fn show_test_pattern(&self, pattern: &TestPattern, screen_index: Option<u32>) {
        self.hard_stop_current();
        self.position_window(screen_index);

        // The pattern replaces whatever the overlay context held (incl. the
        // transparent OSD dummy) and makes the overlay composite opaque.
        TEST_PATTERN_ACTIVE.store(true, Ordering::Relaxed);
        OVERLAY_HAS_DUMMY.store(false, Ordering::Relaxed);

        // Pattern resolution: match the target screen so the grid is 1:1.
        let (w, h) = resolve_output_screen(&self.list_screens(), screen_index)
            .0
            .map(|s| (s.width, s.height))
            .unwrap_or((1920, 1080));
        let url = pattern.mpv_url(w, h);

        apply_geometry_props(&self.mpv_lib, self.mpv_ctx.0, &VideoGeometry::default());

        unsafe {
            // Patterns behave like images: play immediately, no paused-load
            // handshake, and no keep-open (a previous held video may have set it).
            (self.mpv_lib.mpv_set_property_string)(
                self.mpv_ctx.0, cs("pause").as_ptr(), cs("no").as_ptr(),
            );
            (self.mpv_lib.mpv_set_property_string)(
                self.mpv_ctx.0, cs("keep-open").as_ptr(), cs("no").as_ptr(),
            );
            if let Some(m) = OUTPUT_PENDING_VIDEO_START.get() {
                if let Ok(mut p) = m.lock() {
                    *p = None;
                }
            }

            let opts = if pattern.is_file() {
                // A custom image needs image-display-duration to hold on screen.
                cs("audio=no,image-display-duration=inf")
            } else {
                cs("audio=no")
            };
            let path_cstr = match CString::new(url.as_str()) {
                Ok(c) => c,
                Err(_) => {
                    log::warn!("[output] test pattern path contains NUL byte");
                    return;
                }
            };
            let cmd = cs("loadfile");
            let flags = cs("replace");
            let idx = cs("0");
            // loadfile signature: <url> <flags> <index> <options> (see fade.rs).
            let args: [*const std::ffi::c_char; 6] = [
                cmd.as_ptr(), path_cstr.as_ptr(), flags.as_ptr(),
                idx.as_ptr(), opts.as_ptr(), std::ptr::null(),
            ];
            let ret = (self.mpv_lib.mpv_command)(self.mpv_ctx.0, args.as_ptr());
            if ret < 0 {
                log::warn!("[output] test pattern loadfile failed: {ret} ({url})");
            }
        }

        fade::set_overlay_alpha(0);
    }

    /// Clear the test pattern: stop playback and return to opaque black.
    pub fn clear_test_pattern(&self) {
        unsafe {
            let stop = cs("stop");
            let args: [*const std::ffi::c_char; 2] = [stop.as_ptr(), std::ptr::null()];
            (self.mpv_lib.mpv_command)(self.mpv_ctx.0, args.as_ptr());
        }
        TEST_PATTERN_ACTIVE.store(false, Ordering::Relaxed);
        // Timer/Text OSD may still be live — give them their surface back.
        if overlay_active() {
            ensure_overlay_surface();
        }
        render::mark_overlay_dirty();
        fade::set_overlay_alpha(255);
    }

    /// Immediately apply the output-screen preference to the live window.
    ///
    /// `Some(idx)` shows the window fullscreen on that screen right away (same
    /// missing-screen fallback + health banner as a GO), so the operator sees
    /// the effect of the Preferences selection without waiting for the next
    /// visual cue.  `None` (floating) restores the windowed floating rect.
    pub fn apply_output_screen(&self, screen_index: Option<u32>) {
        match screen_index {
            Some(_) => self.position_window(screen_index),
            None => {
                crate::health::clear("output-screen");
                render::set_windowed_floating();
            }
        }
    }

    /// Apply the output-screen preference when a workspace is (re)loaded, so a
    /// configured screen goes live as a black fullscreen surface immediately —
    /// not only on the first visual GO.
    ///
    /// Unlike [`Self::apply_output_screen`], a configured-but-missing screen
    /// only raises the health banner and keeps the window hidden: falling back
    /// to fullscreen-on-primary here would black out the operator's main
    /// display the moment they open a show file without the projector attached.
    pub fn apply_output_screen_on_load(&self, screen_index: Option<u32>) {
        match screen_index {
            None => {
                crate::health::clear("output-screen");
                render::set_windowed_floating();
            }
            Some(idx) => {
                if self.list_screens().iter().any(|s| s.index == idx) {
                    self.position_window(screen_index);
                } else {
                    crate::health::set(crate::health::HealthAlert::new(
                        "output-screen",
                        crate::health::HealthLevel::Warning,
                        format!(
                            "Output screen {} is not connected — the output window stays \
                             hidden; visual cues will fall back to the primary display. \
                             Check Preferences → Display.",
                            idx + 1,
                        ),
                    ));
                }
            }
        }
    }

    // ── Fullscreen ────────────────────────────────────────────────────────────

    /// Toggle the output window between windowed and true fullscreen.
    pub fn toggle_fullscreen(&self) {
        render::toggle_fullscreen();
    }

    // ── Status / GC ──────────────────────────────────────────────────────────

    pub fn push_status(&self, _status: OutputStatus) {}

    /// Drain all pending status events.  Called by the 30 fps event loop.
    pub fn drain_status(&self) -> Vec<OutputStatus> {
        let rx = self.status_rx.lock().unwrap();
        let mut out = Vec::new();
        while let Ok(s) = rx.try_recv() {
            out.push(s);
        }
        out
    }

    /// Remove a completed voice.
    pub fn gc_voice(&self, voice_id: VoiceId) {
        self.voices.lock().unwrap().remove(&voice_id);
    }

    // ── Internal helpers ─────────────────────────────────────────────────────

    fn position_window(&self, screen_index: Option<u32>) {
        // Resolve the configured screen against what is actually connected.
        // A configured-but-missing screen falls back to the primary display
        // (never a silent no-op) and raises a health banner so the operator
        // knows before the show that "Screen 2" is not what they expect.
        let (target_screen, screen_missing) =
            resolve_output_screen(&self.list_screens(), screen_index);
        if screen_missing {
            crate::health::set(crate::health::HealthAlert::new(
                "output-screen",
                crate::health::HealthLevel::Warning,
                format!(
                    "Output screen {} is not connected — using the primary display instead. \
                     Check Preferences → Display.",
                    screen_index.map(|i| i + 1).unwrap_or(0),
                ),
            ));
        } else {
            crate::health::clear("output-screen");
        }

        self.visible.store(true, Ordering::Relaxed);
        if let Some(s) = &target_screen {
            // Windows/Linux: borderless-fullscreen the winit window on the
            // monitor matching the physical rect from list_screens(). macOS:
            // place the NSWindow onto NSScreen[idx] directly (AppKit's own
            // coordinate space — no rect conversion needed; our sorted list and
            // NSScreen both put the primary at index 0, so the fallback index
            // maps correctly too).
            #[cfg(not(target_os = "macos"))]
            render::set_fullscreen_on_rect(s.x, s.y, s.width, s.height);
            #[cfg(target_os = "macos")]
            render::position_on_screen(s.index);
        }
        render::show();
        use tauri::Emitter;
        let _ = self.app_handle.emit("output-window-visible", true);
    }
}

impl Drop for OutputEngine {
    fn drop(&mut self) {
        unsafe { (self.mpv_lib.mpv_terminate_destroy)(self.mpv_ctx.0) };
    }
}

// ---------------------------------------------------------------------------
// Private utility functions
// ---------------------------------------------------------------------------

pub(super) fn cs(s: &str) -> CString {
    CString::new(s).expect("cs(): interior NUL byte in literal")
}

/// Resolve the configured output screen index against the connected screens.
///
/// Returns `(target, missing)`:
/// - `target` — the screen to go fullscreen on (`None` = floating window);
///   when the configured index is absent, falls back to the primary display.
/// - `missing` — `true` when a screen was configured but is not connected.
pub(super) fn resolve_output_screen(
    screens: &[ScreenInfo],
    screen_index: Option<u32>,
) -> (Option<ScreenInfo>, bool) {
    match screen_index {
        None => (None, false),
        Some(idx) => match screens.iter().find(|s| s.index == idx) {
            Some(s) => (Some(s.clone()), false),
            None => (screens.first().cloned(), true),
        },
    }
}

/// Read an int64 mpv property, or `None` when unavailable.
pub(super) unsafe fn get_prop_i64(lib: &MpvLib, ctx: *mut c_void, name: &str) -> Option<i64> {
    let mut val: i64 = 0;
    let n = cs(name);
    let ret = (lib.mpv_get_property)(
        ctx,
        n.as_ptr(),
        MPV_FORMAT_INT64,
        &mut val as *mut i64 as *mut c_void,
    );
    (ret == 0).then_some(val)
}

/// Apply the pixel `video-crop` derived from `geometry` — possible only once
/// the source dimensions (`video-params/w|h`) are known.  Returns `false`
/// when they are not yet available (caller keeps the crop pending).
pub(super) fn try_apply_crop(lib: &MpvLib, ctx: *mut c_void, geometry: &VideoGeometry) -> bool {
    unsafe {
        let w = get_prop_i64(lib, ctx, "video-params/w").unwrap_or(0);
        let h = get_prop_i64(lib, ctx, "video-params/h").unwrap_or(0);
        if w <= 0 || h <= 0 {
            return false;
        }
        match geometry.crop_rect_px(w as u32, h as u32) {
            Some((cw, ch, cx, cy)) => {
                prop_str(lib, ctx, "video-crop", &format!("{cw}x{ch}+{cx}+{cy}"));
            }
            None => prop_str(lib, ctx, "video-crop", ""),
        }
        true
    }
}

/// Push a cue's [`VideoGeometry`] to mpv.
///
/// The cue geometry is applied **pure** — the global [`OutputTransform`]
/// lives in the warp render pass instead, so composing it here would
/// double-apply it.
///
/// The scalar properties (`keepaspect`, `panscan`, `video-zoom`,
/// `video-pan-x/y`, `video-rotate`) are global mpv properties that persist
/// across `loadfile`, so every load — and every live edit — sets **all** of
/// them (a cue without geometry resets the previous cue's values).  The crop
/// is pixel-based: applied immediately when the source dimensions are known,
/// otherwise parked in [`PENDING_CROP`] for the `VIDEO_RECONFIG` handler.
pub(super) fn apply_geometry_props(lib: &MpvLib, ctx: *mut c_void, geometry: &VideoGeometry) {
    // Remember the cue geometry most recently pushed to mpv.
    if let Some(last) = LAST_CUE_GEOMETRY.get() {
        if let Ok(mut g) = last.lock() {
            *g = *geometry;
        }
    }

    apply_scalar_geometry(lib, ctx, geometry);

    let pending_crop = if geometry.has_crop() {
        // Pixel crop needs the source dimensions; park it when unknown.
        if try_apply_crop(lib, ctx, geometry) { None } else { Some(*geometry) }
    } else {
        // No crop on this cue: clear any crop left by the previous cue.
        // (An empty string needs no dimensions.)
        unsafe { prop_str(lib, ctx, "video-crop", "") };
        None
    };
    if let Some(pending) = PENDING_CROP.get() {
        if let Ok(mut p) = pending.lock() {
            *p = pending_crop;
        }
    }
}

/// Push a geometry's scalar mpv properties (everything except the pixel
/// crop, which needs the source dimensions).  Per-context — used by both the
/// overlay context and each video slot.
///
/// The cue geometry is applied **pure** (the global OutputTransform lives in
/// the warp render pass).
pub(super) fn apply_scalar_geometry(lib: &MpvLib, ctx: *mut c_void, geometry: &VideoGeometry) {
    let props = compose_display_props(geometry, &OutputTransform::default());
    unsafe {
        let (keepaspect, panscan) = geometry.fit_props();
        prop_str(lib, ctx, "keepaspect", keepaspect);
        prop_str(lib, ctx, "panscan", panscan);
        prop_str(lib, ctx, "video-zoom", &format!("{:.6}", props.zoom_log2));
        prop_str(lib, ctx, "video-pan-x", &format!("{:.6}", props.pan_x));
        prop_str(lib, ctx, "video-pan-y", &format!("{:.6}", props.pan_y));
        prop_str(lib, ctx, "video-rotate", &props.rotation.to_string());
    }
}

pub(super) unsafe fn opt_str(lib: &MpvLib, ctx: *mut c_void, name: &str, value: &str) {
    let n = cs(name);
    let v = cs(value);
    (lib.mpv_set_option_string)(ctx, n.as_ptr(), v.as_ptr());
}

/// Set an mpv *property* (after `mpv_initialize`).
pub(super) unsafe fn prop_str(lib: &MpvLib, ctx: *mut c_void, name: &str, value: &str) {
    let n = cs(name);
    let v = cs(value);
    (lib.mpv_set_property_string)(ctx, n.as_ptr(), v.as_ptr());
}

/// Show the Text Cue ASS string via mpv's `osd-overlay` command.
///
/// `res_y=720` is the ASS script reference height, so `\fs` sizes stay
/// proportional to the output regardless of its actual resolution.
pub(super) unsafe fn osd_overlay_set(lib: &MpvLib, ctx: *mut c_void, ass_text: &str) {
    let Ok(data_v) = CString::new(ass_text) else {
        log::warn!("[output] osd-overlay text contains an interior NUL — ignored");
        return;
    };
    let (name, id, format, data_k, res_y) =
        (cs("name"), cs("id"), cs("format"), cs("data"), cs("res_y"));
    let (name_v, format_v) = (cs("osd-overlay"), cs("ass-events"));

    let mut keys: [*const c_char; 5] =
        [name.as_ptr(), id.as_ptr(), format.as_ptr(), data_k.as_ptr(), res_y.as_ptr()];
    let mut values: [MpvNode; 5] = [
        MpvNode { u: MpvNodeUnion { string: name_v.as_ptr() },    format: MPV_FORMAT_STRING },
        MpvNode { u: MpvNodeUnion { int64: TEXT_OSD_OVERLAY_ID }, format: MPV_FORMAT_INT64  },
        MpvNode { u: MpvNodeUnion { string: format_v.as_ptr() },  format: MPV_FORMAT_STRING },
        MpvNode { u: MpvNodeUnion { string: data_v.as_ptr() },    format: MPV_FORMAT_STRING },
        MpvNode { u: MpvNodeUnion { int64: 720 },                 format: MPV_FORMAT_INT64  },
    ];
    command_node_map(lib, ctx, &mut keys, &mut values);
}

/// Remove the Text Cue `osd-overlay` (`format=none`).
pub(super) unsafe fn osd_overlay_remove(lib: &MpvLib, ctx: *mut c_void) {
    let (name, id, format) = (cs("name"), cs("id"), cs("format"));
    let (name_v, format_v) = (cs("osd-overlay"), cs("none"));

    let mut keys: [*const c_char; 3] = [name.as_ptr(), id.as_ptr(), format.as_ptr()];
    let mut values: [MpvNode; 3] = [
        MpvNode { u: MpvNodeUnion { string: name_v.as_ptr() },    format: MPV_FORMAT_STRING },
        MpvNode { u: MpvNodeUnion { int64: TEXT_OSD_OVERLAY_ID }, format: MPV_FORMAT_INT64  },
        MpvNode { u: MpvNodeUnion { string: format_v.as_ptr() },  format: MPV_FORMAT_STRING },
    ];
    command_node_map(lib, ctx, &mut keys, &mut values);
}

/// Run `mpv_command_node` with a `MPV_FORMAT_NODE_MAP` built from parallel
/// `keys`/`values` slices, freeing any memory mpv allocates for the result.
unsafe fn command_node_map(
    lib: &MpvLib,
    ctx: *mut c_void,
    keys: &mut [*const c_char],
    values: &mut [MpvNode],
) {
    debug_assert_eq!(keys.len(), values.len());
    let mut list = MpvNodeList {
        num: keys.len() as i32,
        values: values.as_mut_ptr(),
        keys: keys.as_mut_ptr(),
    };
    let arg = MpvNode { u: MpvNodeUnion { list: &mut list }, format: MPV_FORMAT_NODE_MAP };
    let mut result = MpvNode { u: MpvNodeUnion { int64: 0 }, format: MPV_FORMAT_NONE };
    let ret = (lib.mpv_command_node)(ctx, &arg, &mut result);
    (lib.mpv_free_node_contents)(&mut result);
    if ret < 0 {
        log::warn!("[output] mpv_command_node(osd-overlay) failed: {ret}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screens(n: u32) -> Vec<ScreenInfo> {
        (0..n)
            .map(|i| ScreenInfo {
                index: i,
                width: 1920,
                height: 1080,
                x: (i as i32) * 1920,
                y: 0,
                is_primary: i == 0,
            })
            .collect()
    }

    #[test]
    fn resolve_screen_none_is_floating() {
        assert_eq!(resolve_output_screen(&screens(2), None), (None, false));
    }

    #[test]
    fn resolve_screen_found() {
        let (target, missing) = resolve_output_screen(&screens(2), Some(1));
        assert!(!missing);
        assert_eq!(target.unwrap().index, 1);
    }

    #[test]
    fn resolve_screen_missing_falls_back_to_primary() {
        let (target, missing) = resolve_output_screen(&screens(2), Some(4));
        assert!(missing);
        assert_eq!(target.unwrap().index, 0);
    }

    #[test]
    fn resolve_screen_missing_with_no_screens() {
        let (target, missing) = resolve_output_screen(&[], Some(1));
        assert!(missing);
        assert!(target.is_none());
    }
}
