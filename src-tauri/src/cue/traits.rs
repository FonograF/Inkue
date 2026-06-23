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
    types::{ContinueMode, CueColor, CueId, CueState, CueType, FadeAction, GroupMode},
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

/// Live audio parameters to re-apply to a cue's currently-playing voice after an
/// inspector edit, so volume/pan changes take effect without restarting
/// playback.  Returned by [`Cue::live_audio_params`].
#[derive(Debug, Clone, Copy)]
pub struct LiveAudioParams {
    /// The engine voice id to update.
    pub voice_id: CueId,
    /// Linear gain to apply (already converted from dB).
    pub gain: f32,
    /// Stereo pan (-1.0 .. 1.0).
    pub pan: f32,
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
    #[allow(clippy::wrong_self_convention)]
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

    /// Whether this cue is disabled.  Disabled cues are skipped by the
    /// transport — the Playhead advances past them automatically.
    fn is_disabled(&self) -> bool {
        false
    }

    /// Enable or disable this cue.
    fn set_disabled(&mut self, _disabled: bool) {}

    /// Optional timecode trigger: the SMPTE position at which this cue fires
    /// when the CueList's TC sync is enabled.  `None` = not TC-triggered.
    fn tc_trigger(&self) -> Option<&crate::engine::timecode_types::TcTrigger> {
        None
    }

    /// Set or clear the TC trigger for this cue.
    fn set_tc_trigger(&mut self, _trigger: Option<crate::engine::timecode_types::TcTrigger>) {}

    /// For media cues (Audio, Video, Image): the file path as stored in the
    /// cue (may be relative to the workspace directory).  Used by the command
    /// layer to detect broken cues without calling `serialize()`.
    fn media_file_path(&self) -> Option<&std::path::Path> {
        None
    }

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

    /// Seek to `position_ms` from the start of the cue's action.
    ///
    /// For audio cues this repositions the audio voice.  For video cues it
    /// issues an mpv seek and re-anchors the paired audio voice.  Non-seekable
    /// cue types (Memo, Stop, …) use the default no-op.
    ///
    /// The caller is responsible for updating any transport-level timing only
    /// when the cue is actually running or paused; calling seek on a standby
    /// cue has no effect.
    fn seek(&mut self, _position_ms: u64, _ctx: &CueContext) {}

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

    /// Monotonically increasing counter incremented on every `go()` call.
    /// Reserved for diagnostics / future use.  Default: 0.
    fn play_generation(&self) -> u64 {
        0
    }

    /// Returns `true` if Auto-Continue has already been fired for the
    /// **current** play of this cue.  The transport sets this flag
    /// synchronously inside `go()` before chaining, so the event loop
    /// never sees the cue as needing a second chain.
    fn is_auto_continue_fired(&self) -> bool {
        false
    }

    /// Mark Auto-Continue as fired for the current play.
    /// Called by [`Transport::go`] immediately after chaining.
    fn mark_auto_continue_fired(&mut self) {}

    /// Reset the Auto-Continue fired flag.  Called by `go()` (new play) and
    /// `reset()` / `stop()` (cue stopped or completed).
    fn clear_auto_continue_fired(&mut self) {}

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

    /// Called when the underlying media's total duration becomes known at
    /// runtime (e.g., after a video file's metadata loads in the surface
    /// window).  Non-video cues can ignore this call (default no-op).
    fn set_runtime_duration(&mut self, _duration: std::time::Duration) {}

    /// If `true`, [`Transport::go`] will automatically stop this cue when the
    /// next GO fires.  Default: `false`.  Image cues override this based on
    /// their configured stop mode.
    fn stop_on_next_go(&self) -> bool {
        false
    }

    /// Fade Cue only: returns the fade parameters so the transport can resolve
    /// target voices and call [`set_fade_voices`] before the first tick.
    fn fade_specification(&self) -> Option<FadeAction> {
        None
    }

    /// Inject resolved audio voice IDs (and their current gains) into this cue,
    /// plus visual fade parameters for any Video/Image targets.
    ///
    /// - `voices`: `(audio_voice_id, start_gain)` for each audio/video target.
    /// - `has_visual`: true when at least one target is a Video or Image cue.
    /// - `visual_start_alpha`: current overlay alpha at GO time (read from OutputEngine).
    /// - `visual_target_alpha`: desired overlay alpha when fade completes.
    ///
    /// Called by [`crate::show::transport::Transport::go`] after `go()` so that
    /// `tick()` knows which voices/overlay to update.
    fn set_fade_voices(
        &mut self,
        _voices: Vec<(CueId, f32)>,
        _has_visual: bool,
        _visual_start_alpha: u8,
        _visual_target_alpha: u8,
    ) {}

    /// Stop Cue only: describes what to stop after `go()` completes.
    ///
    /// Returns `Some((hard_stop, target_cue_ids))` where:
    /// - `hard_stop` — `true` = immediate cut, `false` = soft fade.
    /// - `target_cue_ids` — empty = stop all, non-empty = stop those UUIDs only.
    ///
    /// Transport reads this and executes the stop **before** evaluating
    /// Auto-Follow chains, preventing the chained cue from being killed.
    fn stop_specification(&self) -> Option<(bool, Vec<CueId>)> {
        None
    }

    /// Resolve stop/fade targets from cue-number strings to UUIDs.
    ///
    /// Called once per cue after the whole cue list is loaded, allowing cues
    /// saved in the old format (number only, no UUID) to be upgraded
    /// in-memory for the current session.  Default implementation is a no-op.
    fn resolve_stop_target(&mut self, _number_to_id: &std::collections::HashMap<String, CueId>) {}

    /// Fade Cue only: resolve target UUIDs from cue-number labels.
    fn resolve_fade_targets(&mut self, _number_to_id: &std::collections::HashMap<String, CueId>) {}

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

    /// Live audio parameters to push to the cue's currently-playing voice after
    /// an inspector edit (volume / pan), so changes apply without restarting.
    /// Returns `None` when the cue has no live voice.  Default is `None`.
    fn live_audio_params(&self) -> Option<LiveAudioParams> {
        None
    }

    // -----------------------------------------------------------------------
    // Group support
    // -----------------------------------------------------------------------

    /// Returns `true` once the cue has naturally finished all of its work and
    /// is ready to be reset.  The default (`false`) means the event loop uses
    /// voice-completion and time-based detection instead.
    ///
    /// [`GroupCue`](crate::cue::group_cue::GroupCue) overrides this: it
    /// becomes `true` when every child has completed.
    fn is_complete(&self) -> bool {
        false
    }

    /// Read-only view of direct child cues.  Returns `None` for non-Group cues.
    fn child_cues(&self) -> Option<&[Box<dyn Cue>]> {
        None
    }

    /// Mutable view of direct child cues.  Returns `None` for non-Group cues.
    fn child_cues_mut(&mut self) -> Option<&mut Vec<Box<dyn Cue>>> {
        None
    }

    /// Consume and return all children (for `ungroup`).  Returns `None` for
    /// non-Group cues.
    fn take_children(&mut self) -> Option<Vec<Box<dyn Cue>>> {
        None
    }

    /// Add a child cue at `position` (−1 = append).  Returns `Err` for
    /// non-Group cues.
    fn add_child(&mut self, _child: Box<dyn Cue>, _position: i32) -> anyhow::Result<()> {
        Err(anyhow::anyhow!("Not a Group cue"))
    }

    /// Remove and return the child with the given ID.  Returns `Err` for
    /// non-Group cues or if the child is not found.
    fn remove_child(&mut self, _id: &CueId) -> anyhow::Result<Box<dyn Cue>> {
        Err(anyhow::anyhow!("Not a Group cue"))
    }

    /// The mode of this Group cue (`None` for non-Group cues).
    fn group_mode(&self) -> Option<GroupMode> {
        None
    }

    /// Update the Group mode.  No-op for non-Group cues.
    fn set_group_mode(&mut self, _mode: GroupMode) {}

    /// Returns `true` if this cue wants to consume the next outer GO press
    /// without the transport advancing the Playhead.
    ///
    /// Only [`GroupCue`](crate::cue::group_cue::GroupCue) in Sequential mode
    /// overrides this: when the internal sequence has paused at a
    /// `DoNotContinue` child and more children remain, it absorbs GO to fire
    /// the next child internally instead of advancing the outer Playhead.
    fn absorbs_go(&self) -> bool {
        false
    }

    /// Returns `true` if this cue retains the outer Playhead on itself while
    /// it is running, so that subsequent GO presses are routed into its own
    /// internal sequence rather than advancing the outer Playhead.
    ///
    /// Only Sequential [`GroupCue`] overrides this.  The event loop is
    /// responsible for advancing the outer Playhead once the cue completes.
    fn holds_playhead(&self) -> bool {
        false
    }

    /// Returns `true` once a cue that held the outer Playhead has fired
    /// everything it will fire and the Playhead should now move on to the next
    /// outer cue — even though this cue may still be running (e.g. overlapping
    /// audio children still playing out).
    ///
    /// Only Sequential [`GroupCue`] overrides this: it becomes `true` the moment
    /// its **last** child is fired, so the next GO continues the outer list
    /// instead of being absorbed.  The transport (on GO) and the event loop (on
    /// auto-advance) both consult it to release the Playhead.
    fn released_playhead(&self) -> bool {
        false
    }

    /// For a running Sequential [`GroupCue`]: the ID of the child that is
    /// currently active — either running right now, or the next one to fire on
    /// GO (when the sequence is paused at a `DoNotContinue` child).
    ///
    /// Returns `None` for non-Group cues and for Simultaneous groups (the
    /// frontend derives activity from each child's own `state()` instead).
    fn active_child_id(&self) -> Option<CueId> {
        None
    }

    /// Point a Sequential [`GroupCue`]'s internal playhead at `child_id` so the
    /// next GO fires that child (and the sequence continues from there).
    ///
    /// Returns `true` if `child_id` is a direct child of a Sequential group.
    /// Default: `false` (non-Group cues and Simultaneous groups ignore it).
    fn set_active_child(&mut self, child_id: &CueId) -> bool {
        let _ = child_id;
        false
    }

    // -----------------------------------------------------------------------
    // Serialisation
    // -----------------------------------------------------------------------

    /// Serialise this cue to a JSON [`Value`] for `.wincue` file persistence.
    /// The returned object must include a `"type"` field matching
    /// [`CueType`]'s serialised form.
    fn serialize(&self) -> Value;
}
