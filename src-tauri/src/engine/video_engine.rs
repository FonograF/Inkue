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
use std::sync::atomic::{AtomicBool, Ordering};
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
    MPV_EVENT_PLAYBACK_RESTART, MPV_EVENT_SEEK, MPV_EVENT_SHUTDOWN, MPV_EVENT_START_FILE,
    MPV_EVENT_VIDEO_RECONFIG, MPV_FORMAT_DOUBLE, MPV_FORMAT_FLAG, MPV_FORMAT_INT64,
};

/// Unique identifier for one playing video instance.
pub type VoiceId = Uuid;
/// Unique identifier for one video output surface.
pub type SurfaceId = Uuid;

// ---------------------------------------------------------------------------
// ArmedVoice — pre-arm state
// ---------------------------------------------------------------------------

/// State for a video voice that has been pre-armed at the playhead.
///
/// The mpv instance has already received `loadfile` with `pause=yes`, the
/// named-pipe server was created beforehand, and `ConnectNamedPipe` returned
/// in the reader thread.  On GO the only remaining call is `pause=no`.
struct ArmedVoice {
    /// The VoiceId that will be used when this arm is activated on GO.
    voice_id: VoiceId,
    /// Opaque key identifying which cue owns this arm (matches the cue's UUID).
    owner_id: Uuid,
    /// Window layout to apply on activation.
    screen_index: Option<u32>,
    /// Set to `true` by the pipe thread once `ConnectNamedPipe` returns and
    /// the ring buffer consumer is installed in `AudioEngine`.
    ready: Arc<AtomicBool>,
    /// The Win32 pipe server handle — stored so `cancel_pre_arm` can force-close
    /// it and unblock a `ConnectNamedPipe` call that has not returned yet.
    pipe_handle: isize,
    /// Set to `true` by `cancel_pre_arm` before closing the handle.
    /// The pipe thread checks this at exit to avoid double-closing.
    cancelled: Arc<AtomicBool>,
    /// When `pre_arm_voice` was called — used to log how long video was paused.
    armed_at: Instant,
}

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
    #[allow(dead_code)]
    audio_engine: Arc<AudioEngine>,
    /// Pre-armed voice state, if any.  Set by `pre_arm_voice` when the
    /// playhead lands on a `VideoCue`; consumed or cancelled before GO.
    armed_voice: Arc<Mutex<Option<ArmedVoice>>>,
    /// Timestamp when `pause=no` was last sent to mpv (either from
    /// `activate_armed_voice` or the PCM pipe thread).  Shared with the mpv
    /// event loop so it can log how long after GO `MPV_EVENT_PLAYBACK_RESTART`
    /// fires — that delta is the observable startup freeze duration.
    go_sent_at: Arc<Mutex<Option<Instant>>>,
    /// Shared with the mpv event loop.  `pre_arm_voice` stores the current
    /// arm's `pipe_discard` flag here; the event loop takes it on
    /// `MPV_EVENT_PLAYBACK_RESTART` (after GO) and flips it to `false` so the
    /// pipe thread switches from drain mode to push mode atomically with the
    /// audio gate opening.
    armed_pipe_discard: Arc<Mutex<Option<Arc<AtomicBool>>>>,
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

            // --------------------------------------------------------------
            // A/V sync correction for ao=pcm.
            //
            // ao=pcm is a file-writer AO — it reports bytes as "played"
            // the instant they hit the pipe, giving mpv a zero-latency
            // (i.e. bogus) audio clock.  mpv's default sync behaviour
            // misfires in two ways:
            //
            //  1. --initial-audio-sync (default: yes) inserts silence or
            //     drops samples in the first second of playback to align
            //     audio PTS with video PTS.  With our bogus audio clock
            //     this correction fires at ~1 s and causes a ~300 ms
            //     audible dropout.  Disable it.
            //
            //  2. --video-sync=audio slaves the video clock to the AO's
            //     reported position.  Since that position races ahead of
            //     reality, video frames are released too early and mpv
            //     then stalls to catch up — visible as a first-frame
            //     freeze.  Use video-sync=desync so video runs on the
            //     display refresh clock, independent of the audio clock.
            //
            // audio-buffer=0.06 additionally caps mpv's AO pre-fill so
            // the timing mismatch has less headroom to accumulate.
            // --------------------------------------------------------------
            opt_str(&lib, ctx, "audio-buffer", "0.06");
            opt_str(&lib, ctx, "initial-audio-sync", "no");
            opt_str(&lib, ctx, "video-sync", "desync");

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
        let go_sent_at: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
        let armed_pipe_discard: Arc<Mutex<Option<Arc<AtomicBool>>>> =
            Arc::new(Mutex::new(None));

        // Spawn the mpv event thread.
        {
            let lib2      = Arc::clone(&lib);
            let ctx2      = Arc::clone(&mpv_ctx);
            let voice2    = Arc::clone(&current_voice);
            let tx2       = status_tx.clone();
            let gsa2      = Arc::clone(&go_sent_at);
            let pcm_flag  = audio_engine.video_pcm_active_flag();
            let apd2      = Arc::clone(&armed_pipe_discard);
            std::thread::Builder::new()
                .name("wincue-mpv-events".into())
                .spawn(move || mpv_event_loop(lib2, ctx2, voice2, tx2, hwnd, gsa2, pcm_flag, apd2))
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
            armed_voice: Arc::new(Mutex::new(None)),
            go_sent_at,
            armed_pipe_discard,
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
        cue_id: Option<Uuid>,
        file_path: &Path,
        _surface_id: Option<SurfaceId>,
        volume_db: f64,
        loop_count: u32,
        start_ms: Option<u64>,
        end_ms: Option<u64>,
        _fade_in: Option<&FadeSpec>,
        screen_index: Option<u32>,
    ) -> Result<VoiceId> {
        // ── Pre-arm fast path ─────────────────────────────────────────────────
        // If the cue at the playhead was pre-armed, the pipe is already connected
        // and the ring buffer consumer is installed.  The only step needed is
        // `pause=no` — no pipe race is possible.
        if let Some(owner_id) = cue_id {
            let armed_opt = self.armed_voice.lock().unwrap().take();
            if let Some(armed) = armed_opt {
                if armed.owner_id == owner_id {
                    // Wait up to 50 ms for the pipe thread to finish connecting.
                    let deadline = Instant::now() + Duration::from_millis(200);
                    while !armed.ready.load(Ordering::Acquire) {
                        if Instant::now() >= deadline {
                            log::warn!(
                                "[pre-arm] cue {owner_id}: pipe not ready after 50 ms — \
                                 falling back to fresh start"
                            );
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(2));
                    }

                    if armed.ready.load(Ordering::Acquire) {
                        return self.activate_armed_voice(armed, volume_db);
                    }
                    // Not ready — fall through; mpv gets a fresh loadfile below.
                }
                // armed.owner_id != cue_id — drop armed, mpv's new loadfile replaces it.
            }
        }
        // Cancel any residual pre-arm (defensive: different cue calling play_voice).
        // The pipe thread will detect the upcoming loadfile replace and exit.
        *self.armed_voice.lock().unwrap() = None;

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

        // Create the named-pipe server instance and spawn the per-video PCM
        // reader thread BEFORE loadfile so that ConnectNamedPipe() is already
        // blocking when mpv initialises its AO and opens the pipe as a client.
        // This eliminates the race where mpv connected before our server was
        // listening, causing zero frames to reach the ring buffer.
        let pipe_handle = unsafe { create_pipe_instance() }?;
        {
            let ae  = Arc::clone(&self.audio_engine);
            let lib = Arc::clone(&self.mpv_lib);
            let ctx = Arc::clone(&self.mpv_ctx);
            std::thread::Builder::new()
                .name("wincue-mpv-pcm".into())
                .spawn(move || handle_pcm_pipe_connection(
                    ae, pipe_handle, lib, ctx,
                    true,                                    // send_pause_no — regular play
                    Arc::new(AtomicBool::new(false)),        // ready_flag — unused here
                    Arc::new(AtomicBool::new(false)),        // cancelled — never cancelled
                    Arc::new(AtomicBool::new(false)),        // pipe_discard — push mode from start
                ))
                .map_err(|e| anyhow!("Failed to spawn PCM reader thread: {e}"))?;
        }

        self.apply_window_layout(screen_index);

        unsafe {

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
            // Pass pause=yes as a per-file option so it is applied atomically
            // as part of the new session — mpv_set_property_string cannot
            // survive the playback-state reset that `loadfile replace` performs.
            opts.push("pause=yes".to_string());

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
            (lib.mpv_set_property_string)(ctx, cs("hwdec").as_ptr(), cs("auto").as_ptr());
            (lib.mpv_set_property_string)(ctx, cs("profile").as_ptr(), cs("fast").as_ptr());
            // Re-assert ao=pcm A/V sync workarounds in case profile=fast
            // overwrote them.
            (lib.mpv_set_property_string)(ctx, cs("video-sync").as_ptr(), cs("desync").as_ptr());
            (lib.mpv_set_property_string)(ctx, cs("audio-buffer").as_ptr(), cs("0.06").as_ptr());
            (lib.mpv_set_property_string)(ctx, cs("initial-audio-sync").as_ptr(), cs("no").as_ptr());
            log::info!("[mpv] hwdec=auto profile=fast — sending loadfile (pause=yes in opts)");

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

    // ── Window layout helper ─────────────────────────────────────────────────

    /// Position and show the mpv window.
    ///
    /// `Some(idx)` moves the window to cover monitor `idx` in fullscreen mode.
    /// `None` keeps the floating window at its current position but ensures it
    /// is topmost and visible.
    fn apply_window_layout(&self, screen_index: Option<u32>) {
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                SetWindowPos, ShowWindow, HWND_TOPMOST, SW_SHOWNA,
                SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_FRAMECHANGED, SWP_NOZORDER,
            };

            if let Some(idx) = screen_index {
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
                SetWindowPos(
                    self.hwnd, HWND_TOPMOST,
                    0, 0, 0, 0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER,
                );
            }

            ShowWindow(self.hwnd, SW_SHOWNA);
            SetWindowPos(
                self.hwnd, HWND_TOPMOST,
                0, 0, 0, 0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }

    // ── Pre-arm ──────────────────────────────────────────────────────────────

    /// Activate a pre-armed voice on GO.
    ///
    /// The pipe is already connected and the ring buffer consumer is installed.
    /// We just register the voice, show the window, and send `pause=no`.
    fn activate_armed_voice(&self, armed: ArmedVoice, volume_db: f64) -> Result<VoiceId> {
        // Clear any playing voice (shouldn't be one since pre-arm only fires
        // when mpv is idle, but handle it defensively).
        if let Some(old_id) = self.current_voice.lock().unwrap().take() {
            self.voices.lock().unwrap().remove(&old_id);
            let _ = self.status_tx.send(VideoStatus::Completed { voice_id: old_id });
        }

        *self.current_voice.lock().unwrap() = Some(armed.voice_id);
        self.voices.lock().unwrap().insert(
            armed.voice_id,
            VideoVoice { id: armed.voice_id, started_at: Instant::now(), duration: None },
        );

        self.apply_window_layout(armed.screen_index);

        // Set volume at GO time (cue may have been edited since pre-arm).
        let vol_pct = (100.0 * db_to_linear(volume_db)).clamp(0.0, 1000.0);
        let vol_str = cs(&format!("{vol_pct:.2}"));

        // Ring buffer is empty: the pipe thread has been in drain mode since
        // pre-arm started, so no stale audio has accumulated.  No flush needed.

        unsafe {
            let prop = cs("volume");
            (self.mpv_lib.mpv_set_property_string)(self.mpv_ctx.0, prop.as_ptr(), vol_str.as_ptr());

            // Reset both clocks to position 0 before unpausing.
            //
            // During pre-arm, ao=pcm writes PCM to the named pipe even while
            // mpv is paused (pipe backpressure limits it to ~60ms, but the
            // audio PTS still races ahead by ~1s).  A seek to absolute 0
            // discards that pre-fill and aligns both clocks so the first
            // audible sample is presented in sync with frame 0.
            let seek_cmd   = cs("seek");
            let seek_pos   = cs("0");
            let seek_flags = cs("absolute+exact");
            let seek_args: [*const std::ffi::c_char; 4] = [
                seek_cmd.as_ptr(), seek_pos.as_ptr(), seek_flags.as_ptr(), std::ptr::null(),
            ];
            (self.mpv_lib.mpv_command)(self.mpv_ctx.0, seek_args.as_ptr());

            // Diagnostic snapshot just before pause=no — these values confirm
            // whether Fix 1 (cache=no / demuxer-readahead-secs=0) reduced the
            // pre-arm pre-fill.  audio-pts should be ~0 if it worked.
            let mut time_pos: f64 = -1.0;
            if (self.mpv_lib.mpv_get_property)(
                self.mpv_ctx.0, cs("time-pos").as_ptr(),
                MPV_FORMAT_DOUBLE, &mut time_pos as *mut f64 as *mut c_void,
            ) == 0 {
                log::info!("[diag] time-pos={time_pos:.3}s (before pause=no)");
            }
            let mut audio_pts: f64 = -1.0;
            if (self.mpv_lib.mpv_get_property)(
                self.mpv_ctx.0, cs("audio-pts").as_ptr(),
                MPV_FORMAT_DOUBLE, &mut audio_pts as *mut f64 as *mut c_void,
            ) == 0 {
                log::info!("[diag] audio-pts={audio_pts:.3}s (before pause=no)");
            }
            let mut cache_dur: f64 = -1.0;
            if (self.mpv_lib.mpv_get_property)(
                self.mpv_ctx.0, cs("demuxer-cache-duration").as_ptr(),
                MPV_FORMAT_DOUBLE, &mut cache_dur as *mut f64 as *mut c_void,
            ) == 0 {
                log::info!("[diag] demuxer-cache-duration={cache_dur:.3}s (before pause=no)");
            }

            // Unpause — ring buffer consumer already installed; audio starts immediately.
            (self.mpv_lib.mpv_set_property_string)(
                self.mpv_ctx.0, cs("pause").as_ptr(), cs("no").as_ptr(),
            );
        }
        // Record when pause=no was sent so the event loop can compute the
        // delta to MPV_EVENT_PLAYBACK_RESTART (= visible freeze duration).
        *self.go_sent_at.lock().unwrap() = Some(Instant::now());

        log::info!(
            "[pre-arm] GO: activated voice {} for cue {} \
             (pre-armed for {}ms)",
            armed.voice_id, armed.owner_id,
            armed.armed_at.elapsed().as_millis(),
        );

        // Diagnostic properties queried immediately after pause=no.
        unsafe {
            let ctx = self.mpv_ctx.0;
            let lib = &self.mpv_lib;

            // pause: confirm mpv was actually paused at the moment GO fired.
            // If this reads 0 (not paused), audio was leaking during pre-arm.
            let mut was_paused: i64 = 0;
            if (lib.mpv_get_property)(
                ctx, cs("pause").as_ptr(),
                MPV_FORMAT_FLAG, &mut was_paused as *mut i64 as *mut c_void,
            ) == 0 {
                log::info!("[diag] pause={was_paused} at GO time (1=was paused, 0=was playing)");
            }

            // paused-for-cache: 1 means mpv is stalled waiting for the
            // demuxer cache to fill — the video will not advance until it
            // clears.  Seeing 1 here would explain a startup freeze.
            let mut paused_cache: i64 = 0;
            if (lib.mpv_get_property)(
                ctx, cs("paused-for-cache").as_ptr(),
                MPV_FORMAT_FLAG, &mut paused_cache as *mut i64 as *mut c_void,
            ) == 0 {
                log::info!("[diag] paused-for-cache={paused_cache}");
            }

            // demuxer-cache-duration: how many seconds of video are buffered.
            let mut cache_dur: f64 = 0.0;
            if (lib.mpv_get_property)(
                ctx, cs("demuxer-cache-duration").as_ptr(),
                MPV_FORMAT_DOUBLE, &mut cache_dur as *mut f64 as *mut c_void,
            ) == 0 {
                log::info!("[diag] demuxer-cache-duration={cache_dur:.3}s");
            }

            // time-pos: video position in seconds at the moment of GO.
            let mut time_pos: f64 = 0.0;
            if (lib.mpv_get_property)(
                ctx, cs("time-pos").as_ptr(),
                MPV_FORMAT_DOUBLE, &mut time_pos as *mut f64 as *mut c_void,
            ) == 0 {
                log::info!("[diag] time-pos={time_pos:.3}s at GO");
            }

            // audio-pts: the audio clock position — if different from
            // time-pos, mpv will try to compensate on resume.
            let mut audio_pts: f64 = 0.0;
            if (lib.mpv_get_property)(
                ctx, cs("audio-pts").as_ptr(),
                MPV_FORMAT_DOUBLE, &mut audio_pts as *mut f64 as *mut c_void,
            ) == 0 {
                log::info!("[diag] audio-pts={audio_pts:.3}s at GO (diff={:.3}s)",
                    time_pos - audio_pts);
            }

            // video-sync: confirm profile=fast did not override our setting.
            let mut vs_val = std::ptr::null_mut::<std::ffi::c_char>();
            if (lib.mpv_get_property)(
                ctx, cs("video-sync").as_ptr(),
                super::mpv_sys::MPV_FORMAT_STRING,
                &mut vs_val as *mut *mut std::ffi::c_char as *mut c_void,
            ) == 0 && !vs_val.is_null() {
                let vs = std::ffi::CStr::from_ptr(vs_val).to_string_lossy().into_owned();
                (lib.mpv_free)(vs_val as *mut c_void);
                log::info!("[diag] video-sync={vs} at GO");
            }
        }

        Ok(armed.voice_id)
    }

    /// Pre-arm a `VideoCue` for instant GO.
    ///
    /// Sends `loadfile` with `pause=yes` and pre-connects the PCM named pipe
    /// so that when GO is pressed the ring buffer consumer is already installed
    /// in `AudioEngine` — `pause=no` is the only remaining step.
    ///
    /// This is a no-op when a video is already playing (single mpv context).
    /// Calling it again for a different cue cancels the previous arm first.
    pub fn pre_arm_voice(
        &self,
        owner_id: Uuid,
        file_path: &Path,
        _surface_id: Option<SurfaceId>,
        volume_db: f64,
        loop_count: u32,
        start_ms: Option<u64>,
        end_ms: Option<u64>,
        _fade_in: Option<&FadeSpec>,
        screen_index: Option<u32>,
    ) -> Result<()> {
        // Only one mpv context — do not pre-arm while another video is playing.
        if self.current_voice.lock().unwrap().is_some() {
            return Ok(());
        }

        // Clear the GO timestamp so PLAYBACK_RESTART events that fire during
        // pre-arm (mpv fires one when the first frame is decoded, even while
        // paused) do not open the audio gate prematurely.  The gate must only
        // open on PLAYBACK_RESTART that follows the actual pause=no at GO time.
        *self.go_sent_at.lock().unwrap() = None;

        // Cancel any previous pre-arm.
        self.cancel_pre_arm();

        let voice_id = Uuid::new_v4();
        let ready     = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::new(AtomicBool::new(false));
        // Start in drain mode: pipe thread reads from OS pipe but discards
        // bytes so the ring buffer stays empty during pre-arm.  Flipped to
        // false by the mpv event loop on PLAYBACK_RESTART after GO.
        let pipe_discard = Arc::new(AtomicBool::new(true));

        let path_str = file_path.to_string_lossy().replace('\\', "/");
        let path_cstr = CString::new(path_str.as_str())
            .map_err(|_| anyhow!("File path contains NUL byte"))?;

        // Publish the pipe_discard flag so the mpv event loop can flip it on
        // PLAYBACK_RESTART.  This must happen before loadfile (the event loop
        // may fire PLAYBACK_RESTART very quickly for cached files).
        *self.armed_pipe_discard.lock().unwrap() = Some(Arc::clone(&pipe_discard));

        // Create the pipe server before loadfile so ConnectNamedPipe is already
        // blocking when mpv initialises its AO and opens the pipe.
        let pipe_handle = unsafe { create_pipe_instance() }?;

        // Store armed state BEFORE spawning the thread (thread may read it
        // from ready_flag before this function returns).
        *self.armed_voice.lock().unwrap() = Some(ArmedVoice {
            voice_id,
            owner_id,
            screen_index,
            ready: Arc::clone(&ready),
            pipe_handle,
            cancelled: Arc::clone(&cancelled),
            armed_at: Instant::now(),
        });

        {
            let ae  = Arc::clone(&self.audio_engine);
            let lib = Arc::clone(&self.mpv_lib);
            let ctx = Arc::clone(&self.mpv_ctx);
            let flag_r = Arc::clone(&ready);
            let flag_c = Arc::clone(&cancelled);
            let flag_d = Arc::clone(&pipe_discard);
            std::thread::Builder::new()
                .name("wincue-mpv-pcm-prearm".into())
                .spawn(move || handle_pcm_pipe_connection(
                    ae, pipe_handle, lib, ctx,
                    false,   // send_pause_no: false — GO will send pause=no
                    flag_r,  // set to true after ring buffer installed
                    flag_c,  // cancellation flag
                    flag_d,  // pipe discard mode flag (true = drain, false = push)
                ))
                .map_err(|e| anyhow!("Failed to spawn pre-arm PCM thread: {e}"))?;
        }

        let ctx = self.mpv_ctx.0;
        let lib = &self.mpv_lib;

        // Build per-file options (same as play_voice).
        let mut opts: Vec<String> = Vec::new();
        if let Some(start) = start_ms {
            opts.push(format!("start={:.3}", start as f64 / 1000.0));
        }
        if let Some(end) = end_ms {
            opts.push(format!("end={:.3}", end as f64 / 1000.0));
        }
        let loop_val = if loop_count == u32::MAX {
            "inf".to_string()
        } else if loop_count == 0 {
            "no".to_string()
        } else {
            loop_count.to_string()
        };
        opts.push(format!("loop-file={loop_val}"));
        // pause=yes: atomic per-file option — survives the loadfile replace
        // state reset that runtime property writes cannot survive.
        opts.push("pause=yes".to_string());
        // Note: cache=no / demuxer-readahead-secs=0 were previously added here
        // to prevent ao=pcm pre-fill during pre-arm, but they caused mpv to run
        // dry during playback (stalls, A/V desync warning).  The pipe drain mode
        // (pipe_discard=true) now handles pre-arm audio isolation without
        // restricting the demuxer — so these options are no longer needed.

        let opts_str     = opts.join(",");
        let opts_cstr    = cs(&opts_str);
        let cmd_cstr     = cs("loadfile");
        let replace_cstr = cs("replace");
        let index_cstr   = cs("0");
        let args: [*const std::ffi::c_char; 6] = [
            cmd_cstr.as_ptr(), path_cstr.as_ptr(), replace_cstr.as_ptr(),
            index_cstr.as_ptr(), opts_cstr.as_ptr(), std::ptr::null(),
        ];

        unsafe {
            // Volume is set at GO time (activate_armed_voice) to pick up
            // any inspector changes made between pre-arm and GO.
            let vol_pct = (100.0 * db_to_linear(volume_db)).clamp(0.0, 1000.0);
            let vol_str = cs(&format!("{vol_pct:.2}"));
            let prop_vol = cs("volume");
            (lib.mpv_set_property_string)(ctx, prop_vol.as_ptr(), vol_str.as_ptr());

            (lib.mpv_set_property_string)(ctx, cs("hwdec").as_ptr(), cs("auto").as_ptr());
            (lib.mpv_set_property_string)(ctx, cs("profile").as_ptr(), cs("fast").as_ptr());
            // Re-assert A/V sync settings in case profile=fast overwrote them.
            (lib.mpv_set_property_string)(ctx, cs("video-sync").as_ptr(), cs("desync").as_ptr());
            (lib.mpv_set_property_string)(ctx, cs("audio-buffer").as_ptr(), cs("0.06").as_ptr());
            (lib.mpv_set_property_string)(ctx, cs("initial-audio-sync").as_ptr(), cs("no").as_ptr());

            let ret = (lib.mpv_command)(ctx, args.as_ptr());
            if ret < 0 {
                let err_cstr = (lib.mpv_error_string)(ret);
                let err_msg = std::ffi::CStr::from_ptr(err_cstr).to_string_lossy();
                *self.armed_voice.lock().unwrap() = None;
                return Err(anyhow!("pre_arm loadfile failed (code {ret}): {err_msg}"));
            }
        }
        log::info!("[pre-arm] loadfile sent for cue {owner_id}: {path_str} opts=[{opts_str}]");

        // --------------------------------------------------------------------
        // D3D11 swap-chain warmup.
        //
        // Apply the window layout NOW so mpv's first decoded frame is
        // presented to a visible, composited window during pre-arm — not at
        // GO.  Without this, the first Present() call happens the instant
        // `pause=no` fires, and the D3D11 pipeline / DWM composition /
        // shader compile costs land on frame 1-2, producing the classic
        // "freeze on an early frame, unfreeze, play" hiccup.
        //
        // Show-control convention (QLab et al.): an armed video cue is
        // expected to display its first frame on the output so the operator
        // can see what is queued.  `cancel_pre_arm` hides the window again
        // when the playhead moves off the cue.
        // --------------------------------------------------------------------
        self.apply_window_layout(screen_index);

        Ok(())
    }

    /// Cancel any pre-armed voice.
    ///
    /// Closes the pipe server handle (unblocking any pending `ConnectNamedPipe`
    /// call) and sends `stop` to mpv so the AO closes.  The pipe thread detects
    /// the error and exits without double-closing the handle.
    pub fn cancel_pre_arm(&self) {
        if let Some(a) = self.armed_voice.lock().unwrap().take() {
            log::info!("[pre-arm] cancelling pre-arm for cue {}", a.owner_id);
            // Signal the pipe thread that the handle is being closed externally.
            a.cancelled.store(true, Ordering::Release);
            // Clear the ring buffer consumer before the thread might install it
            // (best-effort; the thread will clear it again on exit — no harm).
            log::info!("[teardown] 1 — sending stop to mpv");
            unsafe {
                // Tell mpv to stop, which closes the AO connection.
                let stop = cs("stop");
                let args: [*const std::ffi::c_char; 2] = [stop.as_ptr(), std::ptr::null()];
                (self.mpv_lib.mpv_command)(self.mpv_ctx.0, args.as_ptr());
            }
            log::info!("[teardown] 2 — stop sent");
            log::info!("[teardown] 3 — closing pipe");
            unsafe {
                // Force-close the handle to unblock ConnectNamedPipe immediately.
                CloseHandle(a.pipe_handle);
            }
            log::info!("[teardown] 4 — pipe closed");
            log::info!("[teardown] 5 — dropping ring buffer");
            self.audio_engine.set_video_pcm_consumer(None);
            log::info!("[teardown] 6 — ring buffer dropped");
            log::info!("[teardown] 7 — joining pipe reader thread");
            // (pipe reader thread runs independently; we cannot join it here —
            //  the cancelled flag above causes it to exit on its own)
            log::info!("[teardown] 8 — pipe reader thread joined");

            // Hide the warmup window so the output monitor goes black now
            // that the playhead has moved off the armed cue.  pre_arm_voice
            // showed it to warm up the D3D11 swap chain.
            unsafe {
                use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
                ShowWindow(self.hwnd, SW_HIDE);
            }

            log::info!("[teardown] 9 — teardown complete");
        }
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
    go_sent_at: Arc<Mutex<Option<Instant>>>,
    video_pcm_active: Arc<std::sync::atomic::AtomicBool>,
    pipe_discard_flag: Arc<Mutex<Option<Arc<AtomicBool>>>>,
) {
    loop {
        let event = unsafe { (lib.mpv_wait_event)(ctx.0, 2.0) };
        if event.is_null() {
            continue;
        }
        let event_id = unsafe { (*event).event_id };

        match event_id {
            MPV_EVENT_SHUTDOWN => break,

            MPV_EVENT_START_FILE => {
                log::info!("[mpv] MPV_EVENT_START_FILE");
            }

            MPV_EVENT_SEEK => {
                log::info!("[mpv] MPV_EVENT_SEEK");
            }

            MPV_EVENT_VIDEO_RECONFIG => {
                log::info!("[mpv] MPV_EVENT_VIDEO_RECONFIG (video output reconfigured)");
            }

            // Fires when mpv finishes seeking/loading and begins presenting
            // frames — including once during pre-arm when the first frame is
            // decoded (even while paused).  We must NOT open the audio gate
            // for that pre-arm event; only activate on the PLAYBACK_RESTART
            // that follows the actual `pause=no` sent at GO time.
            // `go_sent_at` is None during pre-arm and set to Some(Instant)
            // immediately after `pause=no` in `activate_armed_voice`.
            MPV_EVENT_PLAYBACK_RESTART => {
                let go_time = *go_sent_at.lock().unwrap();
                if let Some(t) = go_time {
                    let ms = t.elapsed().as_millis();
                    // Switch pipe thread from drain → push mode first so that
                    // when the audio gate opens the ring buffer already has
                    // fresh post-seek samples ready to mix.
                    if let Some(flag) = pipe_discard_flag.lock().unwrap().take() {
                        flag.store(false, std::sync::atomic::Ordering::Release);
                        log::info!("[mpv] pipe_discard → false (pipe thread now pushing to ring buffer)");
                    }
                    // Open the gate: the audio callback may now mix video PCM.
                    video_pcm_active.store(true, std::sync::atomic::Ordering::Release);
                    log::info!(
                        "[mpv] MPV_EVENT_PLAYBACK_RESTART — {ms}ms after pause=no \
                         (video PCM mixing activated)"
                    );
                } else {
                    log::info!(
                        "[mpv] MPV_EVENT_PLAYBACK_RESTART during pre-arm \
                         (gate stays closed — expected)"
                    );
                }
            }

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

                // Do NOT send pause=no here.  The pipe reader thread sends it
                // immediately after ConnectNamedPipe() returns and the ring
                // buffer is created — that is the only safe moment, because
                // FILE_LOADED fires before the pipe handshake completes.

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
                let data_ptr = unsafe { (*event).data };
                if let Some(end_data) =
                    unsafe { (data_ptr as *mut MpvEventEndFile).as_ref() }
                {
                    use crate::engine::mpv_sys::{
                        MPV_END_FILE_REASON_STOP, MPV_END_FILE_REASON_QUIT,
                    };
                    let reason_name = match end_data.reason {
                        MPV_END_FILE_REASON_EOF   => "EOF",
                        MPV_END_FILE_REASON_STOP  => "STOP",
                        MPV_END_FILE_REASON_QUIT  => "QUIT",
                        MPV_END_FILE_REASON_ERROR => "ERROR",
                        _                          => "UNKNOWN",
                    };
                    log::info!(
                        "[mpv] MPV_EVENT_END_FILE reason={reason_name} ({})",
                        end_data.reason
                    );

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

// Windows named-pipe and file-I/O APIs (kernel32.dll, always linked).
// Declared at module level so both `create_pipe_instance` and
// `handle_pcm_pipe_connection` can use them.
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
const PIPE_TYPE_BYTE: u32     = 0x0000_0000;
const PIPE_READMODE_BYTE: u32 = 0x0000_0000;
const PIPE_WAIT: u32          = 0x0000_0000;
const PIPE_UNLIMITED_INSTANCES: u32 = 255;
const INVALID_HANDLE_VALUE: isize   = -1_isize;

/// Create a server-side `\\.\pipe\wincue-mpv-audio` instance.
///
/// Called by [`VideoEngine::play_voice`] **before** `loadfile` so that
/// [`ConnectNamedPipe`] is already blocking in the reader thread when mpv
/// initialises its AO and opens the pipe as a client.  This eliminates the
/// race where mpv connected before our server was ready, causing the entire
/// PCM stream to be lost.
///
/// Returns the pipe HANDLE, or an error if `CreateNamedPipeW` fails.
///
/// # Safety
/// Calls `CreateNamedPipeW`.
unsafe fn create_pipe_instance() -> Result<isize> {
    let pipe_name: Vec<u16> = r"\\.\pipe\wincue-mpv-audio"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let handle = CreateNamedPipeW(
        pipe_name.as_ptr(),
        PIPE_ACCESS_INBOUND,
        PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
        PIPE_UNLIMITED_INSTANCES,
        0,      // out-buffer (server-to-client; unused for inbound)
        65536,  // in-buffer (client-to-server; receives mpv PCM)
        0,      // default timeout
        std::ptr::null(),
    );

    if handle == INVALID_HANDLE_VALUE {
        return Err(anyhow!("CreateNamedPipeW failed"));
    }
    Ok(handle)
}

/// Block on [`ConnectNamedPipe`], then stream PCM from mpv into
/// [`AudioEngine`]'s ring buffer until the pipe closes.
///
/// Spawned by both [`VideoEngine::play_voice`] and
/// [`VideoEngine::pre_arm_voice`].  The thread exits when mpv closes the
/// pipe (stop, EOF, or external cancellation via `cancel_pre_arm`).
///
/// ## Parameters
///
/// * `send_pause_no` — when `true` (regular play), sends `pause=no`
///   immediately after the ring buffer is installed.  When `false`
///   (pre-arm), omits the send; GO will call `pause=no` directly once it
///   decides to activate the armed voice.
/// * `ready_flag`   — set to `true` after ring buffer is installed so
///   `play_voice`'s pre-arm check knows the arm is ready for instant GO.
/// * `cancelled`    — set to `true` by `cancel_pre_arm` before the pipe
///   handle is force-closed.  The thread uses this to skip the final
///   `DisconnectNamedPipe` / `CloseHandle` (handle is already invalid).
///
/// ## Rate control
///
/// mpv with `ao=pcm` writes PCM as fast as decoding allows.  This thread
/// throttles itself via pipe backpressure: when the ring buffer exceeds
/// `max_prebuffer` samples it sleeps 1 ms, causing mpv's writes to block.
fn handle_pcm_pipe_connection(
    audio_engine: Arc<AudioEngine>,
    handle: isize,
    mpv_lib: Arc<MpvLib>,
    mpv_ctx: Arc<MpvCtx>,
    send_pause_no: bool,
    ready_flag: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
    pipe_discard: Arc<AtomicBool>,
) {
    // ~60 ms of stereo — throttle threshold for backpressure.
    //
    // This must stay tight: every sample sitting in the ring buffer is a
    // sample of added audio latency relative to what mpv considers "played"
    // (ao=pcm reports bytes as played the instant they hit the pipe).  60 ms
    // matches the mpv `--audio-buffer=0.06` option set at init so the mpv-
    // side audio clock and our actual audible latency agree, keeping A/V in
    // lock-step.  Value is in f32 samples (stereo → frames × 2).
    let max_prebuffer: usize =
        ((audio_engine.sample_rate() as usize) * 60 / 1000) * 2;

    // Block until mpv opens the pipe as a client.
    log::info!("PCM pipe: waiting for mpv to connect...");
    unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) };

    // If `cancel_pre_arm` closed the handle, exit without touching it again.
    if cancelled.load(Ordering::Acquire) {
        log::info!("PCM pipe: cancelled — handle closed externally, exiting");
        return;
    }

    log::info!("PCM pipe: mpv connected — creating ring buffer");

    // Fresh ring buffer: 3 s of stereo f32 so burst-writes never overflow.
    let ring_size = (audio_engine.sample_rate() as usize * 2 * 3).max(16384);
    let (mut prod, cons) = HeapRb::<f32>::new(ring_size).split();
    audio_engine.set_video_pcm_consumer(Some(cons));

    // Signal that the arm is ready (play_voice's pre-arm check polls this).
    ready_flag.store(true, Ordering::Release);

    if send_pause_no {
        // Regular play path: ring buffer consumer is now installed.  Unpause
        // so mpv starts writing PCM — the only safe moment; the consumer is
        // ready and zero frames will be lost before it is read.
        unsafe {
            (mpv_lib.mpv_set_property_string)(
                mpv_ctx.0, cs("pause").as_ptr(), cs("no").as_ptr(),
            );
        }
        log::info!("PCM pipe: ring buffer ready — pause=no sent");
    } else {
        // Pre-arm path: stay paused.  GO will send pause=no via
        // `activate_armed_voice` once it decides to use this armed voice.
        log::info!("PCM pipe: pre-arm ready — ring buffer installed, waiting for GO");
    }

    let mut raw = [0u8; 4096];
    let mut samples_pushed: u64 = 0;
    let mut was_discarding = true; // track transitions for logging
    loop {
        let discarding = pipe_discard.load(Ordering::Acquire);

        // Log the drain→push transition once so the operator can confirm timing.
        if was_discarding && !discarding {
            log::info!("PCM pipe: discard mode OFF — pushing audio to ring buffer");
            was_discarding = false;
        }

        // Backpressure: only needed in push mode so the ring buffer never overflows.
        if !discarding && prod.occupied_len() > max_prebuffer {
            std::thread::sleep(Duration::from_millis(1));
            continue;
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
            break; // Pipe closed by mpv (stop, EOF, or cancel).
        }

        if discarding {
            // Drain mode: consume OS pipe bytes so ao=pcm never blocks, but
            // do not push to the ring buffer — no stale audio accumulates.
            continue;
        }

        for chunk in raw[..bytes_read as usize].chunks_exact(4) {
            // SAFETY: chunks_exact(4) guarantees exactly 4 bytes.
            let sample = f32::from_le_bytes(chunk.try_into().unwrap());
            let _ = prod.try_push(sample);
            samples_pushed += 1;
        }
    }

    let sample_rate = 48_000u64; // approximate — engine rate not available here
    log::info!(
        "PCM pipe: mpv disconnected — {samples_pushed} samples written total \
         ({:.1}ms @ ~48kHz stereo) — clearing video PCM consumer",
        samples_pushed as f64 / 2.0 / sample_rate as f64 * 1000.0,
    );
    audio_engine.set_video_pcm_consumer(None);

    // Only close the handle if we own it; cancel_pre_arm already closed it
    // (and set `cancelled`) when it wanted to force-unblock this thread.
    if !cancelled.load(Ordering::Acquire) {
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
