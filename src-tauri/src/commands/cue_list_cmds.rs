//! Tauri commands for cue list management (create, delete, rename, switch).

use serde::Serialize;
use tauri::{Emitter, State};
use uuid::Uuid;

use crate::{show::{cue_list::CueList, workspace::Workspace}, state::AppState};

/// Compact info about a cue list, sent to the frontend.
#[derive(Debug, Clone, Serialize)]
pub struct CueListInfo {
    pub id: String,
    pub name: String,
}

/// Returns the list of all cue lists and which one is active.
#[tauri::command]
pub fn get_cue_lists(state: State<'_, AppState>) -> Result<Vec<CueListInfo>, String> {
    let ws = state.workspace.lock().map_err(|e| e.to_string())?;
    Ok(cue_list_infos(&ws))
}

/// Creates a new empty cue list, makes it active, and returns its ID.
#[tauri::command]
pub fn add_cue_list(
    state: State<'_, AppState>,
    handle: tauri::AppHandle,
    name: String,
) -> Result<String, String> {
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let new_list = CueList::new(&name);
    let new_id = new_list.id;
    ws.cue_lists.push(new_list);
    ws.active_cue_list_id = new_id;
    ws.mark_modified();
    emit_cue_lists_changed(&handle, &ws);
    let _ = handle.emit("playhead-moved", serde_json::json!({ "cue_id": null }));
    Ok(new_id.to_string())
}

/// Removes a cue list by ID. Errors if it is the last remaining list.
/// If the active list is removed, the first remaining list becomes active.
#[tauri::command]
pub fn remove_cue_list(
    state: State<'_, AppState>,
    handle: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    if ws.cue_lists.len() <= 1 {
        return Err("Cannot remove the last cue list".into());
    }
    let target: Uuid = id.parse().map_err(|_| "Invalid cue list ID")?;
    let was_active = ws.active_cue_list_id == target;
    ws.cue_lists.retain(|cl| cl.id != target);
    if was_active {
        ws.active_cue_list_id = ws.cue_lists[0].id;
    }
    ws.mark_modified();
    emit_cue_lists_changed(&handle, &ws);
    if was_active {
        let new_ph = ws.active_cue_list().and_then(|cl| cl.playhead_cue_id);
        let _ = handle.emit("playhead-moved", serde_json::json!({ "cue_id": new_ph }));
    }
    Ok(())
}

/// Renames a cue list.
#[tauri::command]
pub fn rename_cue_list(
    state: State<'_, AppState>,
    handle: tauri::AppHandle,
    id: String,
    name: String,
) -> Result<(), String> {
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let target: Uuid = id.parse().map_err(|_| "Invalid cue list ID")?;
    let cl = ws
        .cue_lists
        .iter_mut()
        .find(|cl| cl.id == target)
        .ok_or("Cue list not found")?;
    cl.name = name;
    ws.mark_modified();
    emit_cue_lists_changed(&handle, &ws);
    Ok(())
}

/// Switches the active cue list. Emits `cue-lists-changed` and `playhead-moved`.
#[tauri::command]
pub fn set_active_cue_list(
    state: State<'_, AppState>,
    handle: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let target: Uuid = id.parse().map_err(|_| "Invalid cue list ID")?;
    if !ws.cue_lists.iter().any(|cl| cl.id == target) {
        return Err("Cue list not found".into());
    }
    ws.active_cue_list_id = target;
    let new_playhead = ws.active_cue_list().and_then(|cl| cl.playhead_cue_id);
    emit_cue_lists_changed(&handle, &ws);
    let _ = handle.emit("playhead-moved", serde_json::json!({ "cue_id": new_playhead }));
    Ok(())
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn cue_list_infos(ws: &Workspace) -> Vec<CueListInfo> {
    ws.cue_lists
        .iter()
        .map(|cl| CueListInfo {
            id: cl.id.to_string(),
            name: cl.name.clone(),
        })
        .collect()
}

fn emit_cue_lists_changed(handle: &tauri::AppHandle, ws: &Workspace) {
    let _ = handle.emit(
        "cue-lists-changed",
        serde_json::json!({
            "cue_lists": cue_list_infos(ws),
            "active_cue_list_id": ws.active_cue_list_id.to_string(),
        }),
    );
}
