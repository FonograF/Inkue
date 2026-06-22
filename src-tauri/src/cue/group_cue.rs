//! [`GroupCue`] — contains and fires a list of child cues.
//!
//! ## Modes
//! - **Simultaneous**: all children fire at once; the Group completes when
//!   every child has finished.
//! - **Sequential**: children fire one after another using each child's own
//!   Continue Mode (Auto-Continue, Auto-Follow, Do Not Continue) exactly like
//!   a mini Cue List.

use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use uuid::Uuid;

use super::{
    context::{CueContext, CueEvent},
    registry::CueRegistry,
    traits::{Cue, CueFactory},
    types::{
        ContinueMode, CueColor, CueId, CueState, CueType, GroupMode,
    },
};

// ---------------------------------------------------------------------------
// GroupCue
// ---------------------------------------------------------------------------

/// A cue that contains other cues and fires them simultaneously or sequentially.
pub struct GroupCue {
    // ── Identity ──────────────────────────────────────────────────────────
    pub id: CueId,
    name: String,
    number: Option<String>,
    notes: String,
    color: CueColor,

    // ── State ─────────────────────────────────────────────────────────────
    state: CueState,

    // ── Timing ────────────────────────────────────────────────────────────
    pre_wait: Duration,
    post_wait: Duration,
    started_at: Option<Instant>,
    action_started_at: Option<Instant>,
    in_pre_wait: bool,

    // ── Continue ──────────────────────────────────────────────────────────
    continue_mode: ContinueMode,
    auto_continue_fired: bool,

    is_disabled: bool,

    // ── Group-specific ────────────────────────────────────────────────────
    pub mode: GroupMode,
    /// Direct child cues (any type, including nested Groups).
    pub children: Vec<Box<dyn Cue>>,

    // ── Sequential mode state (not persisted) ─────────────────────────────
    /// ID of the child currently at the internal playhead in Sequential mode.
    seq_current_id: Option<CueId>,
    /// Set when the sequential chain has finished (last child completed with
    /// DoNotContinue, or all children exhausted).
    seq_done: bool,
    /// When Some, we are waiting for this instant before firing the next child
    /// (AutoContinue post-wait).
    seq_post_wait_until: Option<Instant>,
}

impl GroupCue {
    /// Create a new, empty Group with a fresh UUID.
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: "Group".to_string(),
            number: None,
            notes: String::new(),
            color: CueColor::Yellow,
            state: CueState::Standby,
            pre_wait: Duration::ZERO,
            post_wait: Duration::ZERO,
            started_at: None,
            action_started_at: None,
            in_pre_wait: false,
            continue_mode: ContinueMode::DoNotContinue,
            auto_continue_fired: false,
            is_disabled: false,
            mode: GroupMode::Simultaneous,
            children: Vec::new(),
            seq_current_id: None,
            seq_done: false,
            seq_post_wait_until: None,
        }
    }

    /// Deserialise a GroupCue from JSON, using `registry` to reconstruct children.
    pub fn from_json_with_registry(value: &Value, registry: &CueRegistry) -> Result<Box<dyn Cue>> {
        let mut cue = GroupCue::new();

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
        if let Some(gm) = value.get("group_mode") {
            if let Ok(m) = serde_json::from_value(gm.clone()) {
                cue.mode = m;
            }
        }

        // Deserialise children recursively.
        if let Some(arr) = value.get("children").and_then(|v| v.as_array()) {
            for child_val in arr {
                match registry.from_json(child_val.clone()) {
                    Ok(child) => cue.children.push(child),
                    Err(e) => log::warn!("[group] skipping unrecognised child: {e}"),
                }
            }
        }
        if let Some(b) = value.get("is_disabled").and_then(|v| v.as_bool()) {
            cue.is_disabled = b;
        }

        Ok(Box::new(cue))
    }

    // ── Private helpers ───────────────────────────────────────────────────

    fn start_action(&mut self, ctx: &CueContext) -> Result<()> {
        self.in_pre_wait = false;
        self.action_started_at = Some(Instant::now());
        self.state = CueState::Running;
        self.seq_done = false;
        self.seq_post_wait_until = None;

        match self.mode {
            GroupMode::Simultaneous => {
                for child in &mut self.children {
                    if let Err(e) = child.go(ctx) {
                        log::warn!("Group simultaneous: child '{}' failed to start: {e}", child.name());
                    }
                }
            }
            GroupMode::Sequential => {
                // Fire from the inner playhead (set by `set_active_child` when the
                // user parks the Playhead on a specific child); `None` starts from
                // the first child.
                let start_after = self.seq_current_id;
                if let Err(e) = self.fire_next_sequential(ctx, start_after) {
                    log::warn!("Group sequential: first child failed to start: {e}");
                    self.seq_done = true;
                }
            }
        }
        Ok(())
    }

    /// Fire the next sequential child after `after_id` (or the first child if
    /// `after_id` is `None`).  Handles Auto-Follow chaining recursively.
    fn fire_next_sequential(&mut self, ctx: &CueContext, after_id: Option<CueId>) -> Result<()> {
        let next_idx = match after_id {
            None => 0,
            Some(prev_id) => {
                match self.children.iter().position(|c| c.id() == prev_id) {
                    Some(i) => i + 1,
                    None => return Ok(()),
                }
            }
        };

        if next_idx >= self.children.len() {
            self.seq_done = true;
            return Ok(());
        }

        let child_id = self.children[next_idx].id();
        self.seq_current_id = Some(child_id);
        if let Err(e) = self.children[next_idx].go(ctx) {
            log::warn!("Group sequential: child '{}' failed to start: {e}", self.children[next_idx].name());
            // Child rolled back to Standby — treat it as done and advance.
            self.seq_done = true;
            return Ok(());
        }

        // Auto-Follow: fire the child after this one immediately when this one starts.
        if self.children[next_idx].is_action_started()
            && self.children[next_idx].continue_mode() == ContinueMode::AutoFollow
        {
            // Mark this child's Auto-Follow as fired so the event loop does not
            // double-chain on the main list level.
            self.children[next_idx].mark_auto_continue_fired();
            let fired_id = child_id;
            self.fire_next_sequential(ctx, Some(fired_id))?;
        }

        Ok(())
    }

    /// Tick a child at `idx` and return whether it is now complete.
    fn tick_child_at(&mut self, idx: usize, ctx: &CueContext) -> Result<bool> {
        let child = &mut self.children[idx];

        if child.state() == CueState::Running {
            child.tick(ctx)?;
        }

        let done = matches!(child.state(), CueState::Completed | CueState::Standby)
            || child
                .duration()
                .map(|d| child.action_elapsed() >= d)
                .unwrap_or(false);

        Ok(done)
    }

    /// `true` when the sequential sequence has paused mid-way (current child
    /// completed with `DoNotContinue`) AND there are more children left to
    /// fire.  Used by [`absorbs_go`](crate::cue::traits::Cue::absorbs_go).
    fn has_next_sequential_child(&self) -> bool {
        match self.seq_current_id {
            Some(current_id) => self
                .children
                .iter()
                .position(|c| c.id() == current_id)
                .map(|i| i + 1 < self.children.len())
                .unwrap_or(false),
            None => !self.children.is_empty(),
        }
    }

    /// Reset all children to Standby and clear sequential state.
    fn reset_children(&mut self) {
        for child in &mut self.children {
            let _ = child.reset();
        }
        self.seq_current_id = None;
        self.seq_done = false;
        self.seq_post_wait_until = None;
    }
}

impl Default for GroupCue {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Cue trait implementation
// ---------------------------------------------------------------------------

impl Cue for GroupCue {
    // ── Identity ──────────────────────────────────────────────────────────

    fn id(&self) -> CueId { self.id }
    fn cue_type(&self) -> CueType { CueType::Group }
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

    // ── State ─────────────────────────────────────────────────────────────

    fn state(&self) -> CueState { self.state }

    // ── Lifecycle ─────────────────────────────────────────────────────────

    fn load(&mut self, ctx: &CueContext) -> Result<()> {
        for child in &mut self.children {
            child.load(ctx)?;
        }
        Ok(())
    }

    fn go(&mut self, ctx: &CueContext) -> Result<()> {
        if self.state == CueState::Running && self.mode == GroupMode::Sequential && !self.in_pre_wait {
            // Sequence paused (DoNotContinue child finished) → fire next child.
            if self.seq_done && self.has_next_sequential_child() {
                self.seq_done = false;
                let prev_id = self.seq_current_id;
                return self.fire_next_sequential(ctx, prev_id);
            }
            // A child is still running → advance to the next child WITHOUT
            // stopping the current one, so audio overlaps exactly like firing
            // cues at the top level.  (Visual cues self-cross-stop inside the
            // output engine, so only one video/image is ever on screen.)
            if let Some(current_id) = self.seq_current_id {
                self.seq_done = false;
                return self.fire_next_sequential(ctx, Some(current_id));
            }
        }

        self.auto_continue_fired = false;
        self.started_at = Some(Instant::now());

        if self.pre_wait > Duration::ZERO {
            self.in_pre_wait = true;
            self.state = CueState::Running;
            return Ok(());
        }

        self.start_action(ctx)
    }

    fn stop(&mut self, ctx: &CueContext) -> Result<()> {
        for child in &mut self.children {
            if child.is_running() || child.is_paused() {
                let _ = child.stop(ctx);
            }
        }
        self.reset_children();
        self.state = CueState::Standby;
        self.started_at = None;
        self.action_started_at = None;
        self.in_pre_wait = false;
        self.auto_continue_fired = false;
        ctx.emit(CueEvent::Stopped { cue_id: self.id });
        Ok(())
    }

    fn pause(&mut self, ctx: &CueContext) -> Result<()> {
        for child in &mut self.children {
            if child.is_running() {
                let _ = child.pause(ctx);
            }
        }
        self.state = CueState::Paused;
        Ok(())
    }

    fn resume(&mut self, ctx: &CueContext) -> Result<()> {
        for child in &mut self.children {
            if child.is_paused() {
                let _ = child.resume(ctx);
            }
        }
        self.state = CueState::Running;
        Ok(())
    }

    fn hard_stop(&mut self, ctx: &CueContext) -> Result<()> {
        for child in &mut self.children {
            if child.is_running() || child.is_paused() {
                let _ = child.hard_stop(ctx);
            }
        }
        self.reset_children();
        self.state = CueState::Standby;
        self.started_at = None;
        self.action_started_at = None;
        self.in_pre_wait = false;
        self.auto_continue_fired = false;
        ctx.emit(CueEvent::Stopped { cue_id: self.id });
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        self.reset_children();
        self.state = CueState::Standby;
        self.started_at = None;
        self.action_started_at = None;
        self.in_pre_wait = false;
        self.auto_continue_fired = false;
        Ok(())
    }

    fn tick(&mut self, ctx: &CueContext) -> Result<()> {
        // ── Pre-wait ──────────────────────────────────────────────────────
        if self.in_pre_wait {
            if self.started_at.map(|t| t.elapsed()).unwrap_or(Duration::ZERO) >= self.pre_wait {
                self.start_action(ctx)?;
            }
            return Ok(());
        }

        if self.state != CueState::Running {
            return Ok(());
        }

        match self.mode {
            // ── Simultaneous ──────────────────────────────────────────────
            GroupMode::Simultaneous => {
                for i in 0..self.children.len() {
                    if self.children[i].state() == CueState::Running {
                        let _ = self.tick_child_at(i, ctx);
                    }
                }
                // is_complete() handles detecting "all done" for the event loop.
            }

            // ── Sequential ────────────────────────────────────────────────
            GroupMode::Sequential => {
                // Tick every running child so overlapping cues (a manual GO or
                // Auto-Follow fired the next child while a previous one is still
                // playing) keep progressing and finish on their own.  Reset any
                // child that finishes EXCEPT the current sequence driver — its
                // completion is what advances the sequence (handled below).
                let current_id_opt = self.seq_current_id;
                let mut current_done = false;
                for i in 0..self.children.len() {
                    if self.children[i].state() == CueState::Running {
                        let done = self.tick_child_at(i, ctx)?;
                        if done {
                            if Some(self.children[i].id()) == current_id_opt {
                                current_done = true;
                            } else {
                                let _ = self.children[i].reset();
                            }
                        }
                    }
                }

                // Post-wait before firing the next child.
                if let Some(deadline) = self.seq_post_wait_until {
                    if Instant::now() >= deadline {
                        self.seq_post_wait_until = None;
                        self.fire_next_sequential(ctx, current_id_opt)?;
                    }
                    return Ok(());
                }

                let current_id = match current_id_opt {
                    Some(id) => id,
                    None => return Ok(()),
                };
                let idx = match self.children.iter().position(|c| c.id() == current_id) {
                    Some(i) => i,
                    None => return Ok(()),
                };
                // The driver child may have finished in a previous tick before
                // we got here (state already Completed/Standby).
                if matches!(self.children[idx].state(), CueState::Completed | CueState::Standby) {
                    current_done = true;
                }

                // `seq_done` means the sequence is intentionally paused at this
                // child (a DoNotContinue boundary, or a manual Playhead placement)
                // — wait for a GO rather than auto-advancing.
                if current_done && !self.seq_done {
                    let cm = self.children[idx].continue_mode();
                    let pw = self.children[idx].post_wait();
                    let _ = self.children[idx].reset();

                    match cm {
                        ContinueMode::DoNotContinue => {
                            self.seq_done = true;
                        }
                        ContinueMode::AutoContinue => {
                            if pw == Duration::ZERO {
                                self.fire_next_sequential(ctx, Some(current_id))?;
                            } else {
                                self.seq_post_wait_until = Some(Instant::now() + pw);
                            }
                        }
                        ContinueMode::AutoFollow => {
                            // The fired child completed — fire the cue AFTER the
                            // one that Auto-Followed.
                            self.fire_next_sequential(ctx, Some(current_id))?;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn is_action_started(&self) -> bool {
        !self.in_pre_wait
    }

    // ── Timing ────────────────────────────────────────────────────────────

    fn pre_wait(&self) -> Duration { self.pre_wait }
    fn set_pre_wait(&mut self, d: Duration) { self.pre_wait = d; }
    fn post_wait(&self) -> Duration { self.post_wait }
    fn set_post_wait(&mut self, d: Duration) { self.post_wait = d; }

    fn duration(&self) -> Option<Duration> {
        // Return None; the event loop uses is_complete() for Group completion.
        None
    }

    fn elapsed(&self) -> Duration {
        self.started_at.map(|t| t.elapsed()).unwrap_or(Duration::ZERO)
    }

    fn action_elapsed(&self) -> Duration {
        self.action_started_at.map(|t| t.elapsed()).unwrap_or(Duration::ZERO)
    }

    // ── Continue ──────────────────────────────────────────────────────────

    fn continue_mode(&self) -> ContinueMode { self.continue_mode }
    fn set_continue_mode(&mut self, mode: ContinueMode) { self.continue_mode = mode; }

    fn is_auto_continue_fired(&self) -> bool { self.auto_continue_fired }
    fn mark_auto_continue_fired(&mut self) { self.auto_continue_fired = true; }
    fn clear_auto_continue_fired(&mut self) { self.auto_continue_fired = false; }

    // ── Group support ─────────────────────────────────────────────────────

    fn is_complete(&self) -> bool {
        if self.state != CueState::Running || self.in_pre_wait {
            return false;
        }
        match self.mode {
            GroupMode::Simultaneous => {
                self.children.iter().all(|c| !c.is_running())
            }
            GroupMode::Sequential => {
                // seq_done means either "paused at DoNotContinue child" OR
                // "all children exhausted".  The group is only truly complete
                // when there are NO more children left to fire.
                self.seq_done
                    && !self.has_next_sequential_child()
                    && self.children.iter().all(|c| !c.is_running())
            }
        }
    }

    fn child_cues(&self) -> Option<&[Box<dyn Cue>]> {
        Some(&self.children)
    }

    fn child_cues_mut(&mut self) -> Option<&mut Vec<Box<dyn Cue>>> {
        Some(&mut self.children)
    }

    fn take_children(&mut self) -> Option<Vec<Box<dyn Cue>>> {
        Some(std::mem::take(&mut self.children))
    }

    fn add_child(&mut self, child: Box<dyn Cue>, position: i32) -> Result<()> {
        if position < 0 || position as usize >= self.children.len() {
            self.children.push(child);
        } else {
            self.children.insert(position as usize, child);
        }
        Ok(())
    }

    fn remove_child(&mut self, id: &CueId) -> Result<Box<dyn Cue>> {
        let idx = self
            .children
            .iter()
            .position(|c| c.id() == *id)
            .ok_or_else(|| anyhow!("Child cue {:?} not found in group", id))?;
        Ok(self.children.remove(idx))
    }

    fn group_mode(&self) -> Option<GroupMode> {
        Some(self.mode)
    }

    fn set_group_mode(&mut self, mode: GroupMode) {
        self.mode = mode;
    }

    fn absorbs_go(&self) -> bool {
        // Absorb a GO only while there is still another child to fire after the
        // current one.  Once the last child has been fired, the GO falls through
        // to the outer list (the Playhead has already been released past us).
        self.state == CueState::Running
            && self.mode == GroupMode::Sequential
            && !self.in_pre_wait
            && self.has_next_sequential_child()
    }

    fn holds_playhead(&self) -> bool {
        // Keep the outer Playhead while pre-waiting (no child fired yet) or while
        // there is still a child to fire.  Released as soon as the last child is
        // fired (see `released_playhead`).
        self.mode == GroupMode::Sequential
            && self.state == CueState::Running
            && (self.in_pre_wait || self.has_next_sequential_child())
    }

    fn released_playhead(&self) -> bool {
        // The last child has been fired: a child was started, none remain, and we
        // are past any pre-wait.  The group may still be running (overlapping
        // children playing out) but the outer Playhead should move on.
        self.mode == GroupMode::Sequential
            && self.state == CueState::Running
            && !self.in_pre_wait
            && self.seq_current_id.is_some()
            && !self.has_next_sequential_child()
    }

    fn active_child_id(&self) -> Option<CueId> {
        if self.mode != GroupMode::Sequential {
            return None;
        }
        // The next child a GO will fire = the child after the inner playhead.
        // Works in every state: Standby (None → first child, or a child the user
        // parked the Playhead on), Running (the child after the current one), and
        // after the last child has fired (None → the Playhead has left the group).
        match self.seq_current_id {
            Some(id) => {
                let idx = self.children.iter().position(|c| c.id() == id)?;
                self.children.get(idx + 1).map(|c| c.id())
            }
            None => self.children.first().map(|c| c.id()),
        }
    }

    fn set_active_child(&mut self, child_id: &CueId) -> bool {
        if self.mode != GroupMode::Sequential {
            return false;
        }
        let Some(idx) = self.children.iter().position(|c| c.id() == *child_id) else {
            return false;
        };
        // Park the inner playhead on the child BEFORE the target so the next fire
        // (start_action on a standby group, or an absorbed GO on a running one)
        // fires the target.  `None` when the target is the first child.
        self.seq_current_id = if idx == 0 {
            None
        } else {
            Some(self.children[idx - 1].id())
        };
        // Pause here: a running sequence waits for the next GO instead of
        // auto-advancing.  A standby group ignores this — start_action clears
        // seq_done and fires from seq_current_id.
        self.seq_done = true;
        self.seq_post_wait_until = None;
        true
    }

    // ── Serialisation ─────────────────────────────────────────────────────

    fn serialize(&self) -> Value {
        let children: Vec<Value> = self.children.iter().map(|c| c.serialize()).collect();
        json!({
            "type": "group",
            "cue_type": "group",
            "id": self.id,
            "number": self.number,
            "name": self.name,
            "notes": self.notes,
            "color": self.color,
            "pre_wait_ms": self.pre_wait.as_millis() as u64,
            "post_wait_ms": self.post_wait.as_millis() as u64,
            "continue_mode": self.continue_mode,
            "group_mode": self.mode,
            "children": children,
            "is_disabled": self.is_disabled,
        })
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Factory for [`GroupCue`].  Register this in [`super::registry::CueRegistry`].
pub struct GroupCueFactory;

impl CueFactory for GroupCueFactory {
    fn create(&self) -> Box<dyn Cue> {
        Box::new(GroupCue::new())
    }

    /// NOTE: This factory's `from_json` is intentionally never called.
    /// [`CueRegistry::from_json`] special-cases `CueType::Group` and calls
    /// [`GroupCue::from_json_with_registry`] directly so that children are
    /// deserialised with the registry.
    fn from_json(&self, _value: Value) -> Result<Box<dyn Cue>> {
        Ok(Box::new(GroupCue::new()))
    }
}

// ---------------------------------------------------------------------------
// Tests — playhead-handling logic (pure, no CueContext required)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cue::memo_cue::MemoCue;

    /// A Sequential group with `n` memo children, marked Running.
    fn running_seq_group(n: usize) -> GroupCue {
        let mut g = GroupCue::new();
        g.mode = GroupMode::Sequential;
        for _ in 0..n {
            g.children.push(Box::new(MemoCue::new()));
        }
        g.state = CueState::Running;
        g
    }

    #[test]
    fn absorbs_and_holds_while_more_children_remain() {
        let mut g = running_seq_group(3);
        // Inner playhead on the first child → another child remains.
        g.seq_current_id = Some(g.children[0].id());
        assert!(g.absorbs_go());
        assert!(g.holds_playhead());
        assert!(!g.released_playhead());
    }

    #[test]
    fn releases_playhead_on_last_child() {
        let mut g = running_seq_group(3);
        g.seq_current_id = Some(g.children[2].id()); // last child fired
        assert!(!g.absorbs_go());
        assert!(!g.holds_playhead());
        assert!(g.released_playhead());
    }

    #[test]
    fn single_child_group_releases_immediately() {
        let mut g = running_seq_group(1);
        g.seq_current_id = Some(g.children[0].id());
        assert!(!g.holds_playhead());
        assert!(g.released_playhead());
        assert!(!g.absorbs_go());
    }

    #[test]
    fn holds_playhead_during_pre_wait() {
        let mut g = running_seq_group(2);
        g.in_pre_wait = true;
        g.seq_current_id = None;
        assert!(g.holds_playhead());
        assert!(!g.released_playhead());
        assert!(!g.absorbs_go()); // a GO during pre-wait is not absorbed
    }

    #[test]
    fn active_child_id_is_next_after_inner_playhead() {
        let mut g = running_seq_group(3);
        g.seq_current_id = Some(g.children[0].id());
        let expected = g.children[1].id();
        assert_eq!(g.active_child_id(), Some(expected));
        g.seq_current_id = Some(g.children[2].id());
        assert_eq!(g.active_child_id(), None); // no child after the last
    }

    #[test]
    fn simultaneous_group_never_holds_playhead() {
        let mut g = running_seq_group(2);
        g.mode = GroupMode::Simultaneous;
        assert!(!g.holds_playhead());
        assert!(!g.released_playhead());
        assert!(!g.absorbs_go());
        assert_eq!(g.active_child_id(), None);
    }

    #[test]
    fn set_active_child_parks_inner_playhead() {
        let mut g = running_seq_group(3);
        let second = g.children[1].id();
        assert!(g.set_active_child(&second));
        assert_eq!(g.active_child_id(), Some(second));

        // Parking on the first child clears the inner playhead.
        let first = g.children[0].id();
        assert!(g.set_active_child(&first));
        assert_eq!(g.seq_current_id, None);
        assert_eq!(g.active_child_id(), Some(first));
    }

    #[test]
    fn set_active_child_rejected_for_simultaneous() {
        let mut g = running_seq_group(2);
        g.mode = GroupMode::Simultaneous;
        let child = g.children[0].id();
        assert!(!g.set_active_child(&child));
    }
}
