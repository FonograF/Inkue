//! Global application state shared across Tauri command handlers.
//!
//! All mutable state is wrapped in `Arc<Mutex<...>>` so it can be safely
//! accessed from multiple Tauri command handler threads.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use uuid::Uuid;

use crate::{
    cue::{
        audio_cue::AudioCueFactory, memo_cue::MemoCueFactory, registry::CueRegistry,
        types::CueType,
    },
    engine::AudioEngine,
    show::Workspace,
};

/// The Tauri managed state object.
pub struct AppState {
    /// The current workspace (project file).
    pub workspace: Arc<Mutex<Workspace>>,
    /// The audio engine (shared; the engine owns its own thread internally).
    pub audio_engine: Arc<AudioEngine>,
    /// The cue type registry used for workspace de/serialisation.
    pub registry: Arc<Mutex<CueRegistry>>,
    /// Set of cue IDs whose audio files are currently being decoded in the
    /// background.  Used to show a "Loading…" indicator in the UI.
    pub loading_cues: Arc<Mutex<HashSet<Uuid>>>,
}

impl AppState {
    /// Build the initial application state:
    /// - Creates a blank workspace named "Untitled".
    /// - Starts the audio engine.
    /// - Registers all built-in cue type factories.
    pub fn new() -> anyhow::Result<Self> {
        let audio_engine = AudioEngine::new()?;

        let workspace = Workspace::new("Untitled");

        let mut registry = CueRegistry::new();
        registry.register(CueType::Audio, Box::new(AudioCueFactory));
        registry.register(CueType::Memo, Box::new(MemoCueFactory));

        Ok(Self {
            workspace: Arc::new(Mutex::new(workspace)),
            audio_engine,
            registry: Arc::new(Mutex::new(registry)),
            loading_cues: Arc::new(Mutex::new(HashSet::new())),
        })
    }
}
