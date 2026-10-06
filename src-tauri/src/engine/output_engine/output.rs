//! One video output = one native window, its GL render thread, the overlay mpv
//! context (timer OSD / Text Cue / test patterns) and its own pool of video
//! slots.
//!
//! The engine used to be a singleton whose state lived in module statics.  An
//! [`Output`] gathers all of that per-window state so several outputs (a
//! façade projector, a return monitor…) can run side by side; the main output
//! is simply the first one.  Outputs are looked up through the registry below —
//! voice-based engine calls resolve a voice to its slot across **all** outputs,
//! so cues never need to know where they play — and an output the show drops is
//! destroyed (`lifecycle.rs`).

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, RwLock};
use std::thread::JoinHandle;

use super::slot::VideoSlot;
use super::types::{FadeAnimState, MpvCtx, OutputId, OutputTransform, VoiceId};
use super::window::NativeWindow;
use crate::engine::mpv_sys::MpvLib;

/// The mpv context that carries an output's overlay: timer OSD, Text Cue and
/// test patterns.  Video slots have their own contexts.
pub(super) struct OverlayMpv {
    pub lib: Arc<MpvLib>,
    pub ctx: Arc<MpvCtx>,
    /// Set by the teardown once the render context is freed and the event
    /// thread has ended: the core is then released with its last user.
    pub destroy_on_drop: AtomicBool,
}

impl OverlayMpv {
    pub(super) fn new(lib: Arc<MpvLib>, ctx: Arc<MpvCtx>) -> Self {
        Self { lib, ctx, destroy_on_drop: AtomicBool::new(false) }
    }
}

impl Drop for OverlayMpv {
    fn drop(&mut self) {
        if self.destroy_on_drop.load(Ordering::Acquire) {
            // SAFETY: the teardown set the flag only after the render thread
            // freed this core's render context and its event thread returned;
            // this is the last reference to the handle.
            unsafe { (self.lib.mpv_terminate_destroy)(self.ctx.0) };
        }
    }
}

/// Shared state of one output window.  Every field is safe to touch from any
/// thread; the render thread reads most of them every frame.
pub(crate) struct Output {
    pub(super) id: OutputId,
    /// Operator-facing name, for logs and banners.
    name: Mutex<String>,
    /// Wakes the render thread (mpv update callbacks, engine changes).
    pub(super) signal: Arc<(Mutex<bool>, Condvar)>,
    /// `true` while the window is user-visible.  The render loop must NOT
    /// commit frames before this is set: on Wayland a `wl_surface.commit()`
    /// with a buffer permanently maps the surface, so a frame emitted while the
    /// window is "hidden" would make it appear at startup.
    pub(super) visible: AtomicBool,
    /// Physical window size, written on resize / screen move, read by the
    /// render thread to resize its GL surface.
    pub(super) width: AtomicU32,
    pub(super) height: AtomicU32,
    /// A Text Cue overlay is active — the render loop must not skip frames when
    /// idle, as mpv signals nothing for OSD-only changes.
    pub(super) text_overlay_active: AtomicBool,
    /// The on-output timer (`osd-msg1`) shows text.
    pub(super) timer_osd_active: AtomicBool,
    /// A test pattern occupies the overlay context.
    pub(super) test_pattern_active: AtomicBool,
    /// The overlay context has the transparent lavfi dummy loaded — mpv needs a
    /// decoded surface to composite OSD at all (see `overlay.rs`).
    pub(super) overlay_has_dummy: AtomicBool,
    /// Inverse homography of the output warp, row-major.  `None` = identity.
    pub(super) warp: Mutex<Option<[f32; 9]>>,
    /// One-shot: warp parameters changed — redraw even without a new frame.
    pub(super) warp_dirty: AtomicBool,
    /// One-shot: the overlay went inactive — redraw once so its last image
    /// leaves the screen.
    pub(super) overlay_dirty: AtomicBool,
    /// Master fade quad: a blackout curtain (startup idle, panic).
    pub(super) fade: Mutex<FadeAnimState>,
    /// Projector alignment applied to this output.
    pub(super) transform: Mutex<OutputTransform>,
    /// Set once the overlay mpv context exists.
    pub(super) overlay: OnceLock<OverlayMpv>,
    /// This output's slot pool (grow-only; the render thread iterates it).
    pub(super) slots: RwLock<Vec<Arc<VideoSlot>>>,
    /// The native window (platform-specific).
    pub(super) window: NativeWindow,
    /// The output is being destroyed: the render thread frees its GL and mpv
    /// render resources and exits.
    pub(super) shutdown: AtomicBool,
    /// Tells the overlay context's event thread to end.
    pub(super) events_stop: Arc<AtomicBool>,
    /// The render and overlay-event threads, joined by the teardown.
    threads: Mutex<Vec<JoinHandle<()>>>,
}

impl Output {
    pub(super) fn new(id: OutputId, name: &str) -> Arc<Self> {
        Arc::new(Self {
            id,
            name: Mutex::new(name.to_owned()),
            signal: Arc::new((Mutex::new(false), Condvar::new())),
            visible: AtomicBool::new(false),
            width: AtomicU32::new(1920),
            height: AtomicU32::new(1080),
            text_overlay_active: AtomicBool::new(false),
            timer_osd_active: AtomicBool::new(false),
            test_pattern_active: AtomicBool::new(false),
            overlay_has_dummy: AtomicBool::new(false),
            warp: Mutex::new(None),
            warp_dirty: AtomicBool::new(false),
            overlay_dirty: AtomicBool::new(false),
            fade: Mutex::new(FadeAnimState::idle()),
            transform: Mutex::new(OutputTransform::default()),
            overlay: OnceLock::new(),
            slots: RwLock::new(Vec::new()),
            window: NativeWindow::new(),
            shutdown: AtomicBool::new(false),
            events_stop: Arc::new(AtomicBool::new(false)),
            threads: Mutex::new(Vec::new()),
        })
    }

    /// Keep a thread of this output, to join when it is destroyed.
    pub(super) fn adopt_thread(&self, handle: JoinHandle<()>) {
        if let Ok(mut threads) = self.threads.lock() {
            threads.push(handle);
        }
    }

    /// The output's threads, for the teardown to join.
    pub(super) fn take_threads(&self) -> Vec<JoinHandle<()>> {
        self.threads.lock().map(|mut t| std::mem::take(&mut *t)).unwrap_or_default()
    }

    pub(super) fn name(&self) -> String {
        self.name.lock().map(|n| n.clone()).unwrap_or_default()
    }

    /// Rename the output — its window title follows.
    pub(super) fn set_name(&self, name: &str) {
        let changed = self.name.lock().map(|mut n| {
            let changed = *n != name;
            *n = name.to_owned();
            changed
        });
        if changed.unwrap_or(false) {
            self.window.set_title(&super::window::window_title(name));
        }
    }

    /// Wake the render thread immediately.
    ///
    /// `tick_fade()` self-paces at 16 ms only while an animation is in
    /// progress.  When a Fade Cue drives alpha or opacity externally at 30 fps
    /// the loop would otherwise sleep up to 100 ms between redraws; calling
    /// this on each change keeps it smooth.
    pub(super) fn wake(&self) {
        wake_signal(&self.signal);
    }

    /// Store new physical window dimensions and wake the render thread so it
    /// resizes the GL surface (called from the window backend on resize).
    pub(super) fn set_surface_size(&self, width: u32, height: u32) {
        self.width.store(width.max(1), Ordering::Relaxed);
        self.height.store(height.max(1), Ordering::Relaxed);
        self.wake();
    }

    /// Force one redraw after an overlay deactivation (timer cleared, Text Cue
    /// ended, test pattern cleared).
    pub(super) fn mark_overlay_dirty(&self) {
        self.overlay_dirty.store(true, Ordering::Relaxed);
        self.wake();
    }

    /// `true` when the overlay context currently shows something and must be
    /// composited on top of the video layers.
    ///
    /// **Load-bearing for the compositor**: the overlay context's idle render
    /// is opaque black on some libmpv builds, so compositing it unconditionally
    /// blacks out every video layer below.
    pub(super) fn overlay_active(&self) -> bool {
        self.timer_osd_active.load(Ordering::Relaxed)
            || self.text_overlay_active.load(Ordering::Relaxed)
            || self.test_pattern_active.load(Ordering::Relaxed)
    }

    /// Install (or clear) the output warp and wake the render thread so the
    /// change shows immediately — even on a paused frame or a test pattern.
    pub(super) fn set_warp(&self, matrix: Option<[f32; 9]>) {
        if let Ok(mut w) = self.warp.lock() {
            *w = matrix;
        }
        self.warp_dirty.store(true, Ordering::Relaxed);
        self.wake();
    }

    /// Hard-cut the master fade quad to `alpha` with no animation.
    ///
    /// Sets current, target and start together and zeroes the duration, so
    /// `tick_fade()` holds this value instead of snapping back to a stale
    /// target.
    pub(super) fn set_overlay_alpha(&self, alpha: u8) {
        if let Ok(mut s) = self.fade.lock() {
            s.current_alpha = alpha;
            s.target_alpha = alpha;
            s.start_alpha = alpha;
            s.duration_ms = 0;
            s.start_time = std::time::Instant::now();
        }
        self.wake();
    }

    /// Current master fade alpha (0 = clear, 255 = black).
    pub(super) fn overlay_alpha(&self) -> u8 {
        self.fade.lock().map(|s| s.current_alpha).unwrap_or(0)
    }

    /// Advance the master fade by one render-thread frame.  Returns the
    /// current alpha and whether it is still moving.
    pub(super) fn tick_fade(&self) -> (u8, bool) {
        let Ok(mut state) = self.fade.lock() else { return (0, false) };
        if state.current_alpha == state.target_alpha {
            return (state.current_alpha, false);
        }
        let elapsed = state.start_time.elapsed().as_millis() as u32;
        let t = if state.duration_ms == 0 {
            1.0_f32
        } else {
            (elapsed as f32 / state.duration_ms as f32).min(1.0)
        };
        let start = state.start_alpha as f32;
        let end = state.target_alpha as f32;
        state.current_alpha = (start + (end - start) * t).round().clamp(0.0, 255.0) as u8;
        if t >= 1.0 {
            state.current_alpha = state.target_alpha;
        }
        (state.current_alpha, t < 1.0)
    }

    /// `true` while any slot of this output holds content.
    pub(super) fn has_content(&self) -> bool {
        self.slots_snapshot().iter().any(|s| {
            s.state.lock().map(|st| st.voice_id.is_some()).unwrap_or(false)
        })
    }

    /// Cheap clone of the slot list for one render pass.
    pub(super) fn slots_snapshot(&self) -> Vec<Arc<VideoSlot>> {
        self.slots.read().map(|v| v.clone()).unwrap_or_default()
    }

    // ── Window control (platform-specific backend) ───────────────────────────

    /// Make the window visible and let the render loop commit frames.
    pub(super) fn show(&self) {
        self.visible.store(true, Ordering::Relaxed);
        self.window.show();
        // On Wayland the surface is only mapped once a buffer arrives; without
        // this wake the window would not appear until the next mpv signal.
        self.wake();
    }

    pub(super) fn hide(&self) {
        self.visible.store(false, Ordering::Relaxed);
        self.window.hide();
    }

    pub(super) fn is_visible(&self) -> bool {
        self.visible.load(Ordering::Relaxed)
    }
}

/// Raise the "frame wanted" flag on a render signal and notify its thread.
pub(super) fn wake_signal(signal: &(Mutex<bool>, Condvar)) {
    if let Ok(mut ready) = signal.0.lock() {
        *ready = true;
        signal.1.notify_one();
    }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

static OUTPUTS: RwLock<Vec<Arc<Output>>> = RwLock::new(Vec::new());

/// Add a freshly created output to the registry.
pub(super) fn register(output: &Arc<Output>) {
    if let Ok(mut all) = OUTPUTS.write() {
        all.push(Arc::clone(output));
    }
}

/// Take an output out of the registry: voice lookups stop finding its slots.
pub(super) fn unregister(output: &Arc<Output>) {
    if let Ok(mut all) = OUTPUTS.write() {
        all.retain(|o| !Arc::ptr_eq(o, output));
    }
}

/// Snapshot of every live output.
pub(super) fn all_outputs() -> Vec<Arc<Output>> {
    OUTPUTS.read().map(|v| v.clone()).unwrap_or_default()
}

/// Every slot of every output.
pub(super) fn all_slots() -> Vec<Arc<VideoSlot>> {
    all_outputs().iter().flat_map(|o| o.slots_snapshot()).collect()
}

/// The slot currently owning `voice`, on whichever output it plays.
pub(super) fn slot_for_voice(voice: VoiceId) -> Option<Arc<VideoSlot>> {
    all_slots().into_iter().find(|s| {
        s.state.lock().map(|st| st.voice_id == Some(voice)).unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output() -> Arc<Output> {
        Output::new(uuid::Uuid::new_v4(), "test")
    }

    #[test]
    fn a_new_output_starts_hidden_and_black() {
        let o = output();
        assert!(!o.is_visible());
        assert_eq!(o.overlay_alpha(), 255, "idle curtain is closed until content lifts it");
        assert!(!o.overlay_active());
        assert!(!o.has_content());
    }

    #[test]
    fn overlay_is_active_when_any_of_timer_text_or_pattern_shows() {
        let o = output();
        for flag in [&o.timer_osd_active, &o.text_overlay_active, &o.test_pattern_active] {
            flag.store(true, Ordering::Relaxed);
            assert!(o.overlay_active());
            flag.store(false, Ordering::Relaxed);
            assert!(!o.overlay_active());
        }
    }

    #[test]
    fn set_overlay_alpha_holds_the_value_without_animating() {
        let o = output();
        o.set_overlay_alpha(0);
        assert_eq!(o.tick_fade(), (0, false));
        o.set_overlay_alpha(255);
        assert_eq!(o.tick_fade(), (255, false));
    }

    #[test]
    fn tick_fade_reaches_its_target_and_reports_completion() {
        let o = output();
        {
            let mut s = o.fade.lock().unwrap();
            s.start_alpha = 255;
            s.current_alpha = 255;
            s.target_alpha = 0;
            s.duration_ms = 0;
            s.start_time = std::time::Instant::now();
        }
        let (alpha, moving) = o.tick_fade();
        assert_eq!(alpha, 0);
        assert!(!moving, "a zero-length fade lands on the first tick");
    }

    #[test]
    fn surface_size_never_reaches_zero() {
        let o = output();
        o.set_surface_size(0, 0);
        assert_eq!(o.width.load(Ordering::Relaxed), 1);
        assert_eq!(o.height.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn warp_changes_flag_a_redraw() {
        let o = output();
        o.set_warp(Some([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]));
        assert!(o.warp_dirty.swap(false, Ordering::Relaxed));
        assert!(o.warp.lock().unwrap().is_some());
    }

    #[test]
    fn wake_raises_the_render_flag() {
        let o = output();
        o.wake();
        assert!(*o.signal.0.lock().unwrap());
    }

    #[test]
    fn registry_finds_outputs_but_not_unknown_voices() {
        let o = output();
        register(&o);
        assert!(all_outputs().iter().any(|x| Arc::ptr_eq(x, &o)));
        assert!(slot_for_voice(uuid::Uuid::new_v4()).is_none());
    }

    #[test]
    fn an_unregistered_output_is_no_longer_found() {
        let o = output();
        register(&o);
        unregister(&o);
        assert!(!all_outputs().iter().any(|x| Arc::ptr_eq(x, &o)));
    }

    #[test]
    fn the_teardown_gets_every_thread_once() {
        let o = output();
        o.adopt_thread(std::thread::spawn(|| {}));
        o.adopt_thread(std::thread::spawn(|| {}));
        let threads = o.take_threads();
        assert_eq!(threads.len(), 2);
        assert!(o.take_threads().is_empty());
        for t in threads {
            t.join().unwrap();
        }
    }
}
