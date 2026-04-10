//! [`CueList`] — an ordered sequence of cues with a Playhead.

use anyhow::{anyhow, Result};
use uuid::Uuid;

use crate::cue::{registry::CueRegistry, traits::Cue, types::CueId};

/// An ordered list of cues with a Playhead indicating the next cue to GO.
pub struct CueList {
    pub id: Uuid,
    pub name: String,
    /// Cues in their display/execution order.
    pub cues: Vec<Box<dyn Cue>>,
    /// ID of the cue at the Playhead (next to be triggered by GO).
    /// `None` means the playhead is past the last cue.
    pub playhead_cue_id: Option<CueId>,
}

impl CueList {
    /// Create a new, empty cue list.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            cues: Vec::new(),
            playhead_cue_id: None,
        }
    }

    // -----------------------------------------------------------------------
    // Cue access
    // -----------------------------------------------------------------------

    /// Find a cue by ID (immutable).
    pub fn get(&self, id: &CueId) -> Option<&dyn Cue> {
        self.cues.iter().find(|c| c.id() == *id).map(|c| c.as_ref())
    }

    /// Find a cue by ID (mutable).
    pub fn get_mut(&mut self, id: &CueId) -> Option<&mut dyn Cue> {
        self.cues
            .iter_mut()
            .find(|c| c.id() == *id)
            .map(|c| c.as_mut() as &mut dyn Cue)
    }

    /// Index of a cue within the list, if found.
    pub fn index_of(&self, id: &CueId) -> Option<usize> {
        self.cues.iter().position(|c| c.id() == *id)
    }

    // -----------------------------------------------------------------------
    // Mutation
    // -----------------------------------------------------------------------

    /// Assign sequential cue numbers ("1", "2", "3", …) to every cue in order.
    ///
    /// Called automatically after any structural mutation (add, remove, move).
    /// Cues whose number was manually cleared are still renumbered; this keeps
    /// the list consistent and matches the user expectation that numbers always
    /// reflect position.
    pub fn renumber_all(&mut self) {
        for (i, cue) in self.cues.iter_mut().enumerate() {
            cue.set_number(Some((i + 1).to_string()));
        }
    }

    /// Append a cue to the end of the list.
    pub fn push(&mut self, cue: Box<dyn Cue>) {
        if self.cues.is_empty() {
            // Auto-advance playhead to first cue.
            self.playhead_cue_id = Some(cue.id());
        }
        self.cues.push(cue);
        self.renumber_all();
    }

    /// Insert a cue at the given index (0-based).
    pub fn insert(&mut self, index: usize, cue: Box<dyn Cue>) {
        let id = cue.id();
        let idx = index.min(self.cues.len());
        self.cues.insert(idx, cue);
        if self.playhead_cue_id.is_none() {
            self.playhead_cue_id = Some(id);
        }
        self.renumber_all();
    }

    /// Remove the cue with the given ID.  If the removed cue was at the
    /// Playhead, advance the Playhead to the next cue.
    pub fn remove(&mut self, id: &CueId) -> Result<Box<dyn Cue>> {
        let idx = self
            .index_of(id)
            .ok_or_else(|| anyhow!("Cue {:?} not found", id))?;

        let removed = self.cues.remove(idx);

        if self.playhead_cue_id == Some(*id) {
            // Move playhead to the cue now at this position, or the last cue.
            self.playhead_cue_id = self
                .cues
                .get(idx)
                .or_else(|| self.cues.last())
                .map(|c| c.id());
        }

        self.renumber_all();
        Ok(removed)
    }

    /// Move a cue from `from_index` to `to_index`.
    pub fn move_cue(&mut self, id: &CueId, to_index: usize) -> Result<()> {
        let from = self.index_of(id).ok_or_else(|| anyhow!("Cue not found"))?;
        let cue = self.cues.remove(from);
        let to = to_index.min(self.cues.len());
        self.cues.insert(to, cue);
        self.renumber_all();
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Playhead
    // -----------------------------------------------------------------------

    /// The cue currently at the Playhead, if any.
    pub fn playhead_cue(&self) -> Option<&dyn Cue> {
        self.playhead_cue_id
            .as_ref()
            .and_then(|id| self.get(id))
    }

    /// Advance the Playhead to the cue immediately after the current one.
    /// Returns the new Playhead cue ID, or `None` if we're past the end.
    pub fn advance_playhead(&mut self) -> Option<CueId> {
        let current_idx = self
            .playhead_cue_id
            .as_ref()
            .and_then(|id| self.index_of(id))?;

        let next = self.cues.get(current_idx + 1).map(|c| c.id());
        self.playhead_cue_id = next;
        next
    }

    /// Move the Playhead to a specific cue.
    pub fn set_playhead(&mut self, id: Option<CueId>) -> Result<()> {
        if let Some(ref cid) = id {
            if self.get(cid).is_none() {
                return Err(anyhow!("Cue {:?} not found", cid));
            }
        }
        self.playhead_cue_id = id;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Serialisation
    // -----------------------------------------------------------------------

    /// Serialise this cue list to a JSON [`serde_json::Value`].
    pub fn to_json(&self) -> serde_json::Value {
        let cues_json: Vec<serde_json::Value> = self.cues.iter().map(|c| c.serialize()).collect();
        serde_json::json!({
            "id": self.id,
            "name": self.name,
            "playhead_cue_id": self.playhead_cue_id,
            "cues": cues_json,
        })
    }

    /// Deserialise a cue list from JSON using the given registry.
    pub fn from_json(value: serde_json::Value, registry: &CueRegistry) -> anyhow::Result<Self> {
        let id: Uuid = value
            .get("id")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(Uuid::new_v4);

        let name = value
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Cue List")
            .to_string();

        let playhead_cue_id: Option<CueId> = value
            .get("playhead_cue_id")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok());

        let mut cues: Vec<Box<dyn Cue>> = Vec::new();
        if let Some(arr) = value.get("cues").and_then(|v| v.as_array()) {
            for cue_val in arr {
                match registry.from_json(cue_val.clone()) {
                    Ok(cue) => cues.push(cue),
                    Err(e) => log::warn!("Skipping unrecognised cue: {e}"),
                }
            }
        }

        Ok(Self {
            id,
            name,
            cues,
            playhead_cue_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cue::{memo_cue::MemoCue, types::CueType};

    fn memo() -> Box<dyn Cue> {
        Box::new(MemoCue::new())
    }

    #[test]
    fn push_sets_playhead_on_first_cue() {
        let mut list = CueList::new("Test");
        let cue = memo();
        let id = cue.id();
        list.push(cue);
        assert_eq!(list.playhead_cue_id, Some(id));
    }

    #[test]
    fn advance_playhead() {
        let mut list = CueList::new("Test");
        let c1 = memo();
        let c2 = memo();
        let id2 = c2.id();
        list.push(c1);
        list.push(c2);
        list.advance_playhead();
        assert_eq!(list.playhead_cue_id, Some(id2));
    }

    #[test]
    fn advance_past_end_returns_none() {
        let mut list = CueList::new("Test");
        list.push(memo());
        let result = list.advance_playhead();
        assert!(result.is_none());
        assert!(list.playhead_cue_id.is_none());
    }

    #[test]
    fn remove_advances_playhead() {
        let mut list = CueList::new("Test");
        let c1 = memo();
        let c2 = memo();
        let id1 = c1.id();
        let id2 = c2.id();
        list.push(c1);
        list.push(c2);
        assert_eq!(list.playhead_cue_id, Some(id1));
        list.remove(&id1).unwrap();
        assert_eq!(list.playhead_cue_id, Some(id2));
    }

    #[test]
    fn move_cue_reorders() {
        let mut list = CueList::new("Test");
        let c1 = memo();
        let c2 = memo();
        let c3 = memo();
        let id1 = c1.id();
        let id3 = c3.id();
        list.push(c1);
        list.push(c2);
        list.push(c3);
        // Move c3 to position 0.
        list.move_cue(&id3, 0).unwrap();
        assert_eq!(list.cues[0].id(), id3);
        assert_eq!(list.cues[1].id(), id1);
    }

    #[test]
    fn serialise_roundtrip() {
        use crate::cue::{memo_cue::MemoCueFactory, registry::CueRegistry};
        let mut registry = CueRegistry::new();
        registry.register(CueType::Memo, Box::new(MemoCueFactory));

        let mut list = CueList::new("Round-trip");
        let mut cue = MemoCue::new();
        cue.set_name("A Memo".to_string());
        list.push(Box::new(cue));

        let json = list.to_json();
        let restored = CueList::from_json(json, &registry).unwrap();
        assert_eq!(restored.name, "Round-trip");
        assert_eq!(restored.cues.len(), 1);
        assert_eq!(restored.cues[0].name(), "A Memo");
    }
}
