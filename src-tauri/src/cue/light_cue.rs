//! [`LightCue`] — fades patched DMX fixtures to a target look.
//!
//! A Light Cue stores only the parameters it *changes* (a list of
//! [`ParamTarget`]); every other channel keeps its current value (tracking),
//! and the latest fade on a channel wins (LTP).  This mirrors a lighting
//! console / QLab and is enforced by the [`DmxEngine`](crate::engine::DmxEngine).
//!
//! At GO the cue resolves each target's `(universe, channel, width)` from the
//! workspace fixture patch and submits a timed fade to the engine.  The fade
//! itself runs on the engine's ~40 Hz thread; the cue stays
//! [`Running`](CueState::Running) for the fade duration so it shows a progress
//! bar and drives Auto-Continue / Auto-Follow, then the event loop completes it.

use std::time::{Duration, Instant};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::engine::fixture::ParamKind;
use crate::engine::ring_command::FadeCurve as EngineFadeCurve;

use super::{
    context::{CueContext, CueEvent},
    traits::{Cue, CueFactory},
    types::{ContinueMode, CueColor, CueId, CueState, CueType, FadeCurve, FadeSpec},
};

/// One thing a Light Cue drives to a target value.
///
/// Either a specific parameter of one fixture, or — by *kind* — every matching
/// parameter of every member of a group (so one colour control fans out to a
/// whole wash).  IDs are stored as strings so a half-configured row never
/// poisons deserialisation of the whole list, and are parsed at GO time.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ParamTarget {
    /// One parameter (`parameters[param_index]`) of one fixture.
    Fixture {
        fixture_id: String,
        param_index: usize,
        value: f64,
    },
    /// Every parameter of `param_kind` on every member of a group.
    Group {
        group_id: String,
        param_kind: ParamKind,
        value: f64,
    },
}

impl ParamTarget {
    /// The target value (0–1), regardless of selector.
    pub fn value(&self) -> f64 {
        match self {
            ParamTarget::Fixture { value, .. } | ParamTarget::Group { value, .. } => *value,
        }
    }
}

/// A cue that fades fixtures to a target look on GO.
pub struct LightCue {
    id: CueId,
    name: String,
    number: Option<String>,
    notes: String,
    color: CueColor,
    state: CueState,
    continue_mode: ContinueMode,
    pre_wait: Duration,
    post_wait: Duration,
    is_disabled: bool,
    started_at: Option<Instant>,
    /// The fixture parameter targets to drive on GO.
    pub targets: Vec<ParamTarget>,
    /// Fade time + curve applied to every target.
    pub fade: FadeSpec,
}

impl LightCue {
    /// Create a new Light cue with a fresh UUID, no targets, and a 3 s fade.
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::from("Light"),
            number: None,
            notes: String::new(),
            color: CueColor::Yellow,
            state: CueState::Standby,
            continue_mode: ContinueMode::DoNotContinue,
            pre_wait: Duration::ZERO,
            post_wait: Duration::ZERO,
            is_disabled: false,
            started_at: None,
            targets: Vec::new(),
            fade: FadeSpec::new(3000),
        }
    }

    /// Convert a [`FadeCurve`] from the cue layer to the engine layer.
    fn engine_curve(c: FadeCurve) -> EngineFadeCurve {
        match c {
            FadeCurve::Linear => EngineFadeCurve::Linear,
            FadeCurve::SCurve => EngineFadeCurve::SCurve,
            FadeCurve::Exponential => EngineFadeCurve::Exponential,
        }
    }

    /// Submit one fade per resolved channel to the DMX engine.
    fn submit_targets(&self, context: &CueContext) {
        let dur = self.fade.duration();
        let curve = Self::engine_curve(self.fade.curve);
        for target in &self.targets {
            match target {
                ParamTarget::Fixture { fixture_id, param_index, value } => {
                    // An empty / unparseable id is an unconfigured row — skip
                    // silently; only warn when a real id no longer resolves.
                    let Ok(id) = fixture_id.parse::<Uuid>() else { continue };
                    let Some(fixture) = context.resolve_fixture(id) else {
                        log::warn!("LightCue {}: fixture {fixture_id} not patched", self.id);
                        continue;
                    };
                    if let Some((u, ch, w)) = fixture.resolve_channel(*param_index) {
                        context.dmx_engine.submit_fade(u, ch, w, *value, dur, curve);
                    }
                }
                ParamTarget::Group { group_id, param_kind, value } => {
                    let Ok(id) = group_id.parse::<Uuid>() else { continue };
                    let Some(group) = context.resolve_group(id) else {
                        log::warn!("LightCue {}: group {group_id} not found", self.id);
                        continue;
                    };
                    for member_id in &group.fixture_ids {
                        let Some(fixture) = context.resolve_fixture(*member_id) else { continue };
                        for (i, p) in fixture.fixture_type.parameters.iter().enumerate() {
                            if p.kind == *param_kind {
                                if let Some((u, ch, w)) = fixture.resolve_channel(i) {
                                    context.dmx_engine.submit_fade(u, ch, w, *value, dur, curve);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

impl Default for LightCue {
    fn default() -> Self {
        Self::new()
    }
}

impl Cue for LightCue {
    fn id(&self) -> CueId { self.id }
    fn cue_type(&self) -> CueType { CueType::Light }
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
        self.submit_targets(context);
        Ok(())
    }

    fn stop(&mut self, context: &CueContext) -> Result<()> {
        // Stopping a Light Cue leaves the lights where they are (tracking) —
        // it only releases the cue back to Standby.
        self.state = CueState::Standby;
        self.started_at = None;
        context.emit(CueEvent::Stopped { cue_id: self.id });
        Ok(())
    }

    // A DMX fade runs on the engine clock and cannot be paused mid-flight, so
    // pause / resume are no-ops: the cue keeps running and the fade completes.
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

    /// The fade duration — used by the event loop for completion detection.
    fn duration(&self) -> Option<Duration> {
        Some(self.fade.duration())
    }

    fn elapsed(&self) -> Duration {
        match self.state {
            CueState::Running => self.started_at.map(|t| t.elapsed()).unwrap_or(Duration::ZERO),
            _ => Duration::ZERO,
        }
    }

    fn action_elapsed(&self) -> Duration {
        self.elapsed()
    }

    fn continue_mode(&self) -> ContinueMode { self.continue_mode }
    fn set_continue_mode(&mut self, mode: ContinueMode) { self.continue_mode = mode; }

    fn serialize(&self) -> Value {
        json!({
            "type": "light",
            "cue_type": "light",
            "id": self.id,
            "number": self.number,
            "name": self.name,
            "notes": self.notes,
            "color": self.color,
            "pre_wait_ms": self.pre_wait.as_millis() as u64,
            "post_wait_ms": self.post_wait.as_millis() as u64,
            "continue_mode": self.continue_mode,
            "targets": self.targets,
            "fade": self.fade,
            "is_disabled": self.is_disabled,
        })
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Deserialise one target, upgrading the pre-group flat form
/// `{ fixture_id, param_index, value }` (no `kind`) to a `Fixture` target.
fn deserialize_target(v: &Value) -> Option<ParamTarget> {
    if v.get("kind").is_some() {
        return serde_json::from_value::<ParamTarget>(v.clone()).ok();
    }
    let fixture_id = v.get("fixture_id")?.as_str()?.to_string();
    let param_index = v.get("param_index")?.as_u64()? as usize;
    let value = v.get("value").and_then(|x| x.as_f64()).unwrap_or(0.0);
    Some(ParamTarget::Fixture { fixture_id, param_index, value })
}

/// Factory for [`LightCue`].
pub struct LightCueFactory;

impl CueFactory for LightCueFactory {
    fn create(&self) -> Box<dyn Cue> {
        Box::new(LightCue::new())
    }

    fn from_json(&self, value: Value) -> anyhow::Result<Box<dyn Cue>> {
        let mut cue = LightCue::new();

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
        if let Some(serde_json::Value::Array(arr)) = value.get("targets") {
            for item in arr {
                match deserialize_target(item) {
                    Some(t) => cue.targets.push(t),
                    None => log::warn!("LightCue: skipping invalid target in JSON"),
                }
            }
        }
        if let Some(fade) = value.get("fade") {
            if let Ok(spec) = serde_json::from_value::<FadeSpec>(fade.clone()) {
                cue.fade = spec;
            }
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
    fn light_cue_serialize_roundtrip() {
        let mut cue = LightCue::new();
        cue.set_name("Look 1".to_string());
        cue.set_number(Some("10".to_string()));
        cue.fade = FadeSpec { duration_ms: 1500, curve: FadeCurve::Exponential };
        cue.targets.push(ParamTarget::Fixture {
            fixture_id: Uuid::nil().to_string(),
            param_index: 2,
            value: 0.75,
        });

        let json = cue.serialize();
        let back = LightCueFactory.from_json(json).unwrap();

        assert_eq!(back.name(), "Look 1");
        assert_eq!(back.number(), Some("10"));
        assert_eq!(back.cue_type(), CueType::Light);
        assert_eq!(back.duration(), Some(Duration::from_millis(1500)));

        let reserialized = back.serialize();
        let targets = reserialized.get("targets").and_then(|v| v.as_array()).unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0]["kind"], "fixture");
        assert_eq!(targets[0]["param_index"], 2);
    }

    #[test]
    fn duration_tracks_fade_time() {
        let mut cue = LightCue::new();
        cue.fade = FadeSpec::new(0);
        assert_eq!(cue.duration(), Some(Duration::ZERO));
    }

    #[test]
    fn unconfigured_target_survives_roundtrip() {
        // A target whose fixture has not been picked yet (empty id) must not
        // poison deserialisation of the whole list (regression).
        let mut cue = LightCue::new();
        cue.targets.push(ParamTarget::Fixture { fixture_id: String::new(), param_index: 0, value: 1.0 });
        cue.targets.push(ParamTarget::Fixture {
            fixture_id: Uuid::new_v4().to_string(),
            param_index: 1,
            value: 0.5,
        });

        let back = LightCueFactory.from_json(cue.serialize()).unwrap();
        let targets = back.serialize();
        let arr = targets.get("targets").and_then(|v| v.as_array()).unwrap();
        assert_eq!(arr.len(), 2, "both targets must survive, including the empty one");
    }

    #[test]
    fn group_target_roundtrip() {
        let mut cue = LightCue::new();
        cue.targets.push(ParamTarget::Group {
            group_id: Uuid::new_v4().to_string(),
            param_kind: ParamKind::Red,
            value: 0.8,
        });
        let back = LightCueFactory.from_json(cue.serialize()).unwrap();
        let arr = back.serialize();
        let targets = arr.get("targets").and_then(|v| v.as_array()).unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0]["kind"], "group");
        assert_eq!(targets[0]["param_kind"], "red");
        assert_eq!(targets[0]["value"], 0.8);
    }

    #[test]
    fn legacy_flat_target_upgrades_to_fixture() {
        // A pre-group cue (flat target, no "kind") must still load.
        let json = json!({
            "type": "light",
            "id": Uuid::new_v4().to_string(),
            "name": "Old",
            "fade": { "duration_ms": 1000, "curve": "s_curve" },
            "targets": [ { "fixture_id": Uuid::nil().to_string(), "param_index": 1, "value": 0.5 } ],
        });
        let back = LightCueFactory.from_json(json).unwrap();
        let targets = back.serialize();
        let arr = targets.get("targets").and_then(|v| v.as_array()).unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["kind"], "fixture");
        assert_eq!(arr[0]["param_index"], 1);
    }
}
