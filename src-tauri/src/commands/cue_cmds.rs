//! Tauri commands for cue CRUD operations.

use std::sync::{atomic::Ordering, Arc};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};
use uuid::Uuid;

use crate::{
    cue::{
        traits::Cue,
        types::{ContinueMode, CueColor, CueState, CueType, GroupMode},
    },
    engine::{ring_command::FadeCurve, voice::Voice},
    state::AppState,
};

// ---------------------------------------------------------------------------
// DTO types
// ---------------------------------------------------------------------------

/// Compact summary of a cue, used to populate the cue list table in the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CueSummary {
    pub id: String,
    pub cue_type: CueType,
    pub name: String,
    pub number: Option<String>,
    /// Free-form notes visible in the Notes column and inspector.
    pub notes: String,
    pub state: CueState,
    pub continue_mode: ContinueMode,
    pub color: CueColor,
    pub pre_wait_ms: u64,
    pub post_wait_ms: u64,
    pub duration_ms: Option<u64>,
    /// File path for audio cues, None for others.
    pub file_path: Option<String>,
    /// True while the audio file is being decoded in a background thread.
    pub is_loading: bool,
    /// True when this cue is disabled — skipped by the transport on GO.
    pub is_disabled: bool,
    /// True when this cue's media file was assigned but is now missing from disk.
    pub is_broken: bool,
    /// True for non-critical problems (no file assigned, zero duration, empty group).
    pub is_warning: bool,
    /// Duration of one loop iteration (file duration without start/end markers applied
    /// and without the loop-count multiplier).  `None` for non-media cues.
    pub file_duration_ms: Option<u64>,
    /// Human-readable description of the warning condition, when `is_warning` is true.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning_message: Option<String>,
    /// For Group cues: their direct children summaries (recursive).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<CueSummary>>,
    /// For Group cues: the playback mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_mode: Option<GroupMode>,
    /// For running Sequential Group cues: ID of the currently active child
    /// (running right now or next to fire on GO after a DoNotContinue pause).
    /// `None` for Simultaneous groups and non-Group cues.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_child_id: Option<String>,
}

/// Returns `true` when a media cue's file was assigned but is missing from disk.
fn check_broken(cue: &dyn Cue, workspace_dir: Option<&std::path::Path>) -> bool {
    match cue.cue_type() {
        CueType::Audio | CueType::Video | CueType::Image => {
            match cue.media_file_path() {
                None => false,
                Some(p) if p.as_os_str().is_empty() => false,
                Some(p) => {
                    if p.is_absolute() {
                        !p.exists()
                    } else {
                        workspace_dir.map(|d| !d.join(p).exists()).unwrap_or(true)
                    }
                }
            }
        }
        _ => false,
    }
}

/// Returns a warning message for non-critical problems, or `None` if the cue is healthy.
fn check_warning(cue: &dyn Cue) -> Option<String> {
    match cue.cue_type() {
        CueType::Audio | CueType::Video | CueType::Image => {
            match cue.media_file_path() {
                None => Some("No file assigned".to_string()),
                Some(p) if p.as_os_str().is_empty() => Some("No file assigned".to_string()),
                _ => None,
            }
        }
        CueType::Wait => {
            if cue.duration() == Some(std::time::Duration::ZERO) {
                Some("Duration is zero".to_string())
            } else {
                None
            }
        }
        CueType::Group => {
            if cue.child_cues().map(|c| c.is_empty()).unwrap_or(false) {
                Some("Group is empty".to_string())
            } else {
                None
            }
        }
        _ => None,
    }
}

fn summarise(cue: &dyn Cue, workspace_dir: Option<&std::path::Path>) -> CueSummary {
    let warning_message = check_warning(cue);
    CueSummary {
        id: cue.id().to_string(),
        cue_type: cue.cue_type(),
        name: cue.name().to_string(),
        number: cue.number().map(|s| s.to_string()),
        notes: cue.notes().to_string(),
        state: cue.state(),
        continue_mode: cue.continue_mode(),
        color: cue.color(),
        pre_wait_ms: cue.pre_wait().as_millis() as u64,
        post_wait_ms: cue.post_wait().as_millis() as u64,
        duration_ms: cue.duration().map(|d| d.as_millis() as u64),
        file_path: cue.media_file_path().map(|p| p.to_string_lossy().into_owned()),
        is_loading: false,
        is_disabled: cue.is_disabled(),
        is_broken: check_broken(cue, workspace_dir),
        is_warning: warning_message.is_some(),
        warning_message,
        file_duration_ms: cue.file_duration().map(|d| d.as_millis() as u64),
        children: cue.child_cues().map(|ch| {
            ch.iter().map(|c| summarise_recursive(c.as_ref(), workspace_dir)).collect()
        }),
        group_mode: cue.group_mode(),
        active_child_id: cue.active_child_id().map(|id| id.to_string()),
    }
}

/// Recursively build a CueSummary, including children for Group cues.
fn summarise_recursive(cue: &dyn Cue, workspace_dir: Option<&std::path::Path>) -> CueSummary {
    summarise(cue, workspace_dir)
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Return a compact summary of every cue in the active cue list.
#[tauri::command]
pub fn get_all_cues(state: State<'_, AppState>) -> Result<Vec<CueSummary>, String> {
    let ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let loading = state.loading_cues.lock().map_err(|e| e.to_string())?;
    let cue_list = ws.active_cue_list().ok_or("No active cue list")?;
    let ws_dir = ws.file_path.as_ref().and_then(|p| p.parent()).map(|p| p.to_owned());
    let ws_dir_ref = ws_dir.as_deref();

    let summaries: Vec<CueSummary> = cue_list
        .cues
        .iter()
        .map(|c| {
            let mut s = summarise(c.as_ref(), ws_dir_ref);
            s.is_loading = loading.contains(&c.id());
            s
        })
        .collect();

    Ok(summaries)
}

/// Return the full serialised JSON for a single cue.
#[tauri::command]
pub fn get_cue(cue_id: String, state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let id: Uuid = cue_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let cue_list = ws.active_cue_list().ok_or("No active cue list")?;
    let cue = cue_list.get_recursive(&id).ok_or("Cue not found")?;
    Ok(cue.serialize())
}

/// Add a new cue of the given type at the given position (index).
/// Pass `position = -1` to append at the end.
#[tauri::command]
pub fn add_cue(
    cue_type: CueType,
    position: i64,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<String, String> {
    super::undo_cmds::push_current_snapshot(&state)?;
    let registry = state.registry.lock().map_err(|e| e.to_string())?;
    let cue = registry.create(&cue_type).map_err(|e| e.to_string())?;
    let id = cue.id().to_string();
    drop(registry);

    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    ws.mark_modified();
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;

    if position < 0 || position as usize >= cue_list.cues.len() {
        cue_list.push(cue);
    } else {
        cue_list.insert(position as usize, cue);
    }

    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(id)
}

/// Remove a cue by ID.
#[tauri::command]
pub fn remove_cue(
    cue_id: String,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    super::undo_cmds::push_current_snapshot(&state)?;
    let id: Uuid = cue_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    ws.mark_modified();
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;
    cue_list.remove_anywhere(&id).map_err(|e| e.to_string())?;
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(())
}

/// Move a cue to a new position.
#[tauri::command]
pub fn move_cue(
    cue_id: String,
    new_position: usize,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    super::undo_cmds::push_current_snapshot(&state)?;
    let id: Uuid = cue_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    ws.mark_modified();
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;
    cue_list.move_cue(&id, new_position).map_err(|e| e.to_string())?;
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(())
}

/// Remove multiple cues in one atomic operation.
#[tauri::command]
pub fn remove_cues(
    ids: Vec<String>,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    super::undo_cmds::push_current_snapshot(&state)?;
    let ids: Vec<Uuid> = ids
        .iter()
        .map(|s| s.parse::<Uuid>().map_err(|e| e.to_string()))
        .collect::<Result<_, _>>()?;
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    ws.mark_modified();
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;
    cue_list.remove_many_anywhere(&ids).map_err(|e| e.to_string())?;
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(())
}

/// Move a group of cues immediately before `before_id`, or to the end if `None`.
#[tauri::command]
pub fn move_cues(
    ids: Vec<String>,
    before_id: Option<String>,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    super::undo_cmds::push_current_snapshot(&state)?;
    let ids: Vec<Uuid> = ids
        .iter()
        .map(|s| s.parse::<Uuid>().map_err(|e| e.to_string()))
        .collect::<Result<_, _>>()?;
    let before_id: Option<Uuid> = before_id
        .as_deref()
        .map(|s| s.parse::<Uuid>().map_err(|e| e.to_string()))
        .transpose()?;
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    ws.mark_modified();
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;
    cue_list.move_before(&ids, before_id).map_err(|e| e.to_string())?;
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(())
}

/// Duplicate a cue (creates a copy with a new ID, inserted immediately after).
#[tauri::command]
pub fn duplicate_cue(
    cue_id: String,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<String, String> {
    super::undo_cmds::push_current_snapshot(&state)?;
    let id: Uuid = cue_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let registry = state.registry.lock().map_err(|e| e.to_string())?;

    let (json, preserved_audio) = {
        let cue_list = ws.active_cue_list().ok_or("No active cue list")?;
        let cue = cue_list.get_recursive(&id).ok_or("Cue not found")?;
        let mut j = cue.serialize();
        // Assign a new UUID to the copy.
        j["id"] = serde_json::json!(Uuid::new_v4().to_string());
        // Transfer decoded audio so the copy is playable immediately,
        // without requiring a background re-decode.
        let audio = cue.extract_decoded_audio();
        (j, audio)
    };

    let mut new_cue = registry.from_json(json).map_err(|e| e.to_string())?;
    if let Some((samples, channels, sample_rate, duration)) = preserved_audio {
        new_cue.accept_preloaded_audio(samples, channels, sample_rate, duration);
    }
    let new_id = new_cue.id().to_string();
    drop(registry);

    ws.mark_modified();
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;
    // Insert the copy right after its source, wherever it lives — so duplicating
    // a cue nested in a group keeps the copy in that same group.
    cue_list
        .insert_after_anywhere(&id, new_cue)
        .map_err(|e| e.to_string())?;

    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(new_id)
}

/// Duplicate multiple cues, inserting each copy immediately after its own
/// source (so a copy of a cue nested in a group stays in that group).
#[tauri::command]
pub fn duplicate_cues(
    ids: Vec<String>,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<Vec<String>, String> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    super::undo_cmds::push_current_snapshot(&state)?;
    let ids: Vec<Uuid> = ids
        .iter()
        .map(|s| s.parse::<Uuid>().map_err(|e| e.to_string()))
        .collect::<Result<_, _>>()?;

    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let registry = state.registry.lock().map_err(|e| e.to_string())?;

    // Collect serialised copies (paired with their source ID) and preserved
    // audio for each source cue.  Recursive lookup so group children duplicate.
    let copies = {
        let cue_list = ws.active_cue_list().ok_or("No active cue list")?;
        ids.iter()
            .map(|id| {
                let cue = cue_list.get_recursive(id).ok_or_else(|| format!("Cue {id:?} not found"))?;
                let mut j = cue.serialize();
                j["id"] = serde_json::json!(Uuid::new_v4().to_string());
                let audio = cue.extract_decoded_audio();
                Ok((*id, j, audio))
            })
            .collect::<Result<Vec<_>, String>>()?
    };

    ws.mark_modified();
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;
    let mut new_ids = Vec::with_capacity(copies.len());
    for (src_id, json, audio) in copies {
        let mut new_cue = registry.from_json(json).map_err(|e| e.to_string())?;
        if let Some((samples, channels, sample_rate, duration)) = audio {
            new_cue.accept_preloaded_audio(samples, channels, sample_rate, duration);
        }
        new_ids.push(new_cue.id().to_string());
        cue_list
            .insert_after_anywhere(&src_id, new_cue)
            .map_err(|e| e.to_string())?;
    }
    drop(registry);
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(new_ids)
}

/// Update cue properties from a partial JSON object.
///
/// All fields present in `properties` are merged into the cue's serialised
/// form and the cue is rebuilt via the [`CueRegistry`].  This correctly
/// handles both generic trait fields (name, number, …) and type-specific
/// fields (volume_db, pan, fade_in_ms, …) without any unsafe downcasting.
#[tauri::command]
pub fn update_cue(
    cue_id: String,
    properties: serde_json::Value,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    super::undo_cmds::push_current_snapshot(&state)?;
    let id: Uuid = cue_id.parse().map_err(|e: uuid::Error| e.to_string())?;

    // Lock order: registry first, then workspace (matches duplicate_cue).
    let registry = state.registry.lock().map_err(|e| e.to_string())?;
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    ws.mark_modified();
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;

    // Serialise → merge → rebuild, working recursively so child cues inside
    // groups can also be updated from the inspector.
    let (mut json, preserved_audio, runtime) = {
        let cue = cue_list
            .get_mut_recursive(&id)
            .ok_or("Cue not found")?;
        let mut json = cue.serialize();
        let old_file_path = json.get("file_path").and_then(|v| v.as_str()).map(|s| s.to_string());

        if let (Some(target), Some(src)) = (json.as_object_mut(), properties.as_object()) {
            for (k, v) in src {
                target.insert(k.clone(), v.clone());
            }
        }

        let new_file_path = json.get("file_path").and_then(|v| v.as_str()).map(|s| s.to_string());
        let preserved_audio = if old_file_path == new_file_path { cue.extract_decoded_audio() } else { None };
        let runtime = cue.runtime_state();
        (json, preserved_audio, runtime)
    };

    // Suppress unused-variable warning when merge produced no change.
    let _ = &mut json;

    let mut new_cue = registry.from_json(json).map_err(|e| e.to_string())?;
    if let Some((samples, channels, sample_rate, duration)) = preserved_audio {
        new_cue.accept_preloaded_audio(samples, channels, sample_rate, duration);
    }
    new_cue.restore_runtime_state(runtime);
    cue_list.replace_cue_recursive(&id, new_cue);

    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(())
}

/// Set the Playhead to a specific cue.
#[tauri::command]
pub fn set_playhead(
    cue_id: Option<String>,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let id: Option<Uuid> = cue_id
        .as_deref()
        .map(|s| s.parse::<Uuid>().map_err(|e| e.to_string()))
        .transpose()?;

    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;
    cue_list.set_playhead(id).map_err(|e| e.to_string())?;
    // The resulting outer Playhead may differ from `id` (a group child parks the
    // Playhead on its ancestor group), so emit the actual outer Playhead.
    let outer_playhead = cue_list.playhead_cue_id;

    let _ = app_handle.emit(
        "playhead-moved",
        serde_json::json!({ "cue_id": outer_playhead.map(|u| u.to_string()) }),
    );
    // Only when the target was a nested cue (resolved to a different outer
    // Playhead) refresh cues so the group's inner playhead (active_child_id)
    // updates — top-level clicks don't need the extra round-trip.
    if id != outer_playhead {
        let _ = app_handle.emit("cue-list-refresh", serde_json::json!({}));
    }
    Ok(())
}

/// Return the current Playhead cue ID (or null).
#[tauri::command]
pub fn get_playhead(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let cue_list = ws.active_cue_list().ok_or("No active cue list")?;
    Ok(cue_list.playhead_cue_id.map(|u| u.to_string()))
}

// ---------------------------------------------------------------------------
// Waveform
// ---------------------------------------------------------------------------

/// Downsampled peak data returned by [`get_waveform_peaks`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaveformData {
    /// Peak values (0.0 – 1.0), one per bin.
    pub peaks: Vec<f32>,
    /// Full file duration in seconds (ignoring start/end markers).
    pub file_duration_s: f64,
}

/// Return waveform peak data for an audio cue.
///
/// `bins` controls the number of columns (typically 400–800 for UI use).
/// Returns an error if the cue has not been decoded yet.
#[tauri::command]
pub fn get_waveform_peaks(
    cue_id: String,
    bins: usize,
    state: State<'_, AppState>,
) -> Result<WaveformData, String> {
    let id: Uuid = cue_id.parse().map_err(|e: uuid::Error| e.to_string())?;

    // Hold the workspace lock only long enough to clone the decoded audio Arc.
    // Peak computation can be expensive (large files) so we do it AFTER releasing
    // the lock so concurrent GO commands are never blocked.
    let (samples, channels, _sample_rate, file_duration) = {
        let ws = state.workspace.lock().map_err(|e| e.to_string())?;
        let cue_list = ws.active_cue_list().ok_or("No active cue list")?;
        let cue = cue_list.get_recursive(&id).ok_or("Cue not found")?;
        cue.extract_decoded_audio()
            .ok_or("Audio not loaded yet — assign a file first")?
        // workspace lock dropped here
    };

    // Compute peaks outside the lock.
    let channels = channels as usize;
    let total_frames = samples.len().checked_div(channels).unwrap_or(0);
    let peaks = if bins == 0 || total_frames == 0 {
        vec![]
    } else {
        (0..bins)
            .map(|i| {
                let start = (i * total_frames) / bins;
                let end = (((i + 1) * total_frames) / bins).max(start + 1);
                let mut peak = 0.0f32;
                for frame in start..end.min(total_frames) {
                    for ch in 0..channels {
                        let v = samples[frame * channels + ch].abs();
                        if v > peak {
                            peak = v;
                        }
                    }
                }
                peak
            })
            .collect()
    };

    Ok(WaveformData {
        peaks,
        file_duration_s: file_duration.as_secs_f64(),
    })
}

/// Compute the `volume_db` that normalises this audio cue's peak to 0 dBFS.
///
/// Reads the already-decoded samples (non-destructively via `Arc::clone`),
/// finds the absolute peak, and returns `20 × log10(1 / peak)` — the gain
/// the fader must be set to so the loudest sample plays at exactly 0 dBFS.
///
/// Errors if the audio has not been decoded yet or if the file is silent.
#[tauri::command]
pub fn get_normalize_db(
    cue_id: String,
    state: State<'_, AppState>,
) -> Result<f64, String> {
    let id: Uuid = cue_id.parse().map_err(|e: uuid::Error| e.to_string())?;

    let samples = {
        let ws = state.workspace.lock().map_err(|e| e.to_string())?;
        let cue_list = ws.active_cue_list().ok_or("No active cue list")?;
        let cue = cue_list.get_recursive(&id).ok_or("Cue not found")?;
        cue.extract_decoded_audio()
            .ok_or("Audio not loaded yet — open the file first")?
            .0
        // workspace lock released here
    };

    let peak: f32 = samples.iter().map(|s| s.abs()).fold(0.0_f32, f32::max);

    if peak < 1e-6 {
        return Err("File is silent — cannot normalize".into());
    }

    let normalize_db = (1.0_f64 / peak as f64).log10() * 20.0;
    Ok(normalize_db.clamp(-60.0, 12.0))
}

/// Set the file path of an audio cue.
/// Uses the same JSON-merge-and-rebuild strategy as [`update_cue`].
#[tauri::command]
pub fn set_audio_file(
    cue_id: String,
    file_path: String,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    super::undo_cmds::push_current_snapshot(&state)?;
    let id: Uuid = cue_id.parse().map_err(|e: uuid::Error| e.to_string())?;

    let registry = state.registry.lock().map_err(|e| e.to_string())?;
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    ws.mark_modified();
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;

    let json = {
        let cue = cue_list.get_mut_recursive(&id).ok_or("Cue not found")?;
        if cue.cue_type() != CueType::Audio {
            return Err("set_audio_file only applies to Audio Cues".to_string());
        }
        let mut json = cue.serialize();
        if let Some(obj) = json.as_object_mut() {
            obj.insert("file_path".to_string(), serde_json::json!(file_path));
        }
        json
    };
    let new_cue = registry.from_json(json).map_err(|e| e.to_string())?;
    drop(registry);
    cue_list.replace_cue_recursive(&id, new_cue);
    // Mark as loading before dropping the workspace lock.
    {
        let mut loading = state.loading_cues.lock().map_err(|e| e.to_string())?;
        loading.insert(id);
    }

    drop(ws); // release workspace lock immediately — do NOT decode while locked

    // Notify the frontend: filename appears in Target column, loading indicator shows.
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));

    // Spawn a background thread to decode the audio file and push the result
    // back via accept_preloaded_audio.  The workspace mutex is only held for
    // the brief moment needed to store the decoded samples — never during I/O.
    let workspace = state.workspace.clone();
    let loading_cues = state.loading_cues.clone();
    let app_handle2 = app_handle.clone();
    let file_path_buf = std::path::PathBuf::from(file_path);

    std::thread::Builder::new()
        .name("wincue-preload".into())
        .spawn(move || {
            // Run at below-normal OS priority so decoding never starves the
            // audio callback thread or causes fan spin-up on the host machine.
            // The decode is I/O + CPU bound; at BELOW_NORMAL it finishes in the
            // same wall-clock time when the system is otherwise idle, but yields
            // automatically under load.
            #[cfg(windows)]
            // SAFETY: only changes the scheduling priority of this thread.
            unsafe {
                use std::os::raw::c_void;
                extern "system" {
                    fn GetCurrentThread() -> *mut c_void;
                    fn SetThreadPriority(h_thread: *mut c_void, n_priority: i32) -> i32;
                }
                SetThreadPriority(GetCurrentThread(), -1); // THREAD_PRIORITY_BELOW_NORMAL
            }

            match crate::cue::audio_cue::AudioCue::decode_file(&file_path_buf) {
                Ok((samples, channels, sample_rate)) => {
                    let duration = std::time::Duration::from_secs_f64(
                        samples.len() as f64 / channels as f64 / sample_rate as f64,
                    );
                    let samples = std::sync::Arc::new(samples);
                    // Brief lock: store the decoded data in whichever list contains the cue.
                    // We search all lists (not just the active one) because the user may
                    // have switched cue lists while the background decode was running.
                    if let Ok(mut ws) = workspace.lock() {
                        'store: {
                            for cl in ws.cue_lists.iter_mut() {
                                if let Some(cue) = cl.get_mut(&id) {
                                    cue.accept_preloaded_audio(
                                        samples, channels, sample_rate, duration,
                                    );
                                    break 'store;
                                }
                            }
                        }
                    }
                    if let Ok(mut loading) = loading_cues.lock() {
                        loading.remove(&id);
                    }
                    // Update duration column in the UI.
                    let _ = app_handle2.emit("workspace-modified", serde_json::json!({}));
                }
                Err(e) => {
                    if let Ok(mut loading) = loading_cues.lock() {
                        loading.remove(&id);
                    }
                    log::warn!("Background preload failed for {:?}: {e}", file_path_buf);
                    let _ = app_handle2.emit("workspace-modified", serde_json::json!({}));
                    let _ = app_handle2.emit("cue-load-error", serde_json::json!({
                        "cue_id": id.to_string(),
                        "error": e.to_string(),
                    }));
                }
            }
        })
        .expect("Failed to spawn preload thread");

    Ok(())
}

// ---------------------------------------------------------------------------
// Waveform preview (plays a temporary voice without touching cue state)
// ---------------------------------------------------------------------------

/// Start a preview playback of a cue's audio between `start_ms` and `end_ms`.
///
/// Plays through the audio engine as an independent voice — the cue's own
/// state, playhead, and runtime fields are completely untouched.  Returns the
/// voice ID needed to stop the preview via [`stop_preview`].
#[tauri::command]
pub fn preview_cue(
    cue_id: String,
    start_ms: Option<u64>,
    end_ms: Option<u64>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let id: Uuid = cue_id.parse().map_err(|e: uuid::Error| e.to_string())?;

    // Brief lock: clone the decoded audio Arc and grab volume/pan.
    let (samples, channels, sample_rate, volume_db, pan) = {
        let ws = state.workspace.lock().map_err(|e| e.to_string())?;
        let cue_list = ws.active_cue_list().ok_or("No active cue list")?;
        let cue = cue_list.get_recursive(&id).ok_or("Cue not found")?;
        let (samples, channels, sample_rate, _dur) = cue
            .extract_decoded_audio()
            .ok_or("Audio not loaded yet — assign a file first")?;
        let json = cue.serialize();
        let volume_db = json.get("volume_db").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let pan = json.get("pan").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
        (samples, channels, sample_rate, volume_db, pan)
        // workspace lock released here
    };

    let gain = crate::cue::types::db_to_linear(volume_db) as f32;
    let voice = Voice::new(samples, channels, sample_rate, gain, pan);

    // Compensate for any sample-rate mismatch between the audio file and the
    // output device so the preview plays at the correct speed and pitch.
    let device_sr = state.audio_engine.sample_rate();
    if device_sr > 0 && sample_rate != device_sr {
        voice.inner.set_rate(sample_rate as f32 / device_sr as f32);
    }

    // Apply start / end frame offsets.
    if let Some(end) = end_ms {
        let end_frame = (end as f64 / 1000.0 * sample_rate as f64) as u64;
        // SAFETY: written once before play_voice(); no RT thread has the voice yet.
        unsafe { *voice.inner.end_frame.get() = Some(end_frame); }
    }
    if let Some(start) = start_ms {
        let start_frame = (start as f64 / 1000.0 * sample_rate as f64) as u64;
        voice.frame_pos.store(start_frame, Ordering::Relaxed);
    }

    let voice_id = state
        .audio_engine
        .play_voice(voice)
        .map_err(|e| e.to_string())?;

    Ok(voice_id.to_string())
}

/// Immediately stop a preview voice started by [`preview_cue`].
#[tauri::command]
pub fn stop_preview(voice_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let id: Uuid = voice_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    state
        .audio_engine
        .stop_voice(id, 0, FadeCurve::Linear)
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Video file management
// ---------------------------------------------------------------------------

/// Set the file path of a Video Cue.
///
/// Unlike [`set_audio_file`], no background decoding is needed — the video
/// streams directly from disk when the cue is triggered.
#[tauri::command]
pub fn set_video_file(
    cue_id: String,
    file_path: String,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    super::undo_cmds::push_current_snapshot(&state)?;
    let id: Uuid = cue_id.parse().map_err(|e: uuid::Error| e.to_string())?;

    let registry = state.registry.lock().map_err(|e| e.to_string())?;
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    ws.mark_modified();
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;

    let idx = cue_list.index_of(&id).ok_or("Cue not found")?;
    if cue_list.cues[idx].cue_type() != CueType::Video {
        return Err("set_video_file only applies to Video Cues".to_string());
    }

    let mut json = cue_list.cues[idx].serialize();
    if let Some(obj) = json.as_object_mut() {
        obj.insert("file_path".to_string(), serde_json::json!(file_path));
    }
    let new_cue = registry.from_json(json).map_err(|e| e.to_string())?;
    drop(registry);
    cue_list.cues[idx] = new_cue;

    // Mark as loading — the audio track is decoded off-thread (the indicator
    // clears when decoding finishes), mirroring Audio Cues.
    {
        let mut loading = state.loading_cues.lock().map_err(|e| e.to_string())?;
        loading.insert(id);
    }

    drop(ws); // release the workspace lock before any background work

    // Probe the video duration (mpv) and decode the audio track (symphonia) so
    // the cue shows its length and has synced audio ready before the first GO.
    {
        let path = std::path::PathBuf::from(&file_path);
        let cue_id = id;
        let output_engine = Arc::clone(&state.output_engine);
        let workspace2 = Arc::clone(&state.workspace);
        let loading_cues = state.loading_cues.clone();
        let handle2 = app_handle.clone();
        std::thread::Builder::new()
            .name("wincue-video-load".into())
            .spawn(move || {
                let lib = output_engine.mpv_lib();
                let duration = crate::engine::OutputEngine::probe_duration(lib, &path);
                let audio = crate::cue::media_decode::decode_audio_track(&path);

                // Search all cue lists — the user may have switched lists while loading.
                if let Ok(mut ws) = workspace2.lock() {
                    'store: {
                        for cl in ws.cue_lists.iter_mut() {
                            if let Some(idx2) = cl.index_of(&cue_id) {
                                if let Some(dur) = duration {
                                    cl.cues[idx2].set_runtime_duration(dur);
                                }
                                match audio {
                                    Ok(Some((samples, channels, sample_rate))) => {
                                        let dur = std::time::Duration::from_secs_f64(
                                            samples.len() as f64
                                                / channels.max(1) as f64
                                                / sample_rate.max(1) as f64,
                                        );
                                        cl.cues[idx2].accept_preloaded_audio(
                                            std::sync::Arc::new(samples),
                                            channels,
                                            sample_rate,
                                            dur,
                                        );
                                    }
                                    Ok(None) => {} // silent video — no audio track
                                    Err(e) => {
                                        log::warn!("Video audio decode failed for {path:?}: {e}");
                                    }
                                }
                                break 'store;
                            }
                        }
                    }
                }
                if let Ok(mut loading) = loading_cues.lock() {
                    loading.remove(&cue_id);
                }
                let _ = handle2.emit("workspace-modified", serde_json::json!({}));
            })
            .ok();
    }

    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(())
}

/// Return the list of connected monitors for the Screen selector in the inspector.
#[tauri::command]
pub fn list_video_screens(
    state: tauri::State<crate::state::AppState>,
) -> Vec<crate::engine::output_engine::ScreenInfo> {
    state.output_engine.list_screens()
}

// ---------------------------------------------------------------------------
// Image file management
// ---------------------------------------------------------------------------

/// Set the file path of an Image Cue.
///
/// Unlike [`set_audio_file`], no background decoding is needed — the image is
/// passed to the OutputEngine at GO time via mpv loadfile.
#[tauri::command]
pub fn set_image_file(
    cue_id: String,
    file_path: String,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    super::undo_cmds::push_current_snapshot(&state)?;
    let id: Uuid = cue_id.parse().map_err(|e: uuid::Error| e.to_string())?;

    let registry = state.registry.lock().map_err(|e| e.to_string())?;
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    ws.mark_modified();
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;

    let idx = cue_list.index_of(&id).ok_or("Cue not found")?;
    if cue_list.cues[idx].cue_type() != CueType::Image {
        return Err("set_image_file only applies to Image Cues".to_string());
    }

    let mut json = cue_list.cues[idx].serialize();
    if let Some(obj) = json.as_object_mut() {
        obj.insert("file_path".to_string(), serde_json::json!(file_path));
    }
    let new_cue = registry.from_json(json).map_err(|e| e.to_string())?;
    drop(registry);
    cue_list.cues[idx] = new_cue;

    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(())
}

/// Toggle the output window visibility (F9 / View menu).
#[tauri::command]
pub fn toggle_output_window(state: State<'_, AppState>) {
    state.output_engine.toggle_visibility();
}

/// Return whether the output window is currently visible.
#[tauri::command]
pub fn get_output_window_visible(state: State<'_, AppState>) -> bool {
    state.output_engine.is_visible()
}

// ---------------------------------------------------------------------------
// Group Cue commands
// ---------------------------------------------------------------------------

/// Wrap the given cues in a new Group Cue inserted at the first selected position.
/// Returns the new Group's ID.
#[tauri::command]
pub fn group_cues(
    ids: Vec<String>,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<String, String> {
    super::undo_cmds::push_current_snapshot(&state)?;
    let ids: Vec<Uuid> = ids
        .iter()
        .map(|s| s.parse::<Uuid>().map_err(|e| e.to_string()))
        .collect::<Result<_, _>>()?;
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    ws.mark_modified();
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;
    let group_id = cue_list.group_cues(&ids).map_err(|e| e.to_string())?;
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(group_id.to_string())
}

/// Dissolve a Group: move its children into the parent list and remove the Group.
#[tauri::command]
pub fn ungroup(
    group_id: String,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    super::undo_cmds::push_current_snapshot(&state)?;
    let id: Uuid = group_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    ws.mark_modified();
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;
    cue_list.ungroup(&id).map_err(|e| e.to_string())?;
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(())
}

/// Change the playback mode of a Group Cue (simultaneous | sequential).
#[tauri::command]
pub fn set_group_mode(
    group_id: String,
    mode: String,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    super::undo_cmds::push_current_snapshot(&state)?;
    let id: Uuid = group_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let group_mode: GroupMode = serde_json::from_value(serde_json::json!(mode))
        .map_err(|_| format!("Unknown group mode: {mode}"))?;
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    ws.mark_modified();
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;
    let cue = cue_list.get_mut(&id).ok_or("Group cue not found")?;
    cue.set_group_mode(group_mode);
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(())
}

/// Move a top-level cue into a Group's children (position = −1 for append).
#[tauri::command]
pub fn add_cue_to_group(
    cue_id: String,
    group_id: String,
    position: i32,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    super::undo_cmds::push_current_snapshot(&state)?;
    let cue_uuid: Uuid = cue_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let grp_uuid: Uuid = group_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    ws.mark_modified();
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;
    cue_list
        .add_to_group(&cue_uuid, &grp_uuid, position)
        .map_err(|e| e.to_string())?;
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(())
}

/// Move a cue from anywhere in the hierarchy to the top-level list, immediately
/// before `before_id` (or at the end if `before_id` is `null`).
#[tauri::command]
pub fn move_to_top_level(
    cue_id: String,
    before_id: Option<String>,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    super::undo_cmds::push_current_snapshot(&state)?;
    let cue_uuid: Uuid = cue_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let before_uuid: Option<Uuid> = before_id
        .as_deref()
        .map(|s| s.parse::<Uuid>().map_err(|e| e.to_string()))
        .transpose()?;
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    ws.mark_modified();
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;
    cue_list
        .move_to_top_level_before(&cue_uuid, before_uuid.as_ref())
        .map_err(|e| e.to_string())?;
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(())
}

/// Remove a child cue from a Group and place it after the Group in the main list.
#[tauri::command]
pub fn remove_cue_from_group(
    group_id: String,
    cue_id: String,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    super::undo_cmds::push_current_snapshot(&state)?;
    let grp_uuid: Uuid = group_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let cue_uuid: Uuid = cue_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    ws.mark_modified();
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;
    cue_list
        .remove_from_group(&grp_uuid, &cue_uuid)
        .map_err(|e| e.to_string())?;
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(())
}
