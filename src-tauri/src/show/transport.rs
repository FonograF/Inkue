//! Show transport — GO, STOP, PAUSE, and continue mode chaining.
//!
//! The [`Transport`] struct is the primary interface between the UI commands
//! and the cue execution system.  It holds a reference to the active
//! [`CueList`] and a [`CueContext`] so it can drive cue lifecycle methods.

use anyhow::{anyhow, Result};

use crate::cue::{
    context::CueContext,
    types::{ContinueMode, CueId, CueState},
};

use super::cue_list::CueList;

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
    /// Returns the IDs of **all** cues triggered in this call, including any
    /// cues chained immediately via Auto-Continue (post_wait = 0) or via
    /// Auto-Follow on an instant (synchronously-completing) cue.
    ///
    /// Sequence:
    /// 1. Read the cue at the Playhead.
    /// 2. Advance the Playhead to the next cue.
    /// 3. Call `cue.go()`.
    /// 4. If the cue has **AutoContinue + post_wait = 0** (audio *or* instant):
    ///    mark it as fired and immediately chain to the next cue — no 30 fps
    ///    delay.  The event loop will not re-fire because `is_auto_continue_fired`
    ///    is already `true`.
    /// 5. If the cue is an **instant cue with AutoFollow** (already done): chain
    ///    immediately.
    /// 6. Delayed chains (post_wait > 0) and audio AutoFollow are handled by
    ///    the 30 fps event loop.
    pub fn go(&mut self, cue_list: &mut CueList) -> Result<Vec<CueId>> {
        let cue_id = match cue_list.playhead_cue_id {
            Some(id) => id,
            None => return Ok(vec![]), // Nothing at the playhead.
        };

        // Advance playhead before triggering (matches QLab behaviour).
        cue_list.advance_playhead();

        // Stop any running cues that should automatically stop on the next GO
        // (e.g. Image Cues in StopOnNextCue mode).
        let stop_ids: Vec<CueId> = cue_list
            .cues
            .iter()
            .filter(|c| c.is_running() && c.stop_on_next_go() && c.id() != cue_id)
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

        // Read continue-mode metadata after go() (state may have changed for
        // instant cues that complete synchronously).
        let (continue_mode, post_wait, is_still_running) = cue_list
            .cues
            .iter()
            .find(|c| c.id() == cue_id)
            .map(|c| (c.continue_mode(), c.post_wait(), c.state() == CueState::Running))
            .ok_or_else(|| anyhow!("Cue not found after go: {:?}", cue_id))?;

        // Determine whether to chain immediately:
        //
        // • AutoContinue + post_wait = 0 → always chain now (audio or instant).
        //   Mark the flag so the event loop skips this cue.
        // • AutoFollow on an instant cue  → chain now (cue already completed).
        //   Audio AutoFollow is handled by the event loop on voice completion.
        let chain_now = (continue_mode == ContinueMode::AutoContinue && post_wait.is_zero())
            || (!is_still_running && continue_mode == ContinueMode::AutoFollow);

        if continue_mode == ContinueMode::AutoContinue && post_wait.is_zero() {
            // Mark before chaining so the event loop never fires a duplicate GO.
            if let Some(cue) = cue_list.get_mut(&cue_id) {
                cue.mark_auto_continue_fired();
            }
        }

        let mut triggered = vec![cue_id];

        if chain_now {
            let mut rest = self.go(cue_list)?;
            triggered.append(&mut rest);
        }

        Ok(triggered)
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
