//! [`ImageCue`] — displays a static or animated image on an output surface.
//!
//! The cue delegates rendering to the [`ImageEngine`], which manages Tauri
//! [`WebviewWindow`] instances showing a fullscreen `<ImageSurface>` React
//! component.  Unlike [`AudioCue`] and [`VideoCue`], images have no intrinsic
//! duration: the cue stays "Running" indefinitely until it is stopped manually
//! OR until the optional [`display_duration`](ImageCue::display_duration) timer
//! fires, after which the image fades out and the cue completes.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::engine::image_engine::ImageVoiceId;

// ---------------------------------------------------------------------------
// ImageStopMode
// ---------------------------------------------------------------------------

/// Controls when a running Image Cue stops displaying.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageStopMode {
    /// Stop automatically when the next GO fires (default).
    StopOnNextCue,
    /// Stay visible until the `display_duration` timer expires.
    DisplayDuration,
}

use super::{
    context::{CueContext, CueEvent},
    traits::{Cue, CueFactory, RuntimeState},
    types::{ContinueMode, CueColor, CueId, CueState, CueType, FadeCurve, FadeSpec},
};

// ---------------------------------------------------------------------------
// ImageCue
// ---------------------------------------------------------------------------

/// A cue that displays a static or animated image file on an output surface.
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
    /// Whether to stop on the next GO or use a timed display duration.
    pub stop_mode: ImageStopMode,
    /// Duration to display the image when `stop_mode == DisplayDuration`.
    pub display_duration: Option<Duration>,
    /// Optional fade-in applied when the image first appears.
    pub fade_in: Option<FadeSpec>,
    /// Optional fade-out applied when the image is hidden.
    pub fade_out: Option<FadeSpec>,
    /// Target monitor index (0 = primary).  `None` = floating window.
    pub screen_index: Option<u32>,

    // --- Runtime ---
    /// Active display voice (surface window handle).
    active_voice_id: Option<ImageVoiceId>,
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
            stop_mode: ImageStopMode::StopOnNextCue,
            display_duration: None,
            fade_in: None,
            fade_out: None,
            screen_index: None,
            active_voice_id: None,
            in_pre_wait: false,
            play_generation: 0,
            auto_continue_fired: false,
        }
    }

    /// Start the actual image display.  Called from `go()` when there is no
    /// pre-wait, or from `tick()` once the pre-wait timer has elapsed.
    fn start_image_action(&mut self, context: &CueContext) -> Result<()> {
        let path = self.file_path.as_ref().ok_or_else(|| {
            anyhow!("ImageCue '{}': no file assigned — set a file in the inspector", self.name)
        })?;

        let fade_in_ms = self.fade_in.as_ref().map(|f| f.duration_ms as u32).unwrap_or(0);

        let voice_id =
            context.image_engine.show_voice(path, self.screen_index, fade_in_ms)?;

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
        // Images are read into memory by ImageEngine at GO time — no pre-load
        // step needed.
        Ok(())
    }

    fn go(&mut self, context: &CueContext) -> Result<()> {
        if self.state == CueState::Running {
            return Ok(()); // Ignore duplicate GO.
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
            let _ = context.image_engine.hide_voice(vid, fade_ms);
            // Close the window immediately; we don't wait for the FadedOut
            // callback because the cue is already transitioning to Standby.
            context.image_engine.gc_voice(vid);
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
            // Immediate cut — no fade.
            let _ = context.image_engine.hide_voice(vid, 0);
            context.image_engine.gc_voice(vid);
        }

        self.state = CueState::Standby;
        self.started_at = None;
        self.action_started_at = None;
        self.auto_continue_fired = false;
        context.emit(CueEvent::Stopped { cue_id: self.id });
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        // Does not stop playback — call stop() first if needed.
        self.state = CueState::Standby;
        self.active_voice_id = None;
        self.started_at = None;
        self.action_started_at = None;
        self.in_pre_wait = false;
        self.auto_continue_fired = false;
        Ok(())
    }

    fn tick(&mut self, context: &CueContext) -> Result<()> {
        // Phase 1: once the pre-wait timer expires, start the action.
        if self.in_pre_wait && self.elapsed() >= self.pre_wait {
            if let Err(e) = self.start_image_action(context) {
                log::warn!("ImageCue '{}' failed to start action: {e}", self.name);
                self.state = CueState::Standby;
            }
            return Ok(());
        }

        // Phase 2: if stop_mode is DisplayDuration, trigger the fade-out once
        // the timer expires.  The event loop detects completion via FadedOut
        // status (voice_done path).
        if !self.in_pre_wait && self.stop_mode == ImageStopMode::DisplayDuration {
            if let Some(disp_dur) = self.display_duration {
                if self.action_elapsed() >= disp_dur {
                    if let Some(vid) = self.active_voice_id {
                        let fade_ms = self
                            .fade_out
                            .as_ref()
                            .map(|f| f.duration_ms as u32)
                            .unwrap_or(0);
                        let _ = context.image_engine.hide_voice(vid, fade_ms);
                        // active_voice_id is NOT cleared here — we need it to
                        // remain set so the event loop can match it against the
                        // FadedOut status and detect completion.
                    }
                }
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
        // Return None so that the event loop's time_done path never fires.
        // Timed completion is driven solely by the FadedOut status (voice_done).
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
        // Returns the active display voice so the event loop can detect
        // completion when a FadedOut status arrives.
        self.active_voice_id
    }

    fn stop_on_next_go(&self) -> bool {
        self.stop_mode == ImageStopMode::StopOnNextCue
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
            "stop_mode": self.stop_mode,
            "display_duration_ms": self.display_duration.map(|d| d.as_millis() as u64),
            "fade_in_ms": self.fade_in.as_ref().map(|f| f.duration_ms),
            "fade_in_curve": self.fade_in.as_ref().map(|f| f.curve),
            "fade_out_ms": self.fade_out.as_ref().map(|f| f.duration_ms),
            "fade_out_curve": self.fade_out.as_ref().map(|f| f.curve),
            "screen_index": self.screen_index,
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
        if let Some(sm) = value.get("stop_mode") {
            if let Ok(mode) = serde_json::from_value(sm.clone()) {
                cue.stop_mode = mode;
            }
        }
        if let Some(ms) = value.get("display_duration_ms").and_then(|v| v.as_u64()) {
            cue.display_duration = Some(Duration::from_millis(ms));
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
        if let Some(si) = value.get("screen_index").and_then(|v| v.as_u64()) {
            cue.screen_index = Some(si as u32);
        }

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
    fn default_duration_is_none() {
        assert!(ImageCue::new().display_duration.is_none());
    }

    #[test]
    fn duration_method_always_none() {
        // duration() returns None so the event loop's time_done path stays
        // disabled for image cues.
        let mut cue = ImageCue::new();
        cue.display_duration = Some(Duration::from_secs(5));
        assert!(cue.duration().is_none());
    }

    #[test]
    fn cue_type_is_image() {
        assert_eq!(ImageCue::new().cue_type(), CueType::Image);
    }

    #[test]
    fn serialize_roundtrip_basic() {
        let mut cue = ImageCue::new();
        cue.set_name("Test Image".to_string());
        cue.display_duration = Some(Duration::from_secs(5));
        cue.screen_index = Some(1);

        let json = cue.serialize();
        assert_eq!(json["type"], "image");
        assert_eq!(json["name"], "Test Image");
        assert_eq!(json["display_duration_ms"], 5000u64);
        assert_eq!(json["screen_index"], 1u32);
        assert_eq!(json["color"], "green");
    }

    #[test]
    fn from_json_roundtrip() {
        let factory = ImageCueFactory;
        let mut cue = ImageCue::new();
        cue.set_name("Round Trip".to_string());
        cue.display_duration = Some(Duration::from_millis(7500));
        cue.screen_index = Some(0);

        let json = cue.serialize();
        let rebuilt = factory.from_json(json).expect("should deserialise");

        assert_eq!(rebuilt.name(), "Round Trip");
        assert_eq!(rebuilt.cue_type(), CueType::Image);
        // display_duration survives round-trip via serialise/from_json.
        let j2 = rebuilt.serialize();
        assert_eq!(j2["display_duration_ms"], 7500u64);
        assert_eq!(j2["screen_index"], 0u32);
    }
}
