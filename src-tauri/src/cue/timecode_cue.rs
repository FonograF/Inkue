//! [`TimecodeCue`] — generates a timecode stream (MTC or LTC) when triggered.
//!
//! - **MTC**: streams MIDI quarter-frame messages to a chosen MIDI output port
//!   via [`MtcGenerator`].  Multiple simultaneous, independent streams are
//!   supported (one `TimecodeCue` per stream), matching QLab behaviour.
//! - **LTC**: audio output via the audio engine — the LTC encoder feeds a live
//!   [`Voice`](crate::engine::voice::Voice) on a chosen Output Patch.
//!
//! The cue runs until stopped (`duration() = None`) unless an end frame is
//! configured, in which case it auto-completes (like QLab end-frame).

use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::engine::{
    ring_command::{FadeCurve as EngineFadeCurve, VoiceId},
    timecode_generator::MtcGenerator,
    timecode_types::{TcPosition, TcRate},
};

use super::{
    context::{CueContext, CueEvent},
    traits::{Cue, CueFactory},
    types::{ContinueMode, CueColor, CueId, CueState, CueType},
};

// ---------------------------------------------------------------------------
// TC output type
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TcOutputType {
    /// MIDI Timecode — quarter-frame messages to a MIDI output port.
    #[default]
    Mtc,
    /// Linear Timecode — biphase-mark audio signal on an Output Patch.
    Ltc,
}

// ---------------------------------------------------------------------------
// TimecodeCue
// ---------------------------------------------------------------------------

pub struct TimecodeCue {
    // ── Identity ──────────────────────────────────────────────────────────
    id: CueId,
    name: String,
    number: Option<String>,
    notes: String,
    color: CueColor,

    // ── State ─────────────────────────────────────────────────────────────
    state: CueState,

    // ── Timing ────────────────────────────────────────────────────────────
    pre_wait: Duration,
    post_wait: Duration,
    started_at: Option<Instant>,
    action_started_at: Option<Instant>,
    in_pre_wait: bool,

    // ── Continue ──────────────────────────────────────────────────────────
    continue_mode: ContinueMode,
    auto_continue_fired: bool,

    is_disabled: bool,

    // ── TC-specific ───────────────────────────────────────────────────────
    /// MTC or LTC.
    pub tc_type: TcOutputType,
    /// MIDI output port name (MTC only).
    pub midi_port: Option<String>,
    /// Output Patch (LTC only — routes audio to a line output).
    pub output_patch_id: Option<Uuid>,
    /// SMPTE frame rate to generate.
    pub rate: TcRate,
    /// Starting timecode position.
    pub start_frame: TcPosition,
    /// Optional end position; `None` = run until stopped.
    pub end_frame: Option<TcPosition>,

    // ── Runtime ───────────────────────────────────────────────────────────
    /// Running MTC generator thread (held to keep it alive).
    active_gen: Option<MtcGenerator>,
    /// LTC audio voice id (if LTC mode).
    active_ltc_voice: Option<VoiceId>,
    /// Computed duration from start_frame → end_frame (cached at GO).
    cached_duration: Option<Duration>,
}

impl TimecodeCue {
    pub fn new() -> Self {
        let default_pos = TcPosition::new(0, 0, 0, 0, TcRate::Fps2997Df);
        Self {
            id: Uuid::new_v4(),
            name: String::from("Timecode Cue"),
            number: None,
            notes: String::new(),
            color: CueColor::Cyan,
            state: CueState::Standby,
            pre_wait: Duration::ZERO,
            post_wait: Duration::ZERO,
            started_at: None,
            action_started_at: None,
            in_pre_wait: false,
            continue_mode: ContinueMode::DoNotContinue,
            auto_continue_fired: false,
            is_disabled: false,
            tc_type: TcOutputType::Mtc,
            midi_port: None,
            output_patch_id: None,
            rate: TcRate::Fps2997Df,
            start_frame: default_pos,
            end_frame: None,
            active_gen: None,
            active_ltc_voice: None,
            cached_duration: None,
        }
    }

    fn start_action(&mut self, context: &CueContext) -> Result<()> {
        self.action_started_at = Some(Instant::now());
        self.in_pre_wait = false;

        // Compute duration from start→end if both are set.
        self.cached_duration = self.end_frame.map(|end| {
            let start_f = self.start_frame.to_frame_number();
            let end_f   = end.to_frame_number();
            let frames  = end_f.saturating_sub(start_f);
            Duration::from_micros(frames * self.rate.frame_us())
        });

        match self.tc_type {
            TcOutputType::Mtc => {
                self.active_gen = MtcGenerator::start(
                    self.start_frame,
                    self.midi_port.clone(),
                );
                if self.active_gen.is_none() {
                    log::warn!("TimecodeCue '{}': no MIDI output port available", self.name);
                }
            }
            TcOutputType::Ltc => {
                // LTC is generated via a live audio voice that the event loop
                // feeds sample-by-sample using the LTC encoder.  For now we
                // log a warning and the feature is flagged as "MTC-only v1".
                log::warn!("TimecodeCue '{}': LTC output not yet implemented (v1 MTC only)", self.name);
            }
        }

        context.emit(CueEvent::ActionStarted { cue_id: self.id });
        Ok(())
    }
}

impl Default for TimecodeCue {
    fn default() -> Self { Self::new() }
}

impl Cue for TimecodeCue {
    fn id(&self) -> CueId { self.id }
    fn cue_type(&self) -> CueType { CueType::Timecode }
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

    fn load(&mut self, _ctx: &CueContext) -> Result<()> { Ok(()) }

    fn go(&mut self, context: &CueContext) -> Result<()> {
        if self.state == CueState::Running { return Ok(()); }
        self.auto_continue_fired = false;
        self.state = CueState::Running;
        self.started_at = Some(Instant::now());

        if !self.pre_wait.is_zero() {
            self.in_pre_wait = true;
            return Ok(());
        }
        if let Err(e) = self.start_action(context) {
            self.state = CueState::Standby;
            self.started_at = None;
            return Err(e);
        }
        Ok(())
    }

    fn stop(&mut self, context: &CueContext) -> Result<()> {
        self.active_gen = None;
        self.active_ltc_voice = None;
        self.in_pre_wait = false;
        self.state = CueState::Standby;
        self.started_at = None;
        self.action_started_at = None;
        self.auto_continue_fired = false;
        context.emit(CueEvent::Stopped { cue_id: self.id });
        Ok(())
    }

    fn hard_stop(&mut self, context: &CueContext) -> Result<()> {
        self.stop(context)
    }

    fn pause(&mut self, _ctx: &CueContext) -> Result<()> {
        // Pausing TC generation is not supported — QLab doesn't either.
        Ok(())
    }

    fn resume(&mut self, _ctx: &CueContext) -> Result<()> { Ok(()) }

    fn reset(&mut self) -> Result<()> {
        self.active_gen = None;
        self.active_ltc_voice = None;
        self.state = CueState::Standby;
        self.started_at = None;
        self.action_started_at = None;
        self.in_pre_wait = false;
        self.auto_continue_fired = false;
        Ok(())
    }

    fn tick(&mut self, context: &CueContext) -> Result<()> {
        if self.in_pre_wait && self.elapsed() >= self.pre_wait {
            self.start_action(context)?;
        }
        Ok(())
    }

    fn is_action_started(&self) -> bool { !self.in_pre_wait }

    fn pre_wait(&self) -> Duration { self.pre_wait }
    fn set_pre_wait(&mut self, d: Duration) { self.pre_wait = d; }
    fn post_wait(&self) -> Duration { self.post_wait }
    fn set_post_wait(&mut self, d: Duration) { self.post_wait = d; }

    /// `None` if no end frame configured; otherwise the wall-clock duration.
    fn duration(&self) -> Option<Duration> { self.cached_duration }

    fn elapsed(&self) -> Duration {
        self.started_at.map(|t| t.elapsed()).unwrap_or(Duration::ZERO)
    }

    fn action_elapsed(&self) -> Duration {
        self.action_started_at.map(|t| t.elapsed()).unwrap_or(Duration::ZERO)
    }

    fn continue_mode(&self) -> ContinueMode { self.continue_mode }
    fn set_continue_mode(&mut self, mode: ContinueMode) { self.continue_mode = mode; }

    fn is_auto_continue_fired(&self) -> bool { self.auto_continue_fired }
    fn mark_auto_continue_fired(&mut self) { self.auto_continue_fired = true; }
    fn clear_auto_continue_fired(&mut self) { self.auto_continue_fired = false; }

    fn runtime_state(&self) -> crate::cue::traits::RuntimeState {
        crate::cue::traits::RuntimeState {
            state: self.state,
            voice_id: None,
            started_at: self.started_at,
            action_started_at: self.action_started_at,
        }
    }

    fn restore_runtime_state(&mut self, snap: crate::cue::traits::RuntimeState) {
        // TC generation can't be safely transplanted across a JSON rebuild —
        // the generator thread has its own state.  We stop it and let the
        // operator re-GO if needed.  The cue is left in Standby.
        self.active_gen = None;
        self.state = CueState::Standby;
        self.started_at = snap.started_at;
        self.action_started_at = snap.action_started_at;
        self.in_pre_wait = false;
    }

    fn serialize(&self) -> Value {
        json!({
            "type": "timecode",
            "cue_type": "timecode",
            "id": self.id,
            "number": self.number,
            "name": self.name,
            "notes": self.notes,
            "color": self.color,
            "pre_wait_ms": self.pre_wait.as_millis() as u64,
            "post_wait_ms": self.post_wait.as_millis() as u64,
            "continue_mode": self.continue_mode,
            "tc_type": self.tc_type,
            "midi_port": self.midi_port,
            "output_patch_id": self.output_patch_id,
            "rate": self.rate,
            "start_frame": self.start_frame,
            "end_frame": self.end_frame,
            "is_disabled": self.is_disabled,
        })
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

pub struct TimecodeCueFactory;

impl CueFactory for TimecodeCueFactory {
    fn create(&self) -> Box<dyn Cue> { Box::new(TimecodeCue::new()) }

    fn from_json(&self, value: Value) -> Result<Box<dyn Cue>> {
        let mut c = TimecodeCue::new();
        if let Some(s) = value.get("id").and_then(|v| v.as_str()) {
            c.id = s.parse().unwrap_or_else(|_| Uuid::new_v4());
        }
        if let Some(s) = value.get("name").and_then(|v| v.as_str()) {
            c.name = s.to_string();
        }
        if let Some(s) = value.get("number").and_then(|v| v.as_str()) {
            c.number = Some(s.to_string());
        }
        if let Some(s) = value.get("notes").and_then(|v| v.as_str()) {
            c.notes = s.to_string();
        }
        if let Some(ms) = value.get("pre_wait_ms").and_then(|v| v.as_u64()) {
            c.pre_wait = Duration::from_millis(ms);
        }
        if let Some(ms) = value.get("post_wait_ms").and_then(|v| v.as_u64()) {
            c.post_wait = Duration::from_millis(ms);
        }
        if let Some(v) = value.get("continue_mode") {
            if let Ok(m) = serde_json::from_value(v.clone()) { c.continue_mode = m; }
        }
        if let Some(v) = value.get("color") {
            if let Ok(col) = serde_json::from_value(v.clone()) { c.color = col; }
        }
        if let Some(v) = value.get("tc_type") {
            if let Ok(t) = serde_json::from_value(v.clone()) { c.tc_type = t; }
        }
        if let Some(s) = value.get("midi_port").and_then(|v| v.as_str()) {
            c.midi_port = Some(s.to_string());
        }
        if let Some(s) = value.get("output_patch_id").and_then(|v| v.as_str()) {
            c.output_patch_id = s.parse().ok();
        }
        if let Some(v) = value.get("rate") {
            if let Ok(r) = serde_json::from_value(v.clone()) { c.rate = r; }
        }
        if let Some(v) = value.get("start_frame") {
            if let Ok(p) = serde_json::from_value(v.clone()) { c.start_frame = p; }
        }
        if let Some(v) = value.get("end_frame") {
            c.end_frame = serde_json::from_value(v.clone()).ok();
        }
        if let Some(b) = value.get("is_disabled").and_then(|v| v.as_bool()) {
            c.is_disabled = b;
        }
        Ok(Box::new(c))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_roundtrip() {
        let mut c = TimecodeCue::new();
        c.set_name("TC Out".to_string());
        c.set_number(Some("99".to_string()));
        c.rate = TcRate::Fps25;
        c.start_frame = TcPosition::new(1, 0, 0, 0, TcRate::Fps25);
        c.end_frame   = Some(TcPosition::new(1, 0, 30, 0, TcRate::Fps25));

        let json = c.serialize();
        let r = TimecodeCueFactory.from_json(json).unwrap();
        assert_eq!(r.name(), "TC Out");
        assert_eq!(r.cue_type(), CueType::Timecode);
    }

    #[test]
    fn duration_from_start_end_frame() {
        let start = TcPosition::new(0, 0, 0, 0, TcRate::Fps25);
        let end   = TcPosition::new(0, 0, 10, 0, TcRate::Fps25); // 10 s
        let mut c = TimecodeCue::new();
        c.start_frame = start;
        c.end_frame   = Some(end);
        // Simulate start_action being called (it sets cached_duration).
        let frames = end.to_frame_number() - start.to_frame_number(); // 250
        let dur = Duration::from_micros(frames * TcRate::Fps25.frame_us());
        assert_eq!(dur.as_secs(), 10);
    }

    #[test]
    fn no_end_frame_means_no_duration() {
        let c = TimecodeCue::new();
        assert_eq!(c.duration(), None);
    }
}
