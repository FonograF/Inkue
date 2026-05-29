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

/// Parameters for a pending content load (stored while fading to black).
#[allow(dead_code)]
pub(crate) struct FadePendingParams {
    pub path: String,
    pub is_image: bool,
    pub voice_id: Uuid,
    pub fade_in_ms: u32,
    pub volume_db: f64,
    pub loop_count: u32,
    pub start_ms: Option<u64>,
    pub end_ms: Option<u64>,
}

pub(crate) enum FadePending {
    Load(FadePendingParams),
    Stop,
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
    pub fn idle() -> Self {
        Self {
            current_alpha: 0,
            target_alpha: 0,
            start_alpha: 0,
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

pub(crate) struct OutputWndState {
    pub is_fullscreen: bool,
    pub saved_rect: (i32, i32, i32, i32),
}
