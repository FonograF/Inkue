//! [`CueContext`] is passed to cue lifecycle methods so they can interact with
//! the audio engine and emit show-level events without direct coupling.

use std::sync::Arc;

use crossbeam_channel::Sender;

use crate::engine::AudioEngine;

/// Events emitted by cues to the Show Engine during execution.
#[derive(Debug, Clone)]
pub enum CueEvent {
    /// The cue's pre-wait has elapsed; the action is about to start.
    ActionStarted { cue_id: uuid::Uuid },
    /// The cue's action has completed naturally (e.g., audio file finished).
    ActionCompleted { cue_id: uuid::Uuid },
    /// The cue has been stopped externally.
    Stopped { cue_id: uuid::Uuid },
    /// The cue's post-wait has elapsed; continue mode should be evaluated.
    PostWaitElapsed { cue_id: uuid::Uuid },
    /// Emitted by [`StopCue`](crate::cue::stop_cue::StopCue) to request that
    /// the transport stop all currently-running cues.
    StopAll,
}

/// Shared context passed to every cue lifecycle call (`go`, `stop`, `pause`, …).
///
/// Provides access to the audio engine and a channel for emitting show events.
/// The struct is cheap to clone (all fields are `Arc` or `Sender`).
#[derive(Clone)]
pub struct CueContext {
    /// The audio engine, used by [`AudioCue`](crate::cue::audio_cue::AudioCue) to request a voice.
    pub audio_engine: Arc<AudioEngine>,
    /// Channel for signalling events back to the Show Engine / transport layer.
    pub event_sender: Sender<CueEvent>,
    /// Duration (ms) of the soft fade-out applied on Stop when the cue has no
    /// explicit `fade_out` spec set.  Comes from `AudioPreferences::default_fade_out_ms`.
    pub stop_fade_ms: u32,
}

impl CueContext {
    /// Create a new context with the given engine, event sender and stop-fade duration.
    pub fn new(audio_engine: Arc<AudioEngine>, event_sender: Sender<CueEvent>, stop_fade_ms: u32) -> Self {
        Self {
            audio_engine,
            event_sender,
            stop_fade_ms,
        }
    }

    /// Convenience: emit an event through the context without unwrapping.
    /// Silently drops the event if the receiver has been dropped.
    pub fn emit(&self, event: CueEvent) {
        let _ = self.event_sender.send(event);
    }
}
