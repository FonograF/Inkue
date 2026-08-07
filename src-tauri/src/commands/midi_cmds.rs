//! Tauri commands for MIDI device enumeration and test-send.

use std::sync::Arc;

use tauri::{Emitter, State};
use uuid::Uuid;

use crate::{
    cue::midi_cue::{send_midi_messages, MidiMessage, MidiMessageType},
    engine::midi_trigger::{self, MidiTrigger, MidiTriggerListener},
    machine_config::{self, MidiTriggerMachineConfig},
    state::AppState,
};

/// Return the names of all available MIDI output ports.
#[tauri::command]
pub fn list_midi_output_ports(_state: State<'_, AppState>) -> Vec<String> {
    match midir::MidiOutput::new("Inkue-list") {
        Ok(out) => out
            .ports()
            .iter()
            .filter_map(|p| out.port_name(p).ok())
            .collect(),
        Err(e) => {
            log::warn!("MIDI: failed to enumerate output ports: {e}");
            Vec::new()
        }
    }
}

/// Send a single MIDI message immediately.  Used by the inspector Test button.
#[tauri::command]
pub fn send_midi_test(
    port_name: String,
    message_type: String,
    channel: u8,
    data1: u8,
    data2: u8,
    _state: State<'_, AppState>,
) -> Result<(), String> {
    let msg_type = match message_type.as_str() {
        "note_on"         => MidiMessageType::NoteOn,
        "note_off"        => MidiMessageType::NoteOff,
        "control_change"  => MidiMessageType::ControlChange,
        "program_change"  => MidiMessageType::ProgramChange,
        other => return Err(format!("Unknown MIDI message type: {other}")),
    };
    send_midi_messages(&[MidiMessage { port_name, message_type: msg_type, channel, data1, data2 }]);
    Ok(())
}

// ---------------------------------------------------------------------------
// Per-cue MIDI triggers
// ---------------------------------------------------------------------------

/// Names of the MIDI **input** ports on this machine (trigger source picker).
#[tauri::command]
pub fn list_midi_input_ports() -> Vec<String> {
    midi_trigger::list_midi_input_ports()
}

/// The machine's MIDI-trigger settings (enabled + input port).
#[tauri::command]
pub fn get_midi_trigger_config() -> MidiTriggerMachineConfig {
    machine_config::load_midi_trigger_config()
}

/// Persist the MIDI-trigger settings and hot-apply them: start, stop or
/// re-open the listener so a port change takes effect without a restart.
#[tauri::command]
pub fn set_midi_trigger_config(
    config: MidiTriggerMachineConfig,
    state: State<'_, AppState>,
) -> Result<(), String> {
    machine_config::save_midi_trigger_config(&config).map_err(|e| e.to_string())?;
    let mut slot = state.midi_listener.lock().map_err(|e| e.to_string())?;
    // The listener owns its connection for its whole life, so a port change is
    // a fresh listener rather than a reconfiguration.
    *slot = config
        .enabled
        .then(|| Arc::new(MidiTriggerListener::new(config.port.clone())));
    Ok(())
}

/// The MIDI trigger bound to a cue, if any.
#[tauri::command]
pub fn get_cue_midi_trigger(
    cue_id: String,
    state: State<'_, AppState>,
) -> Option<MidiTrigger> {
    let id: Uuid = cue_id.parse().ok()?;
    let ws = state.workspace.lock().ok()?;
    ws.active_cue_list()?.midi_triggers.get(&id).copied()
}

/// Bind (or, with `trigger: null`, unbind) a MIDI trigger on a cue.
#[tauri::command]
pub fn set_cue_midi_trigger(
    cue_id: String,
    trigger: Option<MidiTrigger>,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let id: Uuid = cue_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    ws.mark_modified();
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;
    match trigger {
        Some(t) => { cue_list.midi_triggers.insert(id, t); }
        None => { cue_list.midi_triggers.remove(&id); }
    }
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(())
}

/// MIDI learn: the most recent message the listener saw, as a ready-made
/// trigger. Returns `None` until something arrives, so the UI can poll while
/// the operator presses the key they want.
#[tauri::command]
pub fn learn_midi_trigger(state: State<'_, AppState>) -> Option<MidiTrigger> {
    let slot = state.midi_listener.lock().ok()?;
    let listener = slot.as_ref()?;
    MidiTrigger::from_message(&listener.last_message()?)
}

/// Forget the last received message so a Learn starts from a clean slate.
#[tauri::command]
pub fn clear_midi_learn(state: State<'_, AppState>) -> Result<(), String> {
    let slot = state.midi_listener.lock().map_err(|e| e.to_string())?;
    if let Some(listener) = slot.as_ref() {
        listener.clear_last_message();
    }
    Ok(())
}
