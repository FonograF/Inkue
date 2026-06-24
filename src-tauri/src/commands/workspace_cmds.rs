//! Tauri commands for workspace save / load / new.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tauri::{Emitter, State};

use crate::{
    commands::cue_list_cmds,
    cue::types::CueType,
    show::{workspace::CollectReport, Workspace},
    state::AppState,
};

/// Recursively collect (id, path) pairs for every Audio and Video cue,
/// including those nested inside groups at any depth.
fn collect_media_cues(cues: &[Box<dyn crate::cue::traits::Cue>], out: &mut Vec<(uuid::Uuid, PathBuf)>) {
    for cue in cues {
        if matches!(cue.cue_type(), CueType::Audio | CueType::Video) {
            let json = cue.serialize();
            if let Some(p) = json.get("file_path").and_then(|v| v.as_str()) {
                if !p.is_empty() {
                    out.push((cue.id(), PathBuf::from(p)));
                }
            }
        }
        if let Some(children) = cue.child_cues() {
            collect_media_cues(children, out);
        }
    }
}

/// Create a new empty workspace, discarding the current one.
#[tauri::command]
pub fn new_workspace(state: State<'_, AppState>, app_handle: tauri::AppHandle) -> Result<(), String> {
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    *ws = Workspace::new("Untitled");
    cue_list_cmds::emit_cue_lists_changed(&app_handle, &ws);
    let outputs = ws.universe_outputs.clone();
    drop(ws);
    state.output_engine.set_floating_timer_visible(false);
    // A fresh workspace has no DMX outputs — clear any from the previous show.
    state.dmx_engine.set_outputs(outputs);
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(())
}

/// Save the workspace to the given path.
#[tauri::command]
pub fn save_workspace(path: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    ws.save(Some(PathBuf::from(path))).map_err(|e| e.to_string())
}

/// Load a workspace from the given path.
#[tauri::command]
pub fn load_workspace(
    path: String,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let registry = state.registry.lock().map_err(|e| e.to_string())?;
    let loaded = Workspace::load(PathBuf::from(&path), &registry)
        .map_err(|e| e.to_string())?;
    drop(registry);

    // Collect audio + video cue IDs + file paths before storing the workspace.
    // Scan ALL cue lists so non-active lists are also preloaded on open.
    let cues_to_preload: Vec<(uuid::Uuid, PathBuf)> = {
        let mut result = Vec::new();
        for cl in &loaded.cue_lists {
            collect_media_cues(&cl.cues, &mut result);
        }
        result
    };

    // Store the new workspace and apply display preferences.
    let show_floating = loaded.preferences.display.show_output_timer && loaded.preferences.display.timer_floating;
    let dmx_outputs = loaded.universe_outputs.clone();
    {
        let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
        *ws = loaded;
        cue_list_cmds::emit_cue_lists_changed(&app_handle, &ws);
    }
    state.output_engine.set_floating_timer_visible(show_floating);
    // Bind the engine's sinks to the loaded show's universe outputs.
    state.dmx_engine.set_outputs(dmx_outputs);
    {
        let mut loading = state.loading_cues.lock().map_err(|e| e.to_string())?;
        // Clear any stale entries from a previous workspace.
        loading.clear();
        for (id, _) in &cues_to_preload {
            loading.insert(*id);
        }
    }

    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));

    // Spawn a background preload thread for every audio cue that has a file.
    for (cue_id, file_path) in cues_to_preload {
        let workspace = state.workspace.clone();
        let loading_cues = state.loading_cues.clone();
        let app_handle2 = app_handle.clone();

        std::thread::Builder::new()
            .name("wincue-preload".into())
            .spawn(move || {
                match crate::cue::media_decode::decode_audio_track(&file_path) {
                    Ok(Some((samples, channels, sample_rate))) => {
                        let duration = Duration::from_secs_f64(
                            samples.len() as f64
                                / channels.max(1) as f64
                                / sample_rate.max(1) as f64,
                        );
                        let samples = Arc::new(samples);
                        // Search all cue lists — the cue may not be in the active one.
                        if let Ok(mut ws) = workspace.lock() {
                            'store: {
                                for cl in ws.cue_lists.iter_mut() {
                                    if let Some(cue) = cl.get_mut_recursive(&cue_id) {
                                        cue.accept_preloaded_audio(
                                            samples, channels, sample_rate, duration,
                                        );
                                        break 'store;
                                    }
                                }
                            }
                        }
                    }
                    Ok(None) => {} // silent video — nothing to preload
                    Err(e) => log::warn!("Preload failed for cue {cue_id}: {e}"),
                }
                if let Ok(mut loading) = loading_cues.lock() {
                    loading.remove(&cue_id);
                }
                let _ = app_handle2.emit("workspace-modified", serde_json::json!({}));
            })
            .expect("Failed to spawn preload thread");
    }

    Ok(())
}

/// Copy all media files into a self-contained project folder and write a
/// new `.wincue` file there with updated relative paths.
///
/// `target_dir` is the parent directory chosen by the user; the command
/// creates `{target_dir}/{workspace_name}/` automatically.
///
/// The workspace currently open in memory is not affected.
#[tauri::command]
pub fn collect_and_save_workspace(
    target_dir: String,
    state: State<'_, AppState>,
) -> Result<CollectReport, String> {
    let ws = state.workspace.lock().map_err(|e| e.to_string())?;
    ws.collect_and_save(std::path::Path::new(&target_dir))
        .map_err(|e| e.to_string())
}

/// Return basic workspace metadata for the title bar.
#[tauri::command]
pub fn get_workspace_info(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let ws = state.workspace.lock().map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "name": ws.metadata.name,
        "is_modified": ws.is_modified,
        "file_path": ws.file_path.as_ref().map(|p| p.to_string_lossy().to_string()),
    }))
}
