//! [`CameraCue`] — shows a live camera / capture / network video feed on the
//! unified output window, like any other visual cue.
//!
//! The feed is opened by libmpv (which wraps libavformat's capture demuxers):
//! DirectShow on Windows, V4L2 on Linux, AVFoundation on macOS — plus any
//! network stream mpv can play (RTSP / HTTP / UDP…), which covers IP cameras
//! and phone-camera apps.  Video fade in/out (dip-to-black overlay) and the
//! per-cue [`VideoGeometry`] work exactly as they do for Video and Image cues.
//! The feed runs until stopped and is replaced by the next visual GO.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::engine::output_engine::{ContentRequest, LayerStyle, VideoGeometry, VoiceId};

use super::{
    context::{CueContext, CueEvent},
    traits::{Cue, CueFactory, RuntimeState},
    types::{ContinueMode, CueColor, CueId, CueState, CueType, FadeCurve, FadeSpec},
};

// ---------------------------------------------------------------------------
// CameraSource
// ---------------------------------------------------------------------------

/// Where the live feed comes from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum CameraSource {
    /// A local capture device (webcam, USB camera, HDMI capture card).
    ///
    /// `id` is the platform identifier ffmpeg's capture demuxer needs:
    /// the DirectShow device *name* on Windows, the `/dev/videoN` path on
    /// Linux, the AVFoundation device name on macOS.  `name` is what the
    /// operator sees (usually the same string).
    Device { id: String, name: String },
    /// A network stream URL (RTSP / HTTP / UDP…) — IP cameras, NDI-to-RTSP
    /// bridges, phone-camera apps.
    Url { url: String },
}

impl Default for CameraSource {
    fn default() -> Self {
        Self::Device { id: String::new(), name: String::new() }
    }
}

impl CameraSource {
    /// `true` when the source is actually configured.
    pub fn is_configured(&self) -> bool {
        match self {
            Self::Device { id, .. } => !id.is_empty(),
            Self::Url { url } => !url.is_empty(),
        }
    }

    /// The mpv URL that opens this source on the current OS.
    pub fn mpv_url(&self) -> String {
        match self {
            Self::Url { url } => url.clone(),
            Self::Device { id, .. } => {
                #[cfg(target_os = "windows")]
                {
                    format!("av://dshow:video={id}")
                }
                #[cfg(target_os = "linux")]
                {
                    format!("av://v4l2:{id}")
                }
                #[cfg(target_os = "macos")]
                {
                    format!("av://avfoundation:{id}")
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// CameraCue
// ---------------------------------------------------------------------------

/// A cue that shows a live camera / capture / network feed on the output.
pub struct CameraCue {
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

    // --- Camera-specific ---
    /// The live source (device or network URL).
    pub source: CameraSource,
    /// Visual (GL overlay) fade-in from black.
    pub video_fade_in: Option<FadeSpec>,
    /// Visual (GL overlay) fade-out to black on stop.
    pub video_fade_out: Option<FadeSpec>,
    /// Visual geometry (fit / position / scale / rotation / crop).
    pub geometry: VideoGeometry,
    /// Compositing (stacking layer, base opacity, blend mode).
    pub layer_style: LayerStyle,

    is_disabled: bool,

    // --- Runtime ---
    active_voice_id: Option<VoiceId>,
    in_pre_wait: bool,
    play_generation: u64,
    auto_continue_fired: bool,
}

impl CameraCue {
    /// Create a new, empty Camera Cue with a fresh UUID.
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::from("Camera Cue"),
            number: None,
            notes: String::new(),
            color: CueColor::Cyan,
            state: CueState::Standby,
            pre_wait: Duration::ZERO,
            post_wait: Duration::ZERO,
            started_at: None,
            action_started_at: None,
            continue_mode: ContinueMode::DoNotContinue,
            source: CameraSource::default(),
            video_fade_in: None,
            video_fade_out: None,
            geometry: VideoGeometry::default(),
            layer_style: LayerStyle::default(),
            is_disabled: false,
            active_voice_id: None,
            in_pre_wait: false,
            play_generation: 0,
            auto_continue_fired: false,
        }
    }

    /// Open the feed on the output window.
    /// [`Self::start_camera_action`], returning the cue to Standby when it fails.
    ///
    /// A cue whose action never started must not stay at `Running` with no
    /// voice: the UI only leaves Running on a state change it is told about, so
    /// it would freeze on the cue forever (the classic symptom when the output
    /// engine is headless and refuses every `show_content`).
    fn start_action_or_reset(&mut self, context: &CueContext) -> Result<()> {
        let result = self.start_camera_action(context);
        if result.is_err() {
            self.state = CueState::Standby;
            self.started_at = None;
            self.in_pre_wait = false;
        }
        result
    }

    fn start_camera_action(&mut self, context: &CueContext) -> Result<()> {
        let fade_in_ms = self.video_fade_in.as_ref().map(|f| f.duration_ms as u32).unwrap_or(0);
        let url = PathBuf::from(self.source.mpv_url());

        let voice_id = context.output_engine.show_content(ContentRequest {
            file_path: &url,
            is_image: false,
            fade_in_ms,
            loop_count: 0,
            start_ms: None,
            end_ms: None,
            screen_index: context.output_screen,
            audio_voice_id: None,
            display_duration_ms: None,
            hold_last_frame: false,
            geometry: self.geometry,
            live_source: true,
            // A live feed has nothing to decode ahead of time; Load falls back
            // to the trait default (bring up, pause) — which for a camera is
            // still useful: it opens the capture device early.
            preload: false,
            layer_style: self.layer_style,
            slices: Vec::new(),
        })?;

        self.active_voice_id = Some(voice_id);
        self.action_started_at = Some(Instant::now());
        self.in_pre_wait = false;

        context.emit(CueEvent::ActionStarted { cue_id: self.id });
        Ok(())
    }
}

impl Default for CameraCue {
    fn default() -> Self {
        Self::new()
    }
}

impl Cue for CameraCue {
    // -----------------------------------------------------------------------
    // Identity
    // -----------------------------------------------------------------------

    fn id(&self) -> CueId { self.id }
    fn cue_type(&self) -> CueType { CueType::Camera }
    fn name(&self) -> &str { &self.name }
    fn set_name(&mut self, name: String) { self.name = name; }
    fn number(&self) -> Option<&str> { self.number.as_deref() }
    fn set_number(&mut self, number: Option<String>) { self.number = number; }
    fn notes(&self) -> &str { &self.notes }
    fn set_notes(&mut self, notes: String) { self.notes = notes; }
    fn color(&self) -> CueColor { self.color }
    fn set_color(&mut self, color: CueColor) { self.color = color; }
    fn is_disabled(&self) -> bool { self.is_disabled }
    fn set_disabled(&mut self, d: bool) { self.is_disabled = d; }
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

        if !self.source.is_configured() {
            // No source assigned — complete instantly (same pattern as an
            // Image cue without a file) so Auto-Continue/Follow can advance.
            self.state = CueState::Running;
            self.started_at = Some(Instant::now());
            context.emit(CueEvent::ActionStarted { cue_id: self.id });
            self.state = CueState::Completed;
            context.emit(CueEvent::ActionCompleted { cue_id: self.id });
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

        self.start_action_or_reset(context)
    }

    fn stop(&mut self, context: &CueContext) -> Result<()> {
        self.in_pre_wait = false;

        if let Some(vid) = self.active_voice_id.take() {
            let fade_ms = self.video_fade_out.as_ref().map(|f| f.duration_ms as u32).unwrap_or(0);
            context.output_engine.stop_content(vid, fade_ms, 0);
        }

        self.state = CueState::Standby;
        self.started_at = None;
        self.action_started_at = None;
        self.auto_continue_fired = false;
        context.emit(CueEvent::Stopped { cue_id: self.id });
        Ok(())
    }

    fn pause(&mut self, _context: &CueContext) -> Result<()> {
        // A live feed cannot meaningfully pause; keep it running.
        Ok(())
    }

    fn resume(&mut self, _context: &CueContext) -> Result<()> {
        Ok(())
    }

    fn hard_stop(&mut self, context: &CueContext) -> Result<()> {
        self.in_pre_wait = false;

        if let Some(vid) = self.active_voice_id.take() {
            context.output_engine.stop_content(vid, 0, 0);
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
            if let Err(e) = self.start_action_or_reset(context) {
                log::warn!("CameraCue '{}' failed to start: {e}", self.name);
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
        None // A live feed has no natural end — runs until stopped.
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

    fn is_visual(&self) -> bool {
        true
    }

    fn visual_geometry(&self) -> Option<VideoGeometry> {
        Some(self.geometry)
    }

    fn layer_style(&self) -> Option<LayerStyle> {
        Some(self.layer_style)
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
            "type": "camera",
            "cue_type": "camera",
            "id": self.id,
            "number": self.number,
            "name": self.name,
            "notes": self.notes,
            "color": self.color,
            "pre_wait_ms": self.pre_wait.as_millis() as u64,
            "post_wait_ms": self.post_wait.as_millis() as u64,
            "continue_mode": self.continue_mode,
            "source": self.source,
            "video_fade_in_ms": self.video_fade_in.as_ref().map(|f| f.duration_ms),
            "video_fade_in_curve": self.video_fade_in.as_ref().map(|f| f.curve),
            "video_fade_out_ms": self.video_fade_out.as_ref().map(|f| f.duration_ms),
            "video_fade_out_curve": self.video_fade_out.as_ref().map(|f| f.curve),
            "geometry": self.geometry,
            "layer_style": self.layer_style,
            "is_disabled": self.is_disabled,
        })
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Factory for [`CameraCue`].
pub struct CameraCueFactory;

impl CueFactory for CameraCueFactory {
    fn create(&self) -> Box<dyn Cue> {
        Box::new(CameraCue::new())
    }

    fn from_json(&self, value: Value) -> Result<Box<dyn Cue>> {
        let mut cue = CameraCue::new();

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
        if let Some(src) = value.get("source") {
            if let Ok(source) = serde_json::from_value(src.clone()) {
                cue.source = source;
            }
        }
        if let Some(ms) = value.get("video_fade_in_ms").and_then(|v| v.as_u64()) {
            let curve = value
                .get("video_fade_in_curve")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or(FadeCurve::SCurve);
            cue.video_fade_in = Some(FadeSpec { duration_ms: ms, curve });
        }
        if let Some(ms) = value.get("video_fade_out_ms").and_then(|v| v.as_u64()) {
            let curve = value
                .get("video_fade_out_curve")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or(FadeCurve::SCurve);
            cue.video_fade_out = Some(FadeSpec { duration_ms: ms, curve });
        }
        if let Some(g) = value.get("geometry") {
            if let Ok(geometry) = serde_json::from_value::<VideoGeometry>(g.clone()) {
                cue.geometry = geometry;
            }
        }
        if let Some(ls) = value.get("layer_style") {
            if let Ok(style) = serde_json::from_value::<LayerStyle>(ls.clone()) {
                cue.layer_style = style;
            }
        }
        // "stop_on_next_visual" from older workspaces is silently ignored.
        if let Some(b) = value.get("is_disabled").and_then(|v| v.as_bool()) {
            cue.is_disabled = b;
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
    fn cue_type_is_camera_and_visual() {
        let cue = CameraCue::new();
        assert_eq!(cue.cue_type(), CueType::Camera);
        assert!(cue.is_visual());
        // Visual cues stack as layers; only Stop/Fade cues remove one.
        assert!(!cue.stop_on_next_go());
        assert!(cue.duration().is_none());
    }

    #[test]
    fn default_source_is_unconfigured() {
        assert!(!CameraSource::default().is_configured());
        assert!(CameraSource::Url { url: "rtsp://cam".into() }.is_configured());
        assert!(CameraSource::Device { id: "d".into(), name: "Cam".into() }.is_configured());
    }

    #[test]
    fn device_source_builds_platform_mpv_url() {
        let src = CameraSource::Device { id: "Logitech C920".into(), name: "Logitech C920".into() };
        let url = src.mpv_url();
        #[cfg(target_os = "windows")]
        assert_eq!(url, "av://dshow:video=Logitech C920");
        #[cfg(target_os = "linux")]
        assert_eq!(url, "av://v4l2:Logitech C920");
        #[cfg(target_os = "macos")]
        assert_eq!(url, "av://avfoundation:Logitech C920");
    }

    #[test]
    fn url_source_passes_through() {
        let src = CameraSource::Url { url: "rtsp://192.168.1.50:8554/live".into() };
        assert_eq!(src.mpv_url(), "rtsp://192.168.1.50:8554/live");
    }

    #[test]
    fn serialize_roundtrip() {
        let mut cue = CameraCue::new();
        cue.set_name("FOH cam".to_string());
        cue.source = CameraSource::Url { url: "rtsp://cam.local/stream".into() };
        cue.video_fade_in = Some(FadeSpec { duration_ms: 1500, curve: FadeCurve::Linear });

        let json = cue.serialize();
        assert_eq!(json["type"], "camera");
        assert_eq!(json["source"]["kind"], "url");

        let rebuilt = CameraCueFactory.from_json(json).expect("roundtrip");
        assert_eq!(rebuilt.name(), "FOH cam");
        assert_eq!(rebuilt.cue_type(), CueType::Camera);
        let rebuilt_json = rebuilt.serialize();
        assert_eq!(rebuilt_json["video_fade_in_ms"], 1500);
        assert_eq!(rebuilt_json["source"]["url"], "rtsp://cam.local/stream");
    }

    #[test]
    fn from_json_device_source() {
        let json = serde_json::json!({
            "type": "camera",
            "name": "Webcam",
            "source": { "kind": "device", "id": "USB Video Device", "name": "USB Video Device" },
        });
        let cue = CameraCueFactory.from_json(json).expect("load");
        let rebuilt = cue.serialize();
        assert_eq!(rebuilt["source"]["kind"], "device");
        assert_eq!(rebuilt["source"]["id"], "USB Video Device");
    }

    #[test]
    fn go_without_source_completes_instantly() {
        // Serialize path only — go() needs a CueContext; the unconfigured
        // check is exercised through is_configured here.
        assert!(!CameraCue::new().source.is_configured());
    }
}
