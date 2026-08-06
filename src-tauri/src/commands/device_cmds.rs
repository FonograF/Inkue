//! Tauri commands for audio device and Output Patch management.
//!
//! Output Patches live in the **workspace** (`ws.output_patches`) — the same
//! table `CueContext::resolve_patch` reads at GO time — so edits here are
//! persisted in the `.inkue` file and affect playback immediately.

use tauri::{Emitter, Manager, State};
use uuid::Uuid;

use crate::{
    engine::{audio_input, device_manager::{DeviceInfo, OutputPatch}},
    state::AppState,
};

/// Return the audio output devices Output Patches may target.
///
/// DAW convention (Ableton-style): the selected backend defines the output
/// universe.  On the ASIO backend only ASIO drivers are listed — their WASAPI
/// endpoints are held exclusively by ASIO anyway, and the patch channels route
/// inside the full-width main ASIO stream.  On shared backends the normal
/// WASAPI/CoreAudio/ALSA devices are listed (no ASIO entries).
/// `backend` overrides the persisted one so the Preferences panel can show the
/// device universe of the backend **currently selected in the UI**, before it
/// has been applied — switching the Backend dropdown to ASIO used to keep
/// listing WASAPI endpoints until Apply was clicked, which made it look as
/// though ASIO patches were impossible.
#[tauri::command]
pub fn list_output_devices(
    backend: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<DeviceInfo>, String> {
    #[cfg(target_os = "linux")]
    use crate::engine::device_manager::linux_devices;

    // Build ALSA fallback from the engine's cached device list.
    let fallback = {
        let engine = &state.audio_engine;
        let mgr = engine.device_manager.lock().map_err(|e| e.to_string())?;
        mgr.devices().to_vec()
    };

    #[cfg(target_os = "linux")]
    return Ok(linux_devices(false, fallback));
    #[cfg(not(target_os = "linux"))]
    {
        use crate::preferences::AudioBackend;
        let config = crate::machine_config::load();
        // What the operator is looking at wins over what is running.
        let selected = match backend.as_deref() {
            Some("asio") => AudioBackend::Asio,
            Some("wasapi_exclusive") => AudioBackend::WasapiExclusive,
            Some("wasapi_shared") => AudioBackend::WasapiShared,
            Some("system_default") => AudioBackend::SystemDefault,
            _ => config.backend.clone(),
        };

        // ASIO selected but not applied: the engine does not hold the driver
        // yet, so the live stream cannot describe it. Read the driver list
        // from the registry — fast, no cpal, safe to call either way.
        #[cfg(all(windows, feature = "asio-support"))]
        if matches!(selected, AudioBackend::Asio) && !matches!(config.backend, AudioBackend::Asio) {
            return Ok(crate::commands::preferences_cmds::list_asio_drivers_from_registry());
        }

        if cfg!(all(windows, feature = "asio-support"))
            && matches!(selected, AudioBackend::Asio)
        {
            // ASIO drivers are single-client and already held by the engine's
            // main stream — re-enumerating the ASIO host here fails or hangs.
            // Build the entry from the live stream state instead: same id as
            // `current_device_id` (→ patches route inside the full-width main
            // mix) with the open stream's real channel count.
            if let Some(id) = config.device_id.filter(|d| !d.is_empty()) {
                return Ok(vec![DeviceInfo {
                    name: format!("{id} (ASIO)"),
                    id,
                    channels: state.audio_engine.output_channels() as u16,
                    sample_rate: state.audio_engine.sample_rate(),
                }]);
            }
        }
        Ok(fallback)
    }
}

/// Return all available audio **input** devices (for Mic Cues / live capture).
///
/// Like output enumeration, the cpal input query is slow-to-hanging on Windows,
/// so it runs off the main thread and is time-bounded — the Preferences panel
/// never freezes on it.
#[tauri::command]
pub async fn list_input_devices() -> Result<Vec<DeviceInfo>, String> {
    let devices = tauri::async_runtime::spawn_blocking(|| {
        crate::engine::device_manager::run_bounded(
            crate::engine::device_manager::ENUM_TIMEOUT,
            audio_input::list_input_devices,
        )
        .unwrap_or_default()
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(devices)
}

/// Return the workspace's Output Patch table plus the default patch id.
#[derive(serde::Serialize)]
pub struct OutputPatchTable {
    pub patches: Vec<OutputPatch>,
    pub default_patch_id: Option<String>,
}

/// Return all configured Output Patches (workspace-level, persisted).
#[tauri::command]
pub fn get_output_patches(state: State<'_, AppState>) -> Result<OutputPatchTable, String> {
    let ws = state.workspace.lock().map_err(|e| e.to_string())?;
    Ok(OutputPatchTable {
        patches: ws.output_patches.clone(),
        default_patch_id: ws.default_output_patch_id.map(|id| id.to_string()),
    })
}

/// Create or update an Output Patch in the workspace.
#[tauri::command]
pub fn set_output_patch(
    patch_id: Option<String>,
    name: String,
    device_id: String,
    channels: Vec<u16>,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<String, String> {
    let id = patch_id
        .as_deref()
        .and_then(|s| s.parse::<Uuid>().ok())
        .unwrap_or_else(Uuid::new_v4);

    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    match ws.output_patches.iter_mut().find(|p| p.id == id) {
        Some(existing) => {
            existing.name = name;
            existing.device_id = device_id;
            existing.channels = channels;
        }
        None => ws.output_patches.push(OutputPatch { id, name, device_id, channels, gain_db: 0.0 }),
    }
    // The first patch ever created becomes the default automatically.
    if ws.default_output_patch_id.is_none() {
        ws.default_output_patch_id = Some(id);
    }
    ws.mark_modified();
    drop(ws);

    // Routing changed: drop aux streams from the previous patch table so
    // nothing keeps playing on a re-configured output; the next GO reopens
    // exactly what the new table needs.
    state.audio_engine.close_all_aux();
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(id.to_string())
}

/// Remove an Output Patch from the workspace.  Cues referencing it fall back
/// to the default patch (with a preflight warning).
#[tauri::command]
pub fn remove_output_patch(
    patch_id: String,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let id: Uuid = patch_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let before = ws.output_patches.len();
    ws.output_patches.retain(|p| p.id != id);
    if ws.output_patches.len() == before {
        return Err(format!("Output patch {id} not found"));
    }
    if ws.default_output_patch_id == Some(id) {
        ws.default_output_patch_id = ws.output_patches.first().map(|p| p.id);
    }
    ws.mark_modified();
    drop(ws);

    state.audio_engine.close_all_aux();
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(())
}

/// Mixer fader: set an Output Patch's gain (dB), persisting it in the
/// workspace and hot-applying it to every playing voice routed through the
/// patch.
#[tauri::command]
pub fn set_output_patch_gain(
    patch_id: String,
    gain_db: f32,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let id: Uuid = patch_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    {
        let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
        let patch = ws
            .output_patches
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or_else(|| format!("Output patch {id} not found"))?;
        patch.gain_db = gain_db;
        ws.mark_modified();
    }
    let gain = crate::cue::types::db_to_linear(gain_db as f64) as f32;
    state.audio_engine.set_patch_gain(id, gain).map_err(|e| e.to_string())?;
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(())
}

/// Set the workspace's default Output Patch (used by cues with no explicit patch).
#[tauri::command]
pub fn set_default_output_patch(
    patch_id: Option<String>,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let id: Option<Uuid> = patch_id
        .as_deref()
        .map(|s| s.parse::<Uuid>().map_err(|e| e.to_string()))
        .transpose()?;
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    if let Some(id) = id {
        if !ws.output_patches.iter().any(|p| p.id == id) {
            return Err(format!("Output patch {id} not found"));
        }
    }
    ws.default_output_patch_id = id;
    ws.mark_modified();
    drop(ws);

    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(())
}

/// Show and focus the pre-created floating Output Mixer window.
///
/// The window is declared in `tauri.conf.json` with `visible: false` —
/// creating windows dynamically is unreliable on WebView2 (see the
/// preferences/float-timer windows, same pattern).
#[tauri::command]
pub fn open_mixer_window(app_handle: tauri::AppHandle) -> Result<(), String> {
    let w = app_handle
        .get_webview_window("mixer")
        .ok_or("mixer window not found")?;
    w.show().map_err(|e| e.to_string())?;
    w.set_focus().map_err(|e| e.to_string())?;
    Ok(())
}

/// Refresh the cached device list (call after hotplug events).
///
/// Enumeration is bounded and runs off the main thread, so a stuck device can
/// never freeze the UI on a manual refresh.
#[tauri::command]
pub async fn refresh_devices(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let engine = state.audio_engine.clone();
    let enumerated = tauri::async_runtime::spawn_blocking(move || {
        let devices = crate::engine::device_manager::run_bounded(
            crate::engine::device_manager::ENUM_TIMEOUT,
            crate::engine::device_manager::enumerate_output_devices,
        )
        .unwrap_or_default();
        if let Ok(mut mgr) = engine.device_manager.lock() {
            mgr.replace_cache(devices.clone());
        }
        devices
    })
    .await
    .map_err(|e| e.to_string())?;

    #[cfg(target_os = "linux")]
    let devices = crate::engine::device_manager::linux_devices(false, enumerated);
    #[cfg(not(target_os = "linux"))]
    let devices = enumerated;

    let _ = app_handle.emit("device-changed", serde_json::json!({ "devices": devices }));
    Ok(())
}
