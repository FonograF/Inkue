//! [`MicCue`] — routes a live audio **input** through the engine (QLab Mic Cue).
//!
//! Unlike [`AudioCue`](super::audio_cue::AudioCue) it has no file to decode: at
//! GO it resolves its [`InputPatch`](crate::engine::audio_input::InputPatch) to a
//! device, ensures a persistent capture feed exists, and submits a **live**
//! [`Voice`](crate::engine::voice::Voice) that reads from that feed routed to an
//! Output Patch.  It runs until stopped (`duration()` is `None`) and reuses the
//! engine's pan / gain / fade / VU machinery exactly like an Audio Cue.

use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::engine::ring_command::{FadeCurve as EngineFadeCurve, VoiceId};

use super::{
    context::{CueContext, CueEvent},
    traits::{Cue, CueFactory},
    types::{ContinueMode, CueColor, CueId, CueState, CueType, FadeCurve, FadeSpec},
};

/// A cue that routes a live audio input to an Output Patch.
pub struct MicCue {
    // --- Identity ---
    id: CueId,
    name: String,
    number: Option<String>,
    notes: String,
    color: CueColor,

    // --- State ---
    state: CueState,

    // --- Timing ---
    pre_wait: Duration,
    post_wait: Duration,
    started_at: Option<Instant>,
    action_started_at: Option<Instant>,
    in_pre_wait: bool,

    // --- Continue ---
    continue_mode: ContinueMode,
    auto_continue_fired: bool,

    is_disabled: bool,

    // --- Mic-specific ---
    /// Input Patch to capture from (resolved at GO).
    pub input_patch_id: Option<Uuid>,
    /// Device channel indices to take (1 = mono, 2 = stereo).  Empty = use the
    /// patch's own channels.
    pub input_channels: Vec<u16>,
    /// Output Patch to route to.
    pub output_patch_id: Option<Uuid>,
    /// Volume in dB.
    pub volume_db: f64,
    /// Stereo pan (-1.0 to +1.0).
    pub pan: f32,
    /// Optional fade-in on GO.
    pub fade_in: Option<FadeSpec>,
    /// Optional fade-out (also used on soft stop).
    pub fade_out: Option<FadeSpec>,

    // --- Runtime ---
    active_voice_id: Option<VoiceId>,
}

impl MicCue {
    /// Create a new, empty Mic Cue with a fresh UUID.
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::from("Mic Cue"),
            number: None,
            notes: String::new(),
            color: CueColor::Green,
            state: CueState::Standby,
            pre_wait: Duration::ZERO,
            post_wait: Duration::ZERO,
            started_at: None,
            action_started_at: None,
            in_pre_wait: false,
            continue_mode: ContinueMode::DoNotContinue,
            auto_continue_fired: false,
            is_disabled: false,
            input_patch_id: None,
            input_channels: Vec::new(),
            output_patch_id: None,
            volume_db: 0.0,
            pan: 0.0,
            fade_in: None,
            fade_out: None,
            active_voice_id: None,
        }
    }

    fn engine_curve(c: FadeCurve) -> EngineFadeCurve {
        match c {
            FadeCurve::Linear => EngineFadeCurve::Linear,
            FadeCurve::SCurve => EngineFadeCurve::SCurve,
            FadeCurve::Exponential => EngineFadeCurve::Exponential,
        }
    }

    /// Resolve patches, open/ensure the input feed, and submit the live voice.
    fn start_action(&mut self, context: &CueContext) -> Result<()> {
        let patch = context
            .resolve_input_patch(self.input_patch_id)
            .ok_or_else(|| anyhow!("MicCue '{}': no input patch assigned", self.name))?;
        let device = patch.device_id.clone();
        // Channel selection: explicit override, else the patch's own channels.
        let chans: Vec<u16> = if self.input_channels.is_empty() {
            patch.channels.clone()
        } else {
            self.input_channels.clone()
        };

        let feed_id = context
            .audio_engine
            .ensure_input_feed(Some(&device))
            .map_err(|e| anyhow!("MicCue '{}': {e}", self.name))?;

        let in_l = chans.first().copied().unwrap_or(0) as usize;
        let in_r = chans.get(1).copied().unwrap_or(in_l as u16) as usize;

        // Output Patch routing (falls back to the workspace default / 0,1).
        let (mut out_l, mut out_r) = (0usize, 1usize);
        if let Some(op) = context.resolve_patch(self.output_patch_id) {
            if let Some(&c) = op.channels.first() {
                out_l = c as usize;
            }
            if let Some(&c) = op.channels.get(1) {
                out_r = c as usize;
            } else if let Some(&c) = op.channels.first() {
                out_r = c as usize;
            }
        }

        let gain = crate::cue::types::db_to_linear(self.volume_db) as f32;
        let (fade_ms, curve) = self
            .fade_in
            .as_ref()
            .map(|f| (f.duration_ms as u32, Self::engine_curve(f.curve)))
            .unwrap_or((0, EngineFadeCurve::Linear));

        let vid = context
            .audio_engine
            .play_mic_voice(feed_id, in_l, in_r, out_l, out_r, gain, self.pan, fade_ms, curve)?;
        self.active_voice_id = Some(vid);
        self.action_started_at = Some(Instant::now());
        self.in_pre_wait = false;
        context.emit(CueEvent::ActionStarted { cue_id: self.id });
        Ok(())
    }
}

impl Default for MicCue {
    fn default() -> Self {
        Self::new()
    }
}

impl Cue for MicCue {
    fn id(&self) -> CueId { self.id }
    fn cue_type(&self) -> CueType { CueType::Mic }
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

    /// Nothing to pre-load — a Mic Cue captures live input at GO.
    fn load(&mut self, _context: &CueContext) -> Result<()> {
        Ok(())
    }

    fn go(&mut self, context: &CueContext) -> Result<()> {
        if self.state == CueState::Running {
            return Ok(());
        }
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
        self.in_pre_wait = false;
        if let Some(vid) = self.active_voice_id.take() {
            let (fade_ms, curve) = self
                .fade_out
                .as_ref()
                .map(|f| (f.duration_ms as u32, Self::engine_curve(f.curve)))
                .unwrap_or((context.stop_fade_ms, EngineFadeCurve::SCurve));
            context.audio_engine.stop_voice(vid, fade_ms, curve)?;
        }
        self.state = CueState::Standby;
        self.started_at = None;
        self.action_started_at = None;
        self.auto_continue_fired = false;
        context.emit(CueEvent::Stopped { cue_id: self.id });
        Ok(())
    }

    fn hard_stop(&mut self, context: &CueContext) -> Result<()> {
        self.in_pre_wait = false;
        if let Some(vid) = self.active_voice_id.take() {
            context.audio_engine.stop_voice(vid, 0, EngineFadeCurve::Linear)?;
        }
        self.state = CueState::Standby;
        self.started_at = None;
        self.action_started_at = None;
        self.auto_continue_fired = false;
        context.emit(CueEvent::Stopped { cue_id: self.id });
        Ok(())
    }

    fn pause(&mut self, context: &CueContext) -> Result<()> {
        if self.in_pre_wait {
            return Ok(());
        }
        if let Some(vid) = self.active_voice_id {
            context.audio_engine.pause_voice(vid)?;
        }
        self.state = CueState::Paused;
        Ok(())
    }

    fn resume(&mut self, context: &CueContext) -> Result<()> {
        if let Some(vid) = self.active_voice_id {
            context.audio_engine.resume_voice(vid)?;
        }
        self.state = CueState::Running;
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        self.state = CueState::Standby;
        self.active_voice_id = None;
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

    fn is_action_started(&self) -> bool {
        !self.in_pre_wait
    }

    fn is_auto_continue_fired(&self) -> bool { self.auto_continue_fired }
    fn mark_auto_continue_fired(&mut self) { self.auto_continue_fired = true; }
    fn clear_auto_continue_fired(&mut self) { self.auto_continue_fired = false; }

    fn pre_wait(&self) -> Duration { self.pre_wait }
    fn set_pre_wait(&mut self, d: Duration) { self.pre_wait = d; }
    fn post_wait(&self) -> Duration { self.post_wait }
    fn set_post_wait(&mut self, d: Duration) { self.post_wait = d; }

    /// A Mic Cue runs until stopped — no fixed duration.
    fn duration(&self) -> Option<Duration> {
        None
    }

    fn elapsed(&self) -> Duration {
        self.started_at.map(|t| t.elapsed()).unwrap_or(Duration::ZERO)
    }

    fn action_elapsed(&self) -> Duration {
        self.action_started_at.map(|t| t.elapsed()).unwrap_or(Duration::ZERO)
    }

    fn continue_mode(&self) -> ContinueMode { self.continue_mode }
    fn set_continue_mode(&mut self, mode: ContinueMode) { self.continue_mode = mode; }

    fn playing_voice_id(&self) -> Option<CueId> {
        self.active_voice_id
    }

    fn runtime_state(&self) -> crate::cue::traits::RuntimeState {
        crate::cue::traits::RuntimeState {
            state: self.state,
            voice_id: self.active_voice_id,
            started_at: self.started_at,
            action_started_at: self.action_started_at,
        }
    }

    fn restore_runtime_state(&mut self, snap: crate::cue::traits::RuntimeState) {
        self.state = snap.state;
        self.active_voice_id = snap.voice_id;
        self.started_at = snap.started_at;
        self.action_started_at = snap.action_started_at;
        self.in_pre_wait = snap.state == CueState::Running && snap.action_started_at.is_none();
    }

    fn live_audio_params(&self) -> Option<crate::cue::traits::LiveAudioParams> {
        let voice_id = self.active_voice_id?;
        Some(crate::cue::traits::LiveAudioParams {
            voice_id,
            gain: crate::cue::types::db_to_linear(self.volume_db) as f32,
            pan: self.pan,
        })
    }

    fn serialize(&self) -> Value {
        json!({
            "type": "mic",
            "cue_type": "mic",
            "id": self.id,
            "number": self.number,
            "name": self.name,
            "notes": self.notes,
            "color": self.color,
            "pre_wait_ms": self.pre_wait.as_millis() as u64,
            "post_wait_ms": self.post_wait.as_millis() as u64,
            "continue_mode": self.continue_mode,
            "input_patch_id": self.input_patch_id,
            "input_channels": self.input_channels,
            "output_patch_id": self.output_patch_id,
            "volume_db": self.volume_db,
            "pan": self.pan,
            "fade_in_ms": self.fade_in.as_ref().map(|f| f.duration_ms),
            "fade_in_curve": self.fade_in.as_ref().map(|f| f.curve),
            "fade_out_ms": self.fade_out.as_ref().map(|f| f.duration_ms),
            "fade_out_curve": self.fade_out.as_ref().map(|f| f.curve),
            "is_disabled": self.is_disabled,
        })
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Factory for [`MicCue`].
pub struct MicCueFactory;

impl CueFactory for MicCueFactory {
    fn create(&self) -> Box<dyn Cue> {
        Box::new(MicCue::new())
    }

    fn from_json(&self, value: Value) -> Result<Box<dyn Cue>> {
        let mut cue = MicCue::new();

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
            if let Ok(m) = serde_json::from_value(cm.clone()) {
                cue.continue_mode = m;
            }
        }
        if let Some(col) = value.get("color") {
            if let Ok(c) = serde_json::from_value(col.clone()) {
                cue.color = c;
            }
        }
        if let Some(s) = value.get("input_patch_id").and_then(|v| v.as_str()) {
            cue.input_patch_id = s.parse().ok();
        }
        if let Some(arr) = value.get("input_channels").and_then(|v| v.as_array()) {
            cue.input_channels = arr.iter().filter_map(|v| v.as_u64().map(|n| n as u16)).collect();
        }
        if let Some(s) = value.get("output_patch_id").and_then(|v| v.as_str()) {
            cue.output_patch_id = s.parse().ok();
        }
        if let Some(db) = value.get("volume_db").and_then(|v| v.as_f64()) {
            cue.volume_db = db;
        }
        if let Some(pan) = value.get("pan").and_then(|v| v.as_f64()) {
            cue.pan = pan as f32;
        }
        if let Some(ms) = value.get("fade_in_ms").and_then(|v| v.as_u64()) {
            let curve = value.get("fade_in_curve")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or(FadeCurve::SCurve);
            cue.fade_in = Some(FadeSpec { duration_ms: ms, curve });
        }
        if let Some(ms) = value.get("fade_out_ms").and_then(|v| v.as_u64()) {
            let curve = value.get("fade_out_curve")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or(FadeCurve::SCurve);
            cue.fade_out = Some(FadeSpec { duration_ms: ms, curve });
        }
        if let Some(b) = value.get("is_disabled").and_then(|v| v.as_bool()) {
            cue.is_disabled = b;
        }

        Ok(Box::new(cue))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_roundtrip() {
        let mut c = MicCue::new();
        c.set_name("Stage Mic".to_string());
        c.set_number(Some("3".to_string()));
        c.input_channels = vec![0, 1];
        c.volume_db = -3.0;
        c.fade_in = Some(FadeSpec::new(400));
        c.continue_mode = ContinueMode::AutoContinue;

        let json = c.serialize();
        let restored = MicCueFactory.from_json(json).expect("deserialize");
        assert_eq!(restored.name(), "Stage Mic");
        assert_eq!(restored.number(), Some("3"));
        assert_eq!(restored.cue_type(), CueType::Mic);
        assert_eq!(restored.continue_mode(), ContinueMode::AutoContinue);
    }

    #[test]
    fn runs_until_stopped() {
        let c = MicCue::new();
        assert_eq!(c.duration(), None);
        assert_eq!(c.state(), CueState::Standby);
    }

    #[test]
    fn runtime_state_survives_inspector_rebuild() {
        // Simulate a running Mic Cue with an active voice.
        let mut c = MicCue::new();
        c.state = CueState::Running;
        c.active_voice_id = Some(uuid::Uuid::new_v4());
        c.volume_db = -6.0;
        let snap = c.runtime_state();
        let expected_voice = snap.voice_id;

        // update_cue rebuilds from JSON then transplants runtime state.
        let mut rebuilt = MicCue::new();
        rebuilt.restore_runtime_state(snap);

        assert_eq!(rebuilt.state(), CueState::Running, "stays Running after edit");
        assert_eq!(rebuilt.playing_voice_id(), expected_voice, "keeps its voice → stoppable");
        let live = rebuilt.live_audio_params().expect("running voice exposes live params");
        assert_eq!(live.voice_id, expected_voice.unwrap());
    }
}
