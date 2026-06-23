//! Tauri commands for audio device and Output Patch management.

use tauri::{Emitter, State};
use uuid::Uuid;

use crate::{
    engine::{audio_input, device_manager::{DeviceInfo, OutputPatch}},
    state::AppState,
};

/// Return all available audio output devices.
#[tauri::command]
pub fn list_output_devices(state: State<'_, AppState>) -> Result<Vec<DeviceInfo>, String> {
    let engine = &state.audio_engine;
    let mgr = engine.device_manager.lock().map_err(|e| e.to_string())?;
    Ok(mgr.devices().to_vec())
}

/// Return all available audio **input** devices (for Mic Cues / live capture).
#[tauri::command]
pub fn list_input_devices() -> Result<Vec<DeviceInfo>, String> {
    Ok(audio_input::list_input_devices())
}

/// Return all configured Output Patches.
#[tauri::command]
pub fn get_output_patches(state: State<'_, AppState>) -> Result<Vec<OutputPatch>, String> {
    let mgr = state
        .audio_engine
        .device_manager
        .lock()
        .map_err(|e| e.to_string())?;
    Ok(mgr.patches().into_iter().cloned().collect())
}

/// Create or update an Output Patch.
#[tauri::command]
pub fn set_output_patch(
    patch_id: Option<String>,
    name: String,
    device_id: String,
    channels: Vec<u16>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let mut mgr = state
        .audio_engine
        .device_manager
        .lock()
        .map_err(|e| e.to_string())?;

    let id = patch_id
        .as_deref()
        .and_then(|s| s.parse::<Uuid>().ok())
        .unwrap_or_else(Uuid::new_v4);

    let patch = OutputPatch {
        id,
        name,
        device_id,
        channels,
    };
    mgr.upsert_patch(patch);
    Ok(id.to_string())
}

/// Refresh the cached device list (call after hotplug events).
#[tauri::command]
pub fn refresh_devices(state: State<'_, AppState>, app_handle: tauri::AppHandle) -> Result<(), String> {
    let mut mgr = state
        .audio_engine
        .device_manager
        .lock()
        .map_err(|e| e.to_string())?;
    mgr.refresh_devices().map_err(|e| e.to_string())?;
    let devices = mgr.devices().to_vec();
    let _ = app_handle.emit("device-changed", serde_json::json!({ "devices": devices }));
    Ok(())
}
