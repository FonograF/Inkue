//! Video outputs — the windows a show's visual cues play on.
//!
//! The **main output** is configured by Preferences → Display (screen,
//! alignment) and always exists.  A show can add **extra outputs** (a façade
//! projector, a return monitor…), stored in the workspace; each Video / Image /
//! Camera / Text cue picks one by id (none = main).

use serde::Serialize;
use tauri::{Emitter, State};
use uuid::Uuid;

use crate::engine::output_engine::{OutputConfig, OutputId, OutputTransform, MAIN_OUTPUT};
use crate::show::Workspace;
use crate::state::AppState;

/// One output as the UI lists it: the main output first, then the extras.
#[derive(Debug, Clone, Serialize)]
pub struct VideoOutputInfo {
    /// Output id; the main output's is the nil UUID.
    pub id: String,
    pub name: String,
    pub is_main: bool,
    /// Monitor index (0 = primary); `None` = a floating window.
    pub screen: Option<u32>,
    pub transform: OutputTransform,
}

/// Parse the optional output id a command receives.  Absent, empty and the
/// nil UUID all mean the main output (`None`).
pub(crate) fn parse_output_id(id: Option<&str>) -> Result<Option<OutputId>, String> {
    match id.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(s) => {
            let id: Uuid = s.parse().map_err(|e: uuid::Error| format!("Invalid output id: {e}"))?;
            Ok((id != MAIN_OUTPUT).then_some(id))
        }
    }
}

fn list(ws: &Workspace) -> Vec<VideoOutputInfo> {
    let mut all = vec![VideoOutputInfo {
        id: MAIN_OUTPUT.to_string(),
        name: "Main".to_owned(),
        is_main: true,
        screen: ws.preferences.display.output_screen,
        transform: ws.preferences.display.output_transform,
    }];
    all.extend(ws.video_outputs.iter().map(|c| VideoOutputInfo {
        id: c.id.to_string(),
        name: c.name.clone(),
        is_main: false,
        screen: c.screen,
        transform: c.transform,
    }));
    all
}

/// Push the workspace's outputs to the engine and tell the UI.
fn publish(state: &AppState, app_handle: &tauri::AppHandle) {
    if let Ok(ws) = state.workspace.lock() {
        state.output_engine.sync_outputs_config(&ws.outputs_config());
    }
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
}

fn unknown_output() -> String {
    "Output not found".to_owned()
}

/// The main output followed by the show's extra outputs.
#[tauri::command]
pub fn list_video_outputs(state: State<'_, AppState>) -> Result<Vec<VideoOutputInfo>, String> {
    let ws = state.workspace.lock().map_err(|e| e.to_string())?;
    Ok(list(&ws))
}

/// Add an extra output (a floating window until a screen is assigned).
#[tauri::command]
pub fn add_video_output(
    name: String,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<VideoOutputInfo, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("An output needs a name".to_owned());
    }
    let created = OutputConfig::new(name);
    {
        let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
        if ws.video_outputs.iter().any(|c| c.name.eq_ignore_ascii_case(name)) {
            return Err(format!("An output named '{name}' already exists"));
        }
        ws.video_outputs.push(created.clone());
        ws.mark_modified();
    }
    publish(&state, &app_handle);
    Ok(VideoOutputInfo {
        id: created.id.to_string(),
        name: created.name,
        is_main: false,
        screen: created.screen,
        transform: created.transform,
    })
}

/// Rename an extra output.
#[tauri::command]
pub fn rename_video_output(
    output_id: String,
    name: String,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let id = parse_output_id(Some(&output_id))?.ok_or("The main output cannot be renamed")?;
    let name = name.trim();
    if name.is_empty() {
        return Err("An output needs a name".to_owned());
    }
    {
        let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
        if ws.video_outputs.iter().any(|c| c.id != id && c.name.eq_ignore_ascii_case(name)) {
            return Err(format!("An output named '{name}' already exists"));
        }
        let output = ws.video_outputs.iter_mut().find(|c| c.id == id).ok_or_else(unknown_output)?;
        output.name = name.to_owned();
        ws.mark_modified();
    }
    publish(&state, &app_handle);
    Ok(())
}

/// Put an output on a monitor (`None` = a floating window) and show it there.
/// The main output is addressed with an empty or nil id.
#[tauri::command]
pub fn set_video_output_screen(
    output_id: Option<String>,
    screen: Option<u32>,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let id = parse_output_id(output_id.as_deref())?;
    {
        let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
        match id {
            None => ws.preferences.display.output_screen = screen,
            Some(id) => {
                let output =
                    ws.video_outputs.iter_mut().find(|c| c.id == id).ok_or_else(unknown_output)?;
                output.screen = screen;
            }
        }
        ws.mark_modified();
    }
    // Place it first: the engine then already knows the new screen, and the
    // sync that `publish` triggers has nothing left to move.
    state.output_engine.apply_output_screen_of(id, screen);
    publish(&state, &app_handle);
    Ok(())
}

/// Delete an extra output: its window closes and whatever played on it stops.
/// Cues that pointed at it report that their output no longer exists (Check
/// Workspace lists them).
#[tauri::command]
pub fn delete_video_output(
    output_id: String,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let id = parse_output_id(Some(&output_id))?.ok_or("The main output cannot be deleted")?;
    {
        let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
        let before = ws.video_outputs.len();
        ws.video_outputs.retain(|c| c.id != id);
        if ws.video_outputs.len() == before {
            return Err(unknown_output());
        }
        ws.mark_modified();
    }
    publish(&state, &app_handle);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_empty_and_nil_ids_all_mean_the_main_output() {
        assert_eq!(parse_output_id(None), Ok(None));
        assert_eq!(parse_output_id(Some("")), Ok(None));
        assert_eq!(parse_output_id(Some("  ")), Ok(None));
        assert_eq!(parse_output_id(Some(&MAIN_OUTPUT.to_string())), Ok(None));
    }

    #[test]
    fn a_real_id_parses() {
        let id = Uuid::new_v4();
        assert_eq!(parse_output_id(Some(&id.to_string())), Ok(Some(id)));
    }

    #[test]
    fn garbage_is_rejected() {
        assert!(parse_output_id(Some("not-a-uuid")).is_err());
    }

    #[test]
    fn the_listing_starts_with_the_main_output_then_the_extras() {
        let mut ws = Workspace::new("T");
        ws.preferences.display.output_screen = Some(1);
        let extra = OutputConfig::new("Façade");
        ws.video_outputs.push(extra.clone());

        let all = list(&ws);
        assert_eq!(all.len(), 2);
        assert!(all[0].is_main);
        assert_eq!(all[0].screen, Some(1), "the main output mirrors Preferences → Display");
        assert_eq!(all[1].id, extra.id.to_string());
        assert!(!all[1].is_main);
    }
}
