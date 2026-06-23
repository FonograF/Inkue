//! Data types shared across the output_engine module.

use std::ffi::c_void;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Type aliases
// ---------------------------------------------------------------------------

/// Unique identifier for one playing output instance (video or image).
pub type VoiceId = Uuid;
/// Unique identifier for one output surface.
pub type SurfaceId = Uuid;

// ---------------------------------------------------------------------------
// Thread-safety wrapper for the raw mpv context pointer
// ---------------------------------------------------------------------------

pub(crate) struct MpvCtx(pub *mut c_void);
unsafe impl Send for MpvCtx {}
unsafe impl Sync for MpvCtx {}

// ---------------------------------------------------------------------------
// Screen info
// ---------------------------------------------------------------------------

/// Information about a connected monitor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenInfo {
    pub index: u32,
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    pub is_primary: bool,
}

// ---------------------------------------------------------------------------
// OutputStatus
// ---------------------------------------------------------------------------

/// Status events produced by the mpv event thread.
#[derive(Debug, Clone)]
pub enum OutputStatus {
    /// Playback reached its natural end.
    Completed { voice_id: VoiceId },
    /// File metadata loaded; total duration is now known.
    Duration { voice_id: VoiceId, duration_ms: u64 },
    /// A playback error occurred inside mpv.
    Error { voice_id: VoiceId, message: String },
}

// ---------------------------------------------------------------------------
// OutputSurface
// ---------------------------------------------------------------------------

/// A named output surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputSurface {
    pub id: SurfaceId,
    pub name: String,
    pub label: String,
}

// ---------------------------------------------------------------------------
// OutputVoice
// ---------------------------------------------------------------------------

#[derive(Debug)]
#[allow(dead_code)]
pub(super) struct OutputVoice {
    pub id: VoiceId,
    pub started_at: Instant,
    pub duration: Option<Duration>,
}

// ---------------------------------------------------------------------------
// Fade overlay state
// ---------------------------------------------------------------------------

/// Parameters for a pending content load, passed directly to `execute_load_params`.
pub(crate) struct FadePendingParams {
    pub path: String,
    pub is_image: bool,
    #[allow(dead_code)]
    pub voice_id: Uuid,
    pub fade_in_ms: u32,
    pub loop_count: u32,
    pub start_ms: Option<u64>,
    pub end_ms: Option<u64>,
    /// For image cues: how long mpv holds the image before auto-completing.
    /// `None` = infinite (hold until explicitly stopped).
    pub display_duration_ms: Option<u64>,
}

pub(crate) enum FadePending {
    Stop,
}

/// State carried from a video `loadfile` (issued paused) to the
/// `MPV_EVENT_PLAYBACK_RESTART` that fires once frame 0 is decoded and on
/// screen.  At that point the engine reveals the overlay and unpauses, so
/// audio and video both start from frame 0 with no A/V offset and no
/// decoder-warmup freeze.
pub(crate) struct PendingVideoStart {
    /// Fade-from-black duration to run when the first frame is revealed
    /// (0 = hard cut).
    pub fade_in_ms: u32,
}

pub(crate) struct FadeAnimState {
    pub current_alpha: u8,
    pub target_alpha: u8,
    pub start_alpha: u8,
    pub duration_ms: u32,
    pub start_time: Instant,
    pub timer_active: bool,
    pub pending: Option<FadePending>,
}

impl FadeAnimState {
    /// Resting state at startup: opaque black, matching the convention used
    /// everywhere else (overlay stays at alpha=255 until content fades in).
    /// Also load-bearing on the GL path: the render loop only swaps a buffer
    /// when there's an mpv frame OR alpha > 0, so an idle alpha of 0 means the
    /// output window's surface never commits a single frame on Wayland — the
    /// compositor then refuses to map the window no matter what `set_visible`
    /// says, which is why toggling it manually used to show nothing until a
    /// video/image cue forced the first real frame.
    pub fn idle() -> Self {
        Self {
            current_alpha: 255,
            target_alpha: 255,
            start_alpha: 255,
            duration_ms: 0,
            start_time: Instant::now(),
            timer_active: false,
            pending: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Win32 window state
// ---------------------------------------------------------------------------

#[cfg(output_win32)]
pub(crate) struct OutputWndState {
    pub is_fullscreen: bool,
    pub saved_rect: (i32, i32, i32, i32),
}
