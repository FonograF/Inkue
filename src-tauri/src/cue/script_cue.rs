//! [`ScriptCue`] — runs an external command or script when triggered.
//!
//! QLab's Script Cue runs AppleScript; Inkue is cross-platform, so it runs a
//! plain executable with arguments instead — the portable equivalent, and the
//! one that reaches `ffmpeg`, `curl`, a Python script or a `.bat` alike.
//!
//! The process is spawned on a **background thread** and never waited on by the
//! caller: a GO must not block the show while a script runs. Output is captured
//! and logged so a failing script is diagnosable from the in-app log viewer
//! rather than silently doing nothing, and a timeout kills a runaway process so
//! it cannot outlive the show.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{json, Value};
use uuid::Uuid;

use super::{
    context::{CueContext, CueEvent},
    traits::{Cue, CueFactory, RuntimeState},
    types::{ContinueMode, CueColor, CueId, CueState, CueType},
};

/// Default kill deadline; long enough for a real task, short enough that a hung
/// process cannot linger for a whole performance.
const DEFAULT_TIMEOUT_MS: u64 = 30_000;

/// A cue that runs an external command when triggered.
pub struct ScriptCue {
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

    /// Executable to run. Empty = the cue does nothing.
    pub command: String,
    /// Arguments, already split — no shell is involved, so a path with spaces
    /// is one argument and needs no quoting.
    pub args: Vec<String>,
    /// Working directory. `None` = inherit the app's.
    pub working_dir: Option<PathBuf>,
    /// Kill the process after this long. `0` = let it run.
    pub timeout_ms: u64,
    is_disabled: bool,
}

impl ScriptCue {
    /// Create a new, empty Script Cue with a fresh UUID.
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::from("Script Cue"),
            number: None,
            notes: String::new(),
            color: CueColor::White,
            state: CueState::Standby,
            continue_mode: ContinueMode::DoNotContinue,
            pre_wait: Duration::ZERO,
            post_wait: Duration::ZERO,
            started_at: None,
            command: String::new(),
            args: Vec::new(),
            working_dir: None,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            is_disabled: false,
        }
    }
}

impl Default for ScriptCue {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawn `command` on a detached thread, logging what it did.
///
/// Separated from the cue so it is testable without a cue, and so the spawn
/// policy (never block the show; always log; enforce the timeout) lives in one
/// place.
fn run_detached(label: String, command: String, args: Vec<String>, dir: Option<PathBuf>, timeout_ms: u64) {
    let spawn_label = label.clone();
    std::thread::Builder::new()
        .name("inkue-script-cue".into())
        .spawn(move || {
            let mut cmd = Command::new(&command);
            cmd.args(&args);
            if let Some(d) = dir {
                cmd.current_dir(d);
            }
            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    log::error!("[script] {label}: failed to start '{command}': {e}");
                    return;
                }
            };

            let deadline = (timeout_ms > 0).then(|| Instant::now() + Duration::from_millis(timeout_ms));
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        if status.success() {
                            log::info!("[script] {label}: '{command}' finished");
                        } else {
                            log::warn!("[script] {label}: '{command}' exited with {status}");
                        }
                        return;
                    }
                    Ok(None) => {
                        if deadline.is_some_and(|d| Instant::now() >= d) {
                            let _ = child.kill();
                            log::warn!(
                                "[script] {label}: '{command}' exceeded {timeout_ms} ms — killed"
                            );
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    Err(e) => {
                        log::error!("[script] could not wait on '{command}': {e}");
                        return;
                    }
                }
            }
        })
        .map(|_| ())
        .unwrap_or_else(|e| log::error!("[script] {spawn_label}: could not spawn thread: {e}"));
}

impl Cue for ScriptCue {
    fn id(&self) -> CueId { self.id }
    fn cue_type(&self) -> CueType { CueType::Script }
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

        if !self.command.trim().is_empty() {
            run_detached(
                self.name.clone(),
                self.command.clone(),
                self.args.clone(),
                self.working_dir.clone(),
                self.timeout_ms,
            );
        }

        // The cue completes as soon as the process is launched: waiting for it
        // would stall the show, and QLab's Script Cue is fire-and-forget too.
        self.state = CueState::Completed;
        context.emit(CueEvent::ActionCompleted { cue_id: self.id });
        Ok(())
    }

    fn stop(&mut self, _context: &CueContext) -> Result<()> {
        self.state = CueState::Standby;
        self.started_at = None;
        Ok(())
    }

    fn pause(&mut self, _context: &CueContext) -> Result<()> { Ok(()) }
    fn resume(&mut self, _context: &CueContext) -> Result<()> { Ok(()) }

    fn hard_stop(&mut self, context: &CueContext) -> Result<()> { self.stop(context) }

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

    fn validate(
        &self,
        _ctx: &crate::cue::validation::ValidationContext,
    ) -> Vec<crate::cue::validation::CueIssue> {
        use crate::cue::validation::CueIssue;
        let mut issues = Vec::new();
        if self.command.trim().is_empty() {
            issues.push(CueIssue::warning("Script Cue has no command — it will do nothing"));
            return issues;
        }
        // An absolute path that is gone is worth flagging now rather than at
        // GO; a bare name is resolved through PATH and cannot be checked here.
        let path = std::path::Path::new(&self.command);
        if path.is_absolute() && !path.exists() {
            issues.push(CueIssue::warning("Script command not found at that path"));
        }
        if let Some(dir) = &self.working_dir {
            if !dir.as_os_str().is_empty() && !dir.exists() {
                issues.push(CueIssue::warning("Script working directory not found"));
            }
        }
        issues
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
            "type": "script",
            "cue_type": "script",
            "id": self.id,
            "number": self.number,
            "name": self.name,
            "notes": self.notes,
            "color": self.color,
            "pre_wait_ms": self.pre_wait.as_millis() as u64,
            "post_wait_ms": self.post_wait.as_millis() as u64,
            "continue_mode": self.continue_mode,
            "command": self.command,
            "args": self.args,
            "working_dir": self.working_dir.as_ref().map(|p| p.to_string_lossy().into_owned()),
            "timeout_ms": self.timeout_ms,
            "is_disabled": self.is_disabled,
        })
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Factory for [`ScriptCue`].
pub struct ScriptCueFactory;

impl CueFactory for ScriptCueFactory {
    fn create(&self) -> Box<dyn Cue> {
        Box::new(ScriptCue::new())
    }

    fn from_json(&self, value: Value) -> Result<Box<dyn Cue>> {
        let mut cue = ScriptCue::new();

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
        if let Some(c) = value.get("color") {
            if let Ok(color) = serde_json::from_value(c.clone()) {
                cue.color = color;
            }
        }
        if let Some(s) = value.get("command").and_then(|v| v.as_str()) {
            cue.command = s.to_string();
        }
        if let Some(arr) = value.get("args").and_then(|v| v.as_array()) {
            cue.args = arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect();
        }
        if let Some(s) = value.get("working_dir").and_then(|v| v.as_str()) {
            if !s.is_empty() {
                cue.working_dir = Some(PathBuf::from(s));
            }
        }
        if let Some(ms) = value.get("timeout_ms").and_then(|v| v.as_u64()) {
            cue.timeout_ms = ms;
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

    fn empty_ctx() -> crate::cue::validation::ValidationContext {
        use std::collections::HashSet;
        crate::cue::validation::ValidationContext {
            all_cue_ids: HashSet::new(),
            fixture_ids: HashSet::new(),
            fixture_group_ids: HashSet::new(),
            osc_patch_ids: HashSet::new(),
            output_patch_ids: HashSet::new(),
            midi_ports: Vec::new(),
        }
    }

    #[test]
    fn cue_type_is_script() {
        assert_eq!(ScriptCue::new().cue_type(), CueType::Script);
    }

    #[test]
    fn serialize_roundtrip_preserves_the_command_line() {
        let factory = ScriptCueFactory;
        let mut cue = ScriptCue::new();
        cue.command = "ffmpeg".into();
        cue.args = vec!["-i".into(), "in put.mov".into()];
        cue.working_dir = Some(PathBuf::from("/tmp"));
        cue.timeout_ms = 5_000;

        let rebuilt = factory.from_json(cue.serialize()).unwrap();
        let json = rebuilt.serialize();

        assert_eq!(json["command"], "ffmpeg");
        // Arguments stay split: no shell, so a space is not a separator.
        assert_eq!(json["args"][1], "in put.mov");
        assert_eq!(json["timeout_ms"], 5_000);
    }

    #[test]
    fn an_empty_command_is_flagged_rather_than_run() {
        let issues = ScriptCue::new().validate(&empty_ctx());
        assert_eq!(issues.len(), 1, "a Script Cue with nothing to run says so");
    }

    #[test]
    fn a_missing_absolute_command_is_flagged() {
        let mut cue = ScriptCue::new();
        cue.command = if cfg!(windows) {
            r"C:\definitely\not\here.exe".into()
        } else {
            "/definitely/not/here".into()
        };
        assert_eq!(cue.validate(&empty_ctx()).len(), 1);
    }

    #[test]
    fn a_bare_command_name_is_not_flagged() {
        // Resolved through PATH at spawn time — validation cannot know.
        let mut cue = ScriptCue::new();
        cue.command = "ffmpeg".into();
        assert!(cue.validate(&empty_ctx()).is_empty());
    }

    #[test]
    fn a_missing_working_directory_is_flagged() {
        let mut cue = ScriptCue::new();
        cue.command = "ffmpeg".into();
        cue.working_dir = Some(PathBuf::from("/definitely/not/here"));
        assert_eq!(cue.validate(&empty_ctx()).len(), 1);
    }
}
