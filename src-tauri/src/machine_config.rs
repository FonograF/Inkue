//! Persistence for machine-level configs in `%APPDATA%\WinCue\`.
//!
//! These files are intentionally separate from the workspace (`.wincue`)
//! because hardware settings are machine-specific — the workspace travels
//! with the show while these stay on the machine.

use std::path::PathBuf;

use crate::preferences::{MachineAudioConfig, OscReceiveConfig};

/// Per-OS base directory for machine-level config files.
///
/// Falls back to the current directory only if the platform's expected
/// environment variable is unset — this must never resolve into the source
/// tree (`src-tauri/`), or writes during `tauri dev` retrigger its file
/// watcher and restart the whole app.
fn config_base_dir() -> PathBuf {
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
    config_base_dir().join("WinCue").join("audio.json")
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

/// Persist the machine audio config to disk, creating `%APPDATA%\WinCue\` if needed.
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
    config_base_dir().join("WinCue").join("osc.json")
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

/// Persist the OSC receive config to disk, creating `%APPDATA%\WinCue\` if needed.
pub fn save_osc(config: &OscReceiveConfig) -> anyhow::Result<()> {
    let path = osc_config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(config)?;
    std::fs::write(&path, json)?;
    Ok(())
}
