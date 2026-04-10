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
}

impl CueContext {
    /// Create a new context with the given engine and event sender.
    pub fn new(audio_engine: Arc<AudioEngine>, event_sender: Sender<CueEvent>) -> Self {
        Self {
            audio_engine,
            event_sender,
        }
    }

    /// Convenience: emit an event through the context without unwrapping.
    /// Silently drops the event if the receiver has been dropped.
    pub fn emit(&self, event: CueEvent) {
        let _ = self.event_sender.send(event);
    }
}
