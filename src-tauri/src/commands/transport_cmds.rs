//! Tauri commands for transport control (GO, STOP, PAUSE, RESUME).

use tauri::{Emitter, State};

use crate::{
    cue::context::{CueContext, CueEvent},
    show::transport::Transport,
    state::AppState,
};

/// Trigger the cue at the Playhead.
///
/// Audio files must already be decoded (pre-loaded) before GO is called.
/// Decoding is triggered automatically when a workspace is loaded or when a
/// file is assigned to a cue.  GO never decodes — if the cue is still loading
/// it is silently skipped so the command always returns instantly.
#[tauri::command]
pub fn go(state: State<'_, AppState>, app_handle: tauri::AppHandle) -> Result<(), String> {
    let (tx, rx) = crossbeam_channel::unbounded::<CueEvent>();
    let context = CueContext::new(state.audio_engine.clone(), tx);
    let mut transport = Transport::new(context);
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;

    // If the cue at the playhead has no decoded samples yet (still loading),
    // skip silently — never decode inside GO.
    if let Some(cue) = cue_list.playhead_cue() {
        if cue.duration().is_none() && cue.serialize().get("file_path")
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false)
        {
            return Ok(());
        }
    }

    transport.go(cue_list).map_err(|e| e.to_string())?;

    // Handle any events emitted synchronously during go() — notably StopAll
    // from a StopCue, which must stop all running cues immediately.
    while let Ok(event) = rx.try_recv() {
        if let CueEvent::StopAll = event {
            let stop_context = make_context(&state.audio_engine);
            let mut stop_transport = Transport::new(stop_context);
            let stopping: Vec<_> = cue_list.cues
                .iter()
                .filter(|c| c.is_running() || c.is_paused())
                .map(|c| c.id())
                .collect();
            let _ = stop_transport.stop_all(cue_list);
            for id in stopping {
                let _ = app_handle.emit("cue-state-changed", serde_json::json!({
                    "cue_id": id, "old_state": "running", "new_state": "standby",
                }));
            }
        }
    }

    if let Some(phid) = cue_list.playhead_cue_id {
        let _ = app_handle.emit("playhead-moved", serde_json::json!({ "cue_id": phid }));
    }
    Ok(())
}

fn make_context(audio_engine: &std::sync::Arc<crate::engine::AudioEngine>) -> CueContext {
    let (tx, _rx) = crossbeam_channel::unbounded::<CueEvent>();
    CueContext::new(audio_engine.clone(), tx)
}

/// Stop all running cues with a soft fade-out.
#[tauri::command]
pub fn stop_all(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let context = make_context(&state.audio_engine);
    let mut transport = Transport::new(context);
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;
    let stopping: Vec<_> = cue_list.cues
        .iter()
        .filter(|c| c.is_running() || c.is_paused())
        .map(|c| c.id())
        .collect();
    transport.stop_all(cue_list).map_err(|e| e.to_string())?;
    for id in stopping {
        let _ = app_handle.emit("cue-state-changed", serde_json::json!({
            "cue_id": id, "old_state": "running", "new_state": "standby",
        }));
    }
    Ok(())
}

/// Hard-stop all running cues (immediate cut, no fades).
#[tauri::command]
pub fn hard_stop_all(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let context = make_context(&state.audio_engine);
    let mut transport = Transport::new(context);
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;
    let stopping: Vec<_> = cue_list.cues
        .iter()
        .filter(|c| c.is_running() || c.is_paused())
        .map(|c| c.id())
        .collect();
    transport.hard_stop_all(cue_list).map_err(|e| e.to_string())?;
    for id in stopping {
        let _ = app_handle.emit("cue-state-changed", serde_json::json!({
            "cue_id": id, "old_state": "running", "new_state": "standby",
        }));
    }
    Ok(())
}

/// Stop a specific cue with a soft fade-out.
#[tauri::command]
pub fn stop_cue(
    cue_id: String,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let id: uuid::Uuid = cue_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let context = make_context(&state.audio_engine);
    let mut transport = Transport::new(context);
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;
    transport.stop_cue(cue_list, &id).map_err(|e| e.to_string())?;
    let _ = app_handle.emit("cue-state-changed", serde_json::json!({
        "cue_id": cue_id, "old_state": "running", "new_state": "standby",
    }));
    Ok(())
}

/// Pause a specific cue.
#[tauri::command]
pub fn pause_cue(
    cue_id: String,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let id: uuid::Uuid = cue_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let context = make_context(&state.audio_engine);
    let mut transport = Transport::new(context);
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;
    transport.pause_cue(cue_list, &id).map_err(|e| e.to_string())?;
    let _ = app_handle.emit("cue-state-changed", serde_json::json!({
        "cue_id": cue_id, "old_state": "running", "new_state": "paused",
    }));
    Ok(())
}

/// Set the master output gain from a dB value.
///
/// The gain is applied atomically in the audio callback without any lock.
/// Values ≤ −60 dB are treated as silence (gain = 0.0).
#[tauri::command]
pub fn set_master_volume(db: f32, state: State<'_, AppState>) -> Result<(), String> {
    let gain = if db <= -60.0 { 0.0_f32 } else { 10_f32.powf(db / 20.0) };
    state.audio_engine.set_master_gain(gain);
    Ok(())
}

/// Resume a paused cue.
#[tauri::command]
pub fn resume_cue(
    cue_id: String,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let id: uuid::Uuid = cue_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let context = make_context(&state.audio_engine);
    let mut transport = Transport::new(context);
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;
    transport.resume_cue(cue_list, &id).map_err(|e| e.to_string())?;
    let _ = app_handle.emit("cue-state-changed", serde_json::json!({
        "cue_id": cue_id, "old_state": "paused", "new_state": "running",
    }));
    Ok(())
}
