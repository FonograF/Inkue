//! Application-wide preferences, persisted inside the `.inkue` file under
//! the `"preferences"` key.
//!
//! Each top-level category (audio, general, network, display) is its own
//! struct so future categories can be added without touching existing ones.

use serde::{Deserialize, Serialize};

use crate::cue::types::FadeCurve;

// ---------------------------------------------------------------------------
// Audio backend choice
// ---------------------------------------------------------------------------

/// Audio output backend.
///
/// `WasapiShared` / `WasapiExclusive` / `Asio` are Windows-specific.
/// `SystemDefault` is used on Mac / Linux where cpal picks CoreAudio or ALSA.
/// Unknown values from old configs are deserialized as `WasapiShared` on Windows
/// and normalised to `SystemDefault` on Mac / Linux by `get_machine_audio_config`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AudioBackend {
    #[default]
    WasapiShared,
    WasapiExclusive,
    Asio,
    SystemDefault,
}

// ---------------------------------------------------------------------------
// Audio preferences
// ---------------------------------------------------------------------------

/// Hardware-specific audio settings — device, backend, buffer size.
///
/// Stored in `%APPDATA%\Inkue\audio.json`, **not** in the workspace file,
/// because they describe the physical machine rather than the show.
/// Moving a `.inkue` file to another machine keeps its show defaults intact
/// while this config adapts to the local hardware.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineAudioConfig {
    /// WASAPI/ASIO backend to use.
    #[serde(default)]
    pub backend: AudioBackend,

    /// Identifier of the selected output device.  `None` = system default.
    #[serde(default)]
    pub device_id: Option<String>,

    /// Human-readable name of the selected output device, captured at selection
    /// time so a banner can show "Focusrite Scarlett…" instead of the raw
    /// WASAPI endpoint id even when the device is currently absent.  `None` =
    /// system default (or selected before this field existed).
    #[serde(default)]
    pub device_name: Option<String>,

    /// Identifier of the selected audio **input** device for Mic Cues / live
    /// capture.  `None` = system default input.  Machine-specific, like
    /// `device_id`.
    #[serde(default)]
    pub input_device_id: Option<String>,

    /// Output buffer size in samples.
    /// Only applied for WASAPI Exclusive; ignored in Shared mode (Windows
    /// controls the period) and ASIO mode (driver controls its own buffer).
    #[serde(default = "MachineAudioConfig::default_buffer_size")]
    pub buffer_size: u32,

    /// ASIO output pair index (0 = Out 1-2, 1 = Out 3-4, …).
    /// Ignored when backend is not ASIO.
    #[serde(default)]
    pub asio_out_pair: u32,
}

impl MachineAudioConfig {
    fn default_buffer_size() -> u32 { 256 }
}

impl Default for MachineAudioConfig {
    fn default() -> Self {
        Self {
            backend: AudioBackend::default(),
            device_id: None,
            device_name: None,
            input_device_id: None,
            buffer_size: Self::default_buffer_size(),
            asio_out_pair: 0,
        }
    }
}

/// Show-specific audio defaults — stored in the workspace file.
///
/// These travel with the `.inkue` file because they describe the show's
/// intent (how loud new cues are, what fade curve to use) rather than the
/// hardware it runs on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioPreferences {
    /// Default volume (dB) applied to newly created cues.
    #[serde(default)]
    pub default_volume_db: f32,

    /// Duration (ms) of the soft fade-out applied on Stop.
    #[serde(default = "AudioPreferences::default_fade_out_ms")]
    pub default_fade_out_ms: u32,

    /// Default fade curve for newly created cues.
    #[serde(default = "AudioPreferences::default_fade_curve")]
    pub default_fade_curve: FadeCurve,

    /// Runtime-only: the machine's configured audio buffer size, injected at
    /// startup from `MachineAudioConfig` so `CueContext` can pass it to
    /// `ensure_input_feed`.  Never serialised into the workspace file.
    #[serde(skip)]
    pub audio_buffer_size: u32,
}

impl AudioPreferences {
    fn default_fade_out_ms() -> u32 { 500 }
    fn default_fade_curve() -> FadeCurve { FadeCurve::Linear }
}

impl Default for AudioPreferences {
    fn default() -> Self {
        Self {
            default_volume_db: 0.0,
            default_fade_out_ms: Self::default_fade_out_ms(),
            default_fade_curve: Self::default_fade_curve(),
            audio_buffer_size: 256,
        }
    }
}

// ---------------------------------------------------------------------------
// Reserved category structs (empty, ready for future content)
// ---------------------------------------------------------------------------

/// Row height for the Cue List table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CueRowHeight {
    Compact,
    #[default]
    Normal,
    Tall,
}

/// General app-behaviour preferences.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralPreferences {
    /// Minimum time (ms) between two GO triggers.  A second GO fired within
    /// this window is silently ignored to prevent accidental double-presses
    /// during live shows.  Set to 0 to disable.
    #[serde(default = "GeneralPreferences::default_double_go_protection_ms")]
    pub double_go_protection_ms: u32,

    /// When true, deleting a cue via the keyboard shows a confirmation dialog.
    #[serde(default)]
    pub confirm_before_delete: bool,

    /// When true, the cue list automatically scrolls to keep the Playhead
    /// visible after each GO.
    #[serde(default = "GeneralPreferences::default_auto_scroll_to_playhead")]
    pub auto_scroll_to_playhead: bool,

    /// Height of each row in the cue list table.
    #[serde(default)]
    pub cue_row_height: CueRowHeight,
}

impl GeneralPreferences {
    fn default_double_go_protection_ms() -> u32 { 500 }
    fn default_auto_scroll_to_playhead() -> bool { true }
}

impl Default for GeneralPreferences {
    fn default() -> Self {
        Self {
            double_go_protection_ms: Self::default_double_go_protection_ms(),
            confirm_before_delete: false,
            auto_scroll_to_playhead: Self::default_auto_scroll_to_playhead(),
            cue_row_height: CueRowHeight::default(),
        }
    }
}

/// OSC receive server configuration — stored machine-level in
/// `%APPDATA%\Inkue\osc.json`, not in the workspace file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OscReceiveConfig {
    /// When `false` the server is not started (or is stopped if already running).
    #[serde(default)]
    pub enabled: bool,
    /// UDP port to listen on.  Default: 53001.
    #[serde(default = "OscReceiveConfig::default_port")]
    pub port: u16,
    /// IP addresses allowed to send OSC commands.  Empty list = accept all.
    #[serde(default)]
    pub allowed_ips: Vec<String>,

    /// When `true`, Inkue broadcasts the running cue's name and number to
    /// `feedback_host:feedback_port` whenever the active cue changes.
    #[serde(default)]
    pub feedback_enabled: bool,
    /// Destination hostname or IP for OSC feedback (e.g. `"127.0.0.1"`).
    #[serde(default = "OscReceiveConfig::default_feedback_host")]
    pub feedback_host: String,
    /// Destination UDP port for OSC feedback.  Default: 53000.
    #[serde(default = "OscReceiveConfig::default_feedback_port")]
    pub feedback_port: u16,
}

impl OscReceiveConfig {
    fn default_port()          -> u16    { 53001 }
    fn default_feedback_host() -> String { "127.0.0.1".into() }
    fn default_feedback_port() -> u16    { 53000 }
}

impl Default for OscReceiveConfig {
    fn default() -> Self {
        Self {
            enabled:          false,
            port:             Self::default_port(),
            allowed_ips:      Vec::new(),
            feedback_enabled: false,
            feedback_host:    Self::default_feedback_host(),
            feedback_port:    Self::default_feedback_port(),
        }
    }
}

/// Network preferences (OSC, MIDI, Art-Net, …).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkPreferences {}

/// How a cue's colour tag is rendered in the Cue List.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CueColorStyle {
    /// A 4px tinted strip along the left edge of the row (original look).
    #[default]
    Stripe,
    /// The entire row background tinted with the cue's colour.
    FullRow,
}

/// Where on the output window the cue timer is anchored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TimerPosition {
    /// Centered horizontally and vertically — large display.
    #[default]
    Center,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Display preferences (output surface, colour theme, …).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayPreferences {
    /// Monitor index for the unified output surface.
    /// `None` = floating windowed (no fixed screen).
    /// `Some(n)` = fullscreen on monitor n (0 = primary).
    #[serde(default)]
    pub output_screen: Option<u32>,

    /// When `true`, a countdown timer is drawn on the output window showing
    /// the timing of the currently running audio cue.
    #[serde(default)]
    pub show_output_timer: bool,

    /// When `true` the timer counts down (time remaining).
    /// When `false` (default) it counts up (elapsed position in the file).
    #[serde(default)]
    pub timer_count_down: bool,

    /// Font family name for the output timer (e.g. `"Arial"`, `"Courier New"`).
    #[serde(default = "DisplayPreferences::default_timer_font")]
    pub timer_font: String,

    /// Font size for the output timer, in mpv OSD points.
    /// Default 120 is suitable for center; use 60–80 for corner positions.
    #[serde(default = "DisplayPreferences::default_timer_font_size")]
    pub timer_font_size: u32,

    /// Where on the output window the timer is drawn.
    #[serde(default)]
    pub timer_position: TimerPosition,

    /// When `true`, milliseconds are shown after the seconds (e.g. `00:00.000`).
    #[serde(default)]
    pub timer_show_ms: bool,

    /// Margin in pixels from the screen edge for corner positions.
    /// Ignored when `timer_position` is `Center`.
    #[serde(default = "DisplayPreferences::default_timer_margin")]
    pub timer_margin: u32,

    /// When `true` (and `show_output_timer` is also `true`), the timer is shown
    /// in a small always-on-top floating Win32 window instead of as an OSD
    /// overlay on the output surface.
    #[serde(default)]
    pub timer_floating: bool,

    /// UI colour theme: `"dark"`, `"light"`, or `"system"` (follows OS setting).
    #[serde(default = "DisplayPreferences::default_theme")]
    pub theme: String,

    /// How a cue's colour tag is rendered in the Cue List (stripe vs full row).
    #[serde(default)]
    pub cue_color_style: CueColorStyle,
}

impl DisplayPreferences {
    fn default_theme()           -> String { "system".into() }
    fn default_timer_font()      -> String { crate::bundled_fonts::FONT_FAMILY.into() }
    fn default_timer_font_size() -> u32   { 120 }
    fn default_timer_margin()    -> u32   { 50 }
}

impl Default for DisplayPreferences {
    fn default() -> Self {
        Self {
            output_screen:      None,
            show_output_timer:  false,
            timer_count_down:   false,
            timer_font:         Self::default_timer_font(),
            timer_font_size:    Self::default_timer_font_size(),
            timer_position:     TimerPosition::default(),
            timer_show_ms:      false,
            timer_margin:       Self::default_timer_margin(),
            timer_floating:     false,
            theme:              Self::default_theme(),
            cue_color_style:    CueColorStyle::default(),
        }
    }
}

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
