//! Tauri commands backing the runtime-health banner.

use tauri::{Emitter, State};

use crate::{
    health::{self, HealthAlert},
    state::AppState,
};

/// All currently-active health alerts (device/network faults).
#[tauri::command]
pub fn get_health_alerts() -> Vec<HealthAlert> {
    health::snapshot()
}

/// Re-open the operator's chosen audio device after it returned (banner action).
#[tauri::command]
pub fn restore_audio_device(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    state.audio_engine.restore_desired().map_err(|e| e.to_string())?;
    health::clear("audio-device");

    // The stream restart kills all voices — reset any running/paused cues so the
    // UI does not keep showing them as playing on a now-dead voice.
    {
        let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
        if let Some(cl) = ws.active_cue_list_mut() {
            for cue in cl.cues.iter_mut() {
                if cue.is_running() || cue.is_paused() {
                    let _ = cue.reset();
                }
            }
        }
    }

    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    let _ = app_handle.emit("health-changed", ());
    Ok(())
}
