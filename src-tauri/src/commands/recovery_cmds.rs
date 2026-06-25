//! Tauri commands for crash recovery — surfaced to the frontend at startup.

use std::path::PathBuf;

use tauri::State;

use crate::{
    commands::workspace_cmds::install_workspace,
    recovery::{self, RecoveryInfo},
    show::Workspace,
    state::AppState,
};

/// Return metadata for an unsaved-work snapshot left by a previous session, if
/// one exists (i.e. the previous run did not exit cleanly).
#[tauri::command]
pub fn check_recovery() -> Option<RecoveryInfo> {
    recovery::info()
}

/// Discard the crash-recovery snapshot without restoring it.
#[tauri::command]
pub fn discard_recovery() -> Result<(), String> {
    recovery::delete();
    Ok(())
}

/// Restore the crash-recovery snapshot into the running app, replacing the
/// current (empty) workspace.  The snapshot is left on disk until the next
/// explicit save or clean exit, in case the restore itself is interrupted.
#[tauri::command]
pub fn restore_recovery(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let content = recovery::read().map_err(|e| e.to_string())?;

    let registry = state.registry.lock().map_err(|e| e.to_string())?;
    // Recovery snapshots store absolute media paths, so no base dir.
    let mut loaded = Workspace::from_json_str(&content, None, &registry)
        .map_err(|e| e.to_string())?;
    drop(registry);

    // Re-target the original `.wincue` file (if the show had been saved) and
    // mark dirty — the snapshot holds edits not yet written to that file.
    let doc: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| e.to_string())?;
    let original = doc
        .get("recovery_original_path")
        .and_then(|v| v.as_str())
        .map(PathBuf::from);
    if let Some(ref p) = original {
        if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
            loaded.metadata.name = stem.to_string();
        }
    }
    loaded.file_path = original;
    loaded.mark_modified();

    install_workspace(state.inner(), &app_handle, loaded)
}
