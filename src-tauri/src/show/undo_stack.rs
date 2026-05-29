//! Undo / redo history for the cue list.
//!
//! Uses a snapshot-based approach: before each mutating operation the entire
//! active cue list is captured as a [`Snapshot`].  Each cue is serialised to
//! JSON plus its decoded audio `Arc` is cloned (cheap — it is a reference-counted
//! pointer, not a copy of the raw samples).  Restoration rebuilds cues via the
//! `CueRegistry` and injects the saved audio Arc back, so there is never a
//! re-decode round-trip on undo/redo.

use std::sync::Arc;
use std::time::Duration;

/// Maximum number of undo levels kept in memory.
const MAX_UNDO: usize = 50;

/// Snapshot of a single cue: its serialised state plus (optionally) the decoded
/// audio samples it already had in memory.
pub struct CueSnapshot {
    /// Full serialised form produced by [`crate::cue::traits::Cue::serialize`].
    pub json: serde_json::Value,
    /// Decoded audio, if the cue had already loaded it.
    /// Tuple: (samples Arc, channel count, sample rate Hz, total duration).
    pub decoded: Option<(Arc<Vec<f32>>, u16, u32, Duration)>,
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
