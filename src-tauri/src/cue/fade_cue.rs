//! [`FadeCue`] — fades the volume/brightness of one or more running cues.
//!
//! On GO the Fade Cue locates its targets by UUID and:
//! - For **audio** cues: smoothly interpolates the voice gain in `tick()`.
//! - For **video** cues: interpolates the paired audio voice gain AND animates
//!   the OutputEngine overlay from current alpha to the target alpha.
//! - For **image** cues: animates the overlay only (no audio voice).
//!
//! `target_gain_linear = 0.0` → fade to black/silence.
//! `target_gain_linear = 1.0` → fade to full brightness/unity volume.

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
    curve::{CurveKind, FadeShapes},
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

    /// UUIDs of cues to fade (empty = no-op).
    pub target_cue_ids: Vec<CueId>,
    /// Display labels kept in sync with target_cue_ids (for inspector).
    pub target_cue_numbers: Vec<String>,
    /// Target audio volume in dB (−60 = silence, 0 = unity).
    pub target_volume_db: f64,
    /// Target visual brightness in percent (0 = black overlay, 100 = fully visible).
    /// Independent from `target_volume_db`.
    pub target_brightness_pct: f64,
    /// Target stereo pan (-1 = left, 0 = center, +1 = right). `None` = leave pan
    /// untouched (a volume/brightness-only fade); `Some` = fade every audio
    /// target's pan from its current position to this value.
    pub target_pan: Option<f32>,
    /// When `true` (default) the fade drives the target volume toward
    /// `target_volume_db`. Set `false` for a **pan-only** fade that must leave the
    /// level untouched (mirrors QLab's pan-crosspoint fade, which doesn't move the
    /// master).
    pub fade_volume: bool,
    /// Fade duration in milliseconds.
    pub fade_duration_ms: u64,
    /// Rising and falling envelopes. Which one applies is decided per target:
    /// in one Fade Cue some voices may be coming up while others go down, and
    /// QLab shapes those differently on purpose.
    pub shapes: FadeShapes,
    /// Stop the target cue(s) after the fade completes.
    pub stop_at_end: bool,
    is_disabled: bool,

    // Runtime — injected by transport after go()
    /// (audio_voice_id, start_gain, start_pan) for each audio/video audio-track target.
    target_voices: Vec<(Uuid, f32, f32)>,
    /// (output_voice_id, start_opacity) for each visual target's layer.
    visual_targets: Vec<(Uuid, f32)>,
    /// Layer opacity at fade completion (0.0 = black, 1.0 = fully visible).
    visual_target_opacity: f32,
    fade_complete: bool,
    /// Target cue ids to hard-stop once the fade finishes (set on completion when
    /// `stop_at_end` is on).  The event loop drains this via
    /// [`take_fade_stop_targets`](crate::cue::traits::Cue::take_fade_stop_targets)
    /// and stops the actual cues — the fade itself can't reach the cue list.
    stop_targets_pending: Vec<CueId>,
}

impl FadeCue {
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::from("Fade"),
            number: None,
            notes: String::new(),
            color: CueColor::Pink,
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
            target_cue_ids: Vec::new(),
            target_cue_numbers: Vec::new(),
            target_volume_db: -60.0,
            target_brightness_pct: 0.0,
            target_pan: None,
            fade_volume: true,
            fade_duration_ms: 2000,
            shapes: FadeShapes::default(),
            stop_at_end: false,
            is_disabled: false,
            target_voices: Vec::new(),
            visual_targets: Vec::new(),
            visual_target_opacity: 0.0,
            fade_complete: false,
            stop_targets_pending: Vec::new(),
        }
    }

    fn engine_curve(c: FadeCurve) -> EngineFadeCurve {
        match c {
            FadeCurve::Linear => EngineFadeCurve::Linear,
            FadeCurve::SCurve => EngineFadeCurve::SCurve,
            FadeCurve::Exponential => EngineFadeCurve::Exponential,
        }
    }

    /// The legacy single-curve name for this cue, so an older Inkue opening the
    /// file still gets approximately the right shape.
    fn legacy_curve(&self) -> FadeCurve {
        match self.shapes.up.kind {
            CurveKind::Linear => FadeCurve::Linear,
            CurveKind::Exponential => FadeCurve::Exponential,
            _ => FadeCurve::SCurve,
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
        self.target_voices = Vec::new();
        self.visual_targets = Vec::new();
        self.visual_target_opacity = 0.0;
        self.fade_complete = false;
        self.stop_targets_pending.clear();
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
        self.target_voices = Vec::new();
        self.visual_targets = Vec::new();
        self.visual_target_opacity = 0.0;
        self.fade_complete = false;
        self.stop_targets_pending.clear();
        Ok(())
    }

    fn tick(&mut self, context: &CueContext) -> Result<()> {
        if self.state != CueState::Running {
            return Ok(());
        }

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

        // No targets → nothing to drive; just wait for duration to expire.
        if self.target_voices.is_empty() && self.visual_targets.is_empty() {
            return Ok(());
        }

        let elapsed_ms = self.action_elapsed().as_millis() as f64;
        let duration_ms = self.fade_duration_ms as f64;
        let t = if duration_ms <= 0.0 { 1.0_f64 } else { (elapsed_ms / duration_ms).clamp(0.0, 1.0) };
        // Both directions sampled once; each target picks the one that matches
        // the way its own value is travelling.
        let rising_t = self.shapes.sample(t, true) as f32;
        let falling_t = self.shapes.sample(t, false) as f32;
        let curved_t = rising_t;

        // Interpolate gain for each audio voice (skipped for a pan-only fade so
        // the level is left exactly where it was).
        if self.fade_volume {
            let target_gain = db_to_linear(self.target_volume_db) as f32;
            for &(vid, start_gain, _) in &self.target_voices {
                let progress = if target_gain >= start_gain { rising_t } else { falling_t };
                let gain = start_gain + (target_gain - start_gain) * progress;
                let _ = context.audio_engine.set_voice_gain(vid, gain);
            }
        }

        // Interpolate pan for each audio voice when a pan target is set.
        if let Some(target_pan) = self.target_pan {
            for &(vid, _, start_pan) in &self.target_voices {
                let progress = if target_pan >= start_pan { rising_t } else { falling_t };
                let pan = start_pan + (target_pan - start_pan) * progress;
                let _ = context.audio_engine.set_voice_pan(vid, pan);
            }
        }

        // Interpolate each visual target's layer opacity (per-slot — a Fade
        // on one video no longer dips the whole output to black).
        for &(vid, start_opacity) in &self.visual_targets {
            let opacity =
                start_opacity + (self.visual_target_opacity - start_opacity) * curved_t;
            context.output_engine.set_voice_opacity(vid, opacity);
        }

        // At fade completion, optionally stop targets.
        if t >= 1.0 && !self.fade_complete {
            self.fade_complete = true;
            if self.stop_at_end {
                // Immediate audio cut (the fade already reached the target level).
                for &(vid, _, _) in &self.target_voices {
                    let _ = context.audio_engine.stop_voice(vid, 0, Self::engine_curve(self.legacy_curve()));
                }
                for &(vid, _) in &self.visual_targets {
                    let _ = context.output_engine.stop_voice(vid, 0);
                }
                // Queue the target CUES to be stopped by the event loop: stopping
                // voices alone leaves the target cue (and group children) in the
                // Running state.  The event loop drains this and calls their
                // hard_stop(), which resets state and recurses into groups.
                self.stop_targets_pending = self.target_cue_ids.clone();
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

    fn take_fade_stop_targets(&mut self) -> Vec<CueId> {
        std::mem::take(&mut self.stop_targets_pending)
    }

    fn fade_specification(&self) -> Option<FadeAction> {
        let visual_alpha = ((1.0 - self.target_brightness_pct.clamp(0.0, 100.0) / 100.0) * 255.0)
            .round() as u8;
        Some(FadeAction {
            target_cue_ids: self.target_cue_ids.clone(),
            target_gain_linear: db_to_linear(self.target_volume_db) as f32,
            target_visual_alpha: Some(visual_alpha),
            duration_ms: self.fade_duration_ms,
            curve: self.legacy_curve(),
            stop_at_end: self.stop_at_end,
        })
    }

    fn set_fade_voices(
        &mut self,
        voices: Vec<(CueId, f32, f32)>,
        visual_targets: Vec<(CueId, f32)>,
        visual_target_opacity: f32,
    ) {
        self.target_voices = voices;
        self.visual_targets = visual_targets;
        self.visual_target_opacity = visual_target_opacity.clamp(0.0, 1.0);
    }

    fn resolve_fade_targets(&mut self, number_to_id: &std::collections::HashMap<String, CueId>) {
        if self.target_cue_ids.is_empty() {
            for num in &self.target_cue_numbers {
                if let Some(&id) = number_to_id.get(num) {
                    self.target_cue_ids.push(id);
                }
            }
        }
    }

    fn validate(
        &self,
        ctx: &crate::cue::validation::ValidationContext,
    ) -> Vec<crate::cue::validation::CueIssue> {
        use crate::cue::validation::CueIssue;
        let mut issues: Vec<CueIssue> = self
            .target_cue_ids
            .iter()
            .filter(|id| !ctx.all_cue_ids.contains(id))
            .map(|_| CueIssue::warning("Fade target not found (cue deleted)"))
            .collect();
        if self.target_cue_ids.is_empty() && self.target_cue_numbers.is_empty() {
            issues.push(CueIssue::warning("No target selected"));
        }
        issues
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
            "target_cue_ids": self.target_cue_ids.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
            "target_cue_numbers": self.target_cue_numbers,
            "target_volume_db": self.target_volume_db,
            "target_brightness_pct": self.target_brightness_pct,
            "target_pan": self.target_pan,
            "fade_volume": self.fade_volume,
            "fade_duration_ms": self.fade_duration_ms,
            "fade_curve": self.legacy_curve(),
            "fade_shapes": self.shapes,
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
        // New format: target_cue_ids array.
        if let Some(arr) = value.get("target_cue_ids").and_then(|v| v.as_array()) {
            cue.target_cue_ids = arr.iter()
                .filter_map(|v| v.as_str()?.parse().ok())
                .collect();
        }
        // target_cue_numbers array.
        if let Some(arr) = value.get("target_cue_numbers").and_then(|v| v.as_array()) {
            cue.target_cue_numbers = arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
        } else if let Some(s) = value.get("target_cue_number").and_then(|v| v.as_str()) {
            // Backward compat: old single cue-number field.
            cue.target_cue_numbers = vec![s.to_string()];
        }
        if let Some(db) = value.get("target_volume_db").and_then(|v| v.as_f64()) {
            cue.target_volume_db = db;
        }
        if let Some(pct) = value.get("target_brightness_pct").and_then(|v| v.as_f64()) {
            cue.target_brightness_pct = pct;
        }
        cue.target_pan = value.get("target_pan").and_then(|v| v.as_f64()).map(|x| x as f32);
        if let Some(b) = value.get("fade_volume").and_then(|v| v.as_bool()) {
            cue.fade_volume = b;
        }
        if let Some(ms) = value.get("fade_duration_ms").and_then(|v| v.as_u64()) {
            cue.fade_duration_ms = ms;
        }
        // A file written before curve shapes existed carries only the name;
        // turn it into the equivalent locked pair so nothing changes for it.
        if let Some(c) = value.get("fade_curve") {
            if let Ok(curve) = serde_json::from_value::<FadeCurve>(c.clone()) {
                cue.shapes = FadeShapes::of_kind(match curve {
                    FadeCurve::Linear => CurveKind::Linear,
                    FadeCurve::SCurve => CurveKind::SCurve,
                    FadeCurve::Exponential => CurveKind::Exponential,
                });
            }
        }
        if let Some(s) = value.get("fade_shapes") {
            if let Ok(shapes) = serde_json::from_value(s.clone()) {
                cue.shapes = shapes;
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
        assert!(c.target_cue_ids.is_empty());
    }

    #[test]
    fn serialize_roundtrip() {
        let factory = FadeCueFactory;
        let mut cue = FadeCue::new();
        cue.set_name("My Fade".to_string());
        let target_id = Uuid::new_v4();
        cue.target_cue_ids = vec![target_id];
        cue.target_cue_numbers = vec!["3".to_string()];
        cue.target_volume_db = -6.0;
        cue.fade_duration_ms = 3000;
        cue.stop_at_end = true;

        let json = cue.serialize();
        assert_eq!(json["name"], "My Fade");
        assert_eq!(json["target_volume_db"], -6.0);
        assert_eq!(json["fade_duration_ms"], 3000u64);
        assert_eq!(json["stop_at_end"], true);

        let rebuilt = factory.from_json(json).unwrap();
        assert_eq!(rebuilt.name(), "My Fade");
    }

    #[test]
    fn default_fade_has_no_pan_target_and_fades_volume() {
        let c = FadeCue::new();
        assert!(c.target_pan.is_none());
        assert!(c.fade_volume);
    }

    #[test]
    fn pan_fade_serialize_roundtrip() {
        let factory = FadeCueFactory;
        let mut cue = FadeCue::new();
        cue.target_pan = Some(1.0);   // full right
        cue.fade_volume = false;      // pan-only fade
        let json = cue.serialize();
        assert_eq!(json["target_pan"], 1.0);
        assert_eq!(json["fade_volume"], false);

        // Re-parse and re-serialize: the pan target and pan-only flag survive.
        let rebuilt = factory.from_json(json).unwrap().serialize();
        assert_eq!(rebuilt["target_pan"], 1.0);
        assert_eq!(rebuilt["fade_volume"], false);
    }

    #[test]
    fn absent_pan_target_deserializes_to_none() {
        let factory = FadeCueFactory;
        let json = serde_json::json!({
            "type": "fade", "cue_type": "fade",
            "id": Uuid::new_v4().to_string(),
            "name": "Vol Fade", "notes": "", "color": "pink",
            "pre_wait_ms": 0u64, "post_wait_ms": 0u64, "continue_mode": "auto_follow",
            "target_cue_ids": [], "target_cue_numbers": [],
            "target_volume_db": -6.0, "fade_duration_ms": 1000u64,
            "fade_curve": "s_curve", "stop_at_end": false, "is_disabled": false,
        });
        // No target_pan / fade_volume keys → pan untouched, volume still fades.
        assert!(factory.from_json(json).unwrap().serialize()["target_pan"].is_null());
    }

    #[test]
    fn fade_specification_returns_action() {
        let mut cue = FadeCue::new();
        let id = Uuid::new_v4();
        cue.target_cue_ids = vec![id];
        cue.target_volume_db = 0.0;
        cue.fade_duration_ms = 1000;
        cue.stop_at_end = true;

        let spec = cue.fade_specification().unwrap();
        assert_eq!(spec.target_cue_ids, vec![id]);
        assert!((spec.target_gain_linear - 1.0).abs() < 1e-4);
        assert_eq!(spec.duration_ms, 1000);
        assert!(spec.stop_at_end);
    }

    #[test]
    fn backward_compat_single_target_number() {
        let factory = FadeCueFactory;
        // Simulate old-format JSON with target_cue_number (not _ids).
        let old_json = serde_json::json!({
            "type": "fade", "cue_type": "fade",
            "id": Uuid::new_v4().to_string(),
            "name": "Old Fade", "notes": "", "color": "blue",
            "pre_wait_ms": 0u64, "post_wait_ms": 0u64,
            "continue_mode": "auto_follow",
            "target_cue_number": "5",
            "target_volume_db": -60.0, "fade_duration_ms": 2000u64,
            "fade_curve": "s_curve", "stop_at_end": false, "is_disabled": false,
        });
        let cue = factory.from_json(old_json).unwrap();
        let spec = cue.fade_specification().unwrap();
        // UUIDs not resolved yet (no workspace loaded), but number is stored.
        assert!(spec.target_cue_ids.is_empty());
    }

    #[test]
    fn a_show_written_before_curve_shapes_still_loads_its_curve() {
        // Only "fade_curve" — the pre-shapes format. It must come back as the
        // same shape, locked, so the fade behaves exactly as it always did.
        let json = serde_json::json!({
            "type": "fade", "id": Uuid::new_v4().to_string(), "name": "Old",
            "fade_duration_ms": 2000, "fade_curve": "exponential",
        });
        let rebuilt = FadeCueFactory.from_json(json).unwrap();
        let out = rebuilt.serialize();
        assert_eq!(out["fade_shapes"]["up"]["kind"], "exponential");
        assert_eq!(out["fade_shapes"]["mirrored"], true);
        assert_eq!(out["fade_curve"], "exponential", "legacy name still written");
    }

    #[test]
    fn curve_shapes_survive_a_roundtrip_and_win_over_the_legacy_name() {
        use crate::cue::curve::{CurvePoint, CurveShape};
        let mut cue = FadeCue::new();
        cue.shapes.mirrored = false;
        cue.shapes.up.kind = CurveKind::Linear;
        cue.shapes.up.points = vec![CurvePoint::new(0.3, 0.8)];
        cue.shapes.down = CurveShape::of_kind(CurveKind::Parametric);

        let rebuilt = FadeCueFactory.from_json(cue.serialize()).unwrap();
        let out = rebuilt.serialize();
        assert_eq!(out["fade_shapes"]["up"]["kind"], "linear");
        assert_eq!(out["fade_shapes"]["up"]["points"][0]["t"], 0.3);
        assert_eq!(out["fade_shapes"]["down"]["kind"], "parametric");
        assert_eq!(out["fade_shapes"]["mirrored"], false);
    }

    #[test]
    fn the_legacy_name_written_out_reflects_the_shape_that_is_set() {
        let mut cue = FadeCue::new();
        cue.shapes = FadeShapes::of_kind(CurveKind::Linear);
        assert_eq!(cue.serialize()["fade_curve"], "linear");
        // A parametric shape has no legacy equivalent — S-Curve is the honest
        // approximation for an older build reading the file.
        cue.shapes = FadeShapes::of_kind(CurveKind::Parametric);
        assert_eq!(cue.serialize()["fade_curve"], "s_curve");
    }
}
