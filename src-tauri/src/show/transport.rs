//! Show transport — GO, STOP, PAUSE, and continue mode chaining.
//!
//! The [`Transport`] struct is the primary interface between the UI commands
//! and the cue execution system.  It holds a reference to the active
//! [`CueList`] and a [`CueContext`] so it can drive cue lifecycle methods.

use anyhow::{anyhow, Result};

use crate::cue::{
    context::CueContext,
    types::{ContinueMode, CueId, CueState, CueType},
};

use super::cue_list::CueList;

/// Result returned by [`Transport::go`].
pub struct GoResult {
    /// IDs of all cues triggered by this GO (including chained Auto-Continue /
    /// Auto-Follow cues).  The first element is always the primary cue.
    pub triggered: Vec<CueId>,
    /// IDs of cues stopped by a Stop Cue's action during this GO.
    pub stopped: Vec<CueId>,
}

/// Manages playback state for a single [`CueList`].
pub struct Transport {
    pub context: CueContext,
}

impl Transport {
    /// Create a new transport bound to the given context.
    pub fn new(context: CueContext) -> Self {
        Self { context }
    }

    // -----------------------------------------------------------------------
    // GO
    // -----------------------------------------------------------------------

    /// Trigger the cue at the Playhead.
    ///
    /// Returns a [`GoResult`] containing:
    /// - `triggered`: IDs of **all** cues fired in this call (primary + chains).
    /// - `stopped`: IDs of cues stopped by a Stop Cue action.
    ///
    /// Sequence:
    /// 1. Read the cue at the Playhead.
    /// 2. Advance the Playhead to the next cue.
    /// 3. Stop any running cues with `stop_on_next_go()` (visual-only logic).
    /// 4. Call `cue.go()`.
    /// 5. Execute any stop action declared by `cue.stop_specification()` — this
    ///    runs **before** chain evaluation so Auto-Follow cannot start a cue that
    ///    the Stop Cue would immediately kill.
    /// 6. Chain via Auto-Continue (post_wait = 0) or instant Auto-Follow.
    pub fn go(&mut self, cue_list: &mut CueList) -> Result<GoResult> {
        let cue_id = match cue_list.playhead_cue_id {
            Some(id) => id,
            None => return Ok(GoResult { triggered: vec![], stopped: vec![] }),
        };

        // If the cue wants to absorb this GO (e.g., a Sequential Group paused
        // mid-sequence), delegate to the cue and skip outer Playhead advancement.
        if cue_list.get(&cue_id).is_some_and(|c| c.absorbs_go()) {
            if let Some(cue) = cue_list.get_mut(&cue_id) {
                cue.go(&self.context)?;
            }
            return Ok(GoResult { triggered: vec![cue_id], stopped: vec![] });
        }

        // Advance playhead before triggering (matches QLab behaviour).
        cue_list.advance_playhead();

        // Stop any running cues that should automatically stop on the next GO.
        // Visual cues (Image, Video) only stop when the incoming cue is also
        // visual — an audio GO must not cut a displayed image.
        let incoming_is_visual = cue_list
            .get(&cue_id)
            .map(|c| matches!(c.cue_type(), CueType::Video | CueType::Image))
            .unwrap_or(false);

        let stop_ids: Vec<CueId> = cue_list
            .cues
            .iter()
            .filter(|c| {
                if !c.is_running() || c.id() == cue_id || !c.stop_on_next_go() {
                    return false;
                }
                let c_is_visual = matches!(c.cue_type(), CueType::Video | CueType::Image);
                !c_is_visual || incoming_is_visual
            })
            .map(|c| c.id())
            .collect();
        for id in stop_ids {
            if let Some(cue) = cue_list.get_mut(&id) {
                let _ = cue.stop(&self.context);
            }
        }

        // Trigger the cue.
        {
            let cue = cue_list
                .get_mut(&cue_id)
                .ok_or_else(|| anyhow!("Cue not found: {:?}", cue_id))?;
            cue.go(&self.context)?;
        }

        // Execute the stop action declared by Stop Cues **before** evaluating
        // Auto-Follow, so the chained cue is not immediately killed.
        let stop_spec = cue_list.get(&cue_id).and_then(|c| c.stop_specification());
        let mut stopped: Vec<CueId> = Vec::new();
        if let Some((hard, target)) = stop_spec {
            let ids_to_stop: Vec<CueId> = match &target {
                None => cue_list
                    .cues
                    .iter()
                    .filter(|c| (c.is_running() || c.is_paused()) && c.id() != cue_id)
                    .map(|c| c.id())
                    .collect(),
                Some(num) => cue_list
                    .cues
                    .iter()
                    .filter(|c| c.number() == Some(num.as_str()) && c.id() != cue_id)
                    .map(|c| c.id())
                    .collect(),
            };
            for id in &ids_to_stop {
                if let Some(c) = cue_list.get_mut(id) {
                    if hard {
                        let _ = c.hard_stop(&self.context);
                    } else {
                        let _ = c.stop(&self.context);
                    }
                }
            }
            stopped = ids_to_stop;
        }

        // Read continue-mode metadata after go() (state may have changed for
        // instant cues that complete synchronously).
        let (continue_mode, post_wait, is_still_running, holds_playhead) = cue_list
            .cues
            .iter()
            .find(|c| c.id() == cue_id)
            .map(|c| (c.continue_mode(), c.post_wait(), c.state() == CueState::Running, c.holds_playhead()))
            .ok_or_else(|| anyhow!("Cue not found after go: {:?}", cue_id))?;

        // Sequential groups retain the outer Playhead while running so that
        // subsequent GOs are routed into the group's internal sequence via
        // absorbs_go().  The advance_playhead() already moved the Playhead
        // forward; we move it back here.  The event loop will advance it again
        // once the group completes.
        if is_still_running && holds_playhead {
            cue_list.playhead_cue_id = Some(cue_id);
        }

        // Determine whether to chain immediately:
        //
        // • AutoContinue + post_wait = 0 → always chain now (audio or instant).
        //   Mark the flag so the event loop skips this cue.
        // • AutoFollow on an instant cue  → chain now (cue already completed).
        //   Audio AutoFollow is handled by the event loop on voice completion.
        let chain_now = (continue_mode == ContinueMode::AutoContinue && post_wait.is_zero())
            || (!is_still_running && continue_mode == ContinueMode::AutoFollow);

        if continue_mode == ContinueMode::AutoContinue && post_wait.is_zero() {
            if let Some(cue) = cue_list.get_mut(&cue_id) {
                cue.mark_auto_continue_fired();
            }
        }

        let mut triggered = vec![cue_id];

        if chain_now {
            let mut rest = self.go(cue_list)?;
            triggered.append(&mut rest.triggered);
            stopped.extend(rest.stopped);
        }

        Ok(GoResult { triggered, stopped })
    }

    // -----------------------------------------------------------------------
    // STOP / PAUSE / RESUME
    // -----------------------------------------------------------------------

    /// Stop a specific cue (with soft fade-out).
    pub fn stop_cue(&mut self, cue_list: &mut CueList, cue_id: &CueId) -> Result<()> {
        let cue = cue_list
            .get_mut(cue_id)
            .ok_or_else(|| anyhow!("Cue not found: {:?}", cue_id))?;
        cue.stop(&self.context)
    }

    /// Hard-stop a specific cue (immediate cut, no fade).
    pub fn hard_stop_cue(&mut self, cue_list: &mut CueList, cue_id: &CueId) -> Result<()> {
        let cue = cue_list
            .get_mut(cue_id)
            .ok_or_else(|| anyhow!("Cue not found: {:?}", cue_id))?;
        cue.hard_stop(&self.context)
    }

    /// Stop all running cues with a soft fade-out.
    pub fn stop_all(&mut self, cue_list: &mut CueList) -> Result<()> {
        let running_ids: Vec<CueId> = cue_list
            .cues
            .iter()
            .filter(|c| c.is_running() || c.is_paused())
            .map(|c| c.id())
            .collect();

        for id in running_ids {
            if let Some(cue) = cue_list.get_mut(&id) {
                let _ = cue.stop(&self.context);
            }
        }
        Ok(())
    }

    /// Hard-stop all running cues (no fades).
    pub fn hard_stop_all(&mut self, cue_list: &mut CueList) -> Result<()> {
        let running_ids: Vec<CueId> = cue_list
            .cues
            .iter()
            .filter(|c| c.is_running() || c.is_paused())
            .map(|c| c.id())
            .collect();

        for id in running_ids {
            if let Some(cue) = cue_list.get_mut(&id) {
                let _ = cue.hard_stop(&self.context);
            }
        }
        Ok(())
    }

    /// Pause a specific cue.
    pub fn pause_cue(&mut self, cue_list: &mut CueList, cue_id: &CueId) -> Result<()> {
        let cue = cue_list
            .get_mut(cue_id)
            .ok_or_else(|| anyhow!("Cue not found: {:?}", cue_id))?;
        cue.pause(&self.context)
    }

    /// Resume a paused cue.
    pub fn resume_cue(&mut self, cue_list: &mut CueList, cue_id: &CueId) -> Result<()> {
        let cue = cue_list
            .get_mut(cue_id)
            .ok_or_else(|| anyhow!("Cue not found: {:?}", cue_id))?;
        cue.resume(&self.context)
    }
}
