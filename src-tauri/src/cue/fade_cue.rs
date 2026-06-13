//! [`FadeCue`] — fades the volume of a running audio cue over a set duration.
//!
//! On GO, the Fade Cue locates the target cue by number and smoothly
//! interpolates its gain from the current level to the configured target
//! level.  Optionally stops the target cue once the fade completes.
//!
//! The fade is applied in `tick()` at the event-loop frame rate (~30 fps),
//! which is sufficient resolution for all practical fade durations.

use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{
    cue::types::db_to_linear,
    engine::ring_command::FadeCurve as EngineFadeCurve,
};

use super::{
    context::{CueContext, CueEvent},
    traits::{Cue, CueFactory, RuntimeState},
    types::{ContinueMode, CueColor, CueId, CueState, CueType, FadeAction, FadeCurve},
};

// ---------------------------------------------------------------------------
// FadeCue
// ---------------------------------------------------------------------------

pub struct FadeCue {
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
    action_started_at: Option<Instant>,
    in_pre_wait: bool,
    auto_continue_fired: bool,
    elapsed_before_pause: Duration,
    action_elapsed_before_pause: Duration,

    /// Cue number to target (`None` = no target, fade is a no-op).
    pub target_cue_number: Option<String>,
    /// Target volume in dB (-60.0 = silence, 0.0 = unity).
    pub target_volume_db: f64,
    /// Duration of the fade in milliseconds (also the action duration).
    pub fade_duration_ms: u64,
    /// Fade curve shape.
    pub fade_curve: FadeCurve,
    /// Stop the target cue after the fade completes.
    pub stop_at_end: bool,
    is_disabled: bool,

    // Runtime — injected by transport after go()
    target_voice_id: Option<Uuid>,
    start_gain: f32,
    fade_complete: bool,
}

impl FadeCue {
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::from("Fade"),
            number: None,
            notes: String::new(),
            color: CueColor::Blue,
            state: CueState::Standby,
            continue_mode: ContinueMode::AutoFollow,
            pre_wait: Duration::ZERO,
            post_wait: Duration::ZERO,
            started_at: None,
            action_started_at: None,
            in_pre_wait: false,
            auto_continue_fired: false,
            elapsed_before_pause: Duration::ZERO,
            action_elapsed_before_pause: Duration::ZERO,
            target_cue_number: None,
            target_volume_db: -60.0,
            fade_duration_ms: 2000,
            fade_curve: FadeCurve::SCurve,
            stop_at_end: false,
            is_disabled: false,
            target_voice_id: None,
            start_gain: 1.0,
            fade_complete: false,
        }
    }

    fn engine_curve(c: FadeCurve) -> EngineFadeCurve {
        match c {
            FadeCurve::Linear => EngineFadeCurve::Linear,
            FadeCurve::SCurve => EngineFadeCurve::SCurve,
            FadeCurve::Exponential => EngineFadeCurve::Exponential,
        }
    }
}

impl Default for FadeCue {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Cue trait
// ---------------------------------------------------------------------------

impl Cue for FadeCue {
    fn id(&self) -> CueId { self.id }
    fn cue_type(&self) -> CueType { CueType::Fade }
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

    fn go(&mut self, _context: &CueContext) -> Result<()> {
        self.auto_continue_fired = false;
        self.elapsed_before_pause = Duration::ZERO;
        self.action_elapsed_before_pause = Duration::ZERO;
        self.target_voice_id = None;
        self.start_gain = 1.0;
        self.fade_complete = false;
        self.started_at = Some(Instant::now());

        if self.pre_wait.is_zero() {
            self.in_pre_wait = false;
            self.action_started_at = Some(Instant::now());
        } else {
            self.in_pre_wait = true;
            self.action_started_at = None;
        }

        self.state = CueState::Running;
        Ok(())
    }

    fn stop(&mut self, context: &CueContext) -> Result<()> {
        self.state = CueState::Standby;
        self.started_at = None;
        self.action_started_at = None;
        self.in_pre_wait = false;
        self.elapsed_before_pause = Duration::ZERO;
        self.action_elapsed_before_pause = Duration::ZERO;
        context.emit(CueEvent::Stopped { cue_id: self.id });
        Ok(())
    }

    fn pause(&mut self, _context: &CueContext) -> Result<()> {
        if self.state == CueState::Running {
            if let Some(t) = self.started_at.take() {
                self.elapsed_before_pause += t.elapsed();
            }
            if !self.in_pre_wait {
                if let Some(t) = self.action_started_at.take() {
                    self.action_elapsed_before_pause += t.elapsed();
                }
            }
            self.state = CueState::Paused;
        }
        Ok(())
    }

    fn resume(&mut self, _context: &CueContext) -> Result<()> {
        if self.state == CueState::Paused {
            self.started_at = Some(Instant::now());
            if !self.in_pre_wait {
                self.action_started_at = Some(Instant::now());
            }
            self.state = CueState::Running;
        }
        Ok(())
    }

    fn hard_stop(&mut self, context: &CueContext) -> Result<()> {
        self.stop(context)
    }

    fn reset(&mut self) -> Result<()> {
        self.state = CueState::Standby;
        self.started_at = None;
        self.action_started_at = None;
        self.in_pre_wait = false;
        self.elapsed_before_pause = Duration::ZERO;
        self.action_elapsed_before_pause = Duration::ZERO;
        self.target_voice_id = None;
        self.fade_complete = false;
        Ok(())
    }

    fn tick(&mut self, context: &CueContext) -> Result<()> {
        if self.state != CueState::Running {
            return Ok(());
        }

        // Pre-wait: wait until it expires, then start the action.
        if self.in_pre_wait {
            if let Some(st) = self.started_at {
                if st.elapsed() >= self.pre_wait {
                    self.in_pre_wait = false;
                    self.action_started_at = Some(Instant::now());
                    context.emit(CueEvent::ActionStarted { cue_id: self.id });
                }
            }
            return Ok(());
        }

        if self.action_started_at.is_none() {
            return Ok(());
        }

        let Some(vid) = self.target_voice_id else { return Ok(()); };

        let elapsed_ms = self.action_elapsed().as_millis() as f64;
        let duration_ms = self.fade_duration_ms as f64;
        let t = if duration_ms <= 0.0 { 1.0_f64 } else { (elapsed_ms / duration_ms).clamp(0.0, 1.0) };
        let curved_t = self.fade_curve.apply(t) as f32;

        let target_gain = db_to_linear(self.target_volume_db) as f32;
        let gain = self.start_gain + (target_gain - self.start_gain) * curved_t;
        let _ = context.audio_engine.set_voice_gain(vid, gain);

        if t >= 1.0 && !self.fade_complete {
            self.fade_complete = true;
            if self.stop_at_end {
                let _ = context.audio_engine.stop_voice(vid, 0, Self::engine_curve(self.fade_curve));
            }
        }

        Ok(())
    }

    fn is_action_started(&self) -> bool {
        !self.in_pre_wait
    }

    fn pre_wait(&self) -> Duration { self.pre_wait }
    fn set_pre_wait(&mut self, d: Duration) { self.pre_wait = d; }
    fn post_wait(&self) -> Duration { self.post_wait }
    fn set_post_wait(&mut self, d: Duration) { self.post_wait = d; }

    fn duration(&self) -> Option<Duration> {
        Some(Duration::from_millis(self.fade_duration_ms))
    }

    fn elapsed(&self) -> Duration {
        match self.state {
            CueState::Running => {
                self.elapsed_before_pause
                    + self.started_at.map(|t| t.elapsed()).unwrap_or(Duration::ZERO)
            }
            CueState::Paused => self.elapsed_before_pause,
            _ => Duration::ZERO,
        }
    }

    fn action_elapsed(&self) -> Duration {
        if self.in_pre_wait {
            return Duration::ZERO;
        }
        match self.state {
            CueState::Running => {
                self.action_elapsed_before_pause
                    + self.action_started_at.map(|t| t.elapsed()).unwrap_or(Duration::ZERO)
            }
            CueState::Paused => self.action_elapsed_before_pause,
            _ => Duration::ZERO,
        }
    }

    fn continue_mode(&self) -> ContinueMode { self.continue_mode }
    fn set_continue_mode(&mut self, mode: ContinueMode) { self.continue_mode = mode; }

    fn is_auto_continue_fired(&self) -> bool { self.auto_continue_fired }
    fn mark_auto_continue_fired(&mut self) { self.auto_continue_fired = true; }
    fn clear_auto_continue_fired(&mut self) { self.auto_continue_fired = false; }

    fn fade_specification(&self) -> Option<FadeAction> {
        Some(FadeAction {
            target_cue_number: self.target_cue_number.clone(),
            target_gain_linear: db_to_linear(self.target_volume_db) as f32,
            duration_ms: self.fade_duration_ms,
            curve: self.fade_curve,
            stop_at_end: self.stop_at_end,
        })
    }

    fn set_fade_voice(&mut self, voice_id: Option<CueId>, start_gain: f32) {
        self.target_voice_id = voice_id;
        self.start_gain = start_gain;
    }

    fn runtime_state(&self) -> RuntimeState {
        RuntimeState {
            state: self.state,
            voice_id: None,
            started_at: self.started_at,
            action_started_at: self.action_started_at,
        }
    }

    fn restore_runtime_state(&mut self, snap: RuntimeState) {
        self.state = snap.state;
        self.started_at = snap.started_at;
        self.action_started_at = snap.action_started_at;
        self.in_pre_wait = snap.action_started_at.is_none() && snap.state == CueState::Running;
    }

    fn serialize(&self) -> Value {
        json!({
            "type": "fade",
            "cue_type": "fade",
            "id": self.id,
            "number": self.number,
            "name": self.name,
            "notes": self.notes,
            "color": self.color,
            "pre_wait_ms": self.pre_wait.as_millis() as u64,
            "post_wait_ms": self.post_wait.as_millis() as u64,
            "continue_mode": self.continue_mode,
            "target_cue_number": self.target_cue_number,
            "target_volume_db": self.target_volume_db,
            "fade_duration_ms": self.fade_duration_ms,
            "fade_curve": self.fade_curve,
            "stop_at_end": self.stop_at_end,
            "is_disabled": self.is_disabled,
        })
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

pub struct FadeCueFactory;

impl CueFactory for FadeCueFactory {
    fn create(&self) -> Box<dyn Cue> {
        Box::new(FadeCue::new())
    }

    fn from_json(&self, value: Value) -> anyhow::Result<Box<dyn Cue>> {
        let mut cue = FadeCue::new();

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
        if let Some(s) = value.get("target_cue_number").and_then(|v| v.as_str()) {
            cue.target_cue_number = Some(s.to_string());
        }
        if let Some(db) = value.get("target_volume_db").and_then(|v| v.as_f64()) {
            cue.target_volume_db = db;
        }
        if let Some(ms) = value.get("fade_duration_ms").and_then(|v| v.as_u64()) {
            cue.fade_duration_ms = ms;
        }
        if let Some(c) = value.get("fade_curve") {
            if let Ok(curve) = serde_json::from_value(c.clone()) {
                cue.fade_curve = curve;
            }
        }
        if let Some(b) = value.get("stop_at_end").and_then(|v| v.as_bool()) {
            cue.stop_at_end = b;
        }
        if let Some(b) = value.get("is_disabled").and_then(|v| v.as_bool()) {
            cue.is_disabled = b;
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
    fn cue_type_is_fade() {
        assert_eq!(FadeCue::new().cue_type(), CueType::Fade);
    }

    #[test]
    fn default_values() {
        let c = FadeCue::new();
        assert_eq!(c.fade_duration_ms, 2000);
        assert!((c.target_volume_db - (-60.0)).abs() < 1e-9);
        assert!(!c.stop_at_end);
    }

    #[test]
    fn serialize_roundtrip() {
        let factory = FadeCueFactory;
        let mut cue = FadeCue::new();
        cue.set_name("My Fade".to_string());
        cue.target_cue_number = Some("3".to_string());
        cue.target_volume_db = -6.0;
        cue.fade_duration_ms = 3000;
        cue.stop_at_end = true;

        let json = cue.serialize();
        assert_eq!(json["name"], "My Fade");
        assert_eq!(json["target_cue_number"], "3");
        assert_eq!(json["target_volume_db"], -6.0);
        assert_eq!(json["fade_duration_ms"], 3000u64);
        assert_eq!(json["stop_at_end"], true);

        let rebuilt = factory.from_json(json).unwrap();
        assert_eq!(rebuilt.name(), "My Fade");
    }

    #[test]
    fn fade_specification_returns_action() {
        let mut cue = FadeCue::new();
        cue.target_cue_number = Some("1".to_string());
        cue.target_volume_db = 0.0;
        cue.fade_duration_ms = 1000;
        cue.stop_at_end = true;

        let spec = cue.fade_specification().unwrap();
        assert_eq!(spec.target_cue_number.as_deref(), Some("1"));
        assert!((spec.target_gain_linear - 1.0).abs() < 1e-4);
        assert_eq!(spec.duration_ms, 1000);
        assert!(spec.stop_at_end);
    }
}
