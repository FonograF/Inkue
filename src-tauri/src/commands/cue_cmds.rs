//! Tauri commands for cue CRUD operations.

use std::sync::{atomic::Ordering, Arc};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};
use uuid::Uuid;

use crate::{
    cue::{
        traits::Cue,
        types::{ContinueMode, CueColor, CueState, CueType},
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
}

fn summarise(cue: &dyn Cue) -> CueSummary {
    CueSummary {
        id: cue.id().to_string(),
        cue_type: cue.cue_type(),
        name: cue.name().to_string(),
        number: cue.number().map(|s| s.to_string()),
        state: cue.state(),
        continue_mode: cue.continue_mode(),
        color: cue.color(),
        pre_wait_ms: cue.pre_wait().as_millis() as u64,
        post_wait_ms: cue.post_wait().as_millis() as u64,
        duration_ms: cue.duration().map(|d| d.as_millis() as u64),
        file_path: None, // populated below for file-based cues
        is_loading: false, // populated below
    }
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

    let summaries: Vec<CueSummary> = cue_list
        .cues
        .iter()
        .map(|c| {
            let mut s = summarise(c.as_ref());
            s.is_loading = loading.contains(&c.id());
            // Populate file_path for file-based cues (Audio, Video, Image) via the
            // serialised form — avoids unsafe downcasting.
            if matches!(c.cue_type(), CueType::Audio | CueType::Video | CueType::Image) {
                let json = c.serialize();
                s.file_path = json
                    .get("file_path")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
            }
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
    let cue = cue_list.get(&id).ok_or("Cue not found")?;
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
    cue_list.remove(&id).map_err(|e| e.to_string())?;
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
        let cue = cue_list.get(&id).ok_or("Cue not found")?;
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
    let src_idx = cue_list
        .index_of(&id)
        .ok_or("Cue not found")?;
    cue_list.insert(src_idx + 1, new_cue);

    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(new_id)
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

    let idx = cue_list.index_of(&id).ok_or("Cue not found")?;

    // Serialise the current cue, merge the incoming properties, then rebuild.
    let mut json = cue_list.cues[idx].serialize();

    // Capture decoded audio BEFORE the rebuild so we can reuse it when the
    // file path has not changed (e.g. changing Start Time, volume, name).
    let old_file_path = json
        .get("file_path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    if let (Some(target), Some(src)) = (json.as_object_mut(), properties.as_object()) {
        for (k, v) in src {
            target.insert(k.clone(), v.clone());
        }
    }

    let new_file_path = json
        .get("file_path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // If the file hasn't changed, preserve already-decoded samples to avoid
    // a re-decode (and the resulting silence at the start of the next GO).
    let preserved_audio = if old_file_path == new_file_path {
        cue_list.cues[idx].extract_decoded_audio()
    } else {
        None
    };

    // Capture runtime state (playing/paused, voice ID, timing) BEFORE the rebuild
    // so a cue that is currently running continues to be stoppable afterward.
    let runtime = cue_list.cues[idx].runtime_state();

    let mut new_cue = registry.from_json(json).map_err(|e| e.to_string())?;
    if let Some((samples, channels, sample_rate, duration)) = preserved_audio {
        new_cue.accept_preloaded_audio(samples, channels, sample_rate, duration);
    }
    new_cue.restore_runtime_state(runtime);
    cue_list.cues[idx] = new_cue;

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

    crate::show::video_pre_arm::update_video_pre_arm(
        cue_list.playhead_cue_id,
        cue_list,
        &state.video_engine,
    );

    let _ = app_handle.emit(
        "playhead-moved",
        serde_json::json!({ "cue_id": id.map(|u| u.to_string()) }),
    );
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
        let cue = cue_list.get(&id).ok_or("Cue not found")?;
        cue.extract_decoded_audio()
            .ok_or("Audio not loaded yet — assign a file first")?
        // workspace lock dropped here
    };

    // Compute peaks outside the lock.
    let channels = channels as usize;
    let total_frames = if channels > 0 { samples.len() / channels } else { 0 };
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

    let idx = cue_list.index_of(&id).ok_or("Cue not found")?;
    if cue_list.cues[idx].cue_type() != CueType::Audio {
        return Err("set_audio_file only applies to Audio Cues".to_string());
    }

    let mut json = cue_list.cues[idx].serialize();
    if let Some(obj) = json.as_object_mut() {
        obj.insert("file_path".to_string(), serde_json::json!(file_path));
    }
    let new_cue = registry.from_json(json).map_err(|e| e.to_string())?;
    drop(registry);
    cue_list.cues[idx] = new_cue;
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
                    // Brief lock: just store the decoded data.
                    if let Ok(mut ws) = workspace.lock() {
                        if let Some(cl) = ws.active_cue_list_mut() {
                            if let Some(cue) = cl.get_mut(&id) {
                                cue.accept_preloaded_audio(
                                    samples, channels, sample_rate, duration,
                                );
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
        let cue = cue_list.get(&id).ok_or("Cue not found")?;
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

    // Probe duration so the cue shows its length before the first play.
    // Done in a background thread to avoid blocking the Tauri command.
    {
        let path = std::path::PathBuf::from(&file_path);
        let cue_id = id;
        let video_engine = Arc::clone(&state.video_engine);
        let workspace2 = Arc::clone(&state.workspace);
        let handle2 = app_handle.clone();
        std::thread::Builder::new()
            .name("wincue-video-probe".into())
            .spawn(move || {
                let lib = video_engine.mpv_lib();
                if let Some(dur) = crate::engine::VideoEngine::probe_duration(lib, &path) {
                    if let Ok(mut ws) = workspace2.lock() {
                        if let Some(cl) = ws.active_cue_list_mut() {
                            if let Some(idx2) = cl.index_of(&cue_id) {
                                cl.cues[idx2].set_runtime_duration(dur);
                                let _ = handle2.emit("workspace-modified", serde_json::json!({}));
                            }
                        }
                    }
                }
            })
            .ok();
    }

    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(())
}

/// Return the list of connected monitors for the Screen selector in the inspector.
#[tauri::command]
pub fn list_video_screens() -> Vec<crate::engine::video_engine::ScreenInfo> {
    crate::engine::VideoEngine::list_screens()
}

// ---------------------------------------------------------------------------
// Image file management
// ---------------------------------------------------------------------------

/// Set the file path of an Image Cue.
///
/// Unlike [`set_audio_file`], no background decoding is needed — the image is
/// read into memory by [`ImageEngine`] at GO time.
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

/// Called by an image surface window when its fade-out CSS transition ends.
///
/// Pushes a [`FadedOut`](crate::engine::image_engine::ImageStatus::FadedOut)
/// status so the event loop can detect cue completion and fire continue modes.
#[tauri::command]
pub fn report_image_faded_out(
    voice_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    use crate::engine::image_engine::ImageStatus;
    let id: Uuid = voice_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    state
        .image_engine
        .push_status(ImageStatus::FadedOut { voice_id: id });
    Ok(())
}

/// Return the image display data (base64 data URL + fade-in duration) for an
/// image surface window.  Called by the surface's React component on mount so
/// the timing of window creation is not an issue.
#[tauri::command]
pub fn get_image_surface_data(
    voice_id: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let id: Uuid = voice_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    state
        .image_engine
        .get_surface_data(id)
        .map_err(|e| e.to_string())
}
