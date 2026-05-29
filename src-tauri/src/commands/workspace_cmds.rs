//! Tauri commands for workspace save / load / new.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tauri::{Emitter, State};

use crate::{
    cue::types::CueType,
    show::Workspace,
    state::AppState,
};

/// Create a new empty workspace, discarding the current one.
#[tauri::command]
pub fn new_workspace(state: State<'_, AppState>, app_handle: tauri::AppHandle) -> Result<(), String> {
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    *ws = Workspace::new("Untitled");
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

    // Collect audio cue IDs + file paths before storing the workspace.
    let cues_to_preload: Vec<(uuid::Uuid, PathBuf)> = loaded
        .active_cue_list()
        .map(|cl| {
            cl.cues
                .iter()
                .filter(|c| c.cue_type() == CueType::Audio)
                .filter_map(|c| {
                    let json = c.serialize();
                    let p = json.get("file_path")?.as_str()?;
                    if p.is_empty() {
                        return None;
                    }
                    Some((c.id(), PathBuf::from(p)))
                })
                .collect()
        })
        .unwrap_or_default();

    // Store the new workspace and mark all audio cues as loading.
    {
        let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
        *ws = loaded;
    }
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
                match crate::cue::audio_cue::AudioCue::decode_file(&file_path) {
                    Ok((samples, channels, sample_rate)) => {
                        let duration = Duration::from_secs_f64(
                            samples.len() as f64 / channels as f64 / sample_rate as f64,
                        );
                        let samples = Arc::new(samples);
                        if let Ok(mut ws) = workspace.lock() {
                            if let Some(cl) = ws.active_cue_list_mut() {
                                if let Some(cue) = cl.get_mut(&cue_id) {
                                    cue.accept_preloaded_audio(
                                        samples, channels, sample_rate, duration,
                                    );
                                }
                            }
                        }
                        if let Ok(mut loading) = loading_cues.lock() {
                            loading.remove(&cue_id);
                        }
                        let _ = app_handle2.emit("workspace-modified", serde_json::json!({}));
                    }
                    Err(e) => {
                        if let Ok(mut loading) = loading_cues.lock() {
                            loading.remove(&cue_id);
                        }
                        log::warn!("Preload failed for cue {cue_id}: {e}");
                        let _ = app_handle2.emit("workspace-modified", serde_json::json!({}));
                    }
                }
            })
            .expect("Failed to spawn preload thread");
    }

    // Pre-arm the video cue at the playhead so the first GO is instant.
    {
        let ws = state.workspace.lock().map_err(|e| e.to_string())?;
        if let Some(cl) = ws.active_cue_list() {
            crate::show::video_pre_arm::update_video_pre_arm(
                cl.playhead_cue_id,
                cl,
                &state.output_engine,
                ws.preferences.display.output_screen,
            );
        }
    }

    Ok(())
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
