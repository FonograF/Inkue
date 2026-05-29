//! [`ImageCue`] — displays a static or animated image on the output surface.
//!
//! The cue delegates rendering to the [`OutputEngine`], which uses libmpv with
//! `audio=no,image-display-duration=inf` to show the image in the unified
//! output window.  Images stay visible until explicitly stopped — there is no
//! auto-complete via duration.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::engine::output_engine::VoiceId;

use super::{
    context::{CueContext, CueEvent},
    traits::{Cue, CueFactory, RuntimeState},
    types::{ContinueMode, CueColor, CueId, CueState, CueType, FadeCurve, FadeSpec},
};

// ---------------------------------------------------------------------------
// ImageCue
// ---------------------------------------------------------------------------

/// A cue that displays a static or animated image file on the output surface.
pub struct ImageCue {
    // --- Identity ---
    id: CueId,
    name: String,
    number: Option<String>,
    notes: String,
    color: CueColor,

    // --- State ---
    state: CueState,

    // --- Timing ---
    pre_wait: Duration,
    post_wait: Duration,
    started_at: Option<Instant>,
    action_started_at: Option<Instant>,

    // --- Continue ---
    continue_mode: ContinueMode,

    // --- Image-specific ---
    /// Absolute (or workspace-relative) path to the image file.
    pub file_path: Option<PathBuf>,
    /// Optional fade-in applied when the image first appears.
    pub fade_in: Option<FadeSpec>,
    /// Optional fade-out applied when the image is hidden.
    pub fade_out: Option<FadeSpec>,

    // --- Runtime ---
    /// Active output voice ID.
    active_voice_id: Option<VoiceId>,
    /// `true` between `go()` and the moment the action starts after pre-wait.
    in_pre_wait: bool,
    /// Incremented on every `go()` call.
    play_generation: u64,
    /// Prevents double-firing of Auto-Continue.
    auto_continue_fired: bool,
}

impl ImageCue {
    /// Create a new, empty Image Cue with a fresh UUID.
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::from("Image Cue"),
            number: None,
            notes: String::new(),
            color: CueColor::Green,
            state: CueState::Standby,
            pre_wait: Duration::ZERO,
            post_wait: Duration::ZERO,
            started_at: None,
            action_started_at: None,
            continue_mode: ContinueMode::DoNotContinue,
            file_path: None,
            fade_in: None,
            fade_out: None,
            active_voice_id: None,
            in_pre_wait: false,
            play_generation: 0,
            auto_continue_fired: false,
        }
    }

    /// Start the actual image display action.
    fn start_image_action(&mut self, context: &CueContext) -> Result<()> {
        let path = self.file_path.as_ref().ok_or_else(|| {
            anyhow!("ImageCue '{}': no file assigned — set a file in the inspector", self.name)
        })?;

        let fade_in_ms = self.fade_in.as_ref().map(|f| f.duration_ms as u32).unwrap_or(0);
        let fade_out_ms = self.fade_out.as_ref().map(|f| f.duration_ms as u32).unwrap_or(0);

        let voice_id = context.output_engine.show_content(
            path,
            true,
            fade_in_ms,
            fade_out_ms,
            0.0,
            0,
            None,
            None,
            context.output_screen,
        )?;

        self.active_voice_id = Some(voice_id);
        self.action_started_at = Some(Instant::now());
        self.in_pre_wait = false;

        context.emit(CueEvent::ActionStarted { cue_id: self.id });
        Ok(())
    }
}

impl Default for ImageCue {
    fn default() -> Self {
        Self::new()
    }
}

impl Cue for ImageCue {
    // -----------------------------------------------------------------------
    // Identity
    // -----------------------------------------------------------------------

    fn id(&self) -> CueId { self.id }
    fn cue_type(&self) -> CueType { CueType::Image }
    fn name(&self) -> &str { &self.name }
    fn set_name(&mut self, name: String) { self.name = name; }
    fn number(&self) -> Option<&str> { self.number.as_deref() }
    fn set_number(&mut self, number: Option<String>) { self.number = number; }
    fn notes(&self) -> &str { &self.notes }
    fn set_notes(&mut self, notes: String) { self.notes = notes; }
    fn color(&self) -> CueColor { self.color }
    fn set_color(&mut self, color: CueColor) { self.color = color; }
    fn state(&self) -> CueState { self.state }

    // -----------------------------------------------------------------------
    // Lifecycle
    // -----------------------------------------------------------------------

    fn load(&mut self, _context: &CueContext) -> Result<()> {
        Ok(())
    }

    fn go(&mut self, context: &CueContext) -> Result<()> {
        if self.state == CueState::Running {
            return Ok(());
        }

        self.play_generation = self.play_generation.wrapping_add(1);
        self.auto_continue_fired = false;
        self.state = CueState::Running;
        self.started_at = Some(Instant::now());

        if !self.pre_wait.is_zero() {
            self.in_pre_wait = true;
            return Ok(());
        }

        self.start_image_action(context)
    }

    fn stop(&mut self, context: &CueContext) -> Result<()> {
        self.in_pre_wait = false;

        if let Some(vid) = self.active_voice_id.take() {
            let fade_ms = self
                .fade_out
                .as_ref()
                .map(|f| f.duration_ms as u32)
                .unwrap_or(0);
            context.output_engine.stop_content(vid, fade_ms);
        }

        self.state = CueState::Standby;
        self.started_at = None;
        self.action_started_at = None;
        self.auto_continue_fired = false;
        context.emit(CueEvent::Stopped { cue_id: self.id });
        Ok(())
    }

    fn pause(&mut self, _context: &CueContext) -> Result<()> {
        if self.state == CueState::Running {
            self.state = CueState::Paused;
        }
        Ok(())
    }

    fn resume(&mut self, _context: &CueContext) -> Result<()> {
        if self.state == CueState::Paused {
            self.state = CueState::Running;
        }
        Ok(())
    }

    fn hard_stop(&mut self, context: &CueContext) -> Result<()> {
        self.in_pre_wait = false;

        if let Some(vid) = self.active_voice_id.take() {
            context.output_engine.stop_content(vid, 0);
        }

        self.state = CueState::Standby;
        self.started_at = None;
        self.action_started_at = None;
        self.auto_continue_fired = false;
        context.emit(CueEvent::Stopped { cue_id: self.id });
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        self.state = CueState::Standby;
        self.active_voice_id = None;
        self.started_at = None;
        self.action_started_at = None;
        self.in_pre_wait = false;
        self.auto_continue_fired = false;
        Ok(())
    }

    fn tick(&mut self, context: &CueContext) -> Result<()> {
        if self.in_pre_wait && self.elapsed() >= self.pre_wait {
            if let Err(e) = self.start_image_action(context) {
                log::warn!("ImageCue '{}' failed to start action: {e}", self.name);
                self.state = CueState::Standby;
            }
        }
        Ok(())
    }

    fn is_action_started(&self) -> bool {
        !self.in_pre_wait
    }

    // -----------------------------------------------------------------------
    // Timing
    // -----------------------------------------------------------------------

    fn pre_wait(&self) -> Duration { self.pre_wait }
    fn set_pre_wait(&mut self, d: Duration) { self.pre_wait = d; }
    fn post_wait(&self) -> Duration { self.post_wait }
    fn set_post_wait(&mut self, d: Duration) { self.post_wait = d; }

    fn duration(&self) -> Option<Duration> {
        // Images have no intrinsic duration — they stay running until stopped.
        None
    }

    fn elapsed(&self) -> Duration {
        self.started_at.map(|t| t.elapsed()).unwrap_or(Duration::ZERO)
    }

    fn action_elapsed(&self) -> Duration {
        self.action_started_at
            .map(|t| t.elapsed())
            .unwrap_or(Duration::ZERO)
    }

    // -----------------------------------------------------------------------
    // Continue mode
    // -----------------------------------------------------------------------

    fn continue_mode(&self) -> ContinueMode { self.continue_mode }
    fn set_continue_mode(&mut self, mode: ContinueMode) { self.continue_mode = mode; }

    // -----------------------------------------------------------------------
    // Runtime helpers
    // -----------------------------------------------------------------------

    fn playing_voice_id(&self) -> Option<CueId> {
        self.active_voice_id
    }

    fn stop_on_next_go(&self) -> bool {
        // Images always stop on the next GO (StopOnNextCue behavior).
        true
    }

    fn play_generation(&self) -> u64 { self.play_generation }
    fn is_auto_continue_fired(&self) -> bool { self.auto_continue_fired }
    fn mark_auto_continue_fired(&mut self) { self.auto_continue_fired = true; }
    fn clear_auto_continue_fired(&mut self) { self.auto_continue_fired = false; }

    fn runtime_state(&self) -> RuntimeState {
        RuntimeState {
            state: self.state,
            voice_id: self.active_voice_id,
            started_at: self.started_at,
            action_started_at: self.action_started_at,
        }
    }

    fn restore_runtime_state(&mut self, snap: RuntimeState) {
        self.state = snap.state;
        self.active_voice_id = snap.voice_id;
        self.started_at = snap.started_at;
        self.action_started_at = snap.action_started_at;
        self.in_pre_wait = snap.state == CueState::Running && snap.action_started_at.is_none();
    }

    // -----------------------------------------------------------------------
    // Serialisation
    // -----------------------------------------------------------------------

    fn serialize(&self) -> Value {
        json!({
            "type": "image",
            "cue_type": "image",
            "id": self.id,
            "number": self.number,
            "name": self.name,
            "notes": self.notes,
            "color": self.color,
            "pre_wait_ms": self.pre_wait.as_millis() as u64,
            "post_wait_ms": self.post_wait.as_millis() as u64,
            "continue_mode": self.continue_mode,
            "file_path": self.file_path.as_ref().map(|p| p.to_string_lossy().to_string()),
            "fade_in_ms": self.fade_in.as_ref().map(|f| f.duration_ms),
            "fade_in_curve": self.fade_in.as_ref().map(|f| f.curve),
            "fade_out_ms": self.fade_out.as_ref().map(|f| f.duration_ms),
            "fade_out_curve": self.fade_out.as_ref().map(|f| f.curve),
        })
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Factory for [`ImageCue`].
pub struct ImageCueFactory;

impl CueFactory for ImageCueFactory {
    fn create(&self) -> Box<dyn Cue> {
        Box::new(ImageCue::new())
    }

    fn from_json(&self, value: Value) -> Result<Box<dyn Cue>> {
        let mut cue = ImageCue::new();

        if let Some(id_str) = value.get("id").and_then(|v| v.as_str()) {
            cue.id = id_str.parse().unwrap_or_else(|_| Uuid::new_v4());
        }
        if let Some(name) = value.get("name").and_then(|v| v.as_str()) {
            cue.name = name.to_string();
        }
        if let Some(num) = value.get("number").and_then(|v| v.as_str()) {
            cue.number = Some(num.to_string());
        }
        if let Some(notes) = value.get("notes").and_then(|v| v.as_str()) {
            cue.notes = notes.to_string();
        }
        if let Some(ms) = value.get("pre_wait_ms").and_then(|v| v.as_u64()) {
            cue.pre_wait = Duration::from_millis(ms);
        }
        if let Some(ms) = value.get("post_wait_ms").and_then(|v| v.as_u64()) {
            cue.post_wait = Duration::from_millis(ms);
        }
        if let Some(cm) = value.get("continue_mode") {
            if let Ok(mode) = serde_json::from_value(cm.clone()) {
                cue.continue_mode = mode;
            }
        }
        if let Some(col) = value.get("color") {
            if let Ok(color) = serde_json::from_value(col.clone()) {
                cue.color = color;
            }
        }
        if let Some(path) = value.get("file_path").and_then(|v| v.as_str()) {
            cue.file_path = Some(PathBuf::from(path));
        }
        if let Some(ms) = value.get("fade_in_ms").and_then(|v| v.as_u64()) {
            let curve = value
                .get("fade_in_curve")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or(FadeCurve::SCurve);
            cue.fade_in = Some(FadeSpec { duration_ms: ms, curve });
        }
        if let Some(ms) = value.get("fade_out_ms").and_then(|v| v.as_u64()) {
            let curve = value
                .get("fade_out_curve")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or(FadeCurve::SCurve);
            cue.fade_out = Some(FadeSpec { duration_ms: ms, curve });
        }
        // "stop_mode", "display_duration_ms", "screen_index" from older workspaces are
        // silently ignored (fields no longer exist in the new architecture).

        Ok(Box::new(cue))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_color_is_green() {
        assert_eq!(ImageCue::new().color(), CueColor::Green);
    }

    #[test]
    fn duration_method_always_none() {
        assert!(ImageCue::new().duration().is_none());
    }

    #[test]
    fn cue_type_is_image() {
        assert_eq!(ImageCue::new().cue_type(), CueType::Image);
    }

    #[test]
    fn stop_on_next_go_always_true() {
        assert!(ImageCue::new().stop_on_next_go());
    }

    #[test]
    fn serialize_roundtrip_basic() {
        let mut cue = ImageCue::new();
        cue.set_name("Test Image".to_string());

        let json = cue.serialize();
        assert_eq!(json["type"], "image");
        assert_eq!(json["name"], "Test Image");
        assert!(json.get("screen_index").is_none(), "screen_index must not be serialised");
        assert!(json.get("stop_mode").is_none(), "stop_mode must not be serialised");
        assert!(json.get("display_duration_ms").is_none(), "display_duration_ms must not be serialised");
        assert_eq!(json["color"], "green");
    }

    #[test]
    fn from_json_roundtrip() {
        let factory = ImageCueFactory;
        let mut cue = ImageCue::new();
        cue.set_name("Round Trip".to_string());

        let json = cue.serialize();
        let rebuilt = factory.from_json(json).expect("should deserialise");

        assert_eq!(rebuilt.name(), "Round Trip");
        assert_eq!(rebuilt.cue_type(), CueType::Image);
    }

    #[test]
    fn from_json_ignores_legacy_fields() {
        let factory = ImageCueFactory;
        let json = serde_json::json!({
            "type": "image",
            "id": "00000000-0000-0000-0000-000000000001",
            "name": "Legacy Cue",
            "screen_index": 1,
            "stop_mode": "display_duration",
            "display_duration_ms": 5000,
        });
        let cue = factory.from_json(json).expect("should load without error");
        assert_eq!(cue.name(), "Legacy Cue");
    }
}
