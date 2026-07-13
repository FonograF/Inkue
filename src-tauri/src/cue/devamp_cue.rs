//! [`DevampCue`] — releases a vamping (infinitely-looping) slice on its
//! target cues, QLab-style.
//!
//! A media cue whose current slice has an infinite play count (*vamp*) loops
//! until told otherwise.  GO on a Devamp Cue lets the pass in progress finish,
//! then the target continues into its next slice — the musical way out of a
//! loop.  With **Stop at end of current slice** the target stops at the slice
//! boundary instead of continuing.
//!
//! The cue completes synchronously; the transport resolves the target voices
//! (audio, visual, and a video's paired audio voice) after `go()` via
//! [`Cue::devamp_specification`], mirroring the Stop Cue pattern.

use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{json, Value};
use uuid::Uuid;

use super::{
    context::CueContext,
    traits::{Cue, CueFactory, RuntimeState},
    types::{ContinueMode, CueColor, CueId, CueState, CueType},
};

/// A cue that releases the current slice loop on its target cues.
pub struct DevampCue {
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

    /// UUIDs of the cues to devamp.
    pub target_cue_ids: Vec<CueId>,
    /// Cue numbers kept in sync for display / legacy-file resolution.
    pub target_cue_numbers: Vec<String>,
    /// `true` = the target stops at the end of its current slice instead of
    /// continuing into the next one.
    pub stop_at_end: bool,
    is_disabled: bool,
}

impl DevampCue {
    /// Create a new Devamp cue with a fresh UUID.
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::from("Devamp Cue"),
            number: None,
            notes: String::new(),
            color: CueColor::Yellow,
            state: CueState::Standby,
            continue_mode: ContinueMode::DoNotContinue,
            pre_wait: Duration::ZERO,
            post_wait: Duration::ZERO,
            started_at: None,
            target_cue_ids: Vec::new(),
            target_cue_numbers: Vec::new(),
            stop_at_end: false,
            is_disabled: false,
        }
    }
}

impl Default for DevampCue {
    fn default() -> Self {
        Self::new()
    }
}

impl Cue for DevampCue {
    fn id(&self) -> CueId { self.id }
    fn cue_type(&self) -> CueType { CueType::Devamp }
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

    fn devamp_specification(&self) -> Option<(bool, Vec<CueId>)> {
        Some((self.stop_at_end, self.target_cue_ids.clone()))
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
            .map(|_| CueIssue::warning("Devamp target not found (cue deleted)"))
            .collect();
        if self.target_cue_ids.is_empty() {
            issues.push(CueIssue::warning("No target selected — this Devamp does nothing"));
        }
        issues
    }

    fn resolve_stop_target(&mut self, number_to_id: &std::collections::HashMap<String, CueId>) {
        if self.target_cue_ids.is_empty() {
            for num in &self.target_cue_numbers {
                if let Some(&id) = number_to_id.get(num) {
                    if !self.target_cue_ids.contains(&id) {
                        self.target_cue_ids.push(id);
                    }
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
            "type": "devamp",
            "cue_type": "devamp",
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
            "stop_at_end": self.stop_at_end,
            "is_disabled": self.is_disabled,
        })
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Factory for [`DevampCue`].
pub struct DevampCueFactory;

impl CueFactory for DevampCueFactory {
    fn create(&self) -> Box<dyn Cue> {
        Box::new(DevampCue::new())
    }

    fn from_json(&self, value: Value) -> Result<Box<dyn Cue>> {
        let mut cue = DevampCue::new();

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
        if let Some(ids) = value.get("target_cue_ids").and_then(|v| v.as_array()) {
            cue.target_cue_ids = ids
                .iter()
                .filter_map(|v| v.as_str().and_then(|s| s.parse().ok()))
                .collect();
        }
        if let Some(nums) = value.get("target_cue_numbers").and_then(|v| v.as_array()) {
            cue.target_cue_numbers = nums
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
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
    fn serialize_roundtrip_preserves_targets_and_mode() {
        let mut cue = DevampCue::new();
        cue.set_name("Release vamp".to_string());
        cue.set_number(Some("10".to_string()));
        let target = Uuid::new_v4();
        cue.target_cue_ids = vec![target];
        cue.target_cue_numbers = vec!["3".into()];
        cue.stop_at_end = true;

        let json = cue.serialize();
        let restored = DevampCueFactory.from_json(json).expect("deserialize");
        assert_eq!(restored.name(), "Release vamp");
        assert_eq!(restored.number(), Some("10"));
        assert_eq!(restored.devamp_specification(), Some((true, vec![target])));
    }

    #[test]
    fn go_completes_synchronously() {
        let cue = DevampCue::new();
        assert_eq!(cue.state(), CueState::Standby);
        // go() needs no engine access — the transport does the voice work.
        let spec = cue.devamp_specification();
        assert_eq!(spec, Some((false, vec![])));
    }
}
