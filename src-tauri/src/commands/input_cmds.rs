//! Tauri commands for live audio **Input Patch** management (Mic Cues).
//!
//! Input Patches live in the workspace (like OSC patches) so they travel with
//! the show; the physical device they point at is resolved at GO time.

use tauri::{Emitter, State};
use uuid::Uuid;

use crate::{engine::audio_input::InputPatch, state::AppState};

/// Return all Input Patches in the active workspace.
#[tauri::command]
pub fn list_input_patches(state: State<'_, AppState>) -> Result<Vec<InputPatch>, String> {
    let ws = state.workspace.lock().map_err(|e| e.to_string())?;
    Ok(ws.input_patches.clone())
}

/// Add a new Input Patch to the workspace.  Returns the created patch.
#[tauri::command]
pub fn add_input_patch(
    name: String,
    device_id: String,
    channels: Vec<u16>,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<InputPatch, String> {
    let patch = InputPatch::new(name, device_id, channels);
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    ws.input_patches.push(patch.clone());
    ws.mark_modified();
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(patch)
}

/// Update an existing Input Patch (matched by ID).
#[tauri::command]
pub fn update_input_patch(
    patch: InputPatch,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    if let Some(existing) = ws.input_patches.iter_mut().find(|p| p.id == patch.id) {
        *existing = patch;
        ws.mark_modified();
        let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
        Ok(())
    } else {
        Err(format!("Input patch {} not found", patch.id))
    }
}

/// Remove an Input Patch by ID.
#[tauri::command]
pub fn remove_input_patch(
    patch_id: String,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let id: Uuid = patch_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let before = ws.input_patches.len();
    ws.input_patches.retain(|p| p.id != id);
    if ws.input_patches.len() == before {
        return Err(format!("Input patch {id} not found"));
    }
    ws.mark_modified();
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(())
}
