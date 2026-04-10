//! Tauri commands for undo, redo, copy and paste.
//!
//! The undo/redo system is snapshot-based: before every mutating command
//! [`push_current_snapshot`] is called, which captures the full cue list state
//! (serialised JSON + decoded audio Arcs) and pushes it onto the [`UndoStack`].
//! Undo/redo swap the current state with the stored snapshot atomically while
//! holding the registry, workspace and undo_stack locks in that order.
//!
//! Lock ordering (always respected to prevent deadlocks):
//!   registry → workspace → undo_stack → clipboard

use tauri::{Emitter, State};
use uuid::Uuid;

use crate::{
    cue::registry::CueRegistry,
    show::{
        cue_list::CueList,
        undo_stack::{CueSnapshot, Snapshot},
    },
    state::AppState,
};

// ---------------------------------------------------------------------------
// Snapshot helpers (pub so cue_cmds can call push_current_snapshot)
// ---------------------------------------------------------------------------

/// Serialise the current cue list into a [`Snapshot`].
///
/// The decoded audio `Arc` for each cue is cloned — this is a reference-count
/// bump, not a data copy, so it is O(n_cues) not O(total_samples).
pub fn take_snapshot(cue_list: &CueList) -> Snapshot {
    Snapshot {
        cues: cue_list
            .cues
            .iter()
            .map(|c| CueSnapshot {
                json: c.serialize(),
                decoded: c.extract_decoded_audio(),
            })
            .collect(),
        playhead_id: cue_list.playhead_cue_id,
    }
}

/// Restore a [`Snapshot`] into `cue_list`, rebuilding every cue via the
/// registry and re-injecting decoded audio so there is no re-decode round-trip.
fn restore_snapshot(
    snapshot: Snapshot,
    cue_list: &mut CueList,
    registry: &CueRegistry,
) -> anyhow::Result<()> {
    cue_list.cues.clear();
    for cs in snapshot.cues {
        let mut cue = registry.from_json(cs.json)?;
        if let Some((samples, channels, sr, dur)) = cs.decoded {
            cue.accept_preloaded_audio(samples, channels, sr, dur);
        }
        cue_list.cues.push(cue);
    }
    // Restore playhead; clear it if the referenced cue no longer exists.
    cue_list.playhead_cue_id = snapshot
        .playhead_id
        .filter(|id| cue_list.cues.iter().any(|c| c.id() == *id));
    Ok(())
}

/// Capture the current cue list state and push it onto the undo stack.
///
/// **Call this at the very start of every mutating command, before applying
/// any change.**  The function acquires and releases the workspace lock
/// separately from the undo_stack lock so no deadlock is possible.
pub fn push_current_snapshot(state: &AppState) -> Result<(), String> {
    // 1. Briefly lock the workspace to read the current state.
    let snapshot = {
        let ws = state.workspace.lock().map_err(|e| e.to_string())?;
        let cl = ws.active_cue_list().ok_or("No active cue list")?;
        take_snapshot(cl)
        // workspace lock released here
    };
    // 2. Push onto the undo stack (separate lock, no deadlock risk).
    state
        .undo_stack
        .lock()
        .map_err(|e| e.to_string())?
        .push_action(snapshot);
    Ok(())
}

// ---------------------------------------------------------------------------
// Undo / Redo
// ---------------------------------------------------------------------------

/// Whether there is at least one action that can be undone.
#[tauri::command]
pub fn can_undo(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state
        .undo_stack
        .lock()
        .map_err(|e| e.to_string())?
        .can_undo())
}

/// Whether there is at least one action that can be re-done.
#[tauri::command]
pub fn can_redo(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state
        .undo_stack
        .lock()
        .map_err(|e| e.to_string())?
        .can_redo())
}

/// Restore the cue list to its state before the most-recent mutating action.
#[tauri::command]
pub fn undo(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    // Lock order: registry → workspace → undo_stack.
    let registry = state.registry.lock().map_err(|e| e.to_string())?;
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let mut stack = state.undo_stack.lock().map_err(|e| e.to_string())?;

    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;
    let current = take_snapshot(cue_list);

    if let Some(prev) = stack.undo(current) {
        restore_snapshot(prev, cue_list, &registry).map_err(|e| e.to_string())?;
        ws.mark_modified();
    } else {
        return Ok(()); // nothing to undo
    }

    let playhead_id = ws
        .active_cue_list()
        .and_then(|cl| cl.playhead_cue_id)
        .map(|id| id.to_string());
    drop(stack);
    drop(ws);
    drop(registry);

    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    let _ = app_handle.emit(
        "playhead-moved",
        serde_json::json!({ "cue_id": playhead_id }),
    );
    Ok(())
}

/// Re-apply the most-recently undone action.
#[tauri::command]
pub fn redo(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    // Lock order: registry → workspace → undo_stack.
    let registry = state.registry.lock().map_err(|e| e.to_string())?;
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let mut stack = state.undo_stack.lock().map_err(|e| e.to_string())?;

    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;
    let current = take_snapshot(cue_list);

    if let Some(next) = stack.redo(current) {
        restore_snapshot(next, cue_list, &registry).map_err(|e| e.to_string())?;
        ws.mark_modified();
    } else {
        return Ok(()); // nothing to redo
    }

    let playhead_id = ws
        .active_cue_list()
        .and_then(|cl| cl.playhead_cue_id)
        .map(|id| id.to_string());
    drop(stack);
    drop(ws);
    drop(registry);

    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    let _ = app_handle.emit(
        "playhead-moved",
        serde_json::json!({ "cue_id": playhead_id }),
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Copy / Paste
// ---------------------------------------------------------------------------

/// Copy a cue into the in-app clipboard by serialising it to JSON.
///
/// The clipboard is internal to WinCue — it does not interact with the OS
/// clipboard.  Only one cue is stored at a time; copying a new one replaces
/// the previous entry.
#[tauri::command]
pub fn copy_cue(
    cue_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let id: Uuid = cue_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let cue_list = ws.active_cue_list().ok_or("No active cue list")?;
    let cue = cue_list.get(&id).ok_or("Cue not found")?;
    let json = cue.serialize();
    drop(ws);
    *state.clipboard.lock().map_err(|e| e.to_string())? = Some(json);
    Ok(())
}

/// Paste the clipboard cue as a new cue inserted after `after_cue_id`.
///
/// - If `after_cue_id` is `Some`, the new cue is inserted immediately after
///   the specified cue.
/// - If `after_cue_id` is `None`, the new cue is appended at the end.
///
/// The pasted cue gets a fresh UUID so it is independent of the original.
/// Returns the new cue's ID string.
#[tauri::command]
pub fn paste_cue(
    after_cue_id: Option<String>,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<String, String> {
    // 1. Clone the clipboard JSON (brief clipboard lock).
    let template = {
        let clip = state.clipboard.lock().map_err(|e| e.to_string())?;
        clip.clone().ok_or("Clipboard is empty — copy a cue first")?
    };

    // 2. Assign a fresh ID and rebuild the cue via the registry.
    let mut new_json = template;
    new_json["id"] = serde_json::json!(Uuid::new_v4().to_string());
    let new_cue = {
        let registry = state.registry.lock().map_err(|e| e.to_string())?;
        registry.from_json(new_json).map_err(|e| e.to_string())?
    };
    let new_id = new_cue.id().to_string();

    // 3. Push undo snapshot before mutating.
    push_current_snapshot(&state)?;

    // 4. Insert the new cue.
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    ws.mark_modified();
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;

    let insert_idx = match after_cue_id {
        Some(ref s) => {
            let after_id: Uuid = s.parse().map_err(|e: uuid::Error| e.to_string())?;
            cue_list
                .index_of(&after_id)
                .map(|i| i + 1)
                .unwrap_or(cue_list.cues.len())
        }
        None => cue_list.cues.len(),
    };

    cue_list.insert(insert_idx, new_cue);
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(new_id)
}
