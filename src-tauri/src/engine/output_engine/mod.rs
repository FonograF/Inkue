//! [`OutputEngine`] — unified Win32 + libmpv output for both video and image cues.
//!
//! A single persistent Win32 popup window (`WS_POPUP`) is created at startup
//! and stays visible for the lifetime of the application (black when idle).
//! libmpv renders both video files and image files into this window:
//! - Video: normal mpv `loadfile`
//! - Image: `loadfile image.jpg audio=no,image-display-duration=inf`
//!
//! # Fade overlay
//!
//! A child window with `WS_EX_LAYERED | WS_EX_TRANSPARENT` sits above mpv's
//! D3D11 render child.  Its alpha (0 = transparent, 255 = opaque black) is
//! animated via a 16 ms Win32 timer to produce dip-to-black transitions.
//!
//! # Freeze fix
//!
//! The window is created and shown at startup (not lazily on first GO).  mpv
//! is also initialised immediately.  Result: the first GO no longer blocks.

mod fade;
mod mpv_events;
mod types;
mod win32_window;

pub use types::{OutputStatus, OutputSurface, ScreenInfo, SurfaceId, VoiceId};
use types::{
    FadeAnimState, FadePending, FadePendingParams, MpvCtx, OutputVoice, OutputWndState,
    PendingVideoStart,
};

use std::collections::HashMap;
use std::ffi::{c_void, CString};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use crossbeam_channel::{Receiver, Sender};
use uuid::Uuid;

use crate::cue::types::{db_to_linear, FadeSpec};
use crate::engine::AudioEngine;

use super::mpv_sys::{MpvLib, MPV_FORMAT_INT64};

// ---------------------------------------------------------------------------
// Global Win32 / mpv state
// ---------------------------------------------------------------------------

pub(super) static OUTPUT_WND_STATE: OnceLock<Mutex<OutputWndState>> = OnceLock::new();
pub(super) static OUTPUT_PARENT_HWND: OnceLock<isize> = OnceLock::new();
pub(super) static FADE_OVERLAY_HWND: OnceLock<isize> = OnceLock::new();
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
/// Resumed at the first PLAYBACK_RESTART (start in lockstep with frame 0),
/// paused/resumed/stopped together with the video.
pub(super) static OUTPUT_CURRENT_AUDIO_VOICE: OnceLock<Mutex<Option<Uuid>>> = OnceLock::new();
/// Text currently shown on the timer overlay ("" = nothing displayed).
pub(super) static TIMER_TEXT: OnceLock<Mutex<String>> = OnceLock::new();
/// HWND of the timer text overlay child window.
pub(super) static TIMER_OVERLAY_HWND: OnceLock<isize> = OnceLock::new();
/// When `Some`, the timer refresh loop shows this text instead of live cue time.
/// Used for the preferences preview mode.
pub(crate) static TIMER_PREVIEW: OnceLock<Mutex<Option<String>>> = OnceLock::new();
/// Text currently shown in the floating timer window.
pub(super) static FLOAT_TIMER_TEXT: OnceLock<Mutex<String>> = OnceLock::new();
/// HWND of the standalone floating timer window (top-level, always-on-top).
pub(super) static FLOAT_TIMER_HWND: OnceLock<isize> = OnceLock::new();
/// Font family name for the floating timer (mirrors the OSD font setting).
pub(super) static FLOAT_TIMER_FONT: OnceLock<Mutex<String>> = OnceLock::new();

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// `WM_APP + 1`: posted by the mpv event thread after `MPV_EVENT_FILE_LOADED`.
pub(super) const WM_SETUP_MPV_CHILD: u32 = 0x8001;
/// `WM_APP + 2`: posted by show_content/stop_content to start the fade timer.
pub(super) const WM_DO_FADE: u32        = 0x8002;
/// `WM_APP + 4`: posted to the floating timer window to show/hide it.
pub(super) const WM_FLOAT_VISIBILITY: u32 = 0x8004;
pub(super) const FADE_TIMER_ID: usize = 1;

// ---------------------------------------------------------------------------
// OutputEngine
// ---------------------------------------------------------------------------

/// Manages the single native Win32 popup window + libmpv context for all
/// video and image output.  The window is always visible at startup (black
/// when idle) — no creation lag on first GO.
pub struct OutputEngine {
    mpv_lib: Arc<MpvLib>,
    mpv_ctx: Arc<MpvCtx>,
    hwnd: isize,
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
}

impl OutputEngine {
    /// Construct the engine.
    ///
    /// Creates the Win32 window (shown immediately), loads libmpv, and
    /// initialises the mpv context.
    pub fn new(audio_engine: Arc<AudioEngine>) -> Result<Self> {
        let lib = Arc::new(MpvLib::load()?);

        let hwnd = win32_window::create_output_window()?;

        let ctx = unsafe { (lib.mpv_create)() };
        if ctx.is_null() {
            return Err(anyhow!("mpv_create() returned null"));
        }

        unsafe {
            let wid_name = cs("wid");
            let mut wid_val: i64 = hwnd as i64;
            (lib.mpv_set_option)(
                ctx,
                wid_name.as_ptr(),
                MPV_FORMAT_INT64,
                &mut wid_val as *mut i64 as *mut c_void,
            );

            opt_str(&lib, ctx, "vo", "gpu");
            opt_str(&lib, ctx, "gpu-api", "d3d11");
            opt_str(&lib, ctx, "d3d11-sync-interval", "0"); // non-blocking Present(); video-sync=desync needs this
            opt_str(&lib, ctx, "force-window", "immediate");
            opt_str(&lib, ctx, "hwdec", "auto");

            opt_str(&lib, ctx, "osc", "no");
            // osd-level 1: shows explicit osd-msg1/2/3 messages but no automatic
            // position/seek feedback.  Required for the output-window timer overlay.
            opt_str(&lib, ctx, "osd-level", "1");
            opt_str(&lib, ctx, "input-default-bindings", "no");
            opt_str(&lib, ctx, "input-vo-keyboard", "no");
            opt_str(&lib, ctx, "input-cursor", "no");

            opt_str(&lib, ctx, "keep-open", "no");
            opt_str(&lib, ctx, "idle", "yes");

            // mpv plays VIDEO ONLY.  Each video's audio track is decoded
            // separately by the cue and played as a normal AudioEngine voice
            // (Output Patch routing, master volume, VU, fades).  Disabling mpv
            // audio entirely keeps its display clock free of the A/V-sync
            // breakage that piping mpv audio out (ao=pcm) used to cause.
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

            // OSD style for the cue timer overlay (set after init, as properties).
            // Large bold centered text with a dark border for contrast on any content.
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
        FADE_STATE.get_or_init(|| Mutex::new(FadeAnimState::idle()));
        TIMER_PREVIEW.get_or_init(|| Mutex::new(None));

        {
            let lib2  = Arc::clone(&lib);
            let ctx2  = Arc::clone(&mpv_ctx);
            let voice2 = Arc::clone(&current_voice);
            let tx2   = status_tx.clone();
            let gsa2  = Arc::clone(&go_sent_at);
            let ae    = Arc::clone(&audio_engine);
            std::thread::Builder::new()
                .name("wincue-output-mpv-events".into())
                .spawn(move || {
                    mpv_events::mpv_event_loop(lib2, ctx2, voice2, tx2, hwnd, gsa2, ae)
                })
                .map_err(|e| anyhow!("Failed to spawn mpv event thread: {e}"))?;
        }

        Ok(Self {
            mpv_lib: lib,
            mpv_ctx,
            hwnd,
            current_voice,
            voices: Mutex::new(HashMap::new()),
            status_tx,
            status_rx: Mutex::new(status_rx),
            default_surface_id: Uuid::new_v4(),
            audio_engine,
            go_sent_at,
            visible: Arc::new(AtomicBool::new(true)),
        })
    }

    /// Expose the loaded `MpvLib` so callers can use it for probing.
    pub fn mpv_lib(&self) -> &MpvLib {
        &self.mpv_lib
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
            let index_cstr   = cs("0");
            let args: [*const std::ffi::c_char; 5] = [
                cmd_cstr.as_ptr(),
                path_cstr.as_ptr(),
                replace_cstr.as_ptr(),
                index_cstr.as_ptr(),
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
    pub fn list_screens() -> Vec<ScreenInfo> {
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

    /// Display content (video or image) on the output window.
    ///
    /// If the current content has a stored `fade_out_ms > 0`, the transition
    /// is: fade-to-black → load new content → fade-from-black.
    /// Otherwise the new content loads immediately (with optional fade-from-black).
    #[allow(clippy::too_many_arguments)]
    pub fn show_content(
        &self,
        file_path: &Path,
        is_image: bool,
        fade_in_ms: u32,
        this_fade_out_ms: u32,
        loop_count: u32,
        start_ms: Option<u64>,
        end_ms: Option<u64>,
        screen_index: Option<u32>,
        audio_voice_id: Option<VoiceId>,
        display_duration_ms: Option<u64>,
    ) -> Result<VoiceId> {
        let voice_id = Uuid::new_v4();

        if let Some(old_id) = self.current_voice.lock().unwrap().take() {
            self.voices.lock().unwrap().remove(&old_id);
            let _ = self.status_tx.send(OutputStatus::Completed { voice_id: old_id });
        }

        *self.current_voice.lock().unwrap() = Some(voice_id);
        self.voices.lock().unwrap().insert(
            voice_id,
            OutputVoice { id: voice_id, started_at: Instant::now(), duration: None },
        );

        if let Some(cv) = OUTPUT_CURRENT_VOICE.get() {
            *cv.lock().unwrap() = Some(voice_id);
        }

        let current_fade_out_ms = OUTPUT_CURRENT_FADE_OUT_MS
            .get()
            .map(|m| *m.lock().unwrap())
            .unwrap_or(0);

        if let Some(m) = OUTPUT_CURRENT_FADE_OUT_MS.get() {
            *m.lock().unwrap() = this_fade_out_ms;
        }

        // Cross-stop the previous video's audio voice and install the new one
        // (None for an image, which silences any prior video's audio).  A
        // playing previous voice fades out over the dip duration; a paused,
        // never-revealed one hard-stops (handled inside the AudioEngine).
        if let Some(av) = OUTPUT_CURRENT_AUDIO_VOICE.get() {
            let previous = {
                let mut g = av.lock().unwrap();
                std::mem::replace(&mut *g, audio_voice_id)
            };
            if let Some(prev_id) = previous {
                let _ = self.audio_engine.stop_voice(
                    prev_id,
                    current_fade_out_ms,
                    crate::engine::ring_command::FadeCurve::SCurve,
                );
            }
        }

        // go_sent_at marks GO time and tags the next PLAYBACK_RESTART as a video
        // reveal (vs an idle/image restart).  The video is loaded *paused* (see
        // execute_load_params); the first PLAYBACK_RESTART reveals the overlay,
        // unpauses, and resumes the paired audio voice — all from frame 0.
        if !is_image {
            *self.go_sent_at.lock().unwrap() = Some(Instant::now());
        } else {
            *self.go_sent_at.lock().unwrap() = None;
        }

        let path_str = file_path.to_string_lossy().replace('\\', "/");

        self.position_window(screen_index);

        let params = FadePendingParams {
            path: path_str,
            is_image,
            voice_id,
            fade_in_ms,
            loop_count,
            start_ms,
            end_ms,
            display_duration_ms,
        };

        if current_fade_out_ms > 0 {
            if let Some(fs) = FADE_STATE.get() {
                let mut state = fs.lock().unwrap();
                state.start_alpha = state.current_alpha;
                state.target_alpha = 255;
                state.duration_ms = current_fade_out_ms;
                state.start_time = Instant::now();
                state.timer_active = false;
                state.pending = Some(FadePending::Load(params));
            }
            unsafe {
                use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;
                PostMessageW(self.hwnd, WM_DO_FADE, 0, 0);
            }
        } else {
            // Abort any in-progress stop fade so the timer cannot kill this new content
            // or keep the overlay dark over it.
            if let Some(fs) = FADE_STATE.get() {
                if let Ok(mut state) = fs.lock() {
                    if matches!(state.pending, Some(FadePending::Stop)) {
                        state.pending = None;
                        state.target_alpha = 0;
                        state.current_alpha = 0;
                        state.start_alpha = 0;
                        state.duration_ms = 0;
                    }
                }
            }
            if is_image {
                if fade_in_ms > 0 {
                    // Black out the overlay BEFORE loading the image so it stays
                    // hidden until the fade animation reveals it.  Without this the
                    // overlay is at alpha=0 (transparent from the previous operation)
                    // and the image would flash visible before the fade starts.
                    fade::set_overlay_alpha(255);
                }
                fade::execute_load_params(&params, &self.mpv_lib, self.mpv_ctx.0);
                if fade_in_ms > 0 {
                    // Images do not go through the gated PLAYBACK_RESTART reveal,
                    // so start the fade-from-black immediately.
                    if let Some(fs) = FADE_STATE.get() {
                        let mut state = fs.lock().unwrap();
                        state.start_alpha = 255;
                        state.current_alpha = 255;
                        state.target_alpha = 0;
                        state.duration_ms = fade_in_ms;
                        state.start_time = Instant::now();
                        state.timer_active = false;
                        state.pending = None;
                    }
                    unsafe {
                        use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;
                        PostMessageW(self.hwnd, WM_DO_FADE, 0, 0);
                    }
                } else {
                    fade::set_overlay_alpha(0);
                }
            } else {
                // Video: black out the overlay *before* loading so the
                // loadfile/d3d11 reconfigure flash is hidden.  The first
                // PLAYBACK_RESTART then reveals + unpauses once frame 0 is decoded
                // (applying fade_in_ms there if set), aligned with the first frame.
                fade::set_overlay_alpha(255);
                fade::execute_load_params(&params, &self.mpv_lib, self.mpv_ctx.0);
            }
        }

        Ok(voice_id)
    }

    /// Stop the content identified by `voice_id` with an optional fade-to-black.
    pub fn stop_content(&self, voice_id: VoiceId, fade_out_ms: u32) {
        // Only the *current* voice may touch the output.  A newer cue may have
        // already replaced this one (its `Completed` event has not been processed
        // yet); stopping it then would wrongly black out / mute the new content.
        let was_current = {
            let mut cv = self.current_voice.lock().unwrap();
            if *cv == Some(voice_id) {
                *cv = None;
                true
            } else {
                false
            }
        };
        if let Some(cv) = OUTPUT_CURRENT_VOICE.get() {
            let mut cv_lock = cv.lock().unwrap();
            if *cv_lock == Some(voice_id) {
                *cv_lock = None;
            }
        }
        self.voices.lock().unwrap().remove(&voice_id);

        if !was_current {
            return;
        }

        if fade_out_ms > 0 {
            if let Some(fs) = FADE_STATE.get() {
                let mut state = fs.lock().unwrap();
                state.start_alpha = state.current_alpha;
                state.target_alpha = 255;
                state.duration_ms = fade_out_ms;
                state.start_time = Instant::now();
                state.timer_active = false;
                state.pending = Some(FadePending::Stop);
            }
            unsafe {
                use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;
                PostMessageW(self.hwnd, WM_DO_FADE, 0, 0);
            }
        } else {
            unsafe {
                let stop = cs("stop");
                let args: [*const std::ffi::c_char; 2] =
                    [stop.as_ptr(), std::ptr::null()];
                (self.mpv_lib.mpv_command)(self.mpv_ctx.0, args.as_ptr());
            }
        }

        if let Some(m) = OUTPUT_CURRENT_FADE_OUT_MS.get() {
            *m.lock().unwrap() = 0;
        }

        // Cancel any in-flight paused-load reveal so a late PLAYBACK_RESTART
        // cannot unpause / reveal content that has just been stopped.
        if let Some(m) = OUTPUT_PENDING_VIDEO_START.get() {
            *m.lock().unwrap() = None;
        }
        *self.go_sent_at.lock().unwrap() = None;

        // Stop the paired audio voice with the same fade as the video dip.
        if let Some(av) = OUTPUT_CURRENT_AUDIO_VOICE.get() {
            let audio_id = av.lock().unwrap().take();
            if let Some(aid) = audio_id {
                let _ = self.audio_engine.stop_voice(
                    aid,
                    fade_out_ms,
                    crate::engine::ring_command::FadeCurve::SCurve,
                );
            }
        }
    }

    /// Hard-stop all content immediately (no fade).
    pub fn hard_stop_current(&self) {
        let voice_id = *self.current_voice.lock().unwrap();
        if let Some(vid) = voice_id {
            self.stop_content(vid, 0);
        }
    }

    // ── Legacy API kept for VideoCue ─────────────────────────────────────────

    /// Begin video playback.  Delegates to `show_content` with `is_image = false`.
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
        self.show_content(
            file_path, false,
            0, 0, loop_count, start_ms, end_ms, screen_index, None, None,
        )
    }

    /// Stop the given voice.
    pub fn stop_voice(&self, voice_id: VoiceId, fade_ms: u32) -> Result<()> {
        self.stop_content(voice_id, fade_ms);
        Ok(())
    }

    /// Stop the currently-playing voice, if any.
    pub fn stop_current_voice(&self, _fade_ms: u32) {
        self.hard_stop_current();
    }

    /// Pause the given voice — both the mpv video and the paired audio voice.
    pub fn pause_voice(&self, _voice_id: VoiceId) -> Result<()> {
        unsafe {
            (self.mpv_lib.mpv_set_property_string)(
                self.mpv_ctx.0, cs("pause").as_ptr(), cs("yes").as_ptr(),
            );
        }
        if let Some(av) = OUTPUT_CURRENT_AUDIO_VOICE.get() {
            if let Some(aid) = *av.lock().unwrap() {
                let _ = self.audio_engine.pause_voice(aid);
            }
        }
        Ok(())
    }

    /// Resume a paused voice — both the mpv video and the paired audio voice.
    pub fn resume_voice(&self, _voice_id: VoiceId) -> Result<()> {
        unsafe {
            (self.mpv_lib.mpv_set_property_string)(
                self.mpv_ctx.0, cs("pause").as_ptr(), cs("no").as_ptr(),
            );
        }
        if let Some(av) = OUTPUT_CURRENT_AUDIO_VOICE.get() {
            if let Some(aid) = *av.lock().unwrap() {
                let _ = self.audio_engine.resume_voice(aid);
            }
        }
        Ok(())
    }

    /// Update the playback volume of a running voice (the paired audio voice).
    pub fn set_voice_volume(&self, _voice_id: VoiceId, volume_db: f64) -> Result<()> {
        if let Some(av) = OUTPUT_CURRENT_AUDIO_VOICE.get() {
            if let Some(aid) = *av.lock().unwrap() {
                let _ = self.audio_engine.set_voice_gain(aid, db_to_linear(volume_db) as f32);
            }
        }
        Ok(())
    }

    /// Seek the current video to `position_ms` and re-anchor the paired audio voice.
    pub fn seek(&self, position_ms: u64) {
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
            (self.mpv_lib.mpv_command)(self.mpv_ctx.0, args.as_ptr());
        }
        if let Some(av) = OUTPUT_CURRENT_AUDIO_VOICE.get() {
            if let Some(aid) = *av.lock().unwrap() {
                let _ = self.audio_engine.seek_voice_ms(aid, position_ms);
            }
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
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                SetWindowPos, ShowWindow, HWND_TOPMOST, SW_SHOWNA,
                SWP_NOMOVE, SWP_NOSIZE, SWP_NOACTIVATE,
            };
            ShowWindow(self.hwnd, SW_SHOWNA);
            SetWindowPos(
                self.hwnd, HWND_TOPMOST,
                0, 0, 0, 0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }

    /// Hide the output window.
    pub fn hide_output(&self) {
        self.visible.store(false, Ordering::Relaxed);
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
            ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    /// Return whether the output window is currently visible.
    pub fn is_visible(&self) -> bool {
        self.visible.load(Ordering::Relaxed)
    }

    /// Update the countdown text shown on the output window timer overlay.
    ///
    /// Pass `None` (or an empty string) to hide the timer.
    /// The text is drawn via mpv's OSD so it always appears above D3D11 content.
    pub fn set_output_timer(&self, text: Option<&str>) {
        if let (Some(lib), Some(ctx)) = (OUTPUT_MPV_LIB.get(), OUTPUT_MPV_CTX.get()) {
            unsafe {
                prop_str(lib, ctx.0, "osd-msg1", text.unwrap_or(""));
            }
        }
    }

    /// Apply font, size, position and margin settings for the OSD timer overlay.
    ///
    /// Call this whenever the user changes timer display preferences; mpv
    /// picks up property changes immediately, even while the OSD is visible.
    pub fn set_timer_style(
        &self,
        font: &str,
        font_size: u32,
        position: crate::preferences::TimerPosition,
        margin: u32,
    ) {
        use crate::preferences::TimerPosition;
        // Mirror font name to the floating timer window.
        if let Some(m) = FLOAT_TIMER_FONT.get() {
            if let Ok(mut g) = m.lock() { *g = font.to_owned(); }
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

    /// Show or hide the standalone floating timer window.
    ///
    /// Safe to call from any thread — posts `WM_FLOAT_VISIBILITY` to the Win32 window.
    pub fn set_floating_timer_visible(&self, visible: bool) {
        if let Some(&hwnd) = FLOAT_TIMER_HWND.get() {
            if hwnd != 0 {
                unsafe {
                    use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;
                    PostMessageW(hwnd, WM_FLOAT_VISIBILITY, if visible { 1 } else { 0 }, 0);
                }
            }
        }
    }

    /// Write the current timer text to the floating window and request a repaint.
    ///
    /// Only invalidates when the text actually changed to avoid unnecessary paints.
    pub fn update_floating_timer(&self, text: Option<&str>) {
        let new_text = text.unwrap_or("");
        let changed = FLOAT_TIMER_TEXT.get().and_then(|m| m.lock().ok()).map(|mut g| {
            if *g != new_text { *g = new_text.to_owned(); true } else { false }
        }).unwrap_or(false);

        if changed {
            if let Some(&hwnd) = FLOAT_TIMER_HWND.get() {
                if hwnd != 0 {
                    unsafe {
                        windows_sys::Win32::Graphics::Gdi::InvalidateRect(hwnd, std::ptr::null(), 0);
                    }
                }
            }
        }
    }

    /// Set or clear the preview text shown on the OSD timer (overrides live cue time).
    ///
    /// Used by the preferences panel to show a placeholder like `"00:00.000"`
    /// while the user adjusts timer settings.  Pass `None` to return to live mode.
    pub fn set_timer_preview(&self, text: Option<String>) {
        if let Some(m) = TIMER_PREVIEW.get() {
            if let Ok(mut g) = m.lock() {
                *g = text;
            }
        }
    }

    /// Return the current preview text, if any.  Used by the timer refresh loop.
    pub fn get_timer_preview(&self) -> Option<String> {
        TIMER_PREVIEW.get()?.lock().ok()?.clone()
    }

    // ── Fullscreen ────────────────────────────────────────────────────────────

    /// Toggle the output window between windowed and true fullscreen.
    pub fn toggle_fullscreen(&self) {
        if let Some(state_mutex) = OUTPUT_WND_STATE.get() {
            if let Ok(mut state) = state_mutex.lock() {
                win32_window::toggle_fullscreen_impl(self.hwnd, &mut state);
            }
        }
    }

    // ── Status / GC ──────────────────────────────────────────────────────────

    /// No-op kept for API compatibility.
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

    /// Remove a completed voice.  Window stays visible showing black.
    pub fn gc_voice(&self, voice_id: VoiceId) {
        self.voices.lock().unwrap().remove(&voice_id);
    }

    // ── Internal helpers ─────────────────────────────────────────────────────

    fn position_window(&self, screen_index: Option<u32>) {
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                SetWindowPos, ShowWindow, HWND_TOPMOST, SW_SHOWNA,
                SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_FRAMECHANGED,
            };

            if let Some(idx) = screen_index {
                let screens = Self::list_screens();
                if let Some(s) = screens.into_iter().find(|s| s.index == idx) {
                    if let Some(state_mutex) = OUTPUT_WND_STATE.get() {
                        if let Ok(mut state) = state_mutex.lock() {
                            if !state.is_fullscreen {
                                state.saved_rect = (100, 100, 100 + 1280, 100 + 720);
                            }
                            state.is_fullscreen = true;
                        }
                    }
                    win32_window::set_borderless(self.hwnd);
                    SetWindowPos(
                        self.hwnd, HWND_TOPMOST,
                        s.x, s.y, s.width as i32, s.height as i32,
                        SWP_NOACTIVATE | SWP_FRAMECHANGED,
                    );
                }
            }

            // Always show and raise to TOPMOST when content is displayed.
            // If no output_screen is configured, leave the window geometry untouched —
            // the operator may have manually fullscreened or repositioned it.
            // Mirrors the original VideoEngine apply_window_layout behaviour.
            self.visible.store(true, Ordering::Relaxed);
            ShowWindow(self.hwnd, SW_SHOWNA);
            SetWindowPos(
                self.hwnd, HWND_TOPMOST,
                0, 0, 0, 0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }

}

impl Drop for OutputEngine {
    fn drop(&mut self) {
        unsafe { (self.mpv_lib.mpv_terminate_destroy)(self.mpv_ctx.0) };
    }
}

// ---------------------------------------------------------------------------
// Private utility functions (used by methods and sub-modules)
// ---------------------------------------------------------------------------

pub(super) fn cs(s: &str) -> CString {
    CString::new(s).expect("cs(): interior NUL byte in literal")
}

pub(super) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
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
