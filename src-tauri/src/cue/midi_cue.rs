//! [`MidiCue`] — sends one or more MIDI messages when triggered.
//!
//! All messages are dispatched synchronously at GO.  The cue completes
//! instantly.  Supported message types: Note On, Note Off, Control Change,
//! Program Change.

use std::time::{Duration, Instant};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use super::{
    context::{CueContext, CueEvent},
    traits::{Cue, CueFactory},
    types::{ContinueMode, CueColor, CueId, CueState, CueType},
};

// ---------------------------------------------------------------------------
// MIDI message types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MidiMessageType {
    #[default]
    NoteOn,
    NoteOff,
    ControlChange,
    ProgramChange,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MidiMessage {
    /// Name of the MIDI output port to send on.
    pub port_name: String,
    pub message_type: MidiMessageType,
    /// MIDI channel, 1–16.
    pub channel: u8,
    /// Note number / CC number / program number (0–127).
    pub data1: u8,
    /// Velocity / CC value (0–127).  Unused for Program Change.
    pub data2: u8,
}

impl MidiMessage {
    pub fn to_bytes(&self) -> Vec<u8> {
        let ch = self.channel.saturating_sub(1) & 0x0F;
        let d1 = self.data1 & 0x7F;
        let d2 = self.data2 & 0x7F;
        match self.message_type {
            MidiMessageType::NoteOff       => vec![0x80 | ch, d1, d2],
            MidiMessageType::NoteOn        => vec![0x90 | ch, d1, d2],
            MidiMessageType::ControlChange => vec![0xB0 | ch, d1, d2],
            MidiMessageType::ProgramChange => vec![0xC0 | ch, d1],
        }
    }
}

// ---------------------------------------------------------------------------
// MIDI send helper (pub so midi_cmds can reuse it)
// ---------------------------------------------------------------------------

/// Open each required MIDI port, send the message, and close.
/// Port open/close is negligible latency for fire-and-forget MIDI events.
pub fn send_midi_messages(messages: &[MidiMessage]) {
    if messages.is_empty() {
        return;
    }
    let mut midi_out = match midir::MidiOutput::new("WinCue") {
        Ok(m) => m,
        Err(e) => {
            log::warn!("MIDI: failed to create output: {e}");
            return;
        }
    };
    for msg in messages {
        let ports = midi_out.ports();
        let port = ports
            .into_iter()
            .find(|p| midi_out.port_name(p).ok().as_deref() == Some(msg.port_name.as_str()));

        let alert_key = format!("midi:{}", msg.port_name);

        let Some(port) = port else {
            log::warn!("MIDI: port '{}' not found", msg.port_name);
            crate::health::set(crate::health::HealthAlert::new(
                &alert_key,
                crate::health::HealthLevel::Error,
                format!("Port MIDI « {} » introuvable", msg.port_name),
            ));
            continue;
        };

        match midi_out.connect(&port, "wincue") {
            Ok(mut conn) => {
                if let Err(e) = conn.send(&msg.to_bytes()) {
                    log::warn!("MIDI send failed on '{}': {e}", msg.port_name);
                    crate::health::set(crate::health::HealthAlert::new(
                        &alert_key,
                        crate::health::HealthLevel::Error,
                        format!("Envoi MIDI échoué sur « {} »", msg.port_name),
                    ));
                } else {
                    // The port is reachable again — drop any stale alert for it.
                    crate::health::clear(&alert_key);
                }
                midi_out = conn.close();
            }
            Err(e) => {
                log::warn!("MIDI: failed to connect to '{}': {:?}", msg.port_name, e.kind());
                crate::health::set(crate::health::HealthAlert::new(
                    &alert_key,
                    crate::health::HealthLevel::Error,
                    format!("Connexion MIDI impossible : « {} »", msg.port_name),
                ));
                midi_out = e.into_inner();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// MidiCue
// ---------------------------------------------------------------------------

pub struct MidiCue {
    id: CueId,
    name: String,
    number: Option<String>,
    notes: String,
    color: CueColor,
    state: CueState,
    continue_mode: ContinueMode,
    pre_wait: Duration,
    post_wait: Duration,
    started_at: Option<Instant>,
    is_disabled: bool,
    /// The MIDI messages to send on GO.
    pub messages: Vec<MidiMessage>,
}

impl MidiCue {
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::from("MIDI"),
            number: None,
            notes: String::new(),
            color: CueColor::White,
            state: CueState::Standby,
            continue_mode: ContinueMode::DoNotContinue,
            pre_wait: Duration::ZERO,
            post_wait: Duration::ZERO,
            started_at: None,
            is_disabled: false,
            messages: Vec::new(),
        }
    }
}

impl Default for MidiCue {
    fn default() -> Self {
        Self::new()
    }
}

impl Cue for MidiCue {
    fn id(&self) -> CueId { self.id }
    fn cue_type(&self) -> CueType { CueType::Midi }
    fn name(&self) -> &str { &self.name }
    fn set_name(&mut self, name: String) { self.name = name; }
    fn number(&self) -> Option<&str> { self.number.as_deref() }
    fn set_number(&mut self, number: Option<String>) { self.number = number; }
    fn notes(&self) -> &str { &self.notes }
    fn set_notes(&mut self, notes: String) { self.notes = notes; }
    fn color(&self) -> CueColor { self.color }
    fn set_color(&mut self, color: CueColor) { self.color = color; }
    fn is_disabled(&self) -> bool { self.is_disabled }
    fn set_disabled(&mut self, d: bool) { self.is_disabled = d; }
    fn state(&self) -> CueState { self.state }

    fn load(&mut self, _context: &CueContext) -> Result<()> { Ok(()) }

    fn go(&mut self, context: &CueContext) -> Result<()> {
        self.state = CueState::Running;
        self.started_at = Some(Instant::now());
        context.emit(CueEvent::ActionStarted { cue_id: self.id });

        send_midi_messages(&self.messages);

        self.state = CueState::Completed;
        context.emit(CueEvent::ActionCompleted { cue_id: self.id });
        Ok(())
    }

    fn stop(&mut self, context: &CueContext) -> Result<()> {
        self.state = CueState::Standby;
        self.started_at = None;
        context.emit(CueEvent::Stopped { cue_id: self.id });
        Ok(())
    }

    fn pause(&mut self, _context: &CueContext) -> Result<()> { Ok(()) }
    fn resume(&mut self, _context: &CueContext) -> Result<()> { Ok(()) }

    fn hard_stop(&mut self, context: &CueContext) -> Result<()> {
        self.stop(context)
    }

    fn reset(&mut self) -> Result<()> {
        self.state = CueState::Standby;
        self.started_at = None;
        Ok(())
    }

    fn pre_wait(&self) -> Duration { self.pre_wait }
    fn set_pre_wait(&mut self, d: Duration) { self.pre_wait = d; }
    fn post_wait(&self) -> Duration { self.post_wait }
    fn set_post_wait(&mut self, d: Duration) { self.post_wait = d; }

    fn duration(&self) -> Option<Duration> { None }

    fn elapsed(&self) -> Duration {
        self.started_at.map(|t| t.elapsed()).unwrap_or(Duration::ZERO)
    }

    fn action_elapsed(&self) -> Duration { self.elapsed() }

    fn continue_mode(&self) -> ContinueMode { self.continue_mode }
    fn set_continue_mode(&mut self, mode: ContinueMode) { self.continue_mode = mode; }

    fn validate(
        &self,
        ctx: &crate::cue::validation::ValidationContext,
    ) -> Vec<crate::cue::validation::CueIssue> {
        use crate::cue::validation::CueIssue;
        let mut issues = Vec::new();
        if self.messages.is_empty() {
            issues.push(CueIssue::warning("Aucun message MIDI"));
        }
        for msg in &self.messages {
            if msg.port_name.is_empty() {
                issues.push(CueIssue::warning("Port MIDI non configuré"));
            } else if !ctx.midi_ports.iter().any(|p| p == &msg.port_name) {
                issues.push(CueIssue::error(format!(
                    "Port MIDI absent : « {} »",
                    msg.port_name
                )));
            }
        }
        issues
    }

    fn serialize(&self) -> Value {
        json!({
            "type": "midi",
            "cue_type": "midi",
            "id": self.id,
            "number": self.number,
            "name": self.name,
            "notes": self.notes,
            "color": self.color,
            "pre_wait_ms": self.pre_wait.as_millis() as u64,
            "post_wait_ms": self.post_wait.as_millis() as u64,
            "continue_mode": self.continue_mode,
            "is_disabled": self.is_disabled,
            "messages": self.messages,
        })
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

pub struct MidiCueFactory;

impl CueFactory for MidiCueFactory {
    fn create(&self) -> Box<dyn Cue> {
        Box::new(MidiCue::new())
    }

    fn from_json(&self, value: Value) -> anyhow::Result<Box<dyn Cue>> {
        let mut cue = MidiCue::new();

        if let Some(s) = value.get("id").and_then(|v| v.as_str()) {
            cue.id = s.parse().unwrap_or_else(|_| Uuid::new_v4());
        }
        if let Some(s) = value.get("name").and_then(|v| v.as_str()) {
            cue.name = s.to_string();
        }
        if let Some(s) = value.get("number").and_then(|v| v.as_str()) {
            cue.number = Some(s.to_string());
        }
        if let Some(s) = value.get("notes").and_then(|v| v.as_str()) {
            cue.notes = s.to_string();
        }
        if let Some(ms) = value.get("pre_wait_ms").and_then(|v| v.as_u64()) {
            cue.pre_wait = Duration::from_millis(ms);
        }
        if let Some(ms) = value.get("post_wait_ms").and_then(|v| v.as_u64()) {
            cue.post_wait = Duration::from_millis(ms);
        }
        if let Some(cm) = value.get("continue_mode") {
            if let Ok(mode) = serde_json::from_value(cm.clone()) {
                cue.continue_mode = mode;
            }
        }
        if let Some(col) = value.get("color") {
            if let Ok(color) = serde_json::from_value(col.clone()) {
                cue.color = color;
            }
        }
        if let Some(b) = value.get("is_disabled").and_then(|v| v.as_bool()) {
            cue.is_disabled = b;
        }
        if let Some(Value::Array(arr)) = value.get("messages") {
            for item in arr {
                match serde_json::from_value::<MidiMessage>(item.clone()) {
                    Ok(msg) => cue.messages.push(msg),
                    Err(e) => log::warn!("MidiCue: skipping invalid message: {e}"),
                }
            }
        }

        Ok(Box::new(cue))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_on_bytes() {
        let msg = MidiMessage {
            port_name: String::new(),
            message_type: MidiMessageType::NoteOn,
            channel: 1,
            data1: 60,
            data2: 127,
        };
        assert_eq!(msg.to_bytes(), vec![0x90, 60, 127]);
    }

    #[test]
    fn note_on_channel_16() {
        let msg = MidiMessage {
            port_name: String::new(),
            message_type: MidiMessageType::NoteOn,
            channel: 16,
            data1: 60,
            data2: 64,
        };
        assert_eq!(msg.to_bytes(), vec![0x9F, 60, 64]);
    }

    #[test]
    fn program_change_two_bytes() {
        let msg = MidiMessage {
            port_name: String::new(),
            message_type: MidiMessageType::ProgramChange,
            channel: 1,
            data1: 5,
            data2: 0,
        };
        assert_eq!(msg.to_bytes(), vec![0xC0, 5]);
    }

    #[test]
    fn control_change_bytes() {
        let msg = MidiMessage {
            port_name: String::new(),
            message_type: MidiMessageType::ControlChange,
            channel: 3,
            data1: 7,
            data2: 100,
        };
        assert_eq!(msg.to_bytes(), vec![0xB2, 7, 100]);
    }

    #[test]
    fn validate_flags_absent_and_unconfigured_ports() {
        use crate::cue::validation::{Severity, ValidationContext};
        use std::collections::HashSet;
        let ctx = ValidationContext {
            all_cue_ids: HashSet::new(),
            fixture_ids: HashSet::new(),
            fixture_group_ids: HashSet::new(),
            osc_patch_ids: HashSet::new(),
            output_patch_ids: HashSet::new(),
            midi_ports: vec!["Real Port".to_string()],
        };

        let mut cue = MidiCue::new();
        // A port that is not present on this machine → an error.
        cue.messages.push(MidiMessage {
            port_name: "Ghost Port".to_string(),
            message_type: MidiMessageType::NoteOn,
            channel: 1, data1: 60, data2: 100,
        });
        let issues = cue.validate(&ctx);
        assert!(issues.iter().any(|i| i.severity == Severity::Error));

        // A present port → no issue.
        cue.messages = vec![MidiMessage {
            port_name: "Real Port".to_string(),
            message_type: MidiMessageType::NoteOn,
            channel: 1, data1: 60, data2: 100,
        }];
        assert!(cue.validate(&ctx).is_empty());
    }

    #[test]
    fn serialize_roundtrip() {
        let factory = MidiCueFactory;
        let mut cue = MidiCue::new();
        cue.set_name("My MIDI".to_string());
        cue.messages.push(MidiMessage {
            port_name: "Test Port".to_string(),
            message_type: MidiMessageType::NoteOn,
            channel: 1,
            data1: 60,
            data2: 100,
        });

        let json = cue.serialize();
        assert_eq!(json["name"], "My MIDI");
        assert_eq!(json["messages"].as_array().unwrap().len(), 1);

        let rebuilt = factory.from_json(json).unwrap();
        assert_eq!(rebuilt.name(), "My MIDI");
        assert_eq!(rebuilt.cue_type(), CueType::Midi);
    }

    #[test]
    fn cue_type_is_midi() {
        assert_eq!(MidiCue::new().cue_type(), CueType::Midi);
    }
}
