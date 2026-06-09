//! [`OscCue`] — sends one or more OSC messages when triggered.
//!
//! All messages are sent simultaneously at GO over UDP.  The cue completes
//! instantly (duration = `None`), emitting `ActionStarted` + `ActionCompleted`
//! synchronously inside `go()`.

use std::net::UdpSocket;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{json, Value};
use uuid::Uuid;

use super::{
    context::{CueContext, CueEvent},
    osc_types::{OscArg, OscMessage},
    traits::{Cue, CueFactory},
    types::{ContinueMode, CueColor, CueId, CueState, CueType},
};

/// A cue that sends one or more OSC messages on GO.
pub struct OscCue {
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
    /// The messages to send on GO.
    pub messages: Vec<OscMessage>,
}

impl OscCue {
    /// Create a new OSC cue with a fresh UUID and no messages.
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::from("OSC"),
            number: None,
            notes: String::new(),
            color: CueColor::Blue,
            state: CueState::Standby,
            continue_mode: ContinueMode::DoNotContinue,
            pre_wait: Duration::ZERO,
            post_wait: Duration::ZERO,
            started_at: None,
            messages: Vec::new(),
        }
    }
}

impl Default for OscCue {
    fn default() -> Self {
        Self::new()
    }
}

impl Cue for OscCue {
    fn id(&self) -> CueId { self.id }
    fn cue_type(&self) -> CueType { CueType::Osc }
    fn name(&self) -> &str { &self.name }
    fn set_name(&mut self, name: String) { self.name = name; }
    fn number(&self) -> Option<&str> { self.number.as_deref() }
    fn set_number(&mut self, number: Option<String>) { self.number = number; }
    fn notes(&self) -> &str { &self.notes }
    fn set_notes(&mut self, notes: String) { self.notes = notes; }
    fn color(&self) -> CueColor { self.color }
    fn set_color(&mut self, color: CueColor) { self.color = color; }
    fn state(&self) -> CueState { self.state }

    fn load(&mut self, _context: &CueContext) -> Result<()> { Ok(()) }

    fn go(&mut self, context: &CueContext) -> Result<()> {
        self.state = CueState::Running;
        self.started_at = Some(Instant::now());
        context.emit(CueEvent::ActionStarted { cue_id: self.id });

        for msg in &self.messages {
            if let Some(patch) = context.resolve_osc_patch(msg.patch_id) {
                let target = format!("{}:{}", patch.ip, patch.port);
                if let Err(e) = send_osc(&target, &msg.address, &msg.args) {
                    log::warn!("OSC send failed to {target}: {e}");
                }
            } else {
                log::warn!("OSC cue {}: patch {} not found", self.id, msg.patch_id);
            }
        }

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

    fn serialize(&self) -> Value {
        json!({
            "type": "osc",
            "cue_type": "osc",
            "id": self.id,
            "number": self.number,
            "name": self.name,
            "notes": self.notes,
            "color": self.color,
            "pre_wait_ms": self.pre_wait.as_millis() as u64,
            "post_wait_ms": self.post_wait.as_millis() as u64,
            "continue_mode": self.continue_mode,
            "messages": self.messages,
        })
    }
}

// ---------------------------------------------------------------------------
// UDP send helper
// ---------------------------------------------------------------------------

fn send_osc(target: &str, address: &str, args: &[OscArg]) -> Result<()> {
    let osc_args: Vec<rosc::OscType> = args.iter().map(arg_to_rosc).collect();
    let packet = rosc::OscPacket::Message(rosc::OscMessage {
        addr: address.to_string(),
        args: osc_args,
    });
    let bytes = rosc::encoder::encode(&packet)?;
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.send_to(&bytes, target)?;
    Ok(())
}

fn arg_to_rosc(arg: &OscArg) -> rosc::OscType {
    match arg {
        OscArg::Int(i)   => rosc::OscType::Int(*i),
        OscArg::Float(f) => rosc::OscType::Float(*f),
        OscArg::Str(s)   => rosc::OscType::String(s.clone()),
        OscArg::Bool(b)  => rosc::OscType::Bool(*b),
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Factory for [`OscCue`].
pub struct OscCueFactory;

impl CueFactory for OscCueFactory {
    fn create(&self) -> Box<dyn Cue> {
        Box::new(OscCue::new())
    }

    fn from_json(&self, value: Value) -> anyhow::Result<Box<dyn Cue>> {
        let mut cue = OscCue::new();

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
        if let Some(msgs) = value.get("messages") {
            if let Ok(messages) = serde_json::from_value::<Vec<OscMessage>>(msgs.clone()) {
                cue.messages = messages;
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
    fn osc_cue_serialize_roundtrip() {
        let mut cue = OscCue::new();
        cue.set_name("My OSC".to_string());
        cue.set_number(Some("5".to_string()));
        cue.messages.push(OscMessage {
            patch_id: Uuid::nil(),
            address: "/test/go".to_string(),
            args: vec![OscArg::Int(1)],
        });

        let json = cue.serialize();
        let factory = OscCueFactory;
        let back = factory.from_json(json).unwrap();

        assert_eq!(back.name(), "My OSC");
        assert_eq!(back.number(), Some("5"));
        assert_eq!(back.cue_type(), CueType::Osc);
    }
}
