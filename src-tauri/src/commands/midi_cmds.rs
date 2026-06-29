//! Tauri commands for MIDI device enumeration and test-send.

use tauri::State;

use crate::{
    cue::midi_cue::{send_midi_messages, MidiMessage, MidiMessageType},
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
