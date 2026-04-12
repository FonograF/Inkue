//! Global application state shared across Tauri command handlers.
//!
//! All mutable state is wrapped in `Arc<Mutex<...>>` so it can be safely
//! accessed from multiple Tauri command handler threads.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use uuid::Uuid;

use crate::{
    cue::{
        audio_cue::AudioCueFactory,
        memo_cue::MemoCueFactory,
        registry::CueRegistry,
        stop_cue::StopCueFactory,
        types::CueType,
        video_cue::VideoCueFactory,
    },
    engine::{AudioEngine, VideoEngine},
    show::{undo_stack::UndoStack, Workspace},
};

/// The Tauri managed state object.
pub struct AppState {
    /// The current workspace (project file).
    pub workspace: Arc<Mutex<Workspace>>,
    /// The audio engine (shared; owns its own real-time thread internally).
    pub audio_engine: Arc<AudioEngine>,
    /// The video engine (shared; manages output surface windows).
    pub video_engine: Arc<VideoEngine>,
    /// The cue type registry used for workspace de/serialisation.
    pub registry: Arc<Mutex<CueRegistry>>,
    /// Set of cue IDs whose audio files are currently being decoded in the
    /// background.  Used to show a "Loading…" indicator in the UI.
    pub loading_cues: Arc<Mutex<HashSet<Uuid>>>,
    /// Undo / redo history for the active cue list.
    pub undo_stack: Arc<Mutex<UndoStack>>,
    /// In-app clipboard: the last cue copied via Ctrl+C (serialised JSON).
    pub clipboard: Arc<Mutex<Option<serde_json::Value>>>,
}

impl AppState {
    /// Build the initial application state from already-constructed engines.
    ///
    /// The engines are created in `lib.rs`'s Tauri `setup` closure (not here)
    /// so that [`VideoEngine`] can receive the [`tauri::AppHandle`] it needs to
    /// emit events to surface windows.
    pub fn new(audio_engine: Arc<AudioEngine>, video_engine: Arc<VideoEngine>) -> Self {
        let workspace = Workspace::new("Untitled");

        let mut registry = CueRegistry::new();
        registry.register(CueType::Audio, Box::new(AudioCueFactory));
        registry.register(CueType::Memo, Box::new(MemoCueFactory));
        registry.register(CueType::Stop, Box::new(StopCueFactory));
        registry.register(CueType::Video, Box::new(VideoCueFactory));

        Self {
            workspace: Arc::new(Mutex::new(workspace)),
            audio_engine,
            video_engine,
            registry: Arc::new(Mutex::new(registry)),
            loading_cues: Arc::new(Mutex::new(HashSet::new())),
            undo_stack: Arc::new(Mutex::new(UndoStack::new())),
            clipboard: Arc::new(Mutex::new(None)),
        }
    }
}
