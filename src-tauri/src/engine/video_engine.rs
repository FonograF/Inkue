//! [`VideoEngine`] — native Win32 + libmpv video output.
//!
//! # Architecture
//!
//! A dedicated Win32 window (`WS_POPUP`) is created on a background thread
//! with its own `GetMessageW` loop.  libmpv is initialised with that HWND as
//! the `wid` option so it embeds its D3D11 renderer directly inside the window.
//!
//! ## Mouse handling (drag / fullscreen / cursor)
//!
//! mpv creates an internal child window for rendering; that child intercepts
//! all mouse events before the parent's `WndProc` ever sees them.  To handle
//! drag-to-move, double-click fullscreen, and the cursor shape, we create a
//! **transparent overlay child window** that always sits above mpv's render
//! child in z-order.
//!
//! The overlay uses `WS_EX_LAYERED` + `SetLayeredWindowAttributes(alpha=0)`:
//! - **Visually**: fully transparent — DWM composites it as invisible, so
//!   mpv's D3D11 frames show through unobstructed.
//! - **Input**: still receives all mouse events (no `WS_EX_TRANSPARENT`).
//!
//! This is the correct approach for embedding a D3D11 renderer with an
//! interactive overlay on Windows 8+.
//!
//! ## Fullscreen
//!
//! Double-click anywhere on the video window toggles true fullscreen on the
//! current monitor.  [`VideoEngine::toggle_fullscreen`] exposes the same
//! toggle for keyboard shortcuts.

use std::collections::HashMap;
use std::ffi::{c_void, CString};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use crossbeam_channel::{Receiver, Sender};
use ringbuf::traits::{Observer, Producer, Split};
use ringbuf::HeapRb;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::cue::types::{db_to_linear, FadeSpec};
use crate::engine::AudioEngine;

use super::mpv_sys::{
    MpvEventEndFile, MpvEventLogMessage, MpvLib,
    MPV_END_FILE_REASON_EOF, MPV_END_FILE_REASON_ERROR,
    MPV_EVENT_END_FILE, MPV_EVENT_FILE_LOADED, MPV_EVENT_LOG_MESSAGE,
    MPV_EVENT_SHUTDOWN, MPV_FORMAT_DOUBLE, MPV_FORMAT_INT64,
};

/// Unique identifier for one playing video instance.
pub type VoiceId = Uuid;
/// Unique identifier for one video output surface.
pub type SurfaceId = Uuid;

/// Information about a connected monitor, returned by [`VideoEngine::list_screens`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenInfo {
    /// Zero-based index (0 = primary).
    pub index: u32,
    pub width: u32,
    pub height: u32,
    /// Left edge in virtual screen coordinates.
    pub x: i32,
    /// Top edge in virtual screen coordinates.
    pub y: i32,
    pub is_primary: bool,
}

// ---------------------------------------------------------------------------
// VideoStatus — events from the mpv event thread to the 30 fps loop.
// ---------------------------------------------------------------------------

/// Status events produced by the mpv event thread.
#[derive(Debug, Clone)]
pub enum VideoStatus {
    /// Playback reached its natural end (or configured end time).
    Completed { voice_id: VoiceId },
    /// File metadata loaded; total media duration is now known.
    Duration { voice_id: VoiceId, duration_ms: u64 },
    /// A playback error occurred inside mpv.
    Error { voice_id: VoiceId, message: String },
}

// ---------------------------------------------------------------------------
// OutputSurface
// ---------------------------------------------------------------------------

/// A named video output surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputSurface {
    pub id: SurfaceId,
    pub name: String,
    /// Reserved for future multi-surface support.
    pub label: String,
}

// ---------------------------------------------------------------------------
// VideoVoice
// ---------------------------------------------------------------------------

#[derive(Debug)]
#[allow(dead_code)] // fields reserved for elapsed-time display
struct VideoVoice {
    id: VoiceId,
    started_at: Instant,
    duration: Option<Duration>,
}

// ---------------------------------------------------------------------------
// Thread-safety wrapper for the raw mpv context pointer
// ---------------------------------------------------------------------------

struct MpvCtx(*mut c_void);
unsafe impl Send for MpvCtx {}
unsafe impl Sync for MpvCtx {}

// ---------------------------------------------------------------------------
// Global Win32 state shared between the engine and the window procedures.
// ---------------------------------------------------------------------------

struct VideoWndState {
    is_fullscreen: bool,
    saved_rect: (i32, i32, i32, i32), // left, top, right, bottom
}

/// Fullscreen state shared between the engine and the window procedures.
static VIDEO_WND_STATE: OnceLock<Mutex<VideoWndState>> = OnceLock::new();

/// HWND of the parent popup window, stored globally so the mpv event thread
/// can post the `WM_SETUP_MPV_CHILD` message on `MPV_EVENT_FILE_LOADED`.
static VIDEO_PARENT_HWND: OnceLock<isize> = OnceLock::new();

/// `WM_APP + 1`: posted by the mpv event thread after `MPV_EVENT_FILE_LOADED`.
/// The parent `WndProc` reacts by setting `WS_EX_TRANSPARENT` on mpv's
/// freshly-created render child, so all mouse events pass through to the parent.
const WM_SETUP_MPV_CHILD: u32 = 0x8001;

// ---------------------------------------------------------------------------
// VideoEngine
// ---------------------------------------------------------------------------

/// Manages one native Win32 popup window + libmpv context for video playback.
pub struct VideoEngine {
    mpv_lib: Arc<MpvLib>,
    mpv_ctx: Arc<MpvCtx>,
    /// Parent Win32 HWND (popup window that mpv renders into).
    hwnd: isize,
    current_voice: Arc<Mutex<Option<VoiceId>>>,
    voices: Mutex<HashMap<VoiceId, VideoVoice>>,
    #[allow(dead_code)] // kept alive so the receiver channel stays open
    status_tx: Sender<VideoStatus>,
    status_rx: Mutex<Receiver<VideoStatus>>,
    default_surface_id: SurfaceId,
    /// Reference to the audio engine, used to install the video PCM consumer
    /// ring buffer when a video starts (so video audio flows through AudioEngine).
    /// Kept alive here so the `run_pcm_pipe_reader` thread can always reach it.
    #[allow(dead_code)]
    audio_engine: Arc<AudioEngine>,
}

impl VideoEngine {
    /// Construct the engine.
    ///
    /// Creates the Win32 window (hidden), loads libmpv, and initialises a
    /// shared mpv context.  mpv is configured to output audio via a Windows
    /// named pipe (`ao=pcm`) so all video audio flows through [`AudioEngine`]
    /// rather than through a separate WASAPI/ASIO output.
    ///
    /// Returns an error if `libmpv-2.dll` cannot be loaded or if the mpv
    /// context cannot be initialised.
    pub fn new(audio_engine: Arc<AudioEngine>) -> Result<Self> {
        let lib = Arc::new(MpvLib::load()?);

        // Create the output window on its own Win32 thread.
        let hwnd = create_video_window()?;

        // Create the mpv context.
        let ctx = unsafe { (lib.mpv_create)() };
        if ctx.is_null() {
            return Err(anyhow!("mpv_create() returned null"));
        }

        unsafe {
            // Tell mpv which window to render into (embedded / wid mode).
            // IMPORTANT: set this BEFORE mpv_initialize.
            let wid_name = cs("wid");
            let mut wid_val: i64 = hwnd as i64;
            (lib.mpv_set_option)(
                ctx,
                wid_name.as_ptr(),
                MPV_FORMAT_INT64,
                &mut wid_val as *mut i64 as *mut c_void,
            );

            // Video output: force the native D3D11 backend.
            opt_str(&lib, ctx, "vo", "gpu");
            opt_str(&lib, ctx, "gpu-api", "d3d11");
            opt_str(&lib, ctx, "force-window", "immediate");
            opt_str(&lib, ctx, "hwdec", "no");

            // No OSD, no input handling.
            opt_str(&lib, ctx, "osc", "no");
            opt_str(&lib, ctx, "osd-level", "0");
            opt_str(&lib, ctx, "input-default-bindings", "no");
            opt_str(&lib, ctx, "input-vo-keyboard", "no");
            opt_str(&lib, ctx, "input-cursor", "no");

            // Stay alive between files.
            opt_str(&lib, ctx, "keep-open", "no");
            opt_str(&lib, ctx, "idle", "yes");

            // ---------------------------------------------------------------
            // Audio routing: pipe decoded PCM into AudioEngine via a Windows
            // named pipe so that video audio comes out through the same
            // WASAPI / ASIO device as audio cues.
            //
            // ao=pcm writes raw float32 stereo PCM at the engine sample rate
            // to the named pipe.  A background thread reads the pipe and feeds
            // the samples into AudioEngine's video PCM ring buffer, where they
            // are mixed in fill_buffer alongside audio voices.
            // ---------------------------------------------------------------
            opt_str(&lib, ctx, "ao", "pcm");
            opt_str(&lib, ctx, "ao-pcm-file", r"\\.\pipe\wincue-mpv-audio");
            opt_str(&lib, ctx, "ao-pcm-waveheader", "no");
            let sr_str = audio_engine.sample_rate().to_string();
            opt_str(&lib, ctx, "audio-samplerate", &sr_str);
            opt_str(&lib, ctx, "audio-channels", "stereo");
            opt_str(&lib, ctx, "audio-format", "float");

            // Verbose log messages for diagnostics.
            let v = cs("v");
            (lib.mpv_request_log_messages)(ctx, v.as_ptr());

            let ret = (lib.mpv_initialize)(ctx);
            if ret < 0 {
                (lib.mpv_terminate_destroy)(ctx);
                return Err(anyhow!("mpv_initialize() failed with code {ret}"));
            }
        }

        let (status_tx, status_rx) = crossbeam_channel::unbounded();
        let current_voice: Arc<Mutex<Option<VoiceId>>> = Arc::new(Mutex::new(None));
        let mpv_ctx = Arc::new(MpvCtx(ctx));

        // Spawn the mpv event thread.
        {
            let lib2 = Arc::clone(&lib);
            let ctx2 = Arc::clone(&mpv_ctx);
            let voice2 = Arc::clone(&current_voice);
            let tx2 = status_tx.clone();
            std::thread::Builder::new()
                .name("wincue-mpv-events".into())
                .spawn(move || mpv_event_loop(lib2, ctx2, voice2, tx2, hwnd))
                .map_err(|e| anyhow!("Failed to spawn mpv event thread: {e}"))?;
        }

        // Spawn the named-pipe PCM reader thread.  It runs for the lifetime of
        // the process; each mpv connection (one per video file) creates a fresh
        // ring buffer and installs it in AudioEngine.  The thread also holds
        // the mpv lib/ctx so it can unpause mpv once pre-roll is complete.
        {
            let ae   = Arc::clone(&audio_engine);
            let lib3 = Arc::clone(&lib);
            let ctx3 = Arc::clone(&mpv_ctx);
            std::thread::Builder::new()
                .name("wincue-mpv-pcm".into())
                .spawn(move || run_pcm_pipe_reader(ae, lib3, ctx3))
                .map_err(|e| anyhow!("Failed to spawn PCM reader thread: {e}"))?;
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
        })
    }

    /// Expose the loaded `MpvLib` so callers can use it for probing.
    pub fn mpv_lib(&self) -> &MpvLib {
        &self.mpv_lib
    }

    /// Probe the duration of a video file without displaying it.
    ///
    /// Creates a throw-away mpv context with `vo=null` and `ao=null`, loads
    /// the file paused, reads the `duration` property from `MPV_EVENT_FILE_LOADED`,
    /// then destroys the context.  The whole operation is synchronous and
    /// completes in < 200 ms for most containers.
    pub fn probe_duration(lib: &MpvLib, path: &Path) -> Option<Duration> {
        unsafe {
            let ctx = (lib.mpv_create)();
            if ctx.is_null() { return None; }

            // Null outputs — no window, no audio device.
            opt_str(lib, ctx, "vo", "null");
            opt_str(lib, ctx, "ao", "null");
            // Start paused so playback never actually runs.
            opt_str(lib, ctx, "pause", "yes");
            // Disable hwdec to avoid device-initialisation overhead.
            opt_str(lib, ctx, "hwdec", "no");

            if (lib.mpv_initialize)(ctx) < 0 {
                (lib.mpv_terminate_destroy)(ctx);
                return None;
            }

            let path_str = path.to_string_lossy().replace('\\', "/");
            let path_cstr = match CString::new(path_str.as_str()) {
                Ok(c) => c,
                Err(_) => { (lib.mpv_terminate_destroy)(ctx); return None; }
            };
            let cmd_cstr = cs("loadfile");
            let replace_cstr = cs("replace");
            let index_cstr = cs("0");
            let args: [*const std::ffi::c_char; 5] = [
                cmd_cstr.as_ptr(), path_cstr.as_ptr(), replace_cstr.as_ptr(),
                index_cstr.as_ptr(), std::ptr::null(),
            ];
            (lib.mpv_command)(ctx, args.as_ptr());

            // Wait for FILE_LOADED (or error) with a 5-second cap.
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
                    if ret == 0 && val > 0.0 { duration_secs = Some(val); }
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
                        let is_primary = (mi.dwFlags & 1) != 0; // MONITORINFOF_PRIMARY = 1
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
        // Sort: primary first, then by x position.
        screens.sort_by(|a, b| {
            b.is_primary.cmp(&a.is_primary).then(a.x.cmp(&b.x))
        });
        // Re-assign indices after sort.
        for (i, s) in screens.iter_mut().enumerate() {
            s.index = i as u32;
        }
        screens
    }

    /// The ID of the default "Screen 1" surface.
    pub fn default_surface_id(&self) -> SurfaceId {
        self.default_surface_id
    }

    /// Snapshot of all registered output surfaces (currently just one).
    pub fn surfaces(&self) -> Vec<OutputSurface> {
        vec![OutputSurface {
            id: self.default_surface_id,
            name: "Screen 1".into(),
            label: String::new(),
        }]
    }

    /// Begin playback of `file_path` and return the new [`VoiceId`].
    ///
    /// Video audio is routed through [`AudioEngine`] automatically via the
    /// `ao=pcm` named-pipe mechanism initialised in [`VideoEngine::new`].
    ///
    /// `screen_index` — when `Some(n)`, the window is moved to fill monitor n
    /// (0 = primary) before playback starts.  `None` keeps the floating window
    /// at its current position/size.
    pub fn play_voice(
        &self,
        file_path: &Path,
        _surface_id: Option<SurfaceId>,
        volume_db: f64,
        loop_count: u32,
        start_ms: Option<u64>,
        end_ms: Option<u64>,
        _fade_in: Option<&FadeSpec>,
        screen_index: Option<u32>,
    ) -> Result<VoiceId> {
        let voice_id = Uuid::new_v4();

        // If a video is already playing, complete it immediately.
        // mpv's `loadfile replace` sends MPV_END_FILE_REASON_STOP (not EOF),
        // which the event loop ignores — so the old VoiceId would stay in the
        // voices map and the status bar would show two cues as running.
        if let Some(old_id) = self.current_voice.lock().unwrap().take() {
            self.voices.lock().unwrap().remove(&old_id);
            let _ = self.status_tx.send(VideoStatus::Completed { voice_id: old_id });
        }

        *self.current_voice.lock().unwrap() = Some(voice_id);
        self.voices.lock().unwrap().insert(
            voice_id,
            VideoVoice { id: voice_id, started_at: Instant::now(), duration: None },
        );

        let ctx = self.mpv_ctx.0;
        let lib = &self.mpv_lib;

        // mpv on Windows accepts both path separators, but forward slashes are
        // safer with the C string interface.
        let path_str = file_path.to_string_lossy().replace('\\', "/");
        let path_cstr = CString::new(path_str.as_str())
            .map_err(|_| anyhow!("File path contains NUL byte"))?;

        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                SetWindowPos, ShowWindow, HWND_TOPMOST, SW_SHOWNA,
                SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_FRAMECHANGED, SWP_NOZORDER,
            };

            // Position the window based on screen_index.
            if let Some(idx) = screen_index {
                // Fullscreen on a specific monitor — strip the resize border first.
                let screens = Self::list_screens();
                if let Some(s) = screens.into_iter().find(|s| s.index == idx) {
                    if let Some(state_mutex) = VIDEO_WND_STATE.get() {
                        if let Ok(mut state) = state_mutex.lock() {
                            if !state.is_fullscreen {
                                state.saved_rect = (100, 100, 100 + 1280, 100 + 720);
                            }
                            state.is_fullscreen = true;
                        }
                    }
                    set_borderless(self.hwnd);
                    SetWindowPos(
                        self.hwnd, HWND_TOPMOST,
                        s.x, s.y, s.width as i32, s.height as i32,
                        SWP_NOACTIVATE | SWP_FRAMECHANGED,
                    );
                }
            } else {
                // Floating window — restore resize border and windowed size.
                if let Some(state_mutex) = VIDEO_WND_STATE.get() {
                    if let Ok(mut state) = state_mutex.lock() {
                        if state.is_fullscreen {
                            let (l, t, r, b) = state.saved_rect;
                            set_resizable(self.hwnd);
                            SetWindowPos(
                                self.hwnd, HWND_TOPMOST,
                                l, t, r - l, b - t,
                                SWP_NOACTIVATE | SWP_FRAMECHANGED,
                            );
                            state.is_fullscreen = false;
                        }
                    }
                }
                // Always-on-top without activating (keeps WinCue focus).
                SetWindowPos(
                    self.hwnd, HWND_TOPMOST,
                    0, 0, 0, 0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER,
                );
            }

            // Show without activating — WinCue keeps keyboard focus.
            ShowWindow(self.hwnd, SW_SHOWNA);

            // Bring to the front visually (topmost, still no focus steal).
            SetWindowPos(
                self.hwnd, HWND_TOPMOST,
                0, 0, 0, 0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );

            // Set playback volume.
            let vol_pct = (100.0 * db_to_linear(volume_db)).clamp(0.0, 1000.0);
            let vol_str = cs(&format!("{vol_pct:.2}"));
            let prop_vol = cs("volume");
            (lib.mpv_set_property_string)(ctx, prop_vol.as_ptr(), vol_str.as_ptr());

            // Build per-file option string.
            let mut opts: Vec<String> = Vec::new();
            if let Some(start) = start_ms {
                opts.push(format!("start={:.3}", start as f64 / 1000.0));
            }
            if let Some(end) = end_ms {
                opts.push(format!("end={:.3}", end as f64 / 1000.0));
            }

            // loop-file=N means N *extra* loops after the first play:
            //   loop_count=0  → "no"  (play once, no repetition)
            //   loop_count=1  → "1"   (play twice total)
            //   loop_count=∞  → "inf"
            let loop_val = if loop_count == u32::MAX {
                "inf".to_string()
            } else if loop_count == 0 {
                "no".to_string()
            } else {
                loop_count.to_string()
            };
            opts.push(format!("loop-file={loop_val}"));

            let opts_str = opts.join(",");
            let opts_cstr  = cs(&opts_str);
            let cmd_cstr   = cs("loadfile");
            let replace_cstr = cs("replace");
            // loadfile argument order:
            //   0 url
            //   1 flags   — "replace" / "append" / "append-play"
            //   2 index   — integer playlist insertion index (ignored for "replace")
            //   3 options — comma-separated key=value pairs
            let index_cstr = cs("0");
            let args: [*const std::ffi::c_char; 6] = [
                cmd_cstr.as_ptr(),
                path_cstr.as_ptr(),
                replace_cstr.as_ptr(),
                index_cstr.as_ptr(),
                opts_cstr.as_ptr(),
                std::ptr::null(),
            ];
            // Ensure mpv starts paused so that the first video frame is
            // rendered immediately (no black-window freeze) while the PCM
            // pipe completes its handshake and pre-rolls the ring buffer.
            // The pipe reader thread will send "set pause no" once enough
            // audio has accumulated (PCM_PREROLL_THRESHOLD samples ≈ 100ms).
            {
                let set_cstr    = cs("set");
                let pause_name  = cs("pause");
                let pause_yes   = cs("yes");
                let set_args: [*const std::ffi::c_char; 4] = [
                    set_cstr.as_ptr(),
                    pause_name.as_ptr(),
                    pause_yes.as_ptr(),
                    std::ptr::null(),
                ];
                (lib.mpv_command)(ctx, set_args.as_ptr());
            }

            let ret = (lib.mpv_command)(ctx, args.as_ptr());
            if ret < 0 {
                let err_cstr = (lib.mpv_error_string)(ret);
                let err_msg = std::ffi::CStr::from_ptr(err_cstr).to_string_lossy();
                return Err(anyhow!("mpv loadfile failed (code {ret}): {err_msg}"));
            }

            log::info!("[mpv] loadfile sent: {path_str} opts=[{opts_str}]");
        }

        Ok(voice_id)
    }

    /// Stop the given voice.  `fade_ms` is an immediate cut for now.
    pub fn stop_voice(&self, voice_id: VoiceId, _fade_ms: u32) -> Result<()> {
        {
            let mut cv = self.current_voice.lock().unwrap();
            if *cv == Some(voice_id) {
                *cv = None;
            }
        }
        self.voices.lock().unwrap().remove(&voice_id);

        unsafe {
            let stop = cs("stop");
            let args: [*const std::ffi::c_char; 2] = [stop.as_ptr(), std::ptr::null()];
            (self.mpv_lib.mpv_command)(self.mpv_ctx.0, args.as_ptr());

            use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
            ShowWindow(self.hwnd, SW_HIDE);
        }
        Ok(())
    }

    /// Pause the given voice.
    pub fn pause_voice(&self, _voice_id: VoiceId) -> Result<()> {
        unsafe {
            let name = cs("pause");
            let val = cs("yes");
            (self.mpv_lib.mpv_set_property_string)(self.mpv_ctx.0, name.as_ptr(), val.as_ptr());
        }
        Ok(())
    }

    /// Resume a paused voice.
    pub fn resume_voice(&self, _voice_id: VoiceId) -> Result<()> {
        unsafe {
            let name = cs("pause");
            let val = cs("no");
            (self.mpv_lib.mpv_set_property_string)(self.mpv_ctx.0, name.as_ptr(), val.as_ptr());
        }
        Ok(())
    }

    /// Update the playback volume of a running voice.
    pub fn set_voice_volume(&self, _voice_id: VoiceId, volume_db: f64) -> Result<()> {
        unsafe {
            let vol_pct = (100.0 * db_to_linear(volume_db)).clamp(0.0, 1000.0);
            let val = cs(&format!("{vol_pct:.2}"));
            let name = cs("volume");
            (self.mpv_lib.mpv_set_property_string)(self.mpv_ctx.0, name.as_ptr(), val.as_ptr());
        }
        Ok(())
    }

    /// Toggle the video window between windowed (1280×720) and true fullscreen.
    ///
    /// Callable from any thread.  Can also be triggered by double-clicking the
    /// video window.
    pub fn toggle_fullscreen(&self) {
        if let Some(state_mutex) = VIDEO_WND_STATE.get() {
            if let Ok(mut state) = state_mutex.lock() {
                toggle_fullscreen_impl(self.hwnd, &mut state);
            }
        }
    }

    /// No-op — kept for API compatibility.
    pub fn push_status(&self, _status: VideoStatus) {}

    /// Drain all pending status events.  Called by the 30 fps event loop.
    pub fn drain_status(&self) -> Vec<VideoStatus> {
        let rx = self.status_rx.lock().unwrap();
        let mut out = Vec::new();
        while let Ok(s) = rx.try_recv() {
            out.push(s);
        }
        out
    }

    /// Remove a completed voice and hide the window when nothing is left.
    pub fn gc_voice(&self, voice_id: VoiceId) {
        self.voices.lock().unwrap().remove(&voice_id);
        if self.voices.lock().unwrap().is_empty() {
            unsafe {
                use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
                ShowWindow(self.hwnd, SW_HIDE);
            }
        }
    }
}

impl Drop for VideoEngine {
    fn drop(&mut self) {
        unsafe { (self.mpv_lib.mpv_terminate_destroy)(self.mpv_ctx.0) };
    }
}

// ---------------------------------------------------------------------------
// mpv event loop  (thread: wincue-mpv-events)
// ---------------------------------------------------------------------------

fn mpv_event_loop(
    lib: Arc<MpvLib>,
    ctx: Arc<MpvCtx>,
    current_voice: Arc<Mutex<Option<VoiceId>>>,
    status_tx: Sender<VideoStatus>,
    parent_hwnd: isize,
) {
    loop {
        let event = unsafe { (lib.mpv_wait_event)(ctx.0, 2.0) };
        if event.is_null() {
            continue;
        }
        let event_id = unsafe { (*event).event_id };

        match event_id {
            MPV_EVENT_SHUTDOWN => break,

            MPV_EVENT_LOG_MESSAGE => {
                let data = unsafe { (*event).data as *const MpvEventLogMessage };
                if !data.is_null() {
                    let level_cstr = unsafe { std::ffi::CStr::from_ptr((*data).level) };
                    let text_cstr  = unsafe { std::ffi::CStr::from_ptr((*data).text) };
                    let level   = level_cstr.to_string_lossy();
                    let text    = text_cstr.to_string_lossy();
                    let trimmed = text.trim_end_matches('\n');
                    if !trimmed.is_empty() {
                        match level.as_ref() {
                            "fatal" | "error" => log::error!("[mpv] {trimmed}"),
                            "warn"            => log::warn! ("[mpv] {trimmed}"),
                            "info"            => log::info! ("[mpv] {trimmed}"),
                            _                 => log::debug!("[mpv] {trimmed}"),
                        }
                    }
                }
            }

            MPV_EVENT_FILE_LOADED => {
                log::info!("[mpv] MPV_EVENT_FILE_LOADED — video file ready");

                // mpv has created its render child.  Ask the Win32 thread to
                // set WS_EX_TRANSPARENT on that child so mouse events pass
                // through to the parent, which handles drag / fullscreen / cursor.
                unsafe {
                    use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;
                    PostMessageW(parent_hwnd, WM_SETUP_MPV_CHILD, 0, 0);
                }

                // Query total duration.
                let mut duration_secs: f64 = 0.0;
                let ret = unsafe {
                    let name = cs("duration");
                    (lib.mpv_get_property)(
                        ctx.0,
                        name.as_ptr(),
                        MPV_FORMAT_DOUBLE,
                        &mut duration_secs as *mut f64 as *mut c_void,
                    )
                };
                if ret == 0 {
                    if let Some(vid) = *current_voice.lock().unwrap() {
                        let _ = status_tx.send(VideoStatus::Duration {
                            voice_id: vid,
                            duration_ms: (duration_secs * 1000.0) as u64,
                        });
                    }
                }
            }

            MPV_EVENT_END_FILE => {
                log::info!("[mpv] MPV_EVENT_END_FILE received");
                let data_ptr = unsafe { (*event).data };
                if let Some(end_data) =
                    unsafe { (data_ptr as *mut MpvEventEndFile).as_ref() }
                {
                    let voice_id = match end_data.reason {
                        MPV_END_FILE_REASON_EOF | MPV_END_FILE_REASON_ERROR => {
                            current_voice.lock().unwrap().take()
                        }
                        _ => *current_voice.lock().unwrap(),
                    };

                    if let Some(vid) = voice_id {
                        match end_data.reason {
                            MPV_END_FILE_REASON_EOF => {
                                let _ = status_tx.send(VideoStatus::Completed { voice_id: vid });
                            }
                            MPV_END_FILE_REASON_ERROR => {
                                let msg = format!("mpv error (code {})", end_data.error);
                                let _ = status_tx.send(VideoStatus::Error {
                                    voice_id: vid,
                                    message: msg,
                                });
                            }
                            _ => {} // STOP / QUIT — initiated by stop_voice()
                        }
                    }
                }
            }

            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Named-pipe PCM reader  (thread: wincue-mpv-pcm)
// ---------------------------------------------------------------------------

/// Minimum samples (stereo f32) that must be queued before the video is
/// unpaused.  Must be ≥ `MIN_VIDEO_PREBUFFER` in `audio_engine.rs` so that
/// AudioEngine's pre-roll gate opens at the same moment mpv starts playing.
/// At 48 kHz stereo ≈ 100 ms.
const PCM_PREROLL_THRESHOLD: usize = 9_600;

/// Read decoded PCM from mpv's `ao=pcm` named-pipe output and feed it into
/// [`AudioEngine`]'s video PCM ring buffer.
///
/// mpv opens `\\.\pipe\wincue-mpv-audio` as the audio output file.
/// This function runs as a background thread for the lifetime of the process.
///
/// ## Startup sequencing (pause / pre-roll / unpause)
///
/// `play_voice()` issues `set pause yes` before `loadfile` so that mpv
/// renders the first video frame immediately without a black-screen freeze.
/// Once the pipe connects and [`PCM_PREROLL_THRESHOLD`] samples have been
/// buffered in the ring buffer, this thread sends `set pause no` so that
/// video and audio start together.  AudioEngine's own pre-roll gate
/// (`MIN_VIDEO_PREBUFFER`) is set to the same threshold so it starts
/// outputting samples at exactly the same moment.
///
/// ## Rate control
///
/// mpv with `ao=pcm` writes PCM as fast as decoding allows (no real-time
/// pacing).  After the pre-roll phase the reader thread throttles itself:
/// when the ring buffer occupancy exceeds `max_prebuffer` samples it sleeps
/// for 1 ms, letting AudioEngine drain.  This naturally limits mpv's write
/// speed (via pipe backpressure) to approximately real-time, which keeps
/// video and audio in sync.
///
/// ## Per-video lifecycle
///
/// Each mpv connection (one per video file) creates a fresh ring buffer.
/// After mpv disconnects (stop / EOF), the consumer is cleared from
/// AudioEngine so the mixer produces silence until the next video starts.
fn run_pcm_pipe_reader(audio_engine: Arc<AudioEngine>, lib: Arc<MpvLib>, ctx: Arc<MpvCtx>) {
    // Declare the Windows named-pipe and file-I/O APIs directly to avoid
    // windows-sys feature-flag issues.  All functions are in kernel32.dll /
    // advapi32.dll which are always linked on Windows.
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateNamedPipeW(
            lpname: *const u16,
            dwopenmode: u32,
            dwpipemode: u32,
            nmaxinstances: u32,
            noutbuffersize: u32,
            ninbuffersize: u32,
            ndefaulttimeout: u32,
            lpsecurityattributes: *const std::ffi::c_void,
        ) -> isize;
        fn ConnectNamedPipe(hnamedpipe: isize, lpoverlapped: *mut std::ffi::c_void) -> i32;
        fn DisconnectNamedPipe(hnamedpipe: isize) -> i32;
        fn ReadFile(
            hfile: isize,
            lpbuffer: *mut std::ffi::c_void,
            nnumberofbytestoread: u32,
            lpnumberofbytesread: *mut u32,
            lpoverlapped: *mut std::ffi::c_void,
        ) -> i32;
        fn CloseHandle(hobject: isize) -> i32;
    }

    const PIPE_ACCESS_INBOUND: u32 = 0x0000_0001;
    const PIPE_TYPE_BYTE: u32 = 0x0000_0000;
    const PIPE_READMODE_BYTE: u32 = 0x0000_0000;
    const PIPE_WAIT: u32 = 0x0000_0000;
    const PIPE_UNLIMITED_INSTANCES: u32 = 255;
    const INVALID_HANDLE_VALUE: isize = -1_isize;

    // UTF-16 encoded pipe name (null-terminated).
    let pipe_name: Vec<u16> = r"\\.\pipe\wincue-mpv-audio"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    // Keep this many f32 samples pre-buffered before throttling mpv.
    // ~500 ms of stereo audio at 48 kHz.  mpv is blocked via pipe backpressure
    // once this is reached; the ring buffer provides an additional cushion.
    let max_prebuffer: usize = (audio_engine.sample_rate() as usize) / 2 * 2; // sr/2 * 2ch

    log::info!(r"PCM pipe reader: started (\\.\pipe\wincue-mpv-audio)");

    loop {
        // Create a new server-side pipe instance for the next mpv connection.
        let handle = unsafe {
            CreateNamedPipeW(
                pipe_name.as_ptr(),
                PIPE_ACCESS_INBOUND,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                0,      // out-buffer (server-to-client; not used for inbound)
                65536,  // in-buffer (client-to-server; receives mpv's audio)
                0,      // default timeout
                std::ptr::null(),
            )
        };

        if handle == INVALID_HANDLE_VALUE {
            log::warn!("PCM pipe: CreateNamedPipeW failed — retrying in 500 ms");
            std::thread::sleep(Duration::from_millis(500));
            continue;
        }

        // Block until mpv opens the pipe as a client.
        log::info!("PCM pipe: waiting for mpv to connect...");
        unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) };
        log::info!("PCM pipe: mpv connected — creating ring buffer");

        // Fresh ring buffer for this video: 3 seconds of stereo f32.
        // Sized generously so mpv burst-writes and scheduling jitter never
        // overflow the buffer even when the writer thread is briefly preempted.
        let ring_size = (audio_engine.sample_rate() as usize * 2 * 3).max(16384);
        let (mut prod, cons) = HeapRb::<f32>::new(ring_size).split();
        audio_engine.set_video_pcm_consumer(Some(cons));

        // Read raw f32-LE interleaved stereo PCM from mpv in two phases:
        //
        // Phase 1 — Pre-roll (no throttle):
        //   Read until the ring buffer contains >= PCM_PREROLL_THRESHOLD samples
        //   (≈ 100 ms at 48 kHz stereo), then unpause mpv so video and audio
        //   start simultaneously.  AudioEngine's own pre-roll gate opens at the
        //   same threshold so it begins outputting samples at the same moment.
        //
        // Phase 2 — Normal playback (throttled):
        //   Continue reading with backpressure so mpv doesn't run ahead of
        //   real time.
        let mut raw = [0u8; 4096];
        let mut prerolled = false;

        loop {
            // Phase 2 throttle — skip during pre-roll so we fill quickly.
            if prerolled {
                while prod.occupied_len() > max_prebuffer {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }

            let mut bytes_read: u32 = 0;
            let ok = unsafe {
                ReadFile(
                    handle,
                    raw.as_mut_ptr().cast(),
                    raw.len() as u32,
                    &mut bytes_read,
                    std::ptr::null_mut(),
                )
            };

            if ok == 0 || bytes_read == 0 {
                break; // Pipe closed by mpv (stop or EOF).
            }

            for chunk in raw[..bytes_read as usize].chunks_exact(4) {
                // SAFETY: chunks_exact(4) guarantees exactly 4 bytes.
                let sample = f32::from_le_bytes(chunk.try_into().unwrap());
                let _ = prod.try_push(sample);
            }

            // Phase 1 → Phase 2 transition: once enough samples are buffered,
            // unpause mpv so the video clock starts at the same instant that
            // AudioEngine opens its pre-roll gate.
            if !prerolled && prod.occupied_len() >= PCM_PREROLL_THRESHOLD {
                prerolled = true;
                unsafe {
                    let name = cs("pause");
                    let val  = cs("no");
                    (lib.mpv_set_property_string)(ctx.0, name.as_ptr(), val.as_ptr());
                }
                log::info!(
                    "PCM pipe: pre-roll complete ({PCM_PREROLL_THRESHOLD} samples) \
                     — mpv unpaused"
                );
            }
        }

        log::info!("PCM pipe: mpv disconnected — clearing video PCM consumer");
        audio_engine.set_video_pcm_consumer(None);

        unsafe {
            DisconnectNamedPipe(handle);
            CloseHandle(handle);
        }
    }
}

// ---------------------------------------------------------------------------
// Win32 window creation  (thread: wincue-video-win32)
// ---------------------------------------------------------------------------

/// Spawn a dedicated thread that owns the parent popup window and its Win32
/// message loop.  Returns the parent HWND.
///
/// ## Mouse handling strategy
///
/// mpv creates an internal D3D11 render child on `loadfile`.  That child
/// normally intercepts all mouse events.  On `MPV_EVENT_FILE_LOADED` the mpv
/// event thread posts `WM_SETUP_MPV_CHILD` to the parent; the parent WndProc
/// then sets `WS_EX_TRANSPARENT` on that child so all mouse events fall
/// through to the parent, which handles drag / fullscreen / cursor itself.
///
/// ## Focus
///
/// `WS_EX_NOACTIVATE` prevents the window from ever taking keyboard focus,
/// so the main WinCue window keeps its shortcuts while video is playing.
fn create_video_window() -> Result<isize> {
    let (tx, rx) = std::sync::mpsc::channel::<Result<isize>>();

    std::thread::Builder::new()
        .name("wincue-video-win32".into())
        .spawn(move || {
            unsafe {
                use windows_sys::Win32::Graphics::Gdi::{GetStockObject, BLACK_BRUSH};
                use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
                use windows_sys::Win32::UI::WindowsAndMessaging::{
                    CreateWindowExW, DispatchMessageW, GetMessageW, RegisterClassExW,
                    TranslateMessage,
                    CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW,
                    MSG, WS_CLIPCHILDREN, WS_EX_NOACTIVATE, WS_POPUP, WS_SIZEBOX,
                    WNDCLASSEXW,
                };

                let hinstance = GetModuleHandleW(std::ptr::null());

                // -----------------------------------------------------------
                // Register parent window class.
                // CS_DBLCLKS: required to receive WM_LBUTTONDBLCLK.
                // -----------------------------------------------------------
                let parent_class = wide("WinCueVideoWnd\0");
                let wc_parent = WNDCLASSEXW {
                    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                    style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
                    lpfnWndProc: Some(video_wnd_proc),
                    cbClsExtra: 0,
                    cbWndExtra: 0,
                    hInstance: hinstance,
                    hIcon: 0,
                    hCursor: 0,
                    hbrBackground: GetStockObject(BLACK_BRUSH) as isize,
                    lpszMenuName: std::ptr::null(),
                    lpszClassName: parent_class.as_ptr(),
                    hIconSm: 0,
                };
                RegisterClassExW(&wc_parent);

                // -----------------------------------------------------------
                // Create parent popup window (hidden at startup).
                // WS_EX_NOACTIVATE: never steal focus from the main window.
                // WS_CLIPCHILDREN:   do not paint over mpv's render child.
                // -----------------------------------------------------------
                let window_name = wide("WinCue Output 1\0");
                let parent_hwnd = CreateWindowExW(
                    WS_EX_NOACTIVATE,
                    parent_class.as_ptr(),
                    window_name.as_ptr(),
                    WS_POPUP | WS_CLIPCHILDREN | WS_SIZEBOX,
                    100, 100,
                    1280, 720,
                    0, 0,
                    hinstance,
                    std::ptr::null(),
                );

                if parent_hwnd == 0 {
                    let _ = tx.send(Err(anyhow!("CreateWindowExW (parent) failed")));
                    return;
                }

                // Initialise shared state.
                VIDEO_WND_STATE.get_or_init(|| {
                    Mutex::new(VideoWndState {
                        is_fullscreen: false,
                        saved_rect: (100, 100, 100 + 1280, 100 + 720),
                    })
                });
                VIDEO_PARENT_HWND.get_or_init(|| parent_hwnd);

                let _ = tx.send(Ok(parent_hwnd));

                // Run the message loop for the lifetime of the window.
                let mut msg = MSG {
                    hwnd: 0,
                    message: 0,
                    wParam: 0,
                    lParam: 0,
                    time: 0,
                    pt: windows_sys::Win32::Foundation::POINT { x: 0, y: 0 },
                };
                loop {
                    let ret = GetMessageW(&mut msg, 0, 0, 0);
                    if ret == 0 || ret == -1 {
                        break;
                    }
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        })
        .map_err(|e| anyhow!("Failed to spawn Win32 window thread: {e}"))?;

    rx.recv()
        .map_err(|_| anyhow!("Win32 window thread exited before sending HWND"))?
}

// ---------------------------------------------------------------------------
// Parent window procedure
// ---------------------------------------------------------------------------

/// Window procedure for the parent popup window.
///
/// - `WM_MOUSEACTIVATE`  → `MA_NOACTIVATE`: never steal focus.
/// - `WM_SETCURSOR`      → force the arrow cursor (mpv sets a custom one).
/// - `WM_LBUTTONDOWN`    → initiate an OS window-drag.
/// - `WM_LBUTTONDBLCLK`  → toggle fullscreen on current monitor.
/// - `WM_SETUP_MPV_CHILD`→ set `WS_EX_TRANSPARENT` on mpv's render child.
/// - `WM_CLOSE`          → hide (HWND stays valid for the next `play_voice`).
/// - `WM_DESTROY`        → end the message loop.
unsafe extern "system" fn video_wnd_proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: windows_sys::Win32::Foundation::LPARAM,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DefWindowProcW, PostQuitMessage, ShowWindow,
        SW_HIDE, WM_CLOSE, WM_DESTROY, WM_NCCALCSIZE,
        WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_MOUSEACTIVATE, WM_NCHITTEST, WM_SETCURSOR,
        HTCLIENT, HTCAPTION,
    };

    const MA_NOACTIVATE: isize = 3;

    match msg {
        WM_MOUSEACTIVATE => {
            // Never activate the window — the main WinCue window keeps focus.
            MA_NOACTIVATE
        }
        WM_NCCALCSIZE => {
            // Return 0 to eliminate all non-client area (caption bar, borders).
            // WS_SIZEBOX normally adds a non-client resize frame that shows as a
            // white strip at the top.  By handling WM_NCCALCSIZE we keep the
            // resize hit-testing (via WS_SIZEBOX in the style) but remove the
            // visible non-client drawing.
            0
        }
        WM_NCHITTEST => {
            // Let DefWindowProc detect resize borders (WS_SIZEBOX).
            // If it says HTCLIENT (interior), keep it as HTCLIENT so that
            // WM_LBUTTONDOWN fires and our drag handler works.
            let hit = DefWindowProcW(hwnd, msg, wparam, lparam);
            if hit == HTCLIENT as isize || hit == HTCAPTION as isize {
                HTCLIENT as isize
            } else {
                hit
            }
        }
        WM_SETCURSOR => {
            // Only force the arrow cursor when inside the client area.
            // For resize borders (LOWORD(lparam) != HTCLIENT) let the OS set
            // the appropriate resize cursor.
            let ht = (lparam & 0xFFFF) as isize;
            if ht == HTCLIENT as isize {
                use windows_sys::Win32::UI::WindowsAndMessaging::{IDC_ARROW, LoadCursorW, SetCursor};
                SetCursor(LoadCursorW(0, IDC_ARROW));
                1
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
        WM_LBUTTONDOWN => {
            drag_window(hwnd);
            0
        }
        WM_LBUTTONDBLCLK => {
            if let Some(state_mutex) = VIDEO_WND_STATE.get() {
                if let Ok(mut state) = state_mutex.lock() {
                    toggle_fullscreen_impl(hwnd, &mut state);
                }
            }
            0
        }
        WM_SETUP_MPV_CHILD => {
            // mpv just created its D3D11 render child.  Set WS_EX_TRANSPARENT
            // on it so all mouse events fall through to this parent window.
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                GetWindow, GetWindowLongPtrW, SetWindowLongPtrW, GW_CHILD,
            };
            const GWL_EXSTYLE: i32 = -20;
            const WS_EX_TRANSPARENT: isize = 0x20;
            let child = GetWindow(hwnd, GW_CHILD);
            if child != 0 {
                let ex = GetWindowLongPtrW(child, GWL_EXSTYLE);
                SetWindowLongPtrW(child, GWL_EXSTYLE, ex | WS_EX_TRANSPARENT);
            }
            0
        }
        WM_CLOSE => {
            ShowWindow(hwnd, SW_HIDE);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Drag helper
// ---------------------------------------------------------------------------

/// Initiate an OS-managed window drag on `hwnd` by simulating a title-bar
/// left-button-down event.  Safe to call from the Win32 message thread.
fn drag_window(hwnd: isize) {
    unsafe {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetCursorPos, PostMessageW, HTCAPTION, WM_NCLBUTTONDOWN,
        };

        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        let screen_lp = (pt.x as u16 as isize) | ((pt.y as u16 as isize) << 16);
        ReleaseCapture();
        PostMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION as usize, screen_lp);
    }
}

// ---------------------------------------------------------------------------
// Fullscreen toggle
// ---------------------------------------------------------------------------

/// Remove `WS_SIZEBOX` so no resize border is visible in fullscreen.
unsafe fn set_borderless(hwnd: isize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, GWL_STYLE,
    };
    const WS_SIZEBOX: isize = 0x0004_0000;
    let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
    SetWindowLongPtrW(hwnd, GWL_STYLE, style & !WS_SIZEBOX);
}

/// Restore `WS_SIZEBOX` so the floating window is resizable again.
unsafe fn set_resizable(hwnd: isize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, GWL_STYLE,
    };
    const WS_SIZEBOX: isize = 0x0004_0000;
    let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
    SetWindowLongPtrW(hwnd, GWL_STYLE, style | WS_SIZEBOX);
}

/// Toggle the parent window between windowed (1280×720) and true fullscreen.
fn toggle_fullscreen_impl(hwnd: isize, state: &mut VideoWndState) {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowRect, SetWindowPos, HWND_TOPMOST,
        SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOZORDER,
    };

    unsafe {
        if state.is_fullscreen {
            let (l, t, r, b) = state.saved_rect;
            set_resizable(hwnd);
            SetWindowPos(
                hwnd, 0,
                l, t, r - l, b - t,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
            state.is_fullscreen = false;
        } else {
            let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            GetWindowRect(hwnd, &mut rc);
            state.saved_rect = (rc.left, rc.top, rc.right, rc.bottom);

            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut mi: MONITORINFO = std::mem::zeroed();
            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(monitor, &mut mi);

            let mr = mi.rcMonitor;
            set_borderless(hwnd);
            SetWindowPos(
                hwnd, HWND_TOPMOST,
                mr.left, mr.top,
                mr.right - mr.left, mr.bottom - mr.top,
                SWP_NOOWNERZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
            state.is_fullscreen = true;
        }
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Encode a Rust `&str` as a null-terminated UTF-16 slice.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// Wrap `s` in a `CString`, panicking on interior NUL bytes.
fn cs(s: &str) -> CString {
    CString::new(s).expect("cs(): interior NUL byte in literal")
}

/// Set an mpv string option (must be called before `mpv_initialize`).
unsafe fn opt_str(lib: &MpvLib, ctx: *mut c_void, name: &str, value: &str) {
    let n = cs(name);
    let v = cs(value);
    (lib.mpv_set_option_string)(ctx, n.as_ptr(), v.as_ptr());
}
