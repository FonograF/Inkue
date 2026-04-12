//! [`VideoEngine`] — native Win32 + libmpv video output.
//!
//! # Architecture
//!
//! A dedicated Win32 window (`WS_POPUP`, no title bar) is created on a
//! background thread with its own `GetMessageW` loop.  A transparent overlay
//! child window sits on top of mpv's render child and intercepts mouse input:
//!
//! - **Left-click drag anywhere** → moves the parent window.
//! - **Double-click anywhere** → toggles fullscreen on the current monitor.
//!
//! A libmpv context is initialised with the parent HWND as `wid`; mpv renders
//! into a child window it creates inside the parent.  Because mpv creates its
//! child asynchronously (on first `loadfile`), the overlay's z-order is
//! re-asserted from the mpv event thread when `MPV_EVENT_FILE_LOADED` fires.
//!
//! All public methods are callable from any thread.

use std::collections::HashMap;
use std::ffi::{c_void, CString};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use crossbeam_channel::{Receiver, Sender};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::cue::types::{db_to_linear, FadeSpec};

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
// Fullscreen state — single-window design; initialized once.
// ---------------------------------------------------------------------------

struct VideoWndState {
    is_fullscreen: bool,
    saved_rect: (i32, i32, i32, i32), // left, top, right, bottom
}

/// Fullscreen state shared between the engine and the window procedures.
static VIDEO_WND_STATE: OnceLock<Mutex<VideoWndState>> = OnceLock::new();

/// HWND of the transparent overlay child window.
/// Stored globally so the parent `WM_SIZE` handler can resize it, and so the
/// mpv event thread can re-assert its z-order after `MPV_EVENT_FILE_LOADED`.
static VIDEO_OVERLAY_HWND: OnceLock<isize> = OnceLock::new();

// ---------------------------------------------------------------------------
// VideoEngine
// ---------------------------------------------------------------------------

/// Manages one native Win32 popup window + libmpv context for video playback.
pub struct VideoEngine {
    mpv_lib: Arc<MpvLib>,
    mpv_ctx: Arc<MpvCtx>,
    /// Parent Win32 HWND (popup window that mpv renders into).
    hwnd: isize,
    /// Transparent overlay child HWND (intercepts mouse for drag / fullscreen).
    overlay_hwnd: isize,
    current_voice: Arc<Mutex<Option<VoiceId>>>,
    voices: Mutex<HashMap<VoiceId, VideoVoice>>,
    #[allow(dead_code)] // kept alive so the receiver channel stays open
    status_tx: Sender<VideoStatus>,
    status_rx: Mutex<Receiver<VideoStatus>>,
    default_surface_id: SurfaceId,
}

impl VideoEngine {
    /// Construct the engine.
    ///
    /// Creates the Win32 window (hidden), loads libmpv, and initialises a
    /// shared mpv context.  Returns an error if `libmpv-2.dll` cannot be
    /// loaded or if the mpv context cannot be initialised.
    pub fn new() -> Result<Self> {
        let lib = Arc::new(MpvLib::load()?);

        // Create the output window on its own Win32 thread.
        let (hwnd, overlay_hwnd) = create_video_window()?;

        // Create the mpv context.
        let ctx = unsafe { (lib.mpv_create)() };
        if ctx.is_null() {
            return Err(anyhow!("mpv_create() returned null"));
        }

        unsafe {
            // Tell mpv which window to render into.
            let wid_name = cs("wid");
            let mut wid_val: i64 = hwnd as i64;
            (lib.mpv_set_option)(
                ctx,
                wid_name.as_ptr(),
                MPV_FORMAT_INT64,
                &mut wid_val as *mut i64 as *mut c_void,
            );

            // Video output — let mpv auto-select the GPU API (usually D3D11).
            opt_str(&lib, ctx, "vo", "gpu");

            // Create the VO immediately so its window exists before the first
            // file is loaded.  This lets us assert the overlay z-order once
            // instead of racing with mpv's asynchronous child-window creation.
            opt_str(&lib, ctx, "force-window", "immediate");

            // No OSD, no input handling — pure show-control output surface.
            opt_str(&lib, ctx, "osc", "no");
            opt_str(&lib, ctx, "input-default-bindings", "no");
            opt_str(&lib, ctx, "input-vo-keyboard", "no");

            // Stay alive between files for fast successive playback.
            opt_str(&lib, ctx, "keep-open", "always");
            opt_str(&lib, ctx, "idle", "yes");

            // Enable warn-level log messages so renderer failures are visible.
            let warn = cs("warn");
            (lib.mpv_request_log_messages)(ctx, warn.as_ptr());

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
                .spawn(move || mpv_event_loop(lib2, ctx2, voice2, tx2, overlay_hwnd))
                .map_err(|e| anyhow!("Failed to spawn mpv event thread: {e}"))?;
        }

        Ok(Self {
            mpv_lib: lib,
            mpv_ctx,
            hwnd,
            overlay_hwnd,
            current_voice,
            voices: Mutex::new(HashMap::new()),
            status_tx,
            status_rx: Mutex::new(status_rx),
            default_surface_id: Uuid::new_v4(),
        })
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
    pub fn play_voice(
        &self,
        file_path: &Path,
        _surface_id: Option<SurfaceId>,
        volume_db: f64,
        loop_count: u32,
        start_ms: Option<u64>,
        end_ms: Option<u64>,
        _fade_in: Option<&FadeSpec>,
    ) -> Result<VoiceId> {
        let voice_id = Uuid::new_v4();

        *self.current_voice.lock().unwrap() = Some(voice_id);
        self.voices.lock().unwrap().insert(
            voice_id,
            VideoVoice { id: voice_id, started_at: Instant::now(), duration: None },
        );

        let ctx = self.mpv_ctx.0;
        let lib = &self.mpv_lib;

        let path_str = file_path.to_string_lossy().replace('\\', "/");
        let path_cstr = CString::new(path_str.as_str())
            .map_err(|_| anyhow!("File path contains NUL byte"))?;

        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                SetWindowPos, ShowWindow, HWND_TOP, SW_SHOW,
                SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
            };

            // Make the parent popup visible.
            ShowWindow(self.hwnd, SW_SHOW);

            // Ensure the overlay is on top of mpv's render child.
            // (mpv creates its child with `force-window`, but we re-assert here
            //  in case the child was recreated by a prior stop/restart cycle.)
            SetWindowPos(
                self.overlay_hwnd,
                HWND_TOP,
                0, 0, 0, 0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );

            // Set playback volume.
            let vol_pct = (100.0 * db_to_linear(volume_db)).clamp(0.0, 1000.0);
            let vol_str = cs(&format!("{vol_pct:.2}"));
            let prop_vol = cs("volume");
            (lib.mpv_set_property_string)(ctx, prop_vol.as_ptr(), vol_str.as_ptr());

            // Build per-file option string (start / end / loop).
            let mut opts: Vec<String> = Vec::new();
            if let Some(start) = start_ms {
                opts.push(format!("start={:.3}", start as f64 / 1000.0));
            }
            if let Some(end) = end_ms {
                opts.push(format!("end={:.3}", end as f64 / 1000.0));
            }
            let loop_val = if loop_count == u32::MAX {
                "inf".to_string()
            } else {
                (loop_count + 1).to_string()
            };
            opts.push(format!("loop-file={loop_val}"));

            let opts_str = opts.join(",");
            let opts_cstr = cs(&opts_str);
            let cmd_cstr = cs("loadfile");
            let replace_cstr = cs("replace");

            // mpv_command: ["loadfile", "<path>", "replace", "<options>", NULL]
            let args: [*const std::ffi::c_char; 5] = [
                cmd_cstr.as_ptr(),
                path_cstr.as_ptr(),
                replace_cstr.as_ptr(),
                opts_cstr.as_ptr(),
                std::ptr::null(),
            ];
            let ret = (lib.mpv_command)(ctx, args.as_ptr());
            if ret < 0 {
                return Err(anyhow!("mpv loadfile failed with code {ret}"));
            }
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
    overlay_hwnd: isize,
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
                // Forward mpv warn/error messages to the Rust logger so
                // renderer failures appear in the dev console.
                let data = unsafe { (*event).data as *const MpvEventLogMessage };
                if !data.is_null() {
                    let text = unsafe { std::ffi::CStr::from_ptr((*data).text) };
                    let msg = text.to_string_lossy();
                    let trimmed = msg.trim_end_matches('\n');
                    if !trimmed.is_empty() {
                        log::warn!("[mpv] {trimmed}");
                    }
                }
            }

            MPV_EVENT_FILE_LOADED => {
                // mpv has (re)created its render child — put our overlay back
                // on top so mouse events are intercepted correctly.
                unsafe {
                    use windows_sys::Win32::UI::WindowsAndMessaging::{
                        SetWindowPos, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
                    };
                    SetWindowPos(
                        overlay_hwnd,
                        HWND_TOP,
                        0, 0, 0, 0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                    );
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
                        let duration_ms = (duration_secs * 1000.0) as u64;
                        let _ = status_tx.send(VideoStatus::Duration {
                            voice_id: vid,
                            duration_ms,
                        });
                    }
                }
            }

            MPV_EVENT_END_FILE => {
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
                                let _ =
                                    status_tx.send(VideoStatus::Error { voice_id: vid, message: msg });
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
// Win32 window creation  (thread: wincue-video-win32)
// ---------------------------------------------------------------------------

/// Spawn a thread that creates the parent popup window + transparent overlay
/// child and runs their shared message loop.
///
/// Returns `(parent_hwnd, overlay_hwnd)`.
fn create_video_window() -> Result<(isize, isize)> {
    let (tx, rx) = std::sync::mpsc::channel::<Result<(isize, isize)>>();

    std::thread::Builder::new()
        .name("wincue-video-win32".into())
        .spawn(move || {
            unsafe {
                use windows_sys::Win32::Graphics::Gdi::{GetStockObject, BLACK_BRUSH};
                use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
                use windows_sys::Win32::UI::WindowsAndMessaging::{
                    CreateWindowExW, DispatchMessageW, GetMessageW, RegisterClassExW,
                    TranslateMessage, CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW,
                    MSG, WS_CHILD, WS_CLIPCHILDREN, WS_POPUP, WS_VISIBLE, WNDCLASSEXW,
                };

                let hinstance = GetModuleHandleW(std::ptr::null());

                // -----------------------------------------------------------
                // Register parent window class
                // -----------------------------------------------------------
                let parent_class = wide("WinCueVideoWnd\0");
                let wc_parent = WNDCLASSEXW {
                    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(video_wnd_proc),
                    cbClsExtra: 0,
                    cbWndExtra: 0,
                    hInstance: hinstance,
                    hIcon: 0,
                    hCursor: 0,
                    // Black background shows between frames and before video starts.
                    hbrBackground: GetStockObject(BLACK_BRUSH) as isize,
                    lpszMenuName: std::ptr::null(),
                    lpszClassName: parent_class.as_ptr(),
                    hIconSm: 0,
                };
                RegisterClassExW(&wc_parent);

                // -----------------------------------------------------------
                // Register overlay window class
                // CS_DBLCLKS is required to receive WM_LBUTTONDBLCLK.
                // NULL background + WM_ERASEBKGND→1 makes it visually transparent.
                // -----------------------------------------------------------
                let overlay_class = wide("WinCueVideoOverlay\0");
                let wc_overlay = WNDCLASSEXW {
                    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                    style: CS_DBLCLKS,
                    lpfnWndProc: Some(video_overlay_proc),
                    cbClsExtra: 0,
                    cbWndExtra: 0,
                    hInstance: hinstance,
                    hIcon: 0,
                    hCursor: 0,
                    hbrBackground: 0, // NULL — overlay paints nothing
                    lpszMenuName: std::ptr::null(),
                    lpszClassName: overlay_class.as_ptr(),
                    hIconSm: 0,
                };
                RegisterClassExW(&wc_overlay);

                // -----------------------------------------------------------
                // Create parent popup window (no title bar, no border).
                // Starts hidden; shown by play_voice().
                // -----------------------------------------------------------
                let window_name = wide("WinCue Output 1\0");
                let parent_hwnd = CreateWindowExW(
                    0,
                    parent_class.as_ptr(),
                    window_name.as_ptr(),
                    WS_POPUP | WS_CLIPCHILDREN,
                    100, 100, // initial position (doesn't matter; window starts hidden)
                    1280, 720,
                    0, 0,
                    hinstance,
                    std::ptr::null(),
                );

                if parent_hwnd == 0 {
                    let _ = tx.send(Err(anyhow!("CreateWindowExW (parent) failed")));
                    return;
                }

                // -----------------------------------------------------------
                // Create transparent overlay child (fills the client area).
                // WS_CHILD | WS_VISIBLE so it exists and intercepts mouse.
                // -----------------------------------------------------------
                let overlay_hwnd = CreateWindowExW(
                    0,
                    overlay_class.as_ptr(),
                    std::ptr::null(), // no title
                    WS_CHILD | WS_VISIBLE,
                    0, 0, 1280, 720,
                    parent_hwnd, // parent
                    0, hinstance,
                    std::ptr::null(),
                );

                if overlay_hwnd == 0 {
                    let _ = tx.send(Err(anyhow!("CreateWindowExW (overlay) failed")));
                    return;
                }

                // Initialise shared state.
                VIDEO_WND_STATE.get_or_init(|| {
                    Mutex::new(VideoWndState {
                        is_fullscreen: false,
                        saved_rect: (100, 100, 100 + 1280, 100 + 720),
                    })
                });
                VIDEO_OVERLAY_HWND.get_or_init(|| overlay_hwnd);

                let _ = tx.send(Ok((parent_hwnd, overlay_hwnd)));

                // Run the message loop for the lifetime of both windows.
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
        .map_err(|_| anyhow!("Win32 window thread exited before sending HWNDs"))?
}

// ---------------------------------------------------------------------------
// Parent window procedure
// ---------------------------------------------------------------------------

/// Window procedure for the parent popup window.
///
/// - `WM_CLOSE` → hide (HWND stays valid for the next `play_voice`).
/// - `WM_SIZE`  → resize the overlay child to match the new client area.
/// - `WM_DESTROY` → end the message loop.
unsafe extern "system" fn video_wnd_proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: windows_sys::Win32::Foundation::LPARAM,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DefWindowProcW, PostQuitMessage, SetWindowPos, ShowWindow,
        SW_HIDE, SWP_NOACTIVATE, SWP_NOZORDER, WM_CLOSE, WM_DESTROY, WM_SIZE,
    };

    match msg {
        WM_CLOSE => {
            // Hide instead of destroying — HWND remains valid for reuse.
            ShowWindow(hwnd, SW_HIDE);
            0
        }
        WM_SIZE => {
            // Keep the overlay child sized to fill the whole client area.
            if let Some(&overlay) = VIDEO_OVERLAY_HWND.get() {
                let w = (lparam & 0xFFFF) as i32;
                let h = ((lparam >> 16) & 0xFFFF) as i32;
                SetWindowPos(overlay, 0, 0, 0, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ---------------------------------------------------------------------------
// Overlay window procedure
// ---------------------------------------------------------------------------

/// Window procedure for the transparent overlay child.
///
/// The overlay covers the full client area of the parent and sits above mpv's
/// render child.  It is visually transparent (no background paint) but
/// receives all mouse events.
///
/// - `WM_ERASEBKGND` → claim handled without painting (transparent).
/// - `WM_LBUTTONDOWN` → initiate a window-move drag on the parent.
/// - `WM_LBUTTONDBLCLK` → toggle fullscreen on the current monitor.
unsafe extern "system" fn video_overlay_proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: windows_sys::Win32::Foundation::LPARAM,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DefWindowProcW, GetCursorPos, GetParent, PostMessageW,
        HTCAPTION, WM_ERASEBKGND, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_NCLBUTTONDOWN,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
    use windows_sys::Win32::Foundation::POINT;

    match msg {
        WM_ERASEBKGND => {
            // Claim the erase was handled without painting anything so the
            // overlay stays visually transparent over mpv's rendered frames.
            1
        }

        WM_LBUTTONDOWN => {
            // Simulate a caption-bar drag on the parent so the OS moves it.
            let parent = GetParent(hwnd);
            let mut pt = POINT { x: 0, y: 0 };
            GetCursorPos(&mut pt);
            // LPARAM for WM_NCLBUTTONDOWN encodes screen coordinates.
            let screen_lp = (pt.x as u16 as isize) | ((pt.y as u16 as isize) << 16);
            ReleaseCapture();
            PostMessageW(parent, WM_NCLBUTTONDOWN, HTCAPTION as usize, screen_lp);
            0
        }

        WM_LBUTTONDBLCLK => {
            // Toggle fullscreen on the monitor containing the parent window.
            let parent = GetParent(hwnd);
            if let Some(state_mutex) = VIDEO_WND_STATE.get() {
                if let Ok(mut state) = state_mutex.lock() {
                    toggle_fullscreen(parent, &mut state);
                }
            }
            0
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ---------------------------------------------------------------------------
// Fullscreen toggle
// ---------------------------------------------------------------------------

/// Toggle the parent window between windowed (1280×720) and true fullscreen.
fn toggle_fullscreen(hwnd: isize, state: &mut VideoWndState) {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowRect, SetWindowPos, HWND_TOP,
        SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOZORDER,
    };

    unsafe {
        if state.is_fullscreen {
            // Restore to saved windowed rect.
            let (l, t, r, b) = state.saved_rect;
            SetWindowPos(
                hwnd, 0,
                l, t, r - l, b - t,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
            state.is_fullscreen = false;
        } else {
            // Save current rect.
            let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            GetWindowRect(hwnd, &mut rc);
            state.saved_rect = (rc.left, rc.top, rc.right, rc.bottom);

            // Get the monitor that most contains the window.
            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut mi: MONITORINFO = std::mem::zeroed();
            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(monitor, &mut mi);

            // Expand to fill the whole monitor (true fullscreen, no border).
            let mr = mi.rcMonitor;
            SetWindowPos(
                hwnd, HWND_TOP,
                mr.left, mr.top,
                mr.right - mr.left, mr.bottom - mr.top,
                SWP_NOOWNERZORDER | SWP_FRAMECHANGED,
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
