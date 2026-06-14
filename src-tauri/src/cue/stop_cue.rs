//! [`StopCue`] — stops one or all running cues when triggered.
//!
//! The Stop Cue can target:
//! - **All cues** (default) — equivalent to pressing Stop All.
//! - **A specific cue number** — only that cue is stopped.
//!
//! It also supports two stop modes:
//! - **Soft** (default) — applies the workspace's default fade-out.
//! - **Hard** — immediate cut, no fade.
//!
//! The cue completes synchronously; Auto-Follow / Auto-Continue chain
//! *after* the stop action executes (transport handles this ordering).

use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{json, Value};
use uuid::Uuid;

use super::{
    context::CueContext,
    traits::{Cue, CueFactory, RuntimeState},
    types::{ContinueMode, CueColor, CueId, CueState, CueType},
};

/// A cue that stops one or all running cues when triggered.
pub struct StopCue {
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

    /// UUID of the specific cue to stop, or `None` to stop all running cues.
    /// This is the primary key used at runtime.  Stable across cue renumbering
    /// and insertion of new cues before the target.
    pub target_cue_id: Option<CueId>,
    /// Human-readable cue number stored alongside the UUID for display in the
    /// inspector.  Also used as a fallback during load of old workspace files
    /// that pre-date UUID-based targeting (resolved to `target_cue_id` by
    /// `resolve_stop_target` after the full cue list is loaded).
    pub target_cue_number: Option<String>,
    /// `true` = immediate cut; `false` = soft fade using the workspace default.
    pub hard_stop_mode: bool,
    is_disabled: bool,
}

impl StopCue {
    /// Create a new Stop cue with a fresh UUID.
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::from("Stop Cue"),
            number: None,
            notes: String::new(),
            color: CueColor::Red,
            state: CueState::Standby,
            continue_mode: ContinueMode::DoNotContinue,
            pre_wait: Duration::ZERO,
            post_wait: Duration::ZERO,
            started_at: None,
            target_cue_id: None,
            target_cue_number: None,
            hard_stop_mode: false,
            is_disabled: false,
        }
    }
}

impl Default for StopCue {
    fn default() -> Self {
        Self::new()
    }
}

impl Cue for StopCue {
    fn id(&self) -> CueId { self.id }
    fn cue_type(&self) -> CueType { CueType::Stop }
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
        self.state = CueState::Completed;
        self.started_at = Some(Instant::now());
        Ok(())
    }

    fn stop(&mut self, _context: &CueContext) -> Result<()> {
        self.state = CueState::Standby;
        self.started_at = None;
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

    fn stop_specification(&self) -> Option<(bool, Option<CueId>)> {
        Some((self.hard_stop_mode, self.target_cue_id))
    }

    fn resolve_stop_target(&mut self, number_to_id: &std::collections::HashMap<String, CueId>) {
        if self.target_cue_id.is_none() {
            if let Some(num) = &self.target_cue_number {
                if let Some(&id) = number_to_id.get(num) {
                    self.target_cue_id = Some(id);
                }
            }
        }
    }

    fn runtime_state(&self) -> RuntimeState {
        RuntimeState {
            state: self.state,
            voice_id: None,
            started_at: self.started_at,
            action_started_at: self.started_at,
        }
    }

    fn restore_runtime_state(&mut self, snap: RuntimeState) {
        self.state = snap.state;
        self.started_at = snap.started_at;
    }

    fn serialize(&self) -> Value {
        json!({
            "type": "stop",
            "cue_type": "stop",
            "id": self.id,
            "number": self.number,
            "name": self.name,
            "notes": self.notes,
            "color": self.color,
            "pre_wait_ms": self.pre_wait.as_millis() as u64,
            "post_wait_ms": self.post_wait.as_millis() as u64,
            "continue_mode": self.continue_mode,
            "target_cue_id": self.target_cue_id,
            "target_cue_number": self.target_cue_number,
            "hard_stop_mode": self.hard_stop_mode,
            "is_disabled": self.is_disabled,
        })
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Factory for [`StopCue`].
pub struct StopCueFactory;

impl CueFactory for StopCueFactory {
    fn create(&self) -> Box<dyn Cue> {
        Box::new(StopCue::new())
    }

    fn from_json(&self, value: Value) -> anyhow::Result<Box<dyn Cue>> {
        let mut cue = StopCue::new();

        if let Some(id_str) = value.get("id").and_then(|v| v.as_str()) {
            cue.id = id_str.parse().unwrap_or_else(|_| Uuid::new_v4());
        }
        if let Some(name) = value.get("name").and_then(|v| v.as_str()) {
            cue.name = name.to_string();
        }
        if let Some(num) = value.get("number").and_then(|v| v.as_str()) {
            cue.number = Some(num.to_string());
        }
        if let Some(notes) = value.get("notes").and_then(|v| v.as_str()) {
            cue.notes = notes.to_string();
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
        if let Some(id_str) = value.get("target_cue_id").and_then(|v| v.as_str()) {
            cue.target_cue_id = id_str.parse().ok();
        }
        if let Some(target) = value.get("target_cue_number").and_then(|v| v.as_str()) {
            cue.target_cue_number = Some(target.to_string());
        }
        if let Some(hard) = value.get("hard_stop_mode").and_then(|v| v.as_bool()) {
            cue.hard_stop_mode = hard;
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
    fn default_targets_all_with_soft_stop() {
        let cue = StopCue::new();
        let spec = cue.stop_specification().unwrap();
        assert!(!spec.0, "hard_stop_mode should default to false");
        assert!(spec.1.is_none(), "target should default to None (all)");
    }

    #[test]
    fn target_cue_id_roundtrips() {
        let factory = StopCueFactory;
        let mut cue = StopCue::new();
        let target_id = Uuid::new_v4();
        cue.target_cue_id = Some(target_id);
        cue.target_cue_number = Some("5".to_string());
        cue.hard_stop_mode = true;

        let json = cue.serialize();
        let rebuilt = factory.from_json(json).unwrap();

        let spec = rebuilt.stop_specification().unwrap();
        assert!(spec.0);
        assert_eq!(spec.1, Some(target_id));
    }

    #[test]
    fn resolve_stop_target_from_number() {
        let mut cue = StopCue::new();
        cue.target_cue_number = Some("5".to_string());

        let target_id = Uuid::new_v4();
        let mut map = std::collections::HashMap::new();
        map.insert("5".to_string(), target_id);

        cue.resolve_stop_target(&map);
        assert_eq!(cue.target_cue_id, Some(target_id));
    }

    #[test]
    fn go_sets_completed_state() {
        let cue = StopCue::new();
        assert_eq!(cue.state(), CueState::Standby);
    }

    #[test]
    fn cue_type_is_stop() {
        assert_eq!(StopCue::new().cue_type(), CueType::Stop);
    }

    #[test]
    fn serialize_roundtrip() {
        let factory = StopCueFactory;
        let mut cue = StopCue::new();
        cue.set_name("My Stop".to_string());
        cue.target_cue_number = Some("3".to_string());

        let json = cue.serialize();
        assert_eq!(json["name"], "My Stop");
        assert_eq!(json["target_cue_number"], "3");
        assert_eq!(json["hard_stop_mode"], false);

        let rebuilt = factory.from_json(json).unwrap();
        assert_eq!(rebuilt.name(), "My Stop");
    }
}
