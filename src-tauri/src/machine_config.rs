//! Persistence for machine-level configs in `%APPDATA%\Inkue\`.
//!
//! These files are intentionally separate from the workspace (`.inkue`)
//! because hardware settings are machine-specific — the workspace travels
//! with the show while these stay on the machine.

use std::path::PathBuf;

use crate::{
    preferences::{MachineAudioConfig, NetworkInterfaceConfig, OscReceiveConfig},
    engine::timecode_receiver::TcReceiverConfig,
};

/// Per-OS base directory for machine-level config files.
///
/// Falls back to the current directory only if the platform's expected
/// environment variable is unset — this must never resolve into the source
/// tree (`src-tauri/`), or writes during `tauri dev` retrigger its file
/// watcher and restart the whole app.
pub(crate) fn config_base_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        std::env::var("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var("HOME")
            .map(|h| PathBuf::from(h).join("Library/Application Support"))
            .unwrap_or_else(|_| PathBuf::from("."))
    }
    #[cfg(target_os = "linux")]
    {
        std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_else(|_| PathBuf::from("."))
    }
}

/// Absolute path to the machine audio config file.
fn config_path() -> PathBuf {
    config_base_dir().join("Inkue").join("audio.json")
}

/// Load the machine audio config from disk.  Returns [`MachineAudioConfig::default`]
/// on first run or if the file cannot be read/parsed.
pub fn load() -> MachineAudioConfig {
    let path = config_path();
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persist the machine audio config to disk, creating `%APPDATA%\Inkue\` if needed.
pub fn save(config: &MachineAudioConfig) -> anyhow::Result<()> {
    let path = config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(config)?;
    std::fs::write(&path, json)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// OSC receive config
// ---------------------------------------------------------------------------

fn osc_config_path() -> PathBuf {
    config_base_dir().join("Inkue").join("osc.json")
}

/// Load the OSC receive config from disk.  Returns the default config on first
/// run or when the file cannot be read/parsed.
pub fn load_osc() -> OscReceiveConfig {
    let path = osc_config_path();
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persist the OSC receive config to disk, creating `%APPDATA%\Inkue\` if needed.
pub fn save_osc(config: &OscReceiveConfig) -> anyhow::Result<()> {
    let path = osc_config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(config)?;
    std::fs::write(&path, json)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Network interface config
// ---------------------------------------------------------------------------

fn network_config_path() -> PathBuf {
    config_base_dir().join("Inkue").join("network.json")
}

/// Load the network interface config from disk.  Returns the default
/// (Automatic — all interfaces) on first run or when the file cannot be
/// read/parsed.
pub fn load_network() -> NetworkInterfaceConfig {
    let path = network_config_path();
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persist the network interface config to disk.
pub fn save_network(config: &NetworkInterfaceConfig) -> anyhow::Result<()> {
    let path = network_config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(config)?;
    std::fs::write(&path, json)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// TC machine config
// ---------------------------------------------------------------------------

/// Persisted TC machine config (receiver enabled/disabled + source/port).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[derive(Default)]
pub struct TcMachineConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub receiver_config: TcReceiverConfig,
}


// ---------------------------------------------------------------------------
// MIDI trigger machine config
// ---------------------------------------------------------------------------

/// Which MIDI input drives per-cue triggers on **this machine**.
///
/// Machine config, not workspace config: the port name belongs to the hardware
/// in front of the operator, so a show file carried to another rig keeps its
/// triggers but picks up that rig's input.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct MidiTriggerMachineConfig {
    #[serde(default)]
    pub enabled: bool,
    /// MIDI input port name. `None` = the first available port.
    #[serde(default)]
    pub port: Option<String>,
}

fn midi_trigger_config_path() -> std::path::PathBuf {
    config_base_dir().join("Inkue").join("midi_triggers.json")
}

pub fn load_midi_trigger_config() -> MidiTriggerMachineConfig {
    std::fs::read_to_string(midi_trigger_config_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_midi_trigger_config(config: &MidiTriggerMachineConfig) -> anyhow::Result<()> {
    let path = midi_trigger_config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(config)?)?;
    Ok(())
}

fn tc_config_path() -> std::path::PathBuf {
    config_base_dir().join("Inkue").join("timecode.json")
}

pub fn load_tc_config() -> TcMachineConfig {
    let path = tc_config_path();
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_tc_config(config: &TcMachineConfig) -> anyhow::Result<()> {
    let path = tc_config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(config)?;
    std::fs::write(&path, json)?;
    Ok(())
}
