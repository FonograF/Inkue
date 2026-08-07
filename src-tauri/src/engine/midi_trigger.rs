//! Per-cue MIDI triggers — fire a cue when a MIDI message arrives.
//!
//! Two halves, like [`super::midi_file`]: a pure matcher and a listener thread.
//!
//! - [`MidiTrigger::matches`] decides whether an incoming message fires a cue.
//!   It is pure and byte-driven, which is what makes the awkward parts of MIDI
//!   testable: a Note On with velocity 0 is really a Note Off, Program Change
//!   carries no second data byte, and "any channel" / "any value" have to mean
//!   what an operator expects.
//! - [`MidiTriggerListener`] owns a `midir` input connection on a background
//!   thread and queues what arrives. It knows nothing about cues — the show
//!   layer drains the queue and decides what to fire — so this module stays on
//!   the engine side of the layering rule.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use serde::{Deserialize, Serialize};

/// The kinds of MIDI message a cue can be triggered by.
///
/// These four cover how show control actually uses MIDI: a note from a
/// keyboard or drum pad, a controller from a fader or footswitch, a program
/// change from a rack unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MidiTriggerType {
    #[default]
    NoteOn,
    NoteOff,
    ControlChange,
    ProgramChange,
}

/// Channel-voice status bytes run 0x80–0xEF. Anything at or above 0xF0 is a
/// system message (clock, SysEx, MTC…) and carries no channel, so it can never
/// match a per-cue trigger.
fn is_channel_voice(status: u8) -> bool {
    (0x80..0xF0).contains(&status)
}

impl MidiTriggerType {
    /// `true` when the message carries no second data byte.
    fn is_two_byte(self) -> bool {
        matches!(self, MidiTriggerType::ProgramChange)
    }
}

/// A MIDI message that fires the cue it is attached to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MidiTrigger {
    pub message_type: MidiTriggerType,
    /// MIDI channel 1–16, or **0 for any channel** (omni).
    pub channel: u8,
    /// Note number, controller number, or program number.
    pub data1: u8,
    /// Velocity / controller value to require. `None` = any value, which is
    /// what you want for a note; `Some(127)` is what you want for a footswitch
    /// that also sends 0 on release.
    pub data2: Option<u8>,
}

impl Default for MidiTrigger {
    fn default() -> Self {
        Self {
            message_type: MidiTriggerType::NoteOn,
            channel: 1,
            data1: 60,
            data2: None,
        }
    }
}

impl MidiTrigger {
    /// Does `message` fire this trigger?
    pub fn matches(&self, message: &[u8]) -> bool {
        let Some(&status) = message.first() else {
            return false;
        };
        if !is_channel_voice(status) {
            return false;
        }

        let incoming_type = match status & 0xF0 {
            0x80 => MidiTriggerType::NoteOff,
            0x90 => {
                // Velocity 0 on a Note On is the running-status way of writing
                // Note Off. Treating it as a Note On would fire a cue when the
                // player *releases* the key.
                if message.get(2).copied().unwrap_or(0) == 0 {
                    MidiTriggerType::NoteOff
                } else {
                    MidiTriggerType::NoteOn
                }
            }
            0xB0 => MidiTriggerType::ControlChange,
            0xC0 => MidiTriggerType::ProgramChange,
            _ => return false,
        };
        if incoming_type != self.message_type {
            return false;
        }

        // Channel 0 means omni.
        if self.channel != 0 && (status & 0x0F) + 1 != self.channel {
            return false;
        }

        if message.get(1).copied().unwrap_or(0) != self.data1 {
            return false;
        }

        match self.data2 {
            None => true,
            // Program Change has no second byte to compare against, so a value
            // requirement cannot be satisfied and must not silently pass.
            Some(_) if self.message_type.is_two_byte() => true,
            Some(want) => message.get(2).copied() == Some(want),
        }
    }

    /// Build a trigger from a message just received — the "MIDI learn" path.
    ///
    /// Returns `None` for messages that cannot trigger a cue. The captured
    /// value is deliberately **not** required (`data2: None`): learning a note
    /// should fire on any velocity, and the operator can tighten it afterwards.
    pub fn from_message(message: &[u8]) -> Option<Self> {
        let &status = message.first()?;
        if !is_channel_voice(status) {
            return None;
        }
        let message_type = match status & 0xF0 {
            0x80 => MidiTriggerType::NoteOff,
            0x90 => {
                if message.get(2).copied().unwrap_or(0) == 0 {
                    MidiTriggerType::NoteOff
                } else {
                    MidiTriggerType::NoteOn
                }
            }
            0xB0 => MidiTriggerType::ControlChange,
            0xC0 => MidiTriggerType::ProgramChange,
            _ => return None,
        };
        Some(Self {
            message_type,
            channel: (status & 0x0F) + 1,
            data1: *message.get(1)?,
            data2: None,
        })
    }

    /// Short human-readable form, e.g. `"Note On ch1 60"`.
    pub fn describe(&self) -> String {
        let kind = match self.message_type {
            MidiTriggerType::NoteOn => "Note On",
            MidiTriggerType::NoteOff => "Note Off",
            MidiTriggerType::ControlChange => "CC",
            MidiTriggerType::ProgramChange => "Program",
        };
        let channel = if self.channel == 0 {
            "any ch".to_string()
        } else {
            format!("ch{}", self.channel)
        };
        match self.data2 {
            Some(v) if !self.message_type.is_two_byte() => {
                format!("{kind} {channel} {} = {v}", self.data1)
            }
            _ => format!("{kind} {channel} {}", self.data1),
        }
    }
}

// ---------------------------------------------------------------------------
// Listener
// ---------------------------------------------------------------------------

/// Names of the MIDI **input** ports on this machine.
pub fn list_midi_input_ports() -> Vec<String> {
    match midir::MidiInput::new("Inkue-trigger-list") {
        Ok(input) => input
            .ports()
            .iter()
            .filter_map(|p| input.port_name(p).ok())
            .collect(),
        Err(e) => {
            log::warn!("[midi-trigger] port enumeration failed: {e}");
            Vec::new()
        }
    }
}

/// Listens on one MIDI input port and queues the channel-voice messages that
/// arrive, for the show layer to match against its cues.
pub struct MidiTriggerListener {
    receiver: Receiver<Vec<u8>>,
    /// Most recent message, for the inspector's Learn button.
    last_message: Arc<Mutex<Option<Vec<u8>>>>,
    shutdown: Arc<AtomicBool>,
}

impl MidiTriggerListener {
    /// Start listening on `port_name` (or the first available port).
    pub fn new(port_name: Option<String>) -> Self {
        let (sender, receiver) = crossbeam_channel::unbounded();
        let last_message = Arc::new(Mutex::new(None));
        let shutdown = Arc::new(AtomicBool::new(false));

        let thread_last = Arc::clone(&last_message);
        let thread_shutdown = Arc::clone(&shutdown);
        let _ = std::thread::Builder::new()
            .name("inkue-midi-trigger".into())
            .spawn(move || listen(port_name, sender, thread_last, thread_shutdown));

        Self { receiver, last_message, shutdown }
    }

    /// Take everything received since the last call. Never blocks.
    pub fn drain(&self) -> Vec<Vec<u8>> {
        self.receiver.try_iter().collect()
    }

    /// The most recent message received, for MIDI learn.
    pub fn last_message(&self) -> Option<Vec<u8>> {
        self.last_message.lock().ok().and_then(|m| m.clone())
    }

    /// Forget the last message, so Learn waits for a genuinely new one.
    pub fn clear_last_message(&self) {
        if let Ok(mut slot) = self.last_message.lock() {
            *slot = None;
        }
    }
}

impl Drop for MidiTriggerListener {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

fn listen(
    port_name: Option<String>,
    sender: Sender<Vec<u8>>,
    last_message: Arc<Mutex<Option<Vec<u8>>>>,
    shutdown: Arc<AtomicBool>,
) {
    let Ok(mut midi_in) = midir::MidiInput::new("Inkue-trigger") else {
        log::error!("[midi-trigger] failed to create MIDI input");
        return;
    };
    // Clock and active sensing would otherwise flood the queue at 24 ppqn.
    midi_in.ignore(midir::Ignore::All);

    let ports = midi_in.ports();
    let port = match &port_name {
        Some(name) => ports
            .iter()
            .find(|p| midi_in.port_name(p).ok().as_deref() == Some(name.as_str())),
        None => ports.first(),
    };
    let Some(port) = port else {
        log::warn!("[midi-trigger] input port {port_name:?} not found");
        return;
    };
    let display = midi_in.port_name(port).unwrap_or_default();

    let callback_shutdown = Arc::clone(&shutdown);
    let conn = midi_in.connect(
        port,
        "inkue-trigger",
        move |_stamp, message, _| {
            if callback_shutdown.load(Ordering::Relaxed) {
                return;
            }
            // Channel-voice messages only; everything else is noise here.
            let Some(&status) = message.first() else { return };
            if !is_channel_voice(status) {
                return;
            }
            let owned = message.to_vec();
            if let Ok(mut slot) = last_message.lock() {
                *slot = Some(owned.clone());
            }
            let _ = sender.send(owned);
        },
        (),
    );

    if conn.is_err() {
        log::error!("[midi-trigger] could not open '{display}'");
        return;
    }
    log::info!("[midi-trigger] listening on '{display}'");

    // The connection lives as long as this scope; hold it until shutdown.
    while !shutdown.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(50));
    }
    log::info!("[midi-trigger] stopped listening on '{display}'");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn note_on_trigger() -> MidiTrigger {
        MidiTrigger {
            message_type: MidiTriggerType::NoteOn,
            channel: 1,
            data1: 60,
            data2: None,
        }
    }

    #[test]
    fn a_matching_note_fires() {
        assert!(note_on_trigger().matches(&[0x90, 60, 100]));
    }

    #[test]
    fn a_different_note_does_not_fire() {
        assert!(!note_on_trigger().matches(&[0x90, 61, 100]));
    }

    #[test]
    fn a_different_channel_does_not_fire() {
        assert!(!note_on_trigger().matches(&[0x91, 60, 100]));
    }

    #[test]
    fn channel_zero_means_any_channel() {
        let mut trigger = note_on_trigger();
        trigger.channel = 0;
        assert!(trigger.matches(&[0x90, 60, 100]), "channel 1");
        assert!(trigger.matches(&[0x9F, 60, 100]), "channel 16");
    }

    #[test]
    fn note_on_with_velocity_zero_is_a_release_not_a_press() {
        // The classic MIDI trap: many keyboards never send 0x8n at all, so a
        // Note On trigger that ignored this would fire when the key is let go.
        assert!(!note_on_trigger().matches(&[0x90, 60, 0]));

        let release = MidiTrigger {
            message_type: MidiTriggerType::NoteOff,
            channel: 1,
            data1: 60,
            data2: None,
        };
        assert!(release.matches(&[0x90, 60, 0]), "velocity 0 fires a Note Off trigger");
        assert!(release.matches(&[0x80, 60, 64]), "so does a real Note Off");
    }

    #[test]
    fn a_required_value_must_match() {
        // A footswitch on CC64 sends 127 pressed and 0 released; only the
        // press should fire the cue.
        let trigger = MidiTrigger {
            message_type: MidiTriggerType::ControlChange,
            channel: 1,
            data1: 64,
            data2: Some(127),
        };
        assert!(trigger.matches(&[0xB0, 64, 127]));
        assert!(!trigger.matches(&[0xB0, 64, 0]));
    }

    #[test]
    fn no_required_value_accepts_any() {
        let trigger = MidiTrigger {
            message_type: MidiTriggerType::ControlChange,
            channel: 1,
            data1: 64,
            data2: None,
        };
        assert!(trigger.matches(&[0xB0, 64, 0]));
        assert!(trigger.matches(&[0xB0, 64, 127]));
    }

    #[test]
    fn program_change_matches_on_its_single_data_byte() {
        let trigger = MidiTrigger {
            message_type: MidiTriggerType::ProgramChange,
            channel: 3,
            data1: 42,
            data2: Some(99), // meaningless here — must not block the match
        };
        assert!(trigger.matches(&[0xC2, 42]));
        assert!(!trigger.matches(&[0xC2, 43]));
    }

    #[test]
    fn the_wrong_message_type_never_fires() {
        assert!(!note_on_trigger().matches(&[0xB0, 60, 100]), "CC is not a note");
        assert!(!note_on_trigger().matches(&[0xC0, 60]), "program change is not a note");
    }

    #[test]
    fn system_and_malformed_messages_are_ignored() {
        let trigger = note_on_trigger();
        assert!(!trigger.matches(&[]), "empty");
        assert!(!trigger.matches(&[0xF8]), "MIDI clock");
        assert!(!trigger.matches(&[0xF0, 0x7E, 0xF7]), "SysEx");
        assert!(!trigger.matches(&[60, 100]), "no status byte");
    }

    #[test]
    fn learn_captures_the_message_that_was_played() {
        let learned = MidiTrigger::from_message(&[0x93, 64, 120]).unwrap();
        assert_eq!(learned.message_type, MidiTriggerType::NoteOn);
        assert_eq!(learned.channel, 4);
        assert_eq!(learned.data1, 64);
        assert_eq!(learned.data2, None, "velocity is not required by default");
        // …and what was learned matches what was played.
        assert!(learned.matches(&[0x93, 64, 120]));
        assert!(learned.matches(&[0x93, 64, 30]), "any velocity, as learned");
    }

    #[test]
    fn learn_ignores_what_cannot_trigger() {
        assert!(MidiTrigger::from_message(&[0xF8]).is_none());
        assert!(MidiTrigger::from_message(&[]).is_none());
    }

    #[test]
    fn learning_a_release_gives_a_note_off_trigger() {
        let learned = MidiTrigger::from_message(&[0x90, 60, 0]).unwrap();
        assert_eq!(learned.message_type, MidiTriggerType::NoteOff);
    }

    #[test]
    fn describe_reads_like_the_inspector_shows_it() {
        assert_eq!(note_on_trigger().describe(), "Note On ch1 60");
        let mut omni = note_on_trigger();
        omni.channel = 0;
        assert_eq!(omni.describe(), "Note On any ch 60");
        let cc = MidiTrigger {
            message_type: MidiTriggerType::ControlChange,
            channel: 2,
            data1: 7,
            data2: Some(127),
        };
        assert_eq!(cc.describe(), "CC ch2 7 = 127");
    }

    #[test]
    fn serde_roundtrip_keeps_every_field() {
        let trigger = MidiTrigger {
            message_type: MidiTriggerType::ControlChange,
            channel: 0,
            data1: 7,
            data2: Some(64),
        };
        let json = serde_json::to_string(&trigger).unwrap();
        assert!(json.contains("control_change"), "snake_case on the wire: {json}");
        let back: MidiTrigger = serde_json::from_str(&json).unwrap();
        assert_eq!(back, trigger);
    }
}
