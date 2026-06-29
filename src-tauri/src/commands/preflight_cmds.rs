//! Preflight ("Check Workspace") + media relink commands.
//!
//! `check_workspace` walks every cue (all lists, nested groups) and returns the
//! ones whose external dependencies do not resolve — missing media file, dangling
//! Stop/Fade target, unpatched fixture, absent MIDI port, … — so the operator can
//! fix them before the show.  `relink_media` re-points a missing media file and,
//! as a convenience, re-points every other missing file found in the same folder.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use tauri::{Emitter, State};

use crate::{
    cue::{
        registry::CueRegistry,
        traits::Cue,
        types::{CueId, CueType},
        validation::{CueIssue, ValidationContext},
    },
    show::Workspace,
    state::AppState,
};

/// One cue's preflight result — only cues with at least one issue are returned.
#[derive(Debug, Clone, Serialize)]
pub struct CueValidation {
    pub cue_id: String,
    pub cue_number: Option<String>,
    pub cue_name: String,
    pub cue_type: CueType,
    pub issues: Vec<CueIssue>,
    /// The unresolved media path, when the cue's problem is a missing file.
    /// Drives the "Localiser…" relink action in the preflight panel.
    pub missing_file: Option<String>,
}

/// Result of a [`relink_media`] call.
#[derive(Debug, Clone, Serialize)]
pub struct RelinkResult {
    /// How many cues were re-pointed (the picked one plus auto-matched siblings).
    pub relinked: usize,
}

/// Names of MIDI output ports currently available on this machine.
fn available_midi_ports() -> Vec<String> {
    match midir::MidiOutput::new("Inkue-preflight") {
        Ok(out) => out.ports().iter().filter_map(|p| out.port_name(p).ok()).collect(),
        Err(e) => {
            log::warn!("[preflight] MIDI port enumeration failed: {e}");
            Vec::new()
        }
    }
}

/// Recursively collect every cue ID (groups + their children).
fn collect_ids(cues: &[Box<dyn Cue>], out: &mut HashSet<CueId>) {
    for c in cues {
        out.insert(c.id());
        if let Some(children) = c.child_cues() {
            collect_ids(children, out);
        }
    }
}

/// Recursively validate every cue, pushing those with issues into `out`.
fn walk_validate(cues: &[Box<dyn Cue>], ctx: &ValidationContext, out: &mut Vec<CueValidation>) {
    for c in cues {
        let mut issues = c.validate(ctx);
        let mut missing_file = None;

        // Central media-file check (covers Audio / Video / Image uniformly).
        if let Some(path) = c.media_file_path() {
            if !path.as_os_str().is_empty() && !path.exists() {
                issues.insert(
                    0,
                    CueIssue::error(format!("File not found: {}", path.display())),
                );
                missing_file = Some(path.to_string_lossy().to_string());
            }
        }

        if !issues.is_empty() {
            out.push(CueValidation {
                cue_id: c.id().to_string(),
                cue_number: c.number().map(|s| s.to_string()),
                cue_name: c.name().to_string(),
                cue_type: c.cue_type(),
                issues,
                missing_file,
            });
        }

        if let Some(children) = c.child_cues() {
            walk_validate(children, ctx, out);
        }
    }
}

/// Validate the whole workspace; return one entry per cue that has issues.
#[tauri::command]
pub fn check_workspace(state: State<'_, AppState>) -> Result<Vec<CueValidation>, String> {
    let ws = state.workspace.lock().map_err(|e| e.to_string())?;

    let mut all_cue_ids = HashSet::new();
    for cl in &ws.cue_lists {
        collect_ids(&cl.cues, &mut all_cue_ids);
    }

    let ctx = ValidationContext {
        all_cue_ids,
        fixture_ids: ws.fixtures.iter().map(|f| f.id).collect(),
        fixture_group_ids: ws.fixture_groups.iter().map(|g| g.id).collect(),
        osc_patch_ids: ws.osc_patches.iter().map(|p| p.id).collect(),
        output_patch_ids: ws.output_patches.iter().map(|p| p.id).collect(),
        midi_ports: available_midi_ports(),
    };

    let mut results = Vec::new();
    for cl in &ws.cue_lists {
        walk_validate(&cl.cues, &ctx, &mut results);
    }
    Ok(results)
}

// ---------------------------------------------------------------------------
// Relink
// ---------------------------------------------------------------------------

/// Rebuild the cue `id` (searched across all lists, nested) with `new_path` as
/// its media file, reusing the serialize→inject→from_json pattern so decoded
/// state is reset cleanly.  Returns the cue type on success.
fn set_cue_media_path(
    ws: &mut Workspace,
    registry: &CueRegistry,
    id: &CueId,
    new_path: &str,
) -> Option<CueType> {
    for cl in ws.cue_lists.iter_mut() {
        if let Some(cue) = cl.get_mut_recursive(id) {
            let cue_type = cue.cue_type();
            let mut json = cue.serialize();
            if let Some(obj) = json.as_object_mut() {
                obj.insert("file_path".into(), serde_json::json!(new_path));
            }
            let new_cue = registry.from_json(json).ok()?;
            cl.replace_cue_recursive(id, new_cue);
            return Some(cue_type);
        }
    }
    None
}

/// Collect `(cue_id, file_name)` for every cue (≠ `exclude`) whose media file is
/// currently missing — candidates for folder-based auto-relink.
fn collect_missing_media(cues: &[Box<dyn Cue>], exclude: &CueId, out: &mut Vec<(CueId, String)>) {
    for c in cues {
        if &c.id() != exclude {
            if let Some(path) = c.media_file_path() {
                if !path.as_os_str().is_empty() && !path.exists() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        out.push((c.id(), name.to_string()));
                    }
                }
            }
        }
        if let Some(children) = c.child_cues() {
            collect_missing_media(children, exclude, out);
        }
    }
}

/// Re-point cue `cue_id`'s media file to `new_path`, then re-point every other
/// missing-media cue whose filename is present in the same folder.
#[tauri::command]
pub fn relink_media(
    cue_id: String,
    new_path: String,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<RelinkResult, String> {
    super::undo_cmds::push_current_snapshot(&state)?;
    let id: CueId = cue_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let folder = Path::new(&new_path).parent().map(|p| p.to_path_buf());

    let mut to_preload: Vec<(CueId, CueType, PathBuf)> = Vec::new();
    {
        let registry = state.registry.lock().map_err(|e| e.to_string())?;
        let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
        ws.mark_modified();

        let cue_type = set_cue_media_path(&mut ws, &registry, &id, &new_path)
            .ok_or("Cue not found")?;
        to_preload.push((id, cue_type, PathBuf::from(&new_path)));

        // Apply the chosen folder to the workspace's other missing files.
        if let Some(folder) = folder {
            let mut missing: Vec<(CueId, String)> = Vec::new();
            for cl in &ws.cue_lists {
                collect_missing_media(&cl.cues, &id, &mut missing);
            }
            for (mid, file_name) in missing {
                let candidate = folder.join(&file_name);
                if candidate.exists() {
                    let cand_str = candidate.to_string_lossy().to_string();
                    if let Some(ct) = set_cue_media_path(&mut ws, &registry, &mid, &cand_str) {
                        to_preload.push((mid, ct, candidate));
                    }
                }
            }
        }
    }

    for (cid, cue_type, path) in &to_preload {
        spawn_media_preload(state.inner(), &app_handle, *cid, cue_type.clone(), path.clone());
    }
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));

    Ok(RelinkResult { relinked: to_preload.len() })
}

/// Background preload after a relink: probe video duration + decode the audio
/// track so the re-pointed cue is ready to play.  Image cues need nothing.
fn spawn_media_preload(
    state: &AppState,
    app_handle: &tauri::AppHandle,
    cue_id: CueId,
    cue_type: CueType,
    path: PathBuf,
) {
    if matches!(cue_type, CueType::Image) {
        return;
    }
    if let Ok(mut loading) = state.loading_cues.lock() {
        loading.insert(cue_id);
    }
    let workspace = state.workspace.clone();
    let loading_cues = state.loading_cues.clone();
    let output_engine = Arc::clone(&state.output_engine);
    let app_handle2 = app_handle.clone();
    let is_video = matches!(cue_type, CueType::Video);

    std::thread::Builder::new()
        .name("inkue-relink-preload".into())
        .spawn(move || {
            let duration = if is_video {
                crate::engine::OutputEngine::probe_duration(output_engine.mpv_lib(), &path)
            } else {
                None
            };
            let audio = crate::cue::media_decode::decode_audio_track(&path);

            if let Ok(mut ws) = workspace.lock() {
                'store: for cl in ws.cue_lists.iter_mut() {
                    if let Some(cue) = cl.get_mut_recursive(&cue_id) {
                        if let Some(dur) = duration {
                            cue.set_runtime_duration(dur);
                        }
                        if let Ok(Some((samples, channels, sample_rate))) = audio {
                            let dur = std::time::Duration::from_secs_f64(
                                samples.len() as f64
                                    / channels.max(1) as f64
                                    / sample_rate.max(1) as f64,
                            );
                            cue.accept_preloaded_audio(
                                Arc::new(samples),
                                channels,
                                sample_rate,
                                dur,
                            );
                        }
                        break 'store;
                    }
                }
            }
            if let Ok(mut loading) = loading_cues.lock() {
                loading.remove(&cue_id);
            }
            let _ = app_handle2.emit("workspace-modified", serde_json::json!({}));
        })
        .ok();
}
