//! Global application state shared across Tauri command handlers.
//!
//! All mutable state is wrapped in `Arc<Mutex<...>>` so it can be safely
//! accessed from multiple Tauri command handler threads.

use std::collections::HashSet;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

use uuid::Uuid;

use crate::{
    cue::{
        audio_cue::AudioCueFactory,
        fade_cue::FadeCueFactory,
        group_cue::GroupCueFactory,
        light_cue::LightCueFactory,
        midi_cue::MidiCueFactory,
        image_cue::ImageCueFactory,
        memo_cue::MemoCueFactory,
        osc_cue::OscCueFactory,
        registry::CueRegistry,
        stop_cue::StopCueFactory,
        types::CueType,
        video_cue::VideoCueFactory,
        wait_cue::WaitCueFactory,
    },
    engine::{
        AudioEngine, DmxEngine, OscServer, OutputEngine,
        timecode_receiver::TimecodeReceiver,
    },
    show::{undo_stack::UndoStack, Workspace},
};

/// The Tauri managed state object.
pub struct AppState {
    /// The current workspace (project file).
    pub workspace: Arc<Mutex<Workspace>>,
    /// The audio engine (shared; owns its own real-time thread internally).
    pub audio_engine: Arc<AudioEngine>,
    /// The unified output engine (video + image via libmpv Win32 window).
    pub output_engine: Arc<OutputEngine>,
    /// OSC receive server (background UDP listener thread).
    pub osc_server: Arc<OscServer>,
    /// DMX-over-IP lighting engine (owns its own ~40Hz output thread).
    pub dmx_engine: Arc<DmxEngine>,
    /// The cue type registry used for workspace de/serialisation.
    pub registry: Arc<Mutex<CueRegistry>>,
    /// Set of cue IDs whose audio files are currently being decoded in the
    /// background.  Used to show a "Loading…" indicator in the UI.
    pub loading_cues: Arc<Mutex<HashSet<Uuid>>>,
    /// Undo / redo history for the active cue list.
    pub undo_stack: Arc<Mutex<UndoStack>>,
    /// In-app clipboard: the last cue copied via Ctrl+C (serialised JSON).
    pub clipboard: Arc<Mutex<Option<serde_json::Value>>>,
    /// Timestamp of the last GO trigger in ms since Unix epoch.
    /// Used to enforce `double_go_protection_ms` — any GO within that window
    /// is silently dropped.  Lock-free so it adds zero latency to the hot path.
    pub last_go_at: Arc<AtomicU64>,
    /// Timecode receiver (MTC / LTC) — `None` until the first `set_tc_config`.
    pub tc_receiver: Arc<Mutex<Option<Arc<TimecodeReceiver>>>>,
}

impl AppState {
    /// Build the initial application state from already-constructed engines.
    pub fn new(
        audio_engine: Arc<AudioEngine>,
        output_engine: Arc<OutputEngine>,
        osc_server: Arc<OscServer>,
        dmx_engine: Arc<DmxEngine>,
        tc_receiver: Option<Arc<TimecodeReceiver>>,
    ) -> Self {
        let workspace = Workspace::new("Untitled");

        let mut registry = CueRegistry::new();
        registry.register(CueType::Audio, Box::new(AudioCueFactory));
        registry.register(CueType::Fade,  Box::new(FadeCueFactory));
        registry.register(CueType::Midi,  Box::new(MidiCueFactory));
        registry.register(CueType::Group, Box::new(GroupCueFactory));
        registry.register(CueType::Light, Box::new(LightCueFactory));
        registry.register(CueType::Memo, Box::new(MemoCueFactory));
        registry.register(CueType::Osc,   Box::new(OscCueFactory));
        registry.register(CueType::Stop, Box::new(StopCueFactory));
        registry.register(CueType::Video, Box::new(VideoCueFactory));
        registry.register(CueType::Image, Box::new(ImageCueFactory));
        registry.register(CueType::Mic,      Box::new(crate::cue::mic_cue::MicCueFactory));
        registry.register(CueType::Timecode, Box::new(crate::cue::timecode_cue::TimecodeCueFactory));
        registry.register(CueType::Wait, Box::new(WaitCueFactory));

        Self {
            workspace: Arc::new(Mutex::new(workspace)),
            audio_engine,
            output_engine,
            osc_server,
            dmx_engine,
            registry: Arc::new(Mutex::new(registry)),
            loading_cues: Arc::new(Mutex::new(HashSet::new())),
            undo_stack: Arc::new(Mutex::new(UndoStack::new())),
            clipboard: Arc::new(Mutex::new(None)),
            last_go_at: Arc::new(AtomicU64::new(0)),
            tc_receiver: Arc::new(Mutex::new(tc_receiver)),
        }
    }
}
