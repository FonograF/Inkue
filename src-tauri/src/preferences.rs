//! Application-wide preferences, persisted inside the `.wincue` file under
//! the `"preferences"` key.
//!
//! Each top-level category (audio, general, network, display) is its own
//! struct so future categories can be added without touching existing ones.

use serde::{Deserialize, Serialize};

use crate::cue::types::FadeCurve;

// ---------------------------------------------------------------------------
// Audio backend choice
// ---------------------------------------------------------------------------

/// Which WASAPI/ASIO mode the engine uses for output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioBackend {
    WasapiShared,
    WasapiExclusive,
    Asio,
}

impl Default for AudioBackend {
    fn default() -> Self {
        Self::WasapiShared
    }
}

// ---------------------------------------------------------------------------
// Audio preferences
// ---------------------------------------------------------------------------

/// Audio engine settings and per-workspace audio defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioPreferences {
    /// Audio output buffer size in samples.
    #[serde(default = "AudioPreferences::default_buffer_size")]
    pub buffer_size: u32,

    /// WASAPI/ASIO backend to use.
    #[serde(default)]
    pub backend: AudioBackend,

    /// Identifier of the selected output device.  `None` = use system default.
    #[serde(default)]
    pub device_id: Option<String>,

    /// Default volume (dB) applied to newly created cues.
    #[serde(default)]
    pub default_volume_db: f32,

    /// Duration (ms) of the soft fade-out applied on Stop.
    #[serde(default = "AudioPreferences::default_fade_out_ms")]
    pub default_fade_out_ms: u32,

    /// Default fade curve for newly created cues.
    #[serde(default = "AudioPreferences::default_fade_curve")]
    pub default_fade_curve: FadeCurve,

    /// ASIO output pair index (0 = channels 1-2, 1 = channels 3-4, …).
    /// Ignored when backend is not ASIO.
    #[serde(default)]
    pub asio_out_pair: u32,
}

impl AudioPreferences {
    fn default_buffer_size() -> u32 { 256 }
    fn default_fade_out_ms() -> u32 { 500 }
    fn default_fade_curve() -> FadeCurve { FadeCurve::Linear }
}

impl Default for AudioPreferences {
    fn default() -> Self {
        Self {
            buffer_size: Self::default_buffer_size(),
            backend: AudioBackend::default(),
            device_id: None,
            default_volume_db: 0.0,
            default_fade_out_ms: Self::default_fade_out_ms(),
            default_fade_curve: Self::default_fade_curve(),
            asio_out_pair: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Reserved category structs (empty, ready for future content)
// ---------------------------------------------------------------------------

/// General app-behaviour preferences (language, theme, GO behaviour, …).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GeneralPreferences {}

/// Network preferences (OSC, MIDI, Art-Net, …).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkPreferences {}

/// Display preferences (text size, timecode display, kiosk mode, …).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DisplayPreferences {}

// ---------------------------------------------------------------------------
// Root preferences struct
// ---------------------------------------------------------------------------

/// All application preferences, serialised into the workspace file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppPreferences {
    #[serde(default)]
    pub audio: AudioPreferences,
    #[serde(default)]
    pub general: GeneralPreferences,
    #[serde(default)]
    pub network: NetworkPreferences,
    #[serde(default)]
    pub display: DisplayPreferences,
}
