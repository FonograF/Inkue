//! [`CueContext`] is passed to cue lifecycle methods so they can interact with
//! the audio engine, the output engine, and emit show-level events without
//! direct coupling.

use std::sync::Arc;

use crossbeam_channel::Sender;

use crate::engine::{
    audio_input::InputPatch, device_manager::OutputPatch, fixture::{FixtureGroup, PatchedFixture},
    osc_patch::OscPatch, output_engine::OutputEngine, AudioEngine, DmxEngine,
};

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
/// Provides access to the audio engine, the output engine, and a channel for
/// emitting show events.  The struct is cheap to clone (all fields are `Arc`
/// or `Sender`).
#[derive(Clone)]
pub struct CueContext {
    /// The audio engine, used by [`AudioCue`](crate::cue::audio_cue::AudioCue).
    pub audio_engine: Arc<AudioEngine>,
    /// The unified output engine, used by video and image cues.
    pub output_engine: Arc<OutputEngine>,
    /// Channel for signalling events back to the Show Engine / transport layer.
    pub event_sender: Sender<CueEvent>,
    /// Duration (ms) of the soft fade-out applied on Stop when the cue has no
    /// explicit `fade_out` spec set.  Comes from `AudioPreferences::default_fade_out_ms`.
    pub stop_fade_ms: u32,
    /// Snapshot of the workspace's Output Patch table.  Cues look up their
    /// patch here to resolve device names and channel indices at GO time.
    pub output_patches: Arc<Vec<OutputPatch>>,
    /// The workspace's default Output Patch ID.
    pub default_patch_id: Option<uuid::Uuid>,
    /// Monitor index for the unified output surface.
    /// `None` = floating window; `Some(n)` = fullscreen on monitor n.
    pub output_screen: Option<u32>,
    /// Snapshot of the workspace's OSC Patch table.
    pub osc_patches: Arc<Vec<OscPatch>>,
    /// The DMX lighting engine, used by [`LightCue`](crate::cue::light_cue::LightCue).
    pub dmx_engine: Arc<DmxEngine>,
    /// Snapshot of the workspace's fixture patch.  Light Cues resolve their
    /// targets' `(universe, channel, width)` here at GO time.
    pub fixtures: Arc<Vec<PatchedFixture>>,
    /// Snapshot of the workspace's fixture groups.  Light Cue group targets
    /// resolve to their member fixtures here at GO time.
    pub fixture_groups: Arc<Vec<FixtureGroup>>,
    /// Snapshot of the workspace's Input Patch table.  Mic Cues resolve their
    /// capture device + channels here at GO time.
    pub input_patches: Arc<Vec<InputPatch>>,
    /// Output buffer size from machine config — passed to `ensure_input_feed`
    /// so the input stream uses the same period as the output stream.
    pub audio_buffer_size: u32,
}

impl CueContext {
    /// Create a new context.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        audio_engine: Arc<AudioEngine>,
        output_engine: Arc<OutputEngine>,
        event_sender: Sender<CueEvent>,
        stop_fade_ms: u32,
        output_patches: Vec<OutputPatch>,
        default_patch_id: Option<uuid::Uuid>,
        output_screen: Option<u32>,
        osc_patches: Vec<OscPatch>,
        dmx_engine: Arc<DmxEngine>,
        fixtures: Vec<PatchedFixture>,
        fixture_groups: Vec<FixtureGroup>,
        input_patches: Vec<InputPatch>,
        audio_buffer_size: u32,
    ) -> Self {
        Self {
            audio_engine,
            output_engine,
            event_sender,
            stop_fade_ms,
            output_patches: Arc::new(output_patches),
            default_patch_id,
            output_screen,
            osc_patches: Arc::new(osc_patches),
            dmx_engine,
            fixtures: Arc::new(fixtures),
            fixture_groups: Arc::new(fixture_groups),
            input_patches: Arc::new(input_patches),
            audio_buffer_size,
        }
    }

    /// Resolve an Output Patch by ID, falling back to the workspace default,
    /// then to `None` if neither is available.
    pub fn resolve_patch(&self, patch_id: Option<uuid::Uuid>) -> Option<&OutputPatch> {
        let id = patch_id.or(self.default_patch_id)?;
        self.output_patches.iter().find(|p| p.id == id)
    }

    /// Resolve an OSC Patch by ID.  Returns `None` if the patch is not in the
    /// workspace's OSC patch table.
    pub fn resolve_osc_patch(&self, patch_id: uuid::Uuid) -> Option<&OscPatch> {
        self.osc_patches.iter().find(|p| p.id == patch_id)
    }

    /// Resolve a patched fixture by ID.  Returns `None` if the fixture is not
    /// in the workspace's patch.
    pub fn resolve_fixture(&self, fixture_id: uuid::Uuid) -> Option<&PatchedFixture> {
        self.fixtures.iter().find(|f| f.id == fixture_id)
    }

    /// Resolve a fixture group by ID.  Returns `None` if the group is not in the
    /// workspace.
    pub fn resolve_group(&self, group_id: uuid::Uuid) -> Option<&FixtureGroup> {
        self.fixture_groups.iter().find(|g| g.id == group_id)
    }

    /// Resolve an Input Patch by ID.  Returns `None` if the patch is not in the
    /// workspace's Input Patch table.
    pub fn resolve_input_patch(&self, patch_id: Option<uuid::Uuid>) -> Option<&InputPatch> {
        let id = patch_id?;
        self.input_patches.iter().find(|p| p.id == id)
    }

    /// Convenience: emit an event through the context without unwrapping.
    /// Silently drops the event if the receiver has been dropped.
    pub fn emit(&self, event: CueEvent) {
        let _ = self.event_sender.send(event);
    }
}
