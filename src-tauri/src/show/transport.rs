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
    /// Sequence:
    /// 1. Read the cue at the Playhead.
    /// 2. Advance the Playhead to the next cue.
    /// 3. Call `cue.go()`.
    /// 4. If the cue uses Auto-Follow, also trigger the next cue.
    pub fn go(&mut self, cue_list: &mut CueList) -> Result<Option<CueId>> {
        let cue_id = match cue_list.playhead_cue_id {
            Some(id) => id,
            None => return Ok(None), // Nothing at the playhead.
        };

        // Advance playhead before triggering (matches QLab behaviour).
        cue_list.advance_playhead();

        // Trigger the cue.
        let cue = cue_list
            .get_mut(&cue_id)
            .ok_or_else(|| anyhow!("Cue not found: {:?}", cue_id))?;

        cue.go(&self.context)?;

        let continue_mode = cue.continue_mode();
        let post_wait = cue.post_wait();
        let is_still_running = cue.state() == CueState::Running;

        // For instant cues that complete synchronously inside go() (e.g. MemoCue,
        // any cue whose action finishes before go() returns):
        //   - AutoContinue with post_wait = 0 → fire next immediately.
        //   - AutoFollow → fire next immediately (the cue is already done).
        //
        // For running cues (audio playing, pre-wait pending, …):
        //   - AutoContinue → the 30fps event loop fires GO when action_elapsed >= post_wait.
        //   - AutoFollow   → the 30fps event loop fires GO when the voice completes.
        if !is_still_running
            && (continue_mode == ContinueMode::AutoFollow
                || (continue_mode == ContinueMode::AutoContinue && post_wait.is_zero()))
        {
            self.go(cue_list)?;
        }

        Ok(Some(cue_id))
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
