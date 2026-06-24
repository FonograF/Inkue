//! Tauri commands for transport control (GO, STOP, PAUSE, RESUME).

use std::sync::atomic::Ordering;

use tauri::{Emitter, State};

use crate::{
    cue::{
        context::{CueContext, CueEvent},
        types::CueType,
    },
    show::transport::Transport,
    state::AppState,
};


// ---------------------------------------------------------------------------
// Shared helper
// ---------------------------------------------------------------------------

/// Build a [`CueContext`] wired to both engines, with a snapshot of the
/// workspace's Output Patch table and current audio device so cues can
/// resolve their patch and route video audio at GO time.
///
/// `stop_fade_ms` comes from `ws.preferences.audio.default_fade_out_ms` and
/// is used by [`AudioCue::stop`] when no per-cue fade-out spec is set.
fn make_context(state: &AppState, stop_fade_ms: u32) -> CueContext {
    let (tx, _rx) = crossbeam_channel::unbounded::<CueEvent>();
    let (patches, default_patch_id, output_screen, osc_patches, fixtures, groups, input_patches, audio_buffer_size) = state
        .workspace
        .try_lock()
        .map(|ws| (
            ws.output_patches.clone(),
            ws.default_output_patch_id,
            ws.preferences.display.output_screen,
            ws.osc_patches.clone(),
            ws.fixtures.clone(),
            ws.fixture_groups.clone(),
            ws.input_patches.clone(),
            ws.preferences.audio.audio_buffer_size,
        ))
        .unwrap_or_else(|_| (Vec::new(), None, None, Vec::new(), Vec::new(), Vec::new(), Vec::new(), 256));
    CueContext::new(
        state.audio_engine.clone(),
        state.output_engine.clone(),
        tx,
        stop_fade_ms,
        patches,
        default_patch_id,
        output_screen,
        osc_patches,
        state.dmx_engine.clone(),
        fixtures,
        groups,
        input_patches,
        audio_buffer_size,
    )
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Trigger the cue at the Playhead.
///
/// Audio files must already be decoded (pre-loaded) before GO is called.
/// Decoding is triggered automatically when a workspace is loaded or when a
/// file is assigned to a cue.  GO never decodes — if an Audio Cue is still
/// loading it is silently skipped so the command always returns instantly.
/// Video Cues stream directly from disk and are never skipped.
#[tauri::command]
pub fn go(state: State<'_, AppState>, app_handle: tauri::AppHandle) -> Result<(), String> {
    // Double-GO protection: silently ignore a second GO that arrives within
    // `double_go_protection_ms` of the previous one (default 500 ms).
    // This catches duplicate UDP packets from OSC controllers and accidental
    // rapid double-presses without affecting intentional fast GOs.
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let protection_ms = ws.preferences.general.double_go_protection_ms as u64;
    if protection_ms > 0 {
        let last = state.last_go_at.load(Ordering::Relaxed);
        if now_ms.saturating_sub(last) < protection_ms {
            return Ok(());
        }
    }
    state.last_go_at.store(now_ms, Ordering::Relaxed);

    let stop_fade_ms = ws.preferences.audio.default_fade_out_ms;

    let (tx, _rx) = crossbeam_channel::unbounded::<CueEvent>();
    let context = CueContext::new(
        state.audio_engine.clone(),
        state.output_engine.clone(),
        tx,
        stop_fade_ms,
        ws.output_patches.clone(),
        ws.default_output_patch_id,
        ws.preferences.display.output_screen,
        ws.osc_patches.clone(),
        state.dmx_engine.clone(),
        ws.fixtures.clone(),
        ws.fixture_groups.clone(),
        ws.input_patches.clone(),
        ws.preferences.audio.audio_buffer_size,
    );
    let mut transport = Transport::new(context);

    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;

    // If the Audio Cue at the playhead has no decoded samples yet (still
    // loading), skip silently.  Video Cues are exempt — they stream directly
    // from disk and need no pre-loading step.
    // NOTE: use file_duration() (raw decoded length) rather than duration()
    // to avoid blocking infinite-loop cues (loop_count = u32::MAX makes
    // duration() return None even when the file is fully decoded).
    if let Some(cue) = cue_list.playhead_cue() {
        if cue.cue_type() == CueType::Audio
            && cue.file_duration().is_none()
            && cue
                .serialize()
                .get("file_path")
                .and_then(|v| v.as_str())
                .map(|s| !s.is_empty())
                .unwrap_or(false)
        {
            return Ok(());
        }
    }

    let result = transport.go(cue_list).map_err(|e| e.to_string())?;

    // Emit state changes for cues stopped by a Stop Cue action.
    for id in &result.stopped {
        let _ = app_handle.emit(
            "cue-state-changed",
            serde_json::json!({
                "cue_id": id,
                "old_state": "running",
                "new_state": "standby",
            }),
        );
    }

    // Emit state changes for chained cues (skip the primary — the frontend
    // already shows it as triggered via playhead-moved + cue-list-refresh).
    for &id in result.triggered.iter().skip(1) {
        let _ = app_handle.emit(
            "cue-state-changed",
            serde_json::json!({
                "cue_id": id,
                "old_state": "standby",
                "new_state": "running",
            }),
        );
    }

    // Always emit — even when playhead_cue_id is None (cue was last in list).
    let _ = app_handle.emit("playhead-moved", serde_json::json!({
        "cue_id": cue_list.playhead_cue_id.map(|u| u.to_string())
    }));
    // Refresh the cue list so the frontend sees updated group inner-playhead state.
    let _ = app_handle.emit("cue-list-refresh", serde_json::json!({}));
    Ok(())
}

/// Trigger a specific cue by ID (used in Cart Mode).
///
/// Parks the Playhead on the given cue and fires it via the normal GO path.
/// Auto-Continue / Auto-Follow chains still work. The same loading guard as
/// `go` applies to Audio Cues whose file is still being decoded.
#[tauri::command]
pub fn go_cue(
    cue_id: String,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let id: uuid::Uuid = cue_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let stop_fade_ms = ws.preferences.audio.default_fade_out_ms;
    let context = make_context(&state, stop_fade_ms);
    let mut transport = Transport::new(context);
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;

    if let Some(cue) = cue_list.get_recursive(&id) {
        if cue.cue_type() == CueType::Audio
            && cue.file_duration().is_none()
            && cue
                .serialize()
                .get("file_path")
                .and_then(|v| v.as_str())
                .map(|s| !s.is_empty())
                .unwrap_or(false)
        {
            return Ok(());
        }
    }

    let result = transport.go_by_id(cue_list, &id).map_err(|e| e.to_string())?;

    for stopped_id in &result.stopped {
        let _ = app_handle.emit(
            "cue-state-changed",
            serde_json::json!({
                "cue_id": stopped_id,
                "old_state": "running",
                "new_state": "standby",
            }),
        );
    }
    for &triggered_id in result.triggered.iter().skip(1) {
        let _ = app_handle.emit(
            "cue-state-changed",
            serde_json::json!({
                "cue_id": triggered_id,
                "old_state": "standby",
                "new_state": "running",
            }),
        );
    }
    let _ = app_handle.emit("playhead-moved", serde_json::json!({
        "cue_id": cue_list.playhead_cue_id.map(|u| u.to_string())
    }));
    let _ = app_handle.emit("cue-list-refresh", serde_json::json!({}));
    Ok(())
}

/// Stop all running cues with a soft fade-out.
#[tauri::command]
pub fn stop_all(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let stop_fade_ms = ws.preferences.audio.default_fade_out_ms;
    let context = make_context(&state, stop_fade_ms);
    let mut transport = Transport::new(context);
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;
    let stopping: Vec<_> = cue_list
        .cues
        .iter()
        .filter(|c| c.is_running() || c.is_paused())
        .map(|c| c.id())
        .collect();
    transport.stop_all(cue_list).map_err(|e| e.to_string())?;
    for id in stopping {
        let _ = app_handle.emit(
            "cue-state-changed",
            serde_json::json!({
                "cue_id": id,
                "old_state": "running",
                "new_state": "standby",
            }),
        );
    }
    Ok(())
}

/// Hard-stop all running cues (immediate cut, no fades).
#[tauri::command]
pub fn hard_stop_all(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let stop_fade_ms = ws.preferences.audio.default_fade_out_ms;
    let context = make_context(&state, stop_fade_ms);
    let mut transport = Transport::new(context);
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;
    let stopping: Vec<_> = cue_list
        .cues
        .iter()
        .filter(|c| c.is_running() || c.is_paused())
        .map(|c| c.id())
        .collect();
    transport.hard_stop_all(cue_list).map_err(|e| e.to_string())?;
    for id in stopping {
        let _ = app_handle.emit(
            "cue-state-changed",
            serde_json::json!({
                "cue_id": id,
                "old_state": "running",
                "new_state": "standby",
            }),
        );
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
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let stop_fade_ms = ws.preferences.audio.default_fade_out_ms;
    let context = make_context(&state, stop_fade_ms);
    let mut transport = Transport::new(context);
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;
    transport.stop_cue(cue_list, &id).map_err(|e| e.to_string())?;
    let _ = app_handle.emit(
        "cue-state-changed",
        serde_json::json!({
            "cue_id": cue_id,
            "old_state": "running",
            "new_state": "standby",
        }),
    );
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
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let stop_fade_ms = ws.preferences.audio.default_fade_out_ms;
    let context = make_context(&state, stop_fade_ms);
    let mut transport = Transport::new(context);
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;
    transport.pause_cue(cue_list, &id).map_err(|e| e.to_string())?;
    let _ = app_handle.emit(
        "cue-state-changed",
        serde_json::json!({
            "cue_id": cue_id,
            "old_state": "running",
            "new_state": "paused",
        }),
    );
    Ok(())
}

/// Set the master output gain from a dB value.
///
/// The gain is applied atomically in the audio callback without any lock.
/// Values ≤ −60 dB are treated as silence (gain = 0.0).
#[tauri::command]
pub fn set_master_volume(db: f32, state: State<'_, AppState>) -> Result<(), String> {
    let gain = if db <= -60.0 {
        0.0_f32
    } else {
        10_f32.powf(db / 20.0)
    };
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
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let stop_fade_ms = ws.preferences.audio.default_fade_out_ms;
    let context = make_context(&state, stop_fade_ms);
    let mut transport = Transport::new(context);
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;
    transport.resume_cue(cue_list, &id).map_err(|e| e.to_string())?;
    let _ = app_handle.emit(
        "cue-state-changed",
        serde_json::json!({
            "cue_id": cue_id,
            "old_state": "paused",
            "new_state": "running",
        }),
    );
    Ok(())
}

/// Seek a running or paused cue to `position_ms` from its action start.
///
/// No-op for non-seekable cue types (Memo, Stop, …).  Does not change the
/// cue's [`CueState`] — the cue keeps running or stays paused at the new
/// position.
#[tauri::command]
pub fn seek_cue(
    cue_id: String,
    position_ms: u64,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let id: uuid::Uuid = cue_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let stop_fade_ms = ws.preferences.audio.default_fade_out_ms;
    let context = make_context(&state, stop_fade_ms);
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;
    if let Some(cue) = cue_list.get_mut(&id) {
        cue.seek(position_ms, &context);
    }
    Ok(())
}
