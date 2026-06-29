//! Crash-recovery snapshot of the in-memory workspace.
//!
//! While a show has unsaved changes, the autosave thread (`lib.rs`) writes the
//! current workspace to `recovery.inkue` in the per-user config dir every few
//! seconds.  A clean exit (window close) deletes it, so its **presence at
//! startup means the previous session ended abnormally** (crash / power loss)
//! and the unsaved work can be offered back to the operator.
//!
//! The snapshot stores absolute media paths (see [`Workspace::to_recovery_json`]).

use std::path::PathBuf;

use serde::Serialize;

use crate::machine_config::config_base_dir;

/// Absolute path to the recovery snapshot file.
fn recovery_path() -> PathBuf {
    config_base_dir().join("Inkue").join("recovery.inkue")
}

/// Metadata shown in the "recover unsaved work?" prompt — parsed from the
/// snapshot header without fully deserialising the workspace.
#[derive(Debug, Clone, Serialize)]
pub struct RecoveryInfo {
    /// Workspace name (file stem, or "Untitled" if never saved).
    pub name: String,
    /// Original `.inkue` path, if the show had been saved before the crash.
    pub original_path: Option<String>,
    /// ISO-8601 timestamp of the last edit captured in the snapshot.
    pub modified_at: Option<String>,
}

/// Atomically write the recovery snapshot: write to a sibling `.tmp` then
/// rename, so a crash mid-write never leaves a half-written (corrupt) file.
pub fn write(json: &str) -> std::io::Result<()> {
    let path = recovery_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Whether a recovery snapshot currently exists on disk.
pub fn exists() -> bool {
    recovery_path().exists()
}

/// Read the raw recovery snapshot JSON.
pub fn read() -> std::io::Result<String> {
    std::fs::read_to_string(recovery_path())
}

/// Delete the recovery snapshot (no error if it does not exist).
pub fn delete() {
    let _ = std::fs::remove_file(recovery_path());
}

/// Parse the snapshot header for the recovery prompt.  Returns `None` when no
/// snapshot exists or it cannot be parsed.
pub fn info() -> Option<RecoveryInfo> {
    let content = read().ok()?;
    let doc: serde_json::Value = serde_json::from_str(&content).ok()?;
    let original_path = doc
        .get("recovery_original_path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let workspace = doc.get("workspace");
    let name = workspace
        .and_then(|w| w.get("name"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("Untitled")
        .to_string();
    let modified_at = workspace
        .and_then(|w| w.get("modified_at"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Some(RecoveryInfo { name, original_path, modified_at })
}
