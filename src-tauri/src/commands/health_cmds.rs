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
    // Voices are preserved across the restart, so a running cue keeps playing on
    // the restored device — nothing to reset.
    state.audio_engine.restore_desired().map_err(|e| e.to_string())?;
    health::clear("audio-device");

    let _ = app_handle.emit("health-changed", ());
    Ok(())
}
