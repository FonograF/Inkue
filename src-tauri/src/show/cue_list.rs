//! [`CueList`] — an ordered sequence of cues with a Playhead.

use std::collections::{HashMap, HashSet, BTreeMap};

use anyhow::{anyhow, Result};
use uuid::Uuid;

use crate::cue::{registry::CueRegistry, traits::Cue, types::CueId};

// ---------------------------------------------------------------------------
// Recursive helpers (free functions to avoid borrow-checker conflicts)
// ---------------------------------------------------------------------------

/// Recursively assign cue numbers.
///
/// `prefix` is the parent's number (e.g. `"3"` for top-level cue 3).
/// Children of that cue get `"3.1"`, `"3.2"`, etc.
/// An empty prefix means we are at the top level: numbers are `"1"`, `"2"`, …
fn renumber_recursive(cues: &mut Vec<Box<dyn Cue>>, prefix: &str) {
    for (i, cue) in cues.iter_mut().enumerate() {
        let number = if prefix.is_empty() {
            (i + 1).to_string()
        } else {
            format!("{}.{}", prefix, i + 1)
        };
        cue.set_number(Some(number.clone()));
        if let Some(children) = cue.child_cues_mut() {
            renumber_recursive(children, &number);
        }
    }
}

/// Extract a cue from anywhere in the hierarchy (top-level or inside any group).
fn extract_cue_anywhere(cues: &mut Vec<Box<dyn Cue>>, id: &CueId) -> Option<Box<dyn Cue>> {
    if let Some(idx) = cues.iter().position(|c| c.id() == *id) {
        return Some(cues.remove(idx));
    }
    for cue in cues.iter_mut() {
        if let Some(children) = cue.child_cues_mut() {
            if let Some(extracted) = extract_cue_anywhere(children, id) {
                return Some(extracted);
            }
        }
    }
    None
}

/// Add `child` to the group identified by `group_id`, searching recursively.
/// Returns `Ok(None)` when placed successfully, `Ok(Some(child))` when the
/// group was not found (caller must handle the returned child to avoid losing it).
fn add_child_to_group_anywhere(
    cues: &mut Vec<Box<dyn Cue>>,
    group_id: &CueId,
    child: Box<dyn Cue>,
    position: i32,
) -> Result<Option<Box<dyn Cue>>> {
    // First pass: check if any entry IS the target group (avoids mid-loop borrow).
    for cue in cues.iter_mut() {
        if cue.id() == *group_id {
            cue.add_child(child, position)?;
            return Ok(None);
        }
    }
    // Second pass: recurse into children.
    let mut child_opt = Some(child);
    for cue in cues.iter_mut() {
        if let Some(children) = cue.child_cues_mut() {
            let c = child_opt.take().expect("invariant: always Some here");
            match add_child_to_group_anywhere(children, group_id, c, position)? {
                None => return Ok(None),
                Some(returned) => child_opt = Some(returned),
            }
        }
    }
    Ok(child_opt)
}

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

    /// Find a cue by ID (mutable), searching only top-level cues.
    pub fn get_mut(&mut self, id: &CueId) -> Option<&mut dyn Cue> {
        self.cues
            .iter_mut()
            .find(|c| c.id() == *id)
            .map(|c| c.as_mut() as &mut dyn Cue)
    }

    /// Find a cue by ID (mutable), searching recursively through group children.
    ///
    /// Used when applying preloaded audio to cues that may live inside groups.
    pub fn get_mut_recursive(&mut self, id: &CueId) -> Option<&mut dyn Cue> {
        fn search(cues: &mut Vec<Box<dyn crate::cue::traits::Cue>>, id: &CueId) -> Option<*mut dyn crate::cue::traits::Cue> {
            for cue in cues.iter_mut() {
                if cue.id() == *id {
                    return Some(cue.as_mut() as *mut dyn crate::cue::traits::Cue);
                }
                if let Some(children) = cue.child_cues_mut() {
                    if let Some(ptr) = search(children, id) {
                        return Some(ptr);
                    }
                }
            }
            None
        }
        // SAFETY: the pointer is valid as long as `self` is borrowed mutably,
        // which is guaranteed by the lifetime of the returned reference.
        search(&mut self.cues, id).map(|ptr| unsafe { &mut *ptr })
    }

    /// Replace a cue by ID in-place, searching recursively through group children.
    ///
    /// Returns `true` if the cue was found and replaced.
    pub fn replace_cue_recursive(&mut self, id: &CueId, new_cue: Box<dyn crate::cue::traits::Cue>) -> bool {
        fn replace(
            cues: &mut Vec<Box<dyn crate::cue::traits::Cue>>,
            id: &CueId,
            slot: &mut Option<Box<dyn crate::cue::traits::Cue>>,
        ) -> bool {
            for i in 0..cues.len() {
                if cues[i].id() == *id {
                    if let Some(c) = slot.take() {
                        cues[i] = c;
                    }
                    return true;
                }
            }
            for cue in cues.iter_mut() {
                if let Some(children) = cue.child_cues_mut() {
                    if replace(children, id, slot) {
                        return true;
                    }
                }
            }
            false
        }
        let mut slot = Some(new_cue);
        replace(&mut self.cues, id, &mut slot)
    }

    /// Index of a cue within the list, if found.
    pub fn index_of(&self, id: &CueId) -> Option<usize> {
        self.cues.iter().position(|c| c.id() == *id)
    }

    // -----------------------------------------------------------------------
    // Mutation
    // -----------------------------------------------------------------------

    /// Assign sequential cue numbers to every cue in the list, recursively.
    ///
    /// Top-level cues get "1", "2", "3", …
    /// Children of a group numbered "3" get "3.1", "3.2", "3.3", …
    /// Deeply nested groups continue the pattern: "3.2.1", "3.2.2", …
    ///
    /// Called automatically after any structural mutation (add, remove, move).
    pub fn renumber_all(&mut self) {
        renumber_recursive(&mut self.cues, "");
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

    /// Remove multiple cues at once.  If the Playhead is among the removed
    /// cues it advances to the first remaining cue at or after the lowest
    /// removed position, falling back to the last remaining cue.
    pub fn remove_many(&mut self, ids: &[CueId]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let ids_set: HashSet<CueId> = ids.iter().cloned().collect();
        for id in ids {
            if self.index_of(id).is_none() {
                return Err(anyhow!("Cue {:?} not found", id));
            }
        }

        let new_playhead = if self.playhead_cue_id.is_some_and(|ph| ids_set.contains(&ph)) {
            let ph_idx = self
                .playhead_cue_id
                .and_then(|id| self.index_of(&id))
                .unwrap_or(0);
            // First non-removed cue at or after ph_idx.
            self.cues
                .iter()
                .skip(ph_idx)
                .find(|c| !ids_set.contains(&c.id()))
                .or_else(|| self.cues.iter().rev().find(|c| !ids_set.contains(&c.id())))
                .map(|c| c.id())
        } else {
            self.playhead_cue_id
        };

        let all = std::mem::take(&mut self.cues);
        self.cues = all.into_iter().filter(|c| !ids_set.contains(&c.id())).collect();
        self.playhead_cue_id = new_playhead;
        self.renumber_all();
        Ok(())
    }

    /// Move `ids` (preserving their relative order) so they appear immediately
    /// before `before_id` in the list, or at the end if `before_id` is `None`.
    /// If `before_id` is one of the moved cues the call is a no-op.
    pub fn move_before(&mut self, ids: &[CueId], before_id: Option<CueId>) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let ids_set: HashSet<CueId> = ids.iter().cloned().collect();
        for id in ids {
            if self.index_of(id).is_none() {
                return Err(anyhow!("Cue {:?} not found", id));
            }
        }
        if before_id.is_some_and(|bid| ids_set.contains(&bid)) {
            return Ok(());
        }

        let all = std::mem::take(&mut self.cues);
        let mut staying: Vec<Box<dyn Cue>> = Vec::with_capacity(all.len());
        let mut moving_map: HashMap<CueId, Box<dyn Cue>> = HashMap::new();
        for cue in all {
            if ids_set.contains(&cue.id()) {
                moving_map.insert(cue.id(), cue);
            } else {
                staying.push(cue);
            }
        }
        let moving: Vec<Box<dyn Cue>> = ids.iter().filter_map(|id| moving_map.remove(id)).collect();

        let insert_at = before_id
            .and_then(|id| staying.iter().position(|c| c.id() == id))
            .unwrap_or(staying.len());

        let mut result = Vec::with_capacity(staying.len() + moving.len());
        result.extend(staying.drain(..insert_at));
        result.extend(moving);
        result.extend(staying);
        self.cues = result;
        self.renumber_all();
        Ok(())
    }

    /// Wrap the given cues in a new [`GroupCue`] inserted at the first selected
    /// position.  Returns the new Group's ID.
    pub fn group_cues(&mut self, ids: &[CueId]) -> Result<CueId> {
        if ids.is_empty() {
            return Err(anyhow!("No cues to group"));
        }
        let ids_set: HashSet<CueId> = ids.iter().cloned().collect();

        // Record original order for computing insertion position.
        let original: Vec<CueId> = self.cues.iter().map(|c| c.id()).collect();
        let min_pos = ids
            .iter()
            .filter_map(|id| original.iter().position(|o| o == id))
            .min()
            .ok_or_else(|| anyhow!("Cue not found"))?;

        // Count non-selected cues that appear before min_pos — that is the
        // insertion index in the `staying` list.
        let insert_at = original[..min_pos]
            .iter()
            .filter(|id| !ids_set.contains(*id))
            .count();

        // Partition into staying / children; preserve ids order for children.
        let all = std::mem::take(&mut self.cues);
        let mut staying: Vec<Box<dyn Cue>> = Vec::with_capacity(all.len());
        let mut children_map: BTreeMap<usize, Box<dyn Cue>> = BTreeMap::new();
        for cue in all {
            if let Some(order_pos) = ids.iter().position(|id| *id == cue.id()) {
                children_map.insert(order_pos, cue);
            } else {
                staying.push(cue);
            }
        }
        let children: Vec<Box<dyn Cue>> = children_map.into_values().collect();

        let mut group = crate::cue::group_cue::GroupCue::new();
        group.children = children;
        let group_id = group.id;

        staying.insert(insert_at.min(staying.len()), Box::new(group));
        self.cues = staying;
        self.renumber_all();
        Ok(group_id)
    }

    /// Dissolve a Group: insert its children at the Group's position and remove
    /// the Group itself.
    pub fn ungroup(&mut self, group_id: &CueId) -> Result<()> {
        let idx = self
            .index_of(group_id)
            .ok_or_else(|| anyhow!("Group {:?} not found", group_id))?;

        let mut group_box = self.cues.remove(idx);
        let children = group_box
            .take_children()
            .ok_or_else(|| anyhow!("Cue is not a Group"))?;

        // Adjust playhead if it pointed to the group.
        if self.playhead_cue_id == Some(*group_id) {
            self.playhead_cue_id = children.first().map(|c| c.id());
        }

        for (i, child) in children.into_iter().enumerate() {
            self.cues.insert(idx + i, child);
        }

        self.renumber_all();
        Ok(())
    }

    /// Move a cue (from anywhere in the hierarchy) into a group's children at
    /// the given position (−1 = append).  Both source and target are searched
    /// recursively so nested cues and nested groups are handled correctly.
    pub fn add_to_group(
        &mut self,
        cue_id: &CueId,
        group_id: &CueId,
        position: i32,
    ) -> Result<()> {
        let cue = extract_cue_anywhere(&mut self.cues, cue_id)
            .ok_or_else(|| anyhow!("Cue {:?} not found", cue_id))?;

        if self.playhead_cue_id == Some(*cue_id) {
            self.playhead_cue_id = self.cues.first().map(|c| c.id());
        }

        match add_child_to_group_anywhere(&mut self.cues, group_id, cue, position)? {
            None => {
                self.renumber_all();
                Ok(())
            }
            Some(_) => Err(anyhow!("Group {:?} not found", group_id)),
        }
    }

    /// Remove a child from a group and reinsert it into the main list
    /// immediately after the group.
    pub fn remove_from_group(&mut self, group_id: &CueId, cue_id: &CueId) -> Result<()> {
        let group_idx = self
            .index_of(group_id)
            .ok_or_else(|| anyhow!("Group {:?} not found", group_id))?;

        let child = self.cues[group_idx].remove_child(cue_id)?;
        self.cues.insert(group_idx + 1, child);
        self.renumber_all();
        Ok(())
    }

    /// Move a cue from anywhere in the hierarchy to the top-level list,
    /// immediately before `before_id` (or at the end if `None`).
    pub fn move_to_top_level_before(
        &mut self,
        cue_id: &CueId,
        before_id: Option<&CueId>,
    ) -> Result<()> {
        let cue = extract_cue_anywhere(&mut self.cues, cue_id)
            .ok_or_else(|| anyhow!("Cue {:?} not found", cue_id))?;

        if self.playhead_cue_id == Some(*cue_id) {
            self.playhead_cue_id = self.cues.first().map(|c| c.id());
        }

        let insert_at = before_id
            .and_then(|id| self.index_of(id))
            .unwrap_or(self.cues.len());

        self.cues.insert(insert_at, cue);
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
    fn remove_many_basic() {
        let mut list = CueList::new("Test");
        let c1 = memo(); let id1 = c1.id();
        let c2 = memo(); let id2 = c2.id();
        let c3 = memo(); let id3 = c3.id();
        list.push(c1); list.push(c2); list.push(c3);
        list.remove_many(&[id1, id3]).unwrap();
        assert_eq!(list.cues.len(), 1);
        assert_eq!(list.cues[0].id(), id2);
    }

    #[test]
    fn remove_many_advances_playhead() {
        let mut list = CueList::new("Test");
        let c1 = memo(); let id1 = c1.id();
        let c2 = memo(); let id2 = c2.id();
        let c3 = memo(); let id3 = c3.id();
        list.push(c1); list.push(c2); list.push(c3);
        // Playhead starts at c1.
        list.remove_many(&[id1, id2]).unwrap();
        // Only c3 remains; playhead should advance to it.
        assert_eq!(list.playhead_cue_id, Some(id3));
    }

    #[test]
    fn move_before_reorders_group() {
        let mut list = CueList::new("Test");
        let c1 = memo(); let id1 = c1.id();
        let c2 = memo(); let id2 = c2.id();
        let c3 = memo(); let id3 = c3.id();
        list.push(c1); list.push(c2); list.push(c3);
        // Move c2 and c3 before c1 → [c2, c3, c1]
        list.move_before(&[id2, id3], Some(id1)).unwrap();
        assert_eq!(list.cues[0].id(), id2);
        assert_eq!(list.cues[1].id(), id3);
        assert_eq!(list.cues[2].id(), id1);
    }

    #[test]
    fn move_before_none_appends_to_end() {
        let mut list = CueList::new("Test");
        let c1 = memo(); let id1 = c1.id();
        let c2 = memo();
        let c3 = memo(); let id3 = c3.id();
        list.push(c1); list.push(c2); list.push(c3);
        list.move_before(&[id1], None).unwrap();
        assert_eq!(list.cues.last().unwrap().id(), id1);
        assert_eq!(list.cues[1].id(), id3);
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
