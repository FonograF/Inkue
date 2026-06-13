//! [`WaitCue`] — pauses the cue sequence for a configurable duration.
//!
//! When triggered, the Wait cue starts a countdown timer and stays in
//! [`CueState::Running`] until the timer expires.  The 30 fps event loop
//! detects completion via [`Cue::duration`] / [`Cue::action_elapsed`] — no
//! engine interaction required.
//!
//! Pause and resume are supported: the timer freezes on pause and resumes
//! exactly where it left off.

use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{json, Value};
use uuid::Uuid;

use super::{
    context::{CueContext, CueEvent},
    traits::{Cue, CueFactory},
    types::{ContinueMode, CueColor, CueId, CueState, CueType},
};

/// A cue that waits for a configurable duration before the sequence continues.
pub struct WaitCue {
    id: CueId,
    name: String,
    number: Option<String>,
    notes: String,
    color: CueColor,
    state: CueState,
    continue_mode: ContinueMode,
    pre_wait: Duration,
    post_wait: Duration,
    /// The user-configured wait duration.
    wait_duration: Duration,
    /// Elapsed time accumulated before the most recent pause.
    elapsed_before_pause: Duration,
    is_disabled: bool,
    /// Wall-clock instant when `go()` or `resume()` was last called.
    started_at: Option<Instant>,
}

impl WaitCue {
    /// Create a new Wait cue with a 5-second default duration.
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::from("Wait"),
            number: None,
            notes: String::new(),
            color: CueColor::Orange,
            state: CueState::Standby,
            continue_mode: ContinueMode::AutoFollow,
            pre_wait: Duration::ZERO,
            post_wait: Duration::ZERO,
            wait_duration: Duration::from_secs(5),
            elapsed_before_pause: Duration::ZERO,
            is_disabled: false,
            started_at: None,
        }
    }
}

impl Default for WaitCue {
    fn default() -> Self {
        Self::new()
    }
}

impl Cue for WaitCue {
    fn id(&self) -> CueId { self.id }

    fn cue_type(&self) -> CueType { CueType::Wait }

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

    fn load(&mut self, _context: &CueContext) -> Result<()> {
        Ok(())
    }

    fn go(&mut self, context: &CueContext) -> Result<()> {
        self.elapsed_before_pause = Duration::ZERO;
        self.started_at = Some(Instant::now());
        self.state = CueState::Running;
        context.emit(CueEvent::ActionStarted { cue_id: self.id });
        Ok(())
    }

    fn stop(&mut self, context: &CueContext) -> Result<()> {
        self.elapsed_before_pause = Duration::ZERO;
        self.started_at = None;
        self.state = CueState::Standby;
        context.emit(CueEvent::Stopped { cue_id: self.id });
        Ok(())
    }

    fn pause(&mut self, _context: &CueContext) -> Result<()> {
        if self.state == CueState::Running {
            if let Some(t) = self.started_at.take() {
                self.elapsed_before_pause += t.elapsed();
            }
            self.state = CueState::Paused;
        }
        Ok(())
    }

    fn resume(&mut self, _context: &CueContext) -> Result<()> {
        if self.state == CueState::Paused {
            self.started_at = Some(Instant::now());
            self.state = CueState::Running;
        }
        Ok(())
    }

    fn hard_stop(&mut self, context: &CueContext) -> Result<()> {
        self.stop(context)
    }

    fn reset(&mut self) -> Result<()> {
        self.elapsed_before_pause = Duration::ZERO;
        self.started_at = None;
        self.state = CueState::Standby;
        Ok(())
    }

    fn pre_wait(&self) -> Duration { self.pre_wait }
    fn set_pre_wait(&mut self, d: Duration) { self.pre_wait = d; }

    fn post_wait(&self) -> Duration { self.post_wait }
    fn set_post_wait(&mut self, d: Duration) { self.post_wait = d; }

    /// The wait duration — used by the event loop for `time_done` detection.
    fn duration(&self) -> Option<Duration> {
        Some(self.wait_duration)
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
        self.elapsed()
    }

    fn continue_mode(&self) -> ContinueMode { self.continue_mode }
    fn set_continue_mode(&mut self, mode: ContinueMode) { self.continue_mode = mode; }

    fn serialize(&self) -> Value {
        json!({
            "type": "wait",
            "cue_type": "wait",
            "id": self.id,
            "number": self.number,
            "name": self.name,
            "notes": self.notes,
            "color": self.color,
            "pre_wait_ms": self.pre_wait.as_millis() as u64,
            "post_wait_ms": self.post_wait.as_millis() as u64,
            "continue_mode": self.continue_mode,
            "wait_duration_ms": self.wait_duration.as_millis() as u64,
            "is_disabled": self.is_disabled,
        })
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Factory for [`WaitCue`].
pub struct WaitCueFactory;

impl CueFactory for WaitCueFactory {
    fn create(&self) -> Box<dyn Cue> {
        Box::new(WaitCue::new())
    }

    fn from_json(&self, value: Value) -> anyhow::Result<Box<dyn Cue>> {
        let mut cue = WaitCue::new();

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
        if let Some(ms) = value.get("wait_duration_ms").and_then(|v| v.as_u64()) {
            cue.wait_duration = Duration::from_millis(ms);
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
        if let Some(b) = value.get("is_disabled").and_then(|v| v.as_bool()) {
            cue.is_disabled = b;
        }

        Ok(Box::new(cue))
    }
}
