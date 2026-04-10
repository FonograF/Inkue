//! [`MemoCue`] — a text-only cue with no audio or timing action.
//!
//! Equivalent to QLab's Memo cue.  It proves that new cue types can be added
//! without touching the transport, cue list, or UI code.

use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{json, Value};
use uuid::Uuid;

use super::{
    context::CueContext,
    traits::{Cue, CueFactory},
    types::{ContinueMode, CueColor, CueId, CueState, CueType},
};

/// A cue that carries a text message and performs no action when triggered.
/// The operator uses it as an in-list reminder or stage direction.
pub struct MemoCue {
    id: CueId,
    name: String,
    number: Option<String>,
    notes: String,
    color: CueColor,
    state: CueState,
    continue_mode: ContinueMode,
    pre_wait: Duration,
    post_wait: Duration,
    /// Text displayed in the cue list target column.
    pub memo_text: String,
    /// Timestamp when `go()` was last called (for elapsed tracking).
    started_at: Option<Instant>,
}

impl MemoCue {
    /// Create a new, empty Memo cue with a fresh UUID.
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::from("Memo"),
            number: None,
            notes: String::new(),
            color: CueColor::None,
            state: CueState::Standby,
            continue_mode: ContinueMode::DoNotContinue,
            pre_wait: Duration::ZERO,
            post_wait: Duration::ZERO,
            memo_text: String::new(),
            started_at: None,
        }
    }
}

impl Default for MemoCue {
    fn default() -> Self {
        Self::new()
    }
}

impl Cue for MemoCue {
    fn id(&self) -> CueId {
        self.id
    }

    fn cue_type(&self) -> CueType {
        CueType::Memo
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn set_name(&mut self, name: String) {
        self.name = name;
    }

    fn number(&self) -> Option<&str> {
        self.number.as_deref()
    }

    fn set_number(&mut self, number: Option<String>) {
        self.number = number;
    }

    fn notes(&self) -> &str {
        &self.notes
    }

    fn set_notes(&mut self, notes: String) {
        self.notes = notes;
    }

    fn color(&self) -> CueColor {
        self.color
    }

    fn set_color(&mut self, color: CueColor) {
        self.color = color;
    }

    fn state(&self) -> CueState {
        self.state
    }

    fn load(&mut self, _context: &CueContext) -> Result<()> {
        // Nothing to load for a memo.
        Ok(())
    }

    fn go(&mut self, context: &CueContext) -> Result<()> {
        self.state = CueState::Running;
        self.started_at = Some(Instant::now());
        context.emit(super::context::CueEvent::ActionStarted { cue_id: self.id });
        // A memo has no action; complete immediately.
        self.state = CueState::Completed;
        context.emit(super::context::CueEvent::ActionCompleted { cue_id: self.id });
        Ok(())
    }

    fn stop(&mut self, context: &CueContext) -> Result<()> {
        self.state = CueState::Standby;
        self.started_at = None;
        context.emit(super::context::CueEvent::Stopped { cue_id: self.id });
        Ok(())
    }

    fn pause(&mut self, _context: &CueContext) -> Result<()> {
        // Memo completes instantly; pause is a no-op.
        Ok(())
    }

    fn resume(&mut self, _context: &CueContext) -> Result<()> {
        Ok(())
    }

    fn hard_stop(&mut self, context: &CueContext) -> Result<()> {
        self.stop(context)
    }

    fn reset(&mut self) -> Result<()> {
        self.state = CueState::Standby;
        self.started_at = None;
        Ok(())
    }

    fn pre_wait(&self) -> Duration {
        self.pre_wait
    }

    fn set_pre_wait(&mut self, d: Duration) {
        self.pre_wait = d;
    }

    fn post_wait(&self) -> Duration {
        self.post_wait
    }

    fn set_post_wait(&mut self, d: Duration) {
        self.post_wait = d;
    }

    /// Memo has no meaningful duration.
    fn duration(&self) -> Option<Duration> {
        None
    }

    fn elapsed(&self) -> Duration {
        self.started_at
            .map(|t| t.elapsed())
            .unwrap_or(Duration::ZERO)
    }

    fn action_elapsed(&self) -> Duration {
        // Memo has no pre-wait distinction in practice.
        self.elapsed()
    }

    fn continue_mode(&self) -> ContinueMode {
        self.continue_mode
    }

    fn set_continue_mode(&mut self, mode: ContinueMode) {
        self.continue_mode = mode;
    }

    fn serialize(&self) -> Value {
        json!({
            "type": "memo",
            "cue_type": "memo",
            "id": self.id,
            "number": self.number,
            "name": self.name,
            "notes": self.notes,
            "color": self.color,
            "pre_wait_ms": self.pre_wait.as_millis() as u64,
            "post_wait_ms": self.post_wait.as_millis() as u64,
            "continue_mode": self.continue_mode,
            "memo_text": self.memo_text,
        })
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Factory for [`MemoCue`].  Register this in [`super::registry::CueRegistry`].
pub struct MemoCueFactory;

impl CueFactory for MemoCueFactory {
    fn create(&self) -> Box<dyn Cue> {
        Box::new(MemoCue::new())
    }

    fn from_json(&self, value: Value) -> anyhow::Result<Box<dyn Cue>> {
        let mut cue = MemoCue::new();

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
        if let Some(text) = value.get("memo_text").and_then(|v| v.as_str()) {
            cue.memo_text = text.to_string();
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

        Ok(Box::new(cue))
    }
}
