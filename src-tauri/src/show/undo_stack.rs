//! Undo / redo history for the cue list.
//!
//! Uses a snapshot-based approach: before each mutating operation the entire
//! active cue list is captured as a [`Snapshot`].  Each cue is serialised to
//! JSON plus its decoded audio `Arc` is cloned (cheap — it is a reference-counted
//! pointer, not a copy of the raw samples).  Restoration rebuilds cues via the
//! `CueRegistry` and injects the saved audio Arc back, so there is never a
//! re-decode round-trip on undo/redo.
//!
//! [`UndoHistory`] keeps one [`UndoStack`] per cue list (keyed by its id)
//! instead of a single shared stack: without that, switching the active cue
//! list tab and pressing Ctrl+Z would pop a snapshot recorded for a
//! *different* list and splice its cues into the one on screen. Loading a
//! different workspace gives its cue lists fresh ids, so this also keeps one
//! workspace's history from bleeding into the next.

use std::collections::HashMap;

use crate::cue::decoded_audio::DecodedAudioMap;

/// Maximum number of undo levels kept in memory.
const MAX_UNDO: usize = 50;

/// Snapshot of a single cue: its serialised state plus the decoded audio it
/// (and, for a Group, each of its children) already had in memory.
pub struct CueSnapshot {
    /// Full serialised form produced by [`crate::cue::traits::Cue::serialize`].
    pub json: serde_json::Value,
    /// Decoded audio of the cue and its descendants, keyed by cue id.
    pub decoded: DecodedAudioMap,
}

/// Complete snapshot of the active cue list at a single point in time.
pub struct Snapshot {
    /// Ordered cue snapshots, matching the cue list order.
    pub cues: Vec<CueSnapshot>,
    /// Playhead position at the time the snapshot was taken.
    pub playhead_id: Option<uuid::Uuid>,
}

/// Undo / redo history.
pub struct UndoStack {
    /// Past states — oldest at index 0, most-recent at the back.
    past: Vec<Snapshot>,
    /// Future states available after one or more undos — most-recent at back.
    future: Vec<Snapshot>,
}

impl Default for UndoStack {
    fn default() -> Self {
        Self::new()
    }
}

impl UndoStack {
    /// Create an empty history.
    pub fn new() -> Self {
        Self {
            past: Vec::new(),
            future: Vec::new(),
        }
    }

    /// Record the state *before* a mutating action.
    ///
    /// Clears the redo stack — once the user performs a new action the redo
    /// history is no longer meaningful.  Trims the past stack to `MAX_UNDO`.
    pub fn push_action(&mut self, snapshot: Snapshot) {
        self.future.clear();
        self.past.push(snapshot);
        if self.past.len() > MAX_UNDO {
            self.past.remove(0);
        }
    }

    /// Whether there is at least one state to undo.
    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    /// Whether there is at least one state to redo.
    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    /// Pop the most-recent past snapshot and push `current` onto the future stack.
    ///
    /// Returns the snapshot to restore, or `None` if there is nothing to undo.
    pub fn undo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let prev = self.past.pop()?;
        self.future.push(current);
        Some(prev)
    }

    /// Pop the most-recent future snapshot and push `current` onto the past stack.
    ///
    /// Returns the snapshot to restore, or `None` if there is nothing to redo.
    pub fn redo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let next = self.future.pop()?;
        self.past.push(current);
        Some(next)
    }
}

/// Undo/redo histories for every cue list in the current workspace, keyed by
/// cue list id so each list's history is independent of the others.
#[derive(Default)]
pub struct UndoHistory {
    stacks: HashMap<uuid::Uuid, UndoStack>,
}

impl UndoHistory {
    /// Create an empty history.
    pub fn new() -> Self {
        Self::default()
    }

    fn stack_mut(&mut self, cue_list_id: uuid::Uuid) -> &mut UndoStack {
        self.stacks.entry(cue_list_id).or_default()
    }

    /// Record the state *before* a mutating action on the given cue list.
    pub fn push_action(&mut self, cue_list_id: uuid::Uuid, snapshot: Snapshot) {
        self.stack_mut(cue_list_id).push_action(snapshot);
    }

    /// Whether the given cue list has at least one state to undo.
    pub fn can_undo(&self, cue_list_id: uuid::Uuid) -> bool {
        self.stacks.get(&cue_list_id).is_some_and(UndoStack::can_undo)
    }

    /// Whether the given cue list has at least one state to redo.
    pub fn can_redo(&self, cue_list_id: uuid::Uuid) -> bool {
        self.stacks.get(&cue_list_id).is_some_and(UndoStack::can_redo)
    }

    /// Undo the most-recent action recorded for the given cue list.
    pub fn undo(&mut self, cue_list_id: uuid::Uuid, current: Snapshot) -> Option<Snapshot> {
        self.stack_mut(cue_list_id).undo(current)
    }

    /// Redo the most-recently undone action for the given cue list.
    pub fn redo(&mut self, cue_list_id: uuid::Uuid, current: Snapshot) -> Option<Snapshot> {
        self.stack_mut(cue_list_id).redo(current)
    }

    /// Drop every cue list's history. Called when a new, loaded, or imported
    /// workspace replaces the cue lists wholesale — stale histories keyed by
    /// ids that no longer exist in the workspace would just be dead weight.
    pub fn clear(&mut self) {
        self.stacks.clear();
    }
}
