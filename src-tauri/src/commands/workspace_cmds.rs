//! Tauri commands for workspace save / load / new.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tauri::{Emitter, State};

use crate::{
    commands::cue_list_cmds,
    cue::types::CueType,
    show::{workspace::CollectReport, Workspace},
    state::AppState,
};

/// Recursively collect (id, path, type) tuples for every Audio and Video cue,
/// including those nested inside groups at any depth.  The type lets the preload
/// worker surface a decode *failure* for Audio cues only (a video without an
/// audio track, or whose track fails, still plays — no operator warning).
fn collect_media_cues(
    cues: &[Box<dyn crate::cue::traits::Cue>],
    out: &mut Vec<(uuid::Uuid, PathBuf, CueType)>,
) {
    for cue in cues {
        if matches!(cue.cue_type(), CueType::Audio | CueType::Video) {
            let json = cue.serialize();
            if let Some(p) = json.get("file_path").and_then(|v| v.as_str()) {
                if !p.is_empty() {
                    out.push((cue.id(), PathBuf::from(p), cue.cue_type()));
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
    // Starting clean — retire any load-skip banner from the previous show.
    crate::health::clear("workspace-load-skips");
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(())
}

/// Save the workspace to the given path.
#[tauri::command]
pub fn save_workspace(path: String, state: State<'_, AppState>) -> Result<(), String> {
    {
        let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
        ws.save(Some(PathBuf::from(path))).map_err(|e| e.to_string())?;
    }
    // Work is now persisted to the real `.inkue` file — drop the crash-recovery
    // snapshot so a crash right after saving does not prompt a redundant restore.
    crate::recovery::delete();
    Ok(())
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

    install_workspace(state.inner(), &app_handle, loaded)
}

/// Install a freshly loaded/restored workspace into the running app: store it,
/// emit cue-list + modified events, rebind DMX outputs + floating timer, and
/// kick off background media preload.  Shared by [`load_workspace`] and the
/// crash-recovery restore path.
pub(crate) fn install_workspace(
    state: &AppState,
    app_handle: &tauri::AppHandle,
    loaded: Workspace,
) -> Result<(), String> {
    // Collect audio + video cue IDs + file paths before storing the workspace.
    // Scan ALL cue lists so non-active lists are also preloaded on open.
    let cues_to_preload: Vec<(uuid::Uuid, PathBuf, CueType)> = {
        let mut result = Vec::new();
        for cl in &loaded.cue_lists {
            collect_media_cues(&cl.cues, &mut result);
        }
        result
    };

    // Warn the operator when the file carried cues that could not be loaded
    // (unknown type / corrupt data) — otherwise they vanish with no signal.
    if loaded.cues_skipped_on_load > 0 {
        let n = loaded.cues_skipped_on_load;
        crate::health::set(crate::health::HealthAlert::new(
            "workspace-load-skips",
            crate::health::HealthLevel::Warning,
            format!("{n} cue(s) could not be loaded (unknown type or corrupt data) and were skipped."),
        ));
    } else {
        crate::health::clear("workspace-load-skips");
    }

    // Store the new workspace and apply display preferences.
    let show_floating = loaded.preferences.display.show_output_timer && loaded.preferences.display.timer_floating;
    let output_screen = loaded.preferences.display.output_screen;
    let dmx_outputs = loaded.universe_outputs.clone();
    {
        let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
        *ws = loaded;
        // Mirror the (just-loaded) auto-renumber preference onto the cue lists.
        ws.sync_auto_renumber();
        cue_list_cmds::emit_cue_lists_changed(app_handle, &ws);
    }
    state.output_engine.set_floating_timer_visible(show_floating);
    // Light the configured output screen right away (black fullscreen surface)
    // instead of waiting for the first visual GO.
    state.output_engine.apply_output_screen_on_load(output_screen);
    // Bind the engine's sinks to the loaded show's universe outputs.
    state.dmx_engine.set_outputs(dmx_outputs);
    {
        let mut loading = state.loading_cues.lock().map_err(|e| e.to_string())?;
        // Clear any stale entries from a previous workspace.
        loading.clear();
        for (id, _, _) in &cues_to_preload {
            loading.insert(*id);
        }
    }

    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));

    // Background media preload. A show can hold dozens of audio cues (a 58-cue
    // QLab import melted the machine): spawning one decode thread per cue was a
    // thundering herd — every thread fought for the workspace lock and emitted
    // `workspace-modified`, and each of those triggers a full UI refresh.
    //
    // Three fixes: (1) dedup by file — many cues point at the same file (a Canon
    // reuses one clip across sections), so decode each distinct file once and
    // share the decoded buffer (`Arc`) across every cue that uses it; (2) drain
    // the work with a small bounded pool; (3) let one coordinator coalesce the UI
    // refresh to a few times a second instead of once per cue.
    let mut cues_by_path: HashMap<PathBuf, Vec<(uuid::Uuid, bool)>> = HashMap::new();
    for (id, path, cue_type) in cues_to_preload {
        cues_by_path
            .entry(path)
            .or_default()
            .push((id, matches!(cue_type, CueType::Audio)));
    }

    let job_count = cues_by_path.len();
    if job_count > 0 {
        let (tx, rx) = crossbeam_channel::unbounded::<(PathBuf, Vec<(uuid::Uuid, bool)>)>();
        for job in cues_by_path {
            let _ = tx.send(job);
        }
        drop(tx); // close the queue so workers exit once it is drained

        let remaining = Arc::new(AtomicUsize::new(job_count));
        let dirty = Arc::new(AtomicBool::new(false));

        // Leave a core free for the audio callback + UI; cap peak decode memory.
        let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
        let workers = cpus.saturating_sub(1).clamp(1, 4).min(job_count);

        for _ in 0..workers {
            let rx = rx.clone();
            let workspace = state.workspace.clone();
            let loading_cues = state.loading_cues.clone();
            let dirty = Arc::clone(&dirty);
            let remaining = Arc::clone(&remaining);
            std::thread::Builder::new()
                .name("inkue-preload".into())
                .spawn(move || {
                    while let Ok((file_path, cues)) = rx.recv() {
                        match crate::cue::media_decode::decode_audio_track(&file_path) {
                            Ok(Some((samples, channels, sample_rate))) => {
                                let duration = Duration::from_secs_f64(
                                    samples.len() as f64
                                        / channels.max(1) as f64
                                        / sample_rate.max(1) as f64,
                                );
                                let samples = Arc::new(samples);
                                // Store the shared buffer into every cue using this
                                // file (searching all lists — a cue may be in a
                                // non-active list). Voices read the buffer read-only,
                                // so sharing one Arc across simultaneous plays is safe.
                                if let Ok(mut ws) = workspace.lock() {
                                    for (cue_id, _) in &cues {
                                        'store: for cl in ws.cue_lists.iter_mut() {
                                            if let Some(cue) = cl.get_mut_recursive(cue_id) {
                                                cue.accept_preloaded_audio(
                                                    Arc::clone(&samples), channels, sample_rate, duration,
                                                );
                                                break 'store;
                                            }
                                        }
                                    }
                                }
                                // Decoded fine — retire any stale decode-failure banners.
                                for (cue_id, is_audio) in &cues {
                                    if *is_audio {
                                        super::clear_decode_failure(*cue_id);
                                    }
                                }
                            }
                            Ok(None) => {} // silent video — nothing to preload
                            Err(e) => {
                                log::warn!("Preload failed for {}: {e}", file_path.display());
                                // An Audio cue that can't decode would be a silent
                                // no-op at GO — surface it instead of only logging.
                                for (cue_id, is_audio) in &cues {
                                    if *is_audio {
                                        super::surface_decode_failure(*cue_id, &file_path);
                                    }
                                }
                            }
                        }
                        if let Ok(mut loading) = loading_cues.lock() {
                            for (cue_id, _) in &cues {
                                loading.remove(cue_id);
                            }
                        }
                        dirty.store(true, Ordering::Relaxed);
                        remaining.fetch_sub(1, Ordering::Relaxed);
                    }
                })
                .expect("Failed to spawn preload worker");
        }

        // Coordinator: emit one coalesced `workspace-modified` at most ~4×/s while
        // preloads land, plus a final one when the batch is done. This replaces the
        // per-cue emit storm that re-rendered the whole cue list dozens of times.
        let app_handle2 = app_handle.clone();
        std::thread::Builder::new()
            .name("inkue-preload-coord".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_millis(250));
                let done = remaining.load(Ordering::Relaxed) == 0;
                if dirty.swap(false, Ordering::Relaxed) || done {
                    let _ = app_handle2.emit("workspace-modified", serde_json::json!({}));
                }
                if done {
                    break;
                }
            })
            .expect("Failed to spawn preload coordinator");
    }

    Ok(())
}

/// Copy all media files into a self-contained project folder and write a
/// new `.inkue` file there with updated relative paths.
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
