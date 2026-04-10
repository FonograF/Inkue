//! [`StopCue`] — a cue that stops all running cues when triggered.
//!
//! When GO is pressed on a Stop cue it immediately emits a [`CueEvent::StopAll`]
//! signal, which the transport layer honours by calling `stop_all` on the cue
//! list.  The Stop cue itself completes synchronously with no audio action.

use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{json, Value};
use uuid::Uuid;

use super::{
    context::{CueContext, CueEvent},
    traits::{Cue, CueFactory},
    types::{ContinueMode, CueColor, CueId, CueState, CueType},
};

/// A cue that stops all currently-running cues when triggered.
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
}

impl StopCue {
    /// Create a new Stop cue with a fresh UUID.
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::from("Stop All"),
            number: None,
            notes: String::new(),
            color: CueColor::Red,
            state: CueState::Standby,
            continue_mode: ContinueMode::DoNotContinue,
            pre_wait: Duration::ZERO,
            post_wait: Duration::ZERO,
            started_at: None,
        }
    }
}

impl Default for StopCue {
    fn default() -> Self {
        Self::new()
    }
}

impl Cue for StopCue {
    fn id(&self) -> CueId {
        self.id
    }

    fn cue_type(&self) -> CueType {
        CueType::Stop
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
        Ok(())
    }

    fn go(&mut self, context: &CueContext) -> Result<()> {
        self.state = CueState::Running;
        self.started_at = Some(Instant::now());
        context.emit(CueEvent::ActionStarted { cue_id: self.id });
        // Signal the transport to stop all running cues.
        context.emit(CueEvent::StopAll);
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

    fn pause(&mut self, _context: &CueContext) -> Result<()> {
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

    fn duration(&self) -> Option<Duration> {
        None
    }

    fn elapsed(&self) -> Duration {
        self.started_at
            .map(|t| t.elapsed())
            .unwrap_or(Duration::ZERO)
    }

    fn action_elapsed(&self) -> Duration {
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

        Ok(Box::new(cue))
    }
}
