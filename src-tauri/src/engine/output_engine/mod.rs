//! [`OutputEngine`] — unified output for both video and image cues.
//!
//! Unified GL path (`output_gl` — Windows default, Linux, macOS): a native window
//! hosts libmpv via the OpenGL Render API (`vo=libmpv`, see `render.rs`); the
//! dip-to-black fade is a GL quad drawn in the same surface.  The render loop and
//! fade are identical on every OS — only native window creation differs (winit on
//! Windows/Linux, AppKit/objc2 on macOS, see `macos_window.rs`).
//!
//! Windows legacy (`legacy-win32-output` feature): a `WS_POPUP` Win32 window hosts
//! libmpv via the `wid` option, with a `WS_EX_LAYERED` overlay driving the fade.
//!
//! On every OS the floating cue timer is a Tauri WebView window (`float-timer`),
//! and the on-output timer is mpv's OSD (`osd-msg1`).

mod fade;
mod mpv_events;
#[cfg(output_gl)]
mod render;
/// macOS-only: AppKit NSWindow creation + control for the GL output path.
#[cfg(all(output_gl, target_os = "macos"))]
mod macos_window;
mod types;
#[cfg(output_win32)]
mod win32_window;

pub use types::{OutputStatus, OutputSurface, ScreenInfo, SurfaceId, VoiceId};
use types::{
    FadeAnimState, FadePending, FadePendingParams, MpvCtx, OutputVoice, PendingVideoStart,
};
#[cfg(output_win32)]
use types::OutputWndState;

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

use super::mpv_sys::MpvLib;
#[cfg(output_win32)]
use super::mpv_sys::MPV_FORMAT_INT64;

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
/// When `Some`, the timer refresh loop shows this text instead of live cue time.
pub(crate) static TIMER_PREVIEW: OnceLock<Mutex<Option<String>>> = OnceLock::new();
/// Deduplication cache for the floating timer text (avoids redundant Tauri events).
pub(super) static FLOAT_TIMER_TEXT: OnceLock<Mutex<String>> = OnceLock::new();
/// Font family mirrored from OSD settings → emitted to the float-timer window.
pub(super) static FLOAT_TIMER_FONT: OnceLock<Mutex<String>> = OnceLock::new();

// ---------------------------------------------------------------------------
// Global Win32-only state
// ---------------------------------------------------------------------------

/// Fullscreen / saved-rect state for the Win32 output window.
#[cfg(output_win32)]
pub(super) static OUTPUT_WND_STATE: OnceLock<Mutex<OutputWndState>> = OnceLock::new();
/// HWND of the Win32 parent output window.
#[cfg(output_win32)]
pub(super) static OUTPUT_PARENT_HWND: OnceLock<isize> = OnceLock::new();
/// HWND of the Win32 layered fade-overlay popup window.
#[cfg(output_win32)]
pub(super) static FADE_OVERLAY_HWND: OnceLock<isize> = OnceLock::new();

// ---------------------------------------------------------------------------
// Win32 message constants (Windows only)
// ---------------------------------------------------------------------------

/// `WM_APP + 1`: posted by the mpv event thread after `MPV_EVENT_FILE_LOADED`.
#[cfg(output_win32)]
pub(super) const WM_SETUP_MPV_CHILD: u32 = 0x8001;
/// `WM_APP + 2`: posted by show_content/stop_content to start the fade timer.
#[cfg(output_win32)]
pub(super) const WM_DO_FADE: u32 = 0x8002;
#[cfg(output_win32)]
pub(super) const FADE_TIMER_ID: usize = 1;

// ---------------------------------------------------------------------------
// OutputEngine
// ---------------------------------------------------------------------------

/// Manages the output window + libmpv context for all video and image output.
pub struct OutputEngine {
    mpv_lib: Arc<MpvLib>,
    mpv_ctx: Arc<MpvCtx>,
    /// Win32 parent HWND (0 on Mac/Linux — mpv manages its own window).
    #[allow(dead_code)]
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
    /// Tauri app handle — used to show/hide and emit events to the float-timer window.
    app_handle: tauri::AppHandle,
}

impl OutputEngine {
    /// Construct the engine.
    ///
    /// On Windows creates the Win32 window (shown immediately) and starts the Win32
    /// message loop.  On Mac / Linux mpv manages its own window; a cross-platform
    /// 16 ms fade-loop thread is started instead.
    pub fn new(audio_engine: Arc<AudioEngine>, app_handle: tauri::AppHandle) -> Result<Self> {
        let lib = Arc::new(MpvLib::load()?);

        // Legacy Win32 path: create a Win32 parent window and embed mpv into it via wid.
        // winit path (Linux + Windows default) / macOS: the winit or mpv-native window
        // owns presentation, so `hwnd` is unused (0).
        #[cfg(output_win32)]
        let hwnd = win32_window::create_output_window()?;
        #[cfg(not(output_win32))]
        let hwnd: isize = 0;

        // mpv requires LC_NUMERIC=C; set it before mpv_create() on non-Windows.
        #[cfg(not(target_os = "windows"))]
        unsafe {
            libc::setlocale(libc::LC_NUMERIC, b"C\0".as_ptr() as *const libc::c_char);
        }

        let ctx = unsafe { (lib.mpv_create)() };
        if ctx.is_null() {
            return Err(anyhow!("mpv_create() returned null"));
        }

        unsafe {
            // Legacy Win32 path only: embed mpv into the Win32 parent window via wid.
            #[cfg(output_win32)]
            {
                let wid_name = cs("wid");
                let mut wid_val: i64 = hwnd as i64;
                (lib.mpv_set_option)(
                    ctx,
                    wid_name.as_ptr(),
                    MPV_FORMAT_INT64,
                    &mut wid_val as *mut i64 as *mut c_void,
                );
            }

            // Legacy Win32 path: D3D11 backend with non-blocking Present (needed for
            // desync), embedded in our own HWND. `d3d11-flip=no` avoids the DWM
            // DirectFlip→composed transition that flashes black when the layered fade
            // overlay covers the swapchain. The unified GL path below has no such
            // overlay (the fade is a GL quad in the same surface), so it is not needed.
            #[cfg(output_win32)]
            {
                opt_str(&lib, ctx, "vo", "gpu");
                opt_str(&lib, ctx, "gpu-api", "d3d11");
                opt_str(&lib, ctx, "d3d11-sync-interval", "0");
                opt_str(&lib, ctx, "d3d11-flip", "no");
                opt_str(&lib, ctx, "force-window", "immediate");
            }
            // Unified GL path (render.rs): mpv renders into our own native window via
            // mpv_render_context_render() instead of creating its own window.  True on
            // Windows (default), Linux, and macOS.
            #[cfg(output_gl)]
            opt_str(&lib, ctx, "vo", "libmpv");

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
        FADE_STATE.get_or_init(|| Mutex::new(FadeAnimState::idle()));
        TIMER_PREVIEW.get_or_init(|| Mutex::new(None));
        FLOAT_TIMER_TEXT.get_or_init(|| Mutex::new(String::new()));
        // Empty sentinel (never a real font name) so the first set_timer_style()
        // call always emits float-timer-font, regardless of what the persisted
        // preference happens to be — the float-timer window's own React state
        // has no other way to learn the current font.
        FLOAT_TIMER_FONT.get_or_init(|| Mutex::new(String::new()));

        // Unified GL path (Linux + Windows default): create the winit/GL output
        // window and block until mpv's render context is live, so no `loadfile` can
        // race ahead of it.
        #[cfg(output_gl)]
        render::init(&app_handle, Arc::clone(&lib), Arc::clone(&mpv_ctx))?;

        {
            let lib2   = Arc::clone(&lib);
            let ctx2   = Arc::clone(&mpv_ctx);
            let voice2 = Arc::clone(&current_voice);
            let tx2    = status_tx.clone();
            let gsa2   = Arc::clone(&go_sent_at);
            let ae     = Arc::clone(&audio_engine);
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
            visible: Arc::new(AtomicBool::new(false)),
            app_handle,
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

    /// Display content (video or image) on the output window.
    #[allow(clippy::too_many_arguments)]
    pub fn show_content(
        &self,
        file_path: &Path,
        is_image: bool,
        fade_in_ms: u32,
        _this_fade_out_ms: u32,
        loop_count: u32,
        start_ms: Option<u64>,
        end_ms: Option<u64>,
        screen_index: Option<u32>,
        audio_voice_id: Option<VoiceId>,
        display_duration_ms: Option<u64>,
    ) -> Result<VoiceId> {
        let voice_id = Uuid::new_v4();

        // Replace the old voice.
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

        // Stop the previous video's audio track (hard cut — the visual fade-out
        // of the previous cue is handled by its own stop_content() call).
        if let Some(av) = OUTPUT_CURRENT_AUDIO_VOICE.get() {
            let previous = {
                let mut g = av.lock().unwrap();
                std::mem::replace(&mut *g, audio_voice_id)
            };
            if let Some(prev_id) = previous {
                let _ = self.audio_engine.stop_voice(
                    prev_id,
                    0,
                    crate::engine::ring_command::FadeCurve::SCurve,
                );
            }
        }

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

        // Abort any overlay animation that may be running (e.g. a stop-fade
        // that was started by a preceding stop_content call).  Clear timer_active
        // so the Win32 timer tick detects the abort and kills itself.
        if let Some(fs) = FADE_STATE.get() {
            if let Ok(mut s) = fs.lock() {
                s.pending      = None;
                s.duration_ms  = 0;
                s.timer_active = false;
                s.target_alpha = s.current_alpha;
                s.start_alpha  = s.current_alpha;
            }
        }

        if is_image {
            if fade_in_ms > 0 {
                // Black overlay (alpha=255) hides whatever was on screen while the
                // image loads.  The fade then reveals it (255 → 0).
                fade::set_overlay_alpha(255);
            }
            fade::execute_load_params(&params, &self.mpv_lib, self.mpv_ctx.0);
            if fade_in_ms > 0 {
                if let Some(fs) = FADE_STATE.get() {
                    let mut s = fs.lock().unwrap();
                    s.start_alpha  = 255;
                    s.current_alpha = 255;
                    s.target_alpha = 0;
                    s.duration_ms  = fade_in_ms;
                    s.start_time   = Instant::now();
                    s.timer_active = false;
                    s.pending      = None;
                }
                #[cfg(output_win32)]
                unsafe {
                    use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;
                    PostMessageW(self.hwnd, WM_DO_FADE, 0, 0);
                }
                // GL path: wake the render loop so it animates the new FADE_STATE.
                #[cfg(output_gl)]
                render::wake();
            } else {
                fade::set_overlay_alpha(0);
            }
        } else {
            // Video: hold black overlay while the first frame is decoded.
            // PLAYBACK_RESTART in mpv_events reveals + unpauses once frame 0 is ready.
            fade::set_overlay_alpha(255);
            fade::execute_load_params(&params, &self.mpv_lib, self.mpv_ctx.0);
        }

        Ok(voice_id)
    }

    /// Stop the content identified by `voice_id`, fading the visual overlay
    /// to black over `visual_fade_ms` and the audio voice out over `audio_fade_ms`.
    pub fn stop_content(&self, voice_id: VoiceId, visual_fade_ms: u32, audio_fade_ms: u32) {
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

        if visual_fade_ms > 0 {
            if let Some(fs) = FADE_STATE.get() {
                let mut state = fs.lock().unwrap();
                state.start_alpha = state.current_alpha;
                state.target_alpha = 255;
                state.duration_ms = visual_fade_ms;
                state.start_time = Instant::now();
                state.timer_active = false;
                state.pending = Some(FadePending::Stop);
            }
            #[cfg(output_win32)]
            unsafe {
                use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;
                PostMessageW(self.hwnd, WM_DO_FADE, 0, 0);
            }
            // GL path: wake the render loop so it animates the stop fade.
            #[cfg(output_gl)]
            render::wake();
        } else {
            unsafe {
                let stop = cs("stop");
                let args: [*const std::ffi::c_char; 2] =
                    [stop.as_ptr(), std::ptr::null()];
                (self.mpv_lib.mpv_command)(self.mpv_ctx.0, args.as_ptr());
            }
            // Hard cut: mpv goes idle. On the GL path the render loop would otherwise
            // skip drawing (no new frame + transparent overlay) and leave the last
            // frame frozen on screen. Force the overlay to black — it wakes the render
            // loop and paints an opaque black quad over the stale frame, matching the
            // end state of a stop-with-fade.
            fade::set_overlay_alpha(255);
        }

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
                    audio_fade_ms,
                    crate::engine::ring_command::FadeCurve::SCurve,
                );
            }
        }
    }

    /// Hard-stop all content immediately (no fade).
    pub fn hard_stop_current(&self) {
        let voice_id = *self.current_voice.lock().unwrap();
        if let Some(vid) = voice_id {
            self.stop_content(vid, 0, 0);
        }
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
        #[cfg(output_win32)]
        if let Some(&overlay) = FADE_OVERLAY_HWND.get() {
            if alpha > 0 {
                unsafe {
                    use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_SHOWNA};
                    ShowWindow(overlay, SW_SHOWNA);
                }
            }
        }
        fade::set_overlay_alpha(alpha);
    }

    /// Return the AudioEngine voice ID of the current video's audio track.
    pub fn get_current_audio_voice(&self) -> Option<VoiceId> {
        OUTPUT_CURRENT_AUDIO_VOICE.get()
            .and_then(|m| m.lock().ok())
            .and_then(|g| *g)
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
        self.show_content(
            file_path, false,
            0, 0, loop_count, start_ms, end_ms, screen_index, None, None,
        )
    }

    pub fn stop_voice(&self, voice_id: VoiceId, fade_ms: u32) -> Result<()> {
        self.stop_content(voice_id, fade_ms, fade_ms);
        Ok(())
    }

    pub fn stop_current_voice(&self, _fade_ms: u32) {
        self.hard_stop_current();
    }

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

    pub fn set_voice_volume(&self, _voice_id: VoiceId, volume_db: f64) -> Result<()> {
        if let Some(av) = OUTPUT_CURRENT_AUDIO_VOICE.get() {
            if let Some(aid) = *av.lock().unwrap() {
                let _ = self.audio_engine.set_voice_gain(aid, db_to_linear(volume_db) as f32);
            }
        }
        Ok(())
    }

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
        #[cfg(output_win32)]
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                SetWindowPos, HWND_TOPMOST,
                SWP_NOMOVE, SWP_NOSIZE, SWP_NOACTIVATE, SWP_SHOWWINDOW,
            };
            SetWindowPos(
                self.hwnd, HWND_TOPMOST,
                0, 0, 0, 0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
        // Unified GL path: render.rs owns the native window. On Windows/Linux this
        // calls winit inline; on macOS it dispatches the AppKit show onto the main
        // thread (NSWindow methods are main-thread-only).
        #[cfg(output_gl)]
        render::show();
        use tauri::Emitter;
        let _ = self.app_handle.emit("output-window-visible", true);
    }

    /// Hide the output window.
    pub fn hide_output(&self) {
        self.visible.store(false, Ordering::Relaxed);
        #[cfg(output_win32)]
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
            ShowWindow(self.hwnd, SW_HIDE);
            if let Some(&overlay) = FADE_OVERLAY_HWND.get() {
                ShowWindow(overlay, SW_HIDE);
            }
        }
        #[cfg(output_gl)]
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
        if let (Some(lib), Some(ctx)) = (OUTPUT_MPV_LIB.get(), OUTPUT_MPV_CTX.get()) {
            unsafe {
                prop_str(lib, ctx.0, "osd-msg1", text.unwrap_or(""));
            }
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

    // ── Fullscreen ────────────────────────────────────────────────────────────

    /// Toggle the output window between windowed and true fullscreen.
    pub fn toggle_fullscreen(&self) {
        #[cfg(output_win32)]
        if let Some(state_mutex) = OUTPUT_WND_STATE.get() {
            if let Ok(mut state) = state_mutex.lock() {
                win32_window::toggle_fullscreen_impl(self.hwnd, &mut state);
            }
        }
        #[cfg(output_gl)]
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
        #[cfg(output_win32)]
        unsafe {
            use windows_sys::Win32::Foundation::RECT;
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                GetWindowRect, SetWindowPos, ShowWindow, HWND_TOPMOST, SW_SHOWNA,
                SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_FRAMECHANGED,
            };

            if let Some(idx) = screen_index {
                let screens = self.list_screens();
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
                    if let Some(&overlay) = FADE_OVERLAY_HWND.get() {
                        ShowWindow(overlay, SW_SHOWNA);
                        SetWindowPos(
                            overlay, HWND_TOPMOST,
                            s.x, s.y, s.width as i32, s.height as i32,
                            SWP_NOACTIVATE,
                        );
                    }
                }
            }

            self.visible.store(true, Ordering::Relaxed);
            ShowWindow(self.hwnd, SW_SHOWNA);
            SetWindowPos(
                self.hwnd, HWND_TOPMOST,
                0, 0, 0, 0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );

            if screen_index.is_none() {
                if let Some(&overlay) = FADE_OVERLAY_HWND.get() {
                    ShowWindow(overlay, SW_SHOWNA);
                    let mut rc: RECT = std::mem::zeroed();
                    GetWindowRect(self.hwnd, &mut rc);
                    SetWindowPos(
                        overlay, HWND_TOPMOST,
                        rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top,
                        SWP_NOACTIVATE,
                    );
                }
            }

            SetWindowPos(self.hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }

        #[cfg(output_gl)]
        {
            self.visible.store(true, Ordering::Relaxed);
            if let Some(idx) = screen_index {
                // Windows/Linux: position via the winit window using the screen rect
                // from list_screens(). macOS: place the NSWindow onto NSScreen[idx]
                // directly (AppKit's own coordinate space — no rect conversion needed).
                #[cfg(not(target_os = "macos"))]
                if let Some(s) = self.list_screens().into_iter().find(|s| s.index == idx) {
                    render::set_outer_rect(s.x, s.y, s.width, s.height);
                }
                #[cfg(target_os = "macos")]
                render::position_on_screen(idx);
            }
            render::show();
        }
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

#[cfg(output_win32)]
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
