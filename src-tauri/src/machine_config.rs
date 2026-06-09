//! Persistence for [`MachineAudioConfig`] in `%APPDATA%\WinCue\audio.json`.
//!
//! This file is intentionally separate from the workspace (`.wincue`) because
//! hardware settings — device, backend, buffer size — are machine-specific.
//! The workspace file travels with the show; this file stays on the machine.

use std::path::PathBuf;

use crate::preferences::MachineAudioConfig;

/// Absolute path to the machine audio config file.
fn config_path() -> PathBuf {
    std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("WinCue")
        .join("audio.json")
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
