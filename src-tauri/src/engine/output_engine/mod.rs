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
mod pcm_pipe;
mod types;
mod win32_window;

pub use types::{OutputStatus, OutputSurface, ScreenInfo, SurfaceId, VoiceId};
use types::{
    ArmedVoice, FadeAnimState, FadePending, FadePendingParams, MpvCtx, OutputVoice,
    OutputWndState,
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
/// `true` = discard PCM samples (pre-arm / idle / image), `false` = push to ring buffer.
pub(super) static OUTPUT_PCM_DISCARD: OnceLock<Arc<AtomicBool>> = OnceLock::new();

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// `WM_APP + 1`: posted by the mpv event thread after `MPV_EVENT_FILE_LOADED`.
pub(super) const WM_SETUP_MPV_CHILD: u32 = 0x8001;
/// `WM_APP + 2`: posted by show_content/stop_content to start the fade timer.
pub(super) const WM_DO_FADE: u32 = 0x8002;
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
    armed_voice: Arc<Mutex<Option<ArmedVoice>>>,
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
            opt_str(&lib, ctx, "force-window", "immediate");
            opt_str(&lib, ctx, "hwdec", "no");

            opt_str(&lib, ctx, "osc", "no");
            opt_str(&lib, ctx, "osd-level", "0");
            opt_str(&lib, ctx, "input-default-bindings", "no");
            opt_str(&lib, ctx, "input-vo-keyboard", "no");
            opt_str(&lib, ctx, "input-cursor", "no");

            opt_str(&lib, ctx, "keep-open", "no");
            opt_str(&lib, ctx, "idle", "yes");

            opt_str(&lib, ctx, "ao", "pcm");
            opt_str(&lib, ctx, "ao-pcm-file", r"\\.\pipe\wincue-mpv-audio");
            opt_str(&lib, ctx, "ao-pcm-waveheader", "no");
            let sr_str = audio_engine.sample_rate().to_string();
            opt_str(&lib, ctx, "audio-samplerate", &sr_str);
            opt_str(&lib, ctx, "audio-channels", "stereo");
            opt_str(&lib, ctx, "audio-format", "float");

            opt_str(&lib, ctx, "audio-buffer", "0.06");
            opt_str(&lib, ctx, "initial-audio-sync", "no");
            opt_str(&lib, ctx, "video-sync", "desync");

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

        OUTPUT_MPV_CTX.get_or_init(|| Arc::clone(&mpv_ctx));
        OUTPUT_MPV_LIB.get_or_init(|| Arc::clone(&lib));
        OUTPUT_STATUS_TX.get_or_init(|| status_tx.clone());
        OUTPUT_CURRENT_VOICE.get_or_init(|| Mutex::new(None));
        OUTPUT_CURRENT_FADE_OUT_MS.get_or_init(|| Mutex::new(0));
        FADE_STATE.get_or_init(|| Mutex::new(FadeAnimState::idle()));

        let pcm_discard = Arc::new(AtomicBool::new(true));
        OUTPUT_PCM_DISCARD.get_or_init(|| Arc::clone(&pcm_discard));

        {
            let ae = Arc::clone(&audio_engine);
            let d  = Arc::clone(&pcm_discard);
            std::thread::Builder::new()
                .name("wincue-output-pcm".into())
                .spawn(move || pcm_pipe::pcm_pipe_manager(ae, d))
                .map_err(|e| anyhow!("Failed to spawn PCM pipe manager: {e}"))?;
        }

        {
            let lib2     = Arc::clone(&lib);
            let ctx2     = Arc::clone(&mpv_ctx);
            let voice2   = Arc::clone(&current_voice);
            let tx2      = status_tx.clone();
            let gsa2     = Arc::clone(&go_sent_at);
            let pcm_flag = audio_engine.video_pcm_active_flag();
            std::thread::Builder::new()
                .name("wincue-output-mpv-events".into())
                .spawn(move || {
                    mpv_events::mpv_event_loop(lib2, ctx2, voice2, tx2, hwnd, gsa2, pcm_flag)
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
            armed_voice: Arc::new(Mutex::new(None)),
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
        cue_id: Option<Uuid>,
        file_path: &Path,
        is_image: bool,
        fade_in_ms: u32,
        this_fade_out_ms: u32,
        volume_db: f64,
        loop_count: u32,
        start_ms: Option<u64>,
        end_ms: Option<u64>,
        screen_index: Option<u32>,
    ) -> Result<VoiceId> {
        // ── Pre-arm fast path (video only) ───────────────────────────────────
        if !is_image {
            if let Some(owner_id) = cue_id {
                let armed_opt = self.armed_voice.lock().unwrap().take();
                if let Some(armed) = armed_opt {
                    if armed.owner_id == owner_id {
                        if let Some(m) = OUTPUT_CURRENT_FADE_OUT_MS.get() {
                            *m.lock().unwrap() = this_fade_out_ms;
                        }
                        return self.activate_armed_voice(
                            armed, volume_db, screen_index, fade_in_ms,
                        );
                    }
                    log::info!("[pre-arm] stale pre-arm discarded (cue mismatch)");
                    self.cancel_pre_arm_inner();
                }
            }
            *self.armed_voice.lock().unwrap() = None;
        }

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

        if !is_image {
            *self.go_sent_at.lock().unwrap() = Some(Instant::now());
            if let Some(d) = OUTPUT_PCM_DISCARD.get() {
                d.store(false, Ordering::Release);
            }
        }

        let path_str = file_path.to_string_lossy().replace('\\', "/");

        self.position_window(screen_index);

        let params = FadePendingParams {
            path: path_str,
            is_image,
            voice_id,
            fade_in_ms,
            volume_db,
            loop_count,
            start_ms,
            end_ms,
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
            fade::execute_load_params(&params, &self.mpv_lib, self.mpv_ctx.0);
            if fade_in_ms > 0 {
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
        }

        Ok(voice_id)
    }

    /// Stop the content identified by `voice_id` with an optional fade-to-black.
    pub fn stop_content(&self, voice_id: VoiceId, fade_out_ms: u32) {
        {
            let mut cv = self.current_voice.lock().unwrap();
            if *cv == Some(voice_id) {
                *cv = None;
            }
        }
        if let Some(cv) = OUTPUT_CURRENT_VOICE.get() {
            let mut cv_lock = cv.lock().unwrap();
            if *cv_lock == Some(voice_id) {
                *cv_lock = None;
            }
        }
        self.voices.lock().unwrap().remove(&voice_id);

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

        if let Some(d) = OUTPUT_PCM_DISCARD.get() {
            d.store(true, Ordering::Release);
        }
        self.audio_engine.video_pcm_active_flag().store(false, Ordering::Release);
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
        self.show_content(
            cue_id, file_path, false,
            0, 0, volume_db, loop_count, start_ms, end_ms, screen_index,
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

    /// Pause the given voice.
    pub fn pause_voice(&self, _voice_id: VoiceId) -> Result<()> {
        unsafe {
            (self.mpv_lib.mpv_set_property_string)(
                self.mpv_ctx.0, cs("pause").as_ptr(), cs("yes").as_ptr(),
            );
        }
        Ok(())
    }

    /// Resume a paused voice.
    pub fn resume_voice(&self, _voice_id: VoiceId) -> Result<()> {
        unsafe {
            (self.mpv_lib.mpv_set_property_string)(
                self.mpv_ctx.0, cs("pause").as_ptr(), cs("no").as_ptr(),
            );
        }
        Ok(())
    }

    /// Update the playback volume of a running voice.
    pub fn set_voice_volume(&self, _voice_id: VoiceId, volume_db: f64) -> Result<()> {
        unsafe {
            let vol_pct = (100.0 * db_to_linear(volume_db)).clamp(0.0, 1000.0);
            let val = cs(&format!("{vol_pct:.2}"));
            (self.mpv_lib.mpv_set_property_string)(
                self.mpv_ctx.0, cs("volume").as_ptr(), val.as_ptr(),
            );
        }
        Ok(())
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

    // ── Fullscreen ────────────────────────────────────────────────────────────

    /// Toggle the output window between windowed and true fullscreen.
    pub fn toggle_fullscreen(&self) {
        if let Some(state_mutex) = OUTPUT_WND_STATE.get() {
            if let Ok(mut state) = state_mutex.lock() {
                win32_window::toggle_fullscreen_impl(self.hwnd, &mut state);
            }
        }
    }

    // ── Pre-arm (video only) ─────────────────────────────────────────────────

    /// Pre-arm a video cue for instant GO.
    #[allow(clippy::too_many_arguments)]
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
        if self.current_voice.lock().unwrap().is_some() {
            return Ok(());
        }

        *self.go_sent_at.lock().unwrap() = None;
        self.cancel_pre_arm();

        if let Some(d) = OUTPUT_PCM_DISCARD.get() {
            d.store(true, Ordering::Release);
        }
        fade::set_overlay_alpha(255);

        let voice_id = Uuid::new_v4();
        let path_str = file_path.to_string_lossy().replace('\\', "/");
        let path_cstr = CString::new(path_str.as_str())
            .map_err(|_| anyhow!("File path contains NUL byte"))?;

        *self.armed_voice.lock().unwrap() = Some(ArmedVoice {
            voice_id,
            owner_id,
            armed_at: Instant::now(),
        });

        let ctx = self.mpv_ctx.0;
        let lib = &self.mpv_lib;

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
        opts.push("pause=yes".to_string());

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
            let vol_pct = (100.0 * db_to_linear(volume_db)).clamp(0.0, 1000.0);
            let vol_str = cs(&format!("{vol_pct:.2}"));
            (lib.mpv_set_property_string)(ctx, cs("volume").as_ptr(), vol_str.as_ptr());
            (lib.mpv_set_property_string)(ctx, cs("hwdec").as_ptr(), cs("auto").as_ptr());
            (lib.mpv_set_property_string)(ctx, cs("profile").as_ptr(), cs("fast").as_ptr());
            (lib.mpv_set_property_string)(
                ctx, cs("video-sync").as_ptr(), cs("desync").as_ptr(),
            );
            (lib.mpv_set_property_string)(
                ctx, cs("audio-buffer").as_ptr(), cs("0.06").as_ptr(),
            );
            (lib.mpv_set_property_string)(
                ctx, cs("initial-audio-sync").as_ptr(), cs("no").as_ptr(),
            );

            let ret = (lib.mpv_command)(ctx, args.as_ptr());
            if ret < 0 {
                let err_cstr = (lib.mpv_error_string)(ret);
                let err_msg = std::ffi::CStr::from_ptr(err_cstr).to_string_lossy();
                *self.armed_voice.lock().unwrap() = None;
                return Err(anyhow!("pre_arm loadfile failed (code {ret}): {err_msg}"));
            }
        }

        log::info!(
            "[pre-arm] loadfile sent for cue {owner_id}: {path_str} opts=[{opts_str}]"
        );

        self.position_window(screen_index);

        Ok(())
    }

    /// Cancel any pre-armed voice.
    pub fn cancel_pre_arm(&self) {
        self.cancel_pre_arm_inner();
    }

    fn cancel_pre_arm_inner(&self) {
        if let Some(a) = self.armed_voice.lock().unwrap().take() {
            log::info!("[pre-arm] cancelling pre-arm for cue {}", a.owner_id);
            unsafe {
                let stop = cs("stop");
                let args: [*const std::ffi::c_char; 2] =
                    [stop.as_ptr(), std::ptr::null()];
                (self.mpv_lib.mpv_command)(self.mpv_ctx.0, args.as_ptr());
            }
            fade::set_overlay_alpha(0);
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
                SetWindowPos, HWND_TOPMOST,
                SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_FRAMECHANGED, SWP_NOZORDER,
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
            } else {
                if let Some(state_mutex) = OUTPUT_WND_STATE.get() {
                    if let Ok(mut state) = state_mutex.lock() {
                        if state.is_fullscreen {
                            let (l, t, r, b) = state.saved_rect;
                            win32_window::set_resizable(self.hwnd);
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
        }
    }

    fn activate_armed_voice(
        &self,
        armed: ArmedVoice,
        volume_db: f64,
        screen_index: Option<u32>,
        fade_in_ms: u32,
    ) -> Result<VoiceId> {
        if let Some(old_id) = self.current_voice.lock().unwrap().take() {
            self.voices.lock().unwrap().remove(&old_id);
            let _ = self.status_tx.send(OutputStatus::Completed { voice_id: old_id });
        }

        *self.current_voice.lock().unwrap() = Some(armed.voice_id);
        self.voices.lock().unwrap().insert(
            armed.voice_id,
            OutputVoice { id: armed.voice_id, started_at: Instant::now(), duration: None },
        );
        if let Some(cv) = OUTPUT_CURRENT_VOICE.get() {
            *cv.lock().unwrap() = Some(armed.voice_id);
        }

        self.position_window(screen_index);

        if let Some(d) = OUTPUT_PCM_DISCARD.get() {
            d.store(false, Ordering::Release);
        }
        *self.go_sent_at.lock().unwrap() = Some(Instant::now());

        let vol_pct = (100.0 * db_to_linear(volume_db)).clamp(0.0, 1000.0);
        let vol_str = cs(&format!("{vol_pct:.2}"));
        unsafe {
            (self.mpv_lib.mpv_set_property_string)(
                self.mpv_ctx.0, cs("volume").as_ptr(), vol_str.as_ptr(),
            );
            (self.mpv_lib.mpv_set_property_string)(
                self.mpv_ctx.0, cs("pause").as_ptr(), cs("no").as_ptr(),
            );
        }

        log::info!(
            "[pre-arm] GO: activated voice {} for cue {} (armed {}ms ago)",
            armed.voice_id, armed.owner_id,
            armed.armed_at.elapsed().as_millis(),
        );

        if fade_in_ms > 0 {
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

        Ok(armed.voice_id)
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
