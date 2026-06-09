//! Tauri commands for OSC Patch management and OSC receive configuration.

use tauri::{Emitter, State};
use uuid::Uuid;

use crate::{
    engine::osc_patch::OscPatch,
    preferences::OscReceiveConfig,
    state::AppState,
};

// ---------------------------------------------------------------------------
// OSC Patch commands
// ---------------------------------------------------------------------------

/// Return all OSC patches in the active workspace.
#[tauri::command]
pub fn list_osc_patches(state: State<'_, AppState>) -> Result<Vec<OscPatch>, String> {
    let ws = state.workspace.lock().map_err(|e| e.to_string())?;
    Ok(ws.osc_patches.clone())
}

/// Add a new OSC patch to the workspace.  Returns the created patch.
#[tauri::command]
pub fn add_osc_patch(
    name: String,
    ip: String,
    port: u16,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<OscPatch, String> {
    let patch = OscPatch::new(name, ip, port);
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    ws.osc_patches.push(patch.clone());
    ws.mark_modified();
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(patch)
}

/// Update an existing OSC patch (matched by ID).
#[tauri::command]
pub fn update_osc_patch(
    patch: OscPatch,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    if let Some(existing) = ws.osc_patches.iter_mut().find(|p| p.id == patch.id) {
        *existing = patch;
        ws.mark_modified();
        let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
        Ok(())
    } else {
        Err(format!("OSC patch {} not found", patch.id))
    }
}

/// Remove an OSC patch by ID.
#[tauri::command]
pub fn remove_osc_patch(
    patch_id: String,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let id: Uuid = patch_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let before = ws.osc_patches.len();
    ws.osc_patches.retain(|p| p.id != id);
    if ws.osc_patches.len() == before {
        return Err(format!("OSC patch {id} not found"));
    }
    ws.mark_modified();
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(())
}

// ---------------------------------------------------------------------------
// OSC receive config commands
// ---------------------------------------------------------------------------

/// Return the current machine-level OSC receive configuration.
#[tauri::command]
pub fn get_osc_config() -> OscReceiveConfig {
    crate::machine_config::load_osc()
}

/// Save a new OSC receive configuration and hot-apply it to the running server.
#[tauri::command]
pub fn set_osc_config(
    config: OscReceiveConfig,
    state: State<'_, AppState>,
) -> Result<(), String> {
    crate::machine_config::save_osc(&config).map_err(|e| e.to_string())?;
    state.osc_server.reconfigure(config);
    Ok(())
}
