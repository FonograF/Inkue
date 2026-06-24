//! [`CueList`] — an ordered sequence of cues with a Playhead.

use std::collections::{HashMap, HashSet, BTreeMap};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::cue::{registry::CueRegistry, traits::Cue, types::CueId};
use crate::engine::timecode_types::{CueListTcConfig, TcTrigger};

// ---------------------------------------------------------------------------
// CueListMode
// ---------------------------------------------------------------------------

/// Playback mode for a cue list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CueListMode {
    /// Sequential: GO fires the cue at the Playhead then advances it.
    #[default]
    Sequential,
    /// Cart: each cue is triggered independently by clicking its tile.
    Cart,
}

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
    /// Sequential (playhead-driven) or Cart (each tile fires independently).
    pub mode: CueListMode,
    /// Cues in their display/execution order.
    pub cues: Vec<Box<dyn Cue>>,
    /// ID of the cue at the Playhead (next to be triggered by GO).
    /// `None` means the playhead is past the last cue.
    pub playhead_cue_id: Option<CueId>,
    /// Timecode synchronisation settings for this list.
    pub tc_config: CueListTcConfig,
    /// Per-cue TC triggers: cue_id → TcTrigger.  Stored on the list so
    /// every cue type gains TC triggering without a per-type code change.
    pub tc_triggers: HashMap<CueId, TcTrigger>,
    /// Last absolute frame number at which a TC trigger was fired in this
    /// list (monotone guard — prevents re-firing on the same position).
    /// `u64::MAX` means nothing has been fired yet.
    pub tc_last_triggered_frame: u64,
}

impl CueList {
    /// Create a new, empty cue list.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            mode: CueListMode::default(),
            cues: Vec::new(),
            playhead_cue_id: None,
            tc_config: CueListTcConfig::default(),
            tc_triggers: HashMap::new(),
            tc_last_triggered_frame: u64::MAX,
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

    /// Find a cue by ID (immutable), searching recursively through group
    /// children.  Used by the inspector / per-cue commands so cues nested in a
    /// group are reachable, not just top-level ones.
    pub fn get_recursive(&self, id: &CueId) -> Option<&dyn Cue> {
        fn search<'a>(cues: &'a [Box<dyn Cue>], id: &CueId) -> Option<&'a dyn Cue> {
            for cue in cues {
                if cue.id() == *id {
                    return Some(cue.as_ref());
                }
                if let Some(children) = cue.child_cues() {
                    if let Some(found) = search(children, id) {
                        return Some(found);
                    }
                }
            }
            None
        }
        search(&self.cues, id)
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
            for cue in cues.iter_mut() {
                if cue.id() == *id {
                    if let Some(c) = slot.take() {
                        *cue = c;
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

    /// Remove a cue by ID from anywhere in the hierarchy (top-level or nested in
    /// a group).  Top-level removals adjust the Playhead exactly like [`remove`];
    /// nested removals never touch the Playhead, which only ever holds
    /// top-level IDs.
    pub fn remove_anywhere(&mut self, id: &CueId) -> Result<Box<dyn Cue>> {
        if self.index_of(id).is_some() {
            return self.remove(id);
        }
        let removed = extract_cue_anywhere(&mut self.cues, id)
            .ok_or_else(|| anyhow!("Cue {:?} not found", id))?;
        self.renumber_all();
        Ok(removed)
    }

    /// Remove multiple cues from anywhere in the hierarchy in one operation.
    /// Nested cues are removed first (no Playhead impact); the remaining
    /// top-level cues go through the Playhead-aware [`remove_many`].
    pub fn remove_many_anywhere(&mut self, ids: &[CueId]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        for id in ids {
            if self.get_recursive(id).is_none() {
                return Err(anyhow!("Cue {:?} not found", id));
            }
        }
        let nested: Vec<CueId> =
            ids.iter().copied().filter(|id| self.index_of(id).is_none()).collect();
        let top_level: Vec<CueId> =
            ids.iter().copied().filter(|id| self.index_of(id).is_some()).collect();
        for id in &nested {
            extract_cue_anywhere(&mut self.cues, id);
        }
        if top_level.is_empty() {
            self.renumber_all();
        } else {
            self.remove_many(&top_level)?;
        }
        Ok(())
    }

    /// Insert `cue` immediately after `anchor_id`, wherever the anchor lives
    /// (top-level or inside a group).  Used to place a duplicate right after its
    /// source so duplicating a group child keeps it in the same group.
    pub fn insert_after_anywhere(&mut self, anchor_id: &CueId, cue: Box<dyn Cue>) -> Result<()> {
        fn insert(
            cues: &mut Vec<Box<dyn Cue>>,
            anchor: &CueId,
            cue: Box<dyn Cue>,
        ) -> std::result::Result<(), Box<dyn Cue>> {
            if let Some(idx) = cues.iter().position(|c| c.id() == *anchor) {
                cues.insert(idx + 1, cue);
                return Ok(());
            }
            let mut carry = cue;
            for c in cues.iter_mut() {
                if let Some(children) = c.child_cues_mut() {
                    match insert(children, anchor, carry) {
                        Ok(()) => return Ok(()),
                        Err(returned) => carry = returned,
                    }
                }
            }
            Err(carry)
        }
        insert(&mut self.cues, anchor_id, cue)
            .map_err(|_| anyhow!("Anchor cue {:?} not found", anchor_id))?;
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

    /// Advance the Playhead to the next non-disabled cue after the current one.
    /// Returns the new Playhead cue ID, or `None` if we're past the end.
    pub fn advance_playhead(&mut self) -> Option<CueId> {
        let current_idx = self
            .playhead_cue_id
            .as_ref()
            .and_then(|id| self.index_of(id))?;

        let next = self.cues[current_idx + 1..]
            .iter()
            .find(|c| !c.is_disabled())
            .map(|c| c.id());
        self.playhead_cue_id = next;
        next
    }

    /// Move the Playhead to a specific cue.
    ///
    /// A top-level cue ID parks the Playhead directly on it.  A cue nested in a
    /// group parks the outer Playhead on that cue's top-level ancestor and, when
    /// the ancestor is a Sequential group with the cue as a direct child, points
    /// the group's internal sequence at it so the next GO fires that child.
    pub fn set_playhead(&mut self, id: Option<CueId>) -> Result<()> {
        let Some(cid) = id else {
            self.playhead_cue_id = None;
            return Ok(());
        };
        if self.index_of(&cid).is_some() {
            self.playhead_cue_id = Some(cid);
            return Ok(());
        }
        let ancestor = self
            .top_level_ancestor_of(&cid)
            .ok_or_else(|| anyhow!("Cue {:?} not found", cid))?;
        if let Some(group) = self.get_mut(&ancestor) {
            group.set_active_child(&cid);
        }
        self.playhead_cue_id = Some(ancestor);
        Ok(())
    }

    /// The ID of the top-level cue whose subtree contains `id` (the cue itself
    /// when it is already top-level).
    fn top_level_ancestor_of(&self, id: &CueId) -> Option<CueId> {
        fn contains(cue: &dyn Cue, id: &CueId) -> bool {
            if cue.id() == *id {
                return true;
            }
            cue.child_cues()
                .map(|children| children.iter().any(|c| contains(c.as_ref(), id)))
                .unwrap_or(false)
        }
        self.cues.iter().find(|c| contains(c.as_ref(), id)).map(|c| c.id())
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
            "mode": self.mode,
            "playhead_cue_id": self.playhead_cue_id,
            "tc_config": self.tc_config,
            "tc_triggers": self.tc_triggers.iter().map(|(id, t)| {
                serde_json::json!({ "cue_id": id, "trigger": t })
            }).collect::<Vec<_>>(),
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

        let mode: CueListMode = value
            .get("mode")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

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

        // TODO(compat): Remove once all workspace files have been re-saved with
        // `target_cue_id`.  This pass converts the legacy `target_cue_number`
        // string into a stable UUID for the current session.  It is a no-op for
        // files that already carry `target_cue_id`.  When removed, also delete:
        //   - `StopCue.target_cue_number` field and its serialize/from_json code
        //   - `Cue::resolve_stop_target` trait method and its impl in StopCue
        //   - The `target_cue_number` field in `StopCueData` (types.ts)
        let number_to_id: std::collections::HashMap<String, crate::cue::types::CueId> = cues
            .iter()
            .filter_map(|c| c.number().map(|n| (n.to_string(), c.id())))
            .collect();
        for cue in &mut cues {
            cue.resolve_stop_target(&number_to_id);
            cue.resolve_fade_targets(&number_to_id);
        }

        let tc_config: CueListTcConfig = value
            .get("tc_config")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let tc_triggers: HashMap<CueId, TcTrigger> = value
            .get("tc_triggers")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter().filter_map(|item| {
                    let id: CueId = item.get("cue_id")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse().ok())?;
                    let trigger: TcTrigger = serde_json::from_value(
                        item.get("trigger")?.clone()).ok()?;
                    Some((id, trigger))
                }).collect()
            })
            .unwrap_or_default();

        Ok(Self {
            id,
            name,
            mode,
            cues,
            playhead_cue_id,
            tc_config,
            tc_triggers,
            tc_last_triggered_frame: u64::MAX,
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

    /// Build a list holding one group whose only child is a memo.
    /// Returns (list, group_id, child_id).
    fn list_with_group_child() -> (CueList, CueId, CueId) {
        let mut list = CueList::new("Test");
        let mut group = crate::cue::group_cue::GroupCue::new();
        let child = memo();
        let child_id = child.id();
        group.children.push(child);
        let group_id = group.id;
        list.push(Box::new(group));
        (list, group_id, child_id)
    }

    #[test]
    fn get_recursive_finds_nested_child() {
        let (list, _group_id, child_id) = list_with_group_child();
        // Top-level get cannot see a nested cue …
        assert!(list.get(&child_id).is_none());
        // … but the recursive variant can.
        assert!(list.get_recursive(&child_id).is_some());
    }

    #[test]
    fn remove_anywhere_removes_nested_child() {
        let (mut list, group_id, child_id) = list_with_group_child();
        list.remove_anywhere(&child_id).unwrap();
        assert!(list.get_recursive(&child_id).is_none());
        // The group is still present, now empty.
        let group = list.get(&group_id).expect("group still present");
        assert_eq!(group.child_cues().unwrap().len(), 0);
    }

    #[test]
    fn remove_many_anywhere_mixes_nested_and_top_level() {
        let (mut list, _group_id, child_id) = list_with_group_child();
        let top = memo();
        let top_id = top.id();
        list.push(top);
        list.remove_many_anywhere(&[child_id, top_id]).unwrap();
        assert!(list.get_recursive(&child_id).is_none());
        assert!(list.get(&top_id).is_none());
    }

    #[test]
    fn insert_after_anywhere_keeps_copy_in_group() {
        let (mut list, group_id, child_id) = list_with_group_child();
        let sibling = memo();
        let sibling_id = sibling.id();
        list.insert_after_anywhere(&child_id, sibling).unwrap();
        let group = list.get(&group_id).unwrap();
        let children = group.child_cues().unwrap();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].id(), child_id);
        assert_eq!(children[1].id(), sibling_id);
    }

    #[test]
    fn set_playhead_on_sequential_child_parks_on_group() {
        use crate::cue::types::GroupMode;
        let mut list = CueList::new("Test");
        let mut group = crate::cue::group_cue::GroupCue::new();
        group.mode = GroupMode::Sequential;
        let c0 = memo();
        let c1 = memo();
        let c1_id = c1.id();
        group.children.push(c0);
        group.children.push(c1);
        let group_id = group.id;
        list.push(Box::new(group));

        list.set_playhead(Some(c1_id)).unwrap();
        // Outer playhead parks on the group …
        assert_eq!(list.playhead_cue_id, Some(group_id));
        // … and the group's next-to-fire child is the one we picked.
        assert_eq!(list.get(&group_id).unwrap().active_child_id(), Some(c1_id));
    }
}
