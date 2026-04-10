//! The [`Cue`] trait — the universal contract for every cue type in WinCue.
//!
//! All cue types (Audio, Memo, Wait, Group, …) implement this trait so that the
//! Show Engine can drive them uniformly through `dyn Cue`.  The trait is
//! **object-safe**: every method either takes `&self`/`&mut self` or returns
//! a type with a known size.

use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::Value;

use super::{
    context::CueContext,
    types::{ContinueMode, CueColor, CueId, CueState, CueType},
};

// ---------------------------------------------------------------------------
// RuntimeState — volatile playback state that survives a cue rebuild
// ---------------------------------------------------------------------------

/// Snapshot of the volatile runtime state that must survive a cue rebuild
/// performed by `update_cue`.  Captured from the old instance and injected
/// into the freshly-rebuilt one so a running cue is not interrupted.
pub struct RuntimeState {
    pub state: CueState,
    /// Active audio voice ID (audio cues only).
    pub voice_id: Option<CueId>,
    /// Instant when `go()` was called (start of pre-wait).
    pub started_at: Option<Instant>,
    /// Instant when the action began (after pre-wait expired).
    pub action_started_at: Option<Instant>,
}

// ---------------------------------------------------------------------------
// CueFactory trait
// ---------------------------------------------------------------------------

/// Factory for a specific cue type.  Each cue type registers one factory in
/// the [`super::registry::CueRegistry`].
pub trait CueFactory: Send + Sync {
    /// Create a new, empty cue of this type with a fresh UUID.
    fn create(&self) -> Box<dyn Cue>;

    /// Deserialise a cue from its JSON representation.
    fn from_json(&self, value: Value) -> Result<Box<dyn Cue>>;
}

// ---------------------------------------------------------------------------
// Cue trait
// ---------------------------------------------------------------------------

/// The universal cue contract.  Implementors must be `Send` so they can be
/// moved across thread boundaries (e.g., when loading a workspace on a worker
/// thread).
pub trait Cue: Send {
    // -----------------------------------------------------------------------
    // Identity
    // -----------------------------------------------------------------------

    /// Unique identifier for this cue instance.
    fn id(&self) -> CueId;

    /// The discriminant type of this cue (Audio, Memo, …).
    fn cue_type(&self) -> CueType;

    /// Human-readable name of the cue (editable by the operator).
    fn name(&self) -> &str;

    /// Update the cue's name.
    fn set_name(&mut self, name: String);

    /// Optional alphanumeric cue number (e.g. "1", "1.5", "Intro").
    /// This is a *string*, not a numeric index.
    fn number(&self) -> Option<&str>;

    /// Update the cue number.  Pass `None` to clear it.
    fn set_number(&mut self, number: Option<String>);

    /// Free-form notes visible in the inspector.
    fn notes(&self) -> &str;

    /// Update the notes field.
    fn set_notes(&mut self, notes: String);

    /// Colour label shown on the cue row in the Cue List.
    fn color(&self) -> CueColor;

    /// Update the colour label.
    fn set_color(&mut self, color: CueColor);

    // -----------------------------------------------------------------------
    // State
    // -----------------------------------------------------------------------

    /// Current lifecycle state.
    fn state(&self) -> CueState;

    /// `true` if the cue is currently in [`CueState::Running`].
    fn is_running(&self) -> bool {
        self.state() == CueState::Running
    }

    /// `true` if the cue is currently in [`CueState::Paused`].
    fn is_paused(&self) -> bool {
        self.state() == CueState::Paused
    }

    // -----------------------------------------------------------------------
    // Lifecycle
    // -----------------------------------------------------------------------

    /// Pre-load any resources needed for fast execution (e.g., decode audio
    /// into memory).  Called when the workspace is opened or when the
    /// operator manually loads a cue.
    fn load(&mut self, context: &CueContext) -> Result<()>;

    /// Trigger the cue at the Playhead.  Starts the pre-wait timer.
    fn go(&mut self, context: &CueContext) -> Result<()>;

    /// Stop the cue with a short fade-out (default 0.5 s).  Resets to Standby.
    fn stop(&mut self, context: &CueContext) -> Result<()>;

    /// Suspend execution mid-action.
    fn pause(&mut self, context: &CueContext) -> Result<()>;

    /// Resume a paused cue.
    fn resume(&mut self, context: &CueContext) -> Result<()>;

    /// Immediately cut playback without any fade.  Used on double-Escape.
    fn hard_stop(&mut self, context: &CueContext) -> Result<()>;

    /// Reset the cue to its initial Standby state (clears elapsed time etc.).
    fn reset(&mut self) -> Result<()>;

    /// Called by the event loop at ~30 fps for every Running cue.
    ///
    /// The default implementation is a no-op.  Audio cues override this to
    /// handle the pre-wait phase: once `pre_wait` has elapsed the audio
    /// action starts without `go()` having to block on a timer.
    fn tick(&mut self, _context: &CueContext) -> Result<()> {
        Ok(())
    }

    /// Returns `false` while the cue is in its Pre-Wait phase (i.e. `go()`
    /// has been called but the action has not yet started).
    ///
    /// The event loop uses this to avoid firing Auto-Continue before the
    /// action — and therefore Post-Wait — has actually begun.
    /// Default: `true` (most cue types start their action synchronously).
    fn is_action_started(&self) -> bool {
        true
    }

    // -----------------------------------------------------------------------
    // Timing
    // -----------------------------------------------------------------------

    /// Delay inserted before the cue's action begins.
    fn pre_wait(&self) -> Duration;

    /// Update the Pre-Wait duration.
    fn set_pre_wait(&mut self, d: Duration);

    /// Delay after the action *starts* (not ends) before continue mode fires.
    fn post_wait(&self) -> Duration;

    /// Update the Post-Wait duration.
    fn set_post_wait(&mut self, d: Duration);

    /// Total duration of the cue's action, if known in advance.
    /// Returns `None` for open-ended cues (e.g., looping audio).
    fn duration(&self) -> Option<Duration>;

    /// Total time elapsed since `go()` was called (including pre-wait).
    fn elapsed(&self) -> Duration;

    /// Time elapsed since the action started (i.e., after pre-wait).
    fn action_elapsed(&self) -> Duration;

    // -----------------------------------------------------------------------
    // Continue mode
    // -----------------------------------------------------------------------

    /// What happens after this cue's Post-Wait expires.
    fn continue_mode(&self) -> ContinueMode;

    /// Update the continue mode.
    fn set_continue_mode(&mut self, mode: ContinueMode);

    // -----------------------------------------------------------------------
    // Runtime helpers
    // -----------------------------------------------------------------------

    /// Inject pre-decoded audio samples that were decoded *outside* the
    /// workspace mutex.  The caller decodes on a background thread, then
    /// briefly re-acquires the mutex to call this method.  Non-audio cues
    /// ignore the call (default no-op).
    fn accept_preloaded_audio(
        &mut self,
        _samples: std::sync::Arc<Vec<f32>>,
        _channels: u16,
        _sample_rate: u32,
        _duration: std::time::Duration,
    ) {
    }

    /// Returns the active audio voice ID if this cue is currently playing
    /// through the audio engine.  Non-audio cues return `None` (default).
    /// Used by the event loop to correlate [`crate::engine::ring_command::AudioStatus::Completed`]
    /// events back to the owning cue.
    fn playing_voice_id(&self) -> Option<CueId> {
        None
    }

    /// Full duration of the underlying source file, **without** start/end
    /// markers applied.  Audio cues override this; other types return `duration()`.
    fn file_duration(&self) -> Option<Duration> {
        self.duration()
    }

    /// Return a cheap clone of the pre-decoded audio data already in memory,
    /// or `None` if the cue has not been decoded yet.
    ///
    /// Used by [`update_cue`](crate::commands::cue_cmds::update_cue) to
    /// preserve decoded samples across cue rebuilds (name/colour/timing
    /// changes must not force a re-decode).  Non-audio cues return `None`.
    fn extract_decoded_audio(
        &self,
    ) -> Option<(std::sync::Arc<Vec<f32>>, u16, u32, Duration)> {
        None
    }

    /// Downsample the decoded audio into `bins` peak values (0.0 – 1.0) for
    /// waveform display.  Returns `None` if no audio data is loaded yet.
    /// Non-audio cues always return `None` (default).
    fn waveform_peaks(&self, _bins: usize) -> Option<Vec<f32>> {
        None
    }

    /// Capture the volatile runtime state so it can be transplanted into a
    /// freshly-rebuilt instance.  Called by `update_cue` just before the
    /// old cue is replaced.  Default returns a Standby snapshot with no voice
    /// or timing — cue types that carry runtime state must override this.
    fn runtime_state(&self) -> RuntimeState {
        RuntimeState {
            state: self.state(),
            voice_id: self.playing_voice_id(),
            started_at: None,
            action_started_at: None,
        }
    }

    /// Inject a previously captured [`RuntimeState`] into this instance.
    /// Called by `update_cue` after rebuilding so a running cue continues
    /// uninterrupted.  Default is a no-op.
    fn restore_runtime_state(&mut self, _snap: RuntimeState) {}

    // -----------------------------------------------------------------------
    // Serialisation
    // -----------------------------------------------------------------------

    /// Serialise this cue to a JSON [`Value`] for `.wincue` file persistence.
    /// The returned object must include a `"type"` field matching
    /// [`CueType`]'s serialised form.
    fn serialize(&self) -> Value;
}
