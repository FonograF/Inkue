//! Data types shared across the output_engine module.

use std::ffi::c_void;
use std::path::Path;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Type aliases
// ---------------------------------------------------------------------------

/// Unique identifier for one playing output instance (video or image).
pub type VoiceId = Uuid;
/// Unique identifier for one output surface.
pub type SurfaceId = Uuid;

// ---------------------------------------------------------------------------
// Thread-safety wrapper for the raw mpv context pointer
// ---------------------------------------------------------------------------

pub(crate) struct MpvCtx(pub *mut c_void);
unsafe impl Send for MpvCtx {}
unsafe impl Sync for MpvCtx {}

// ---------------------------------------------------------------------------
// Screen info
// ---------------------------------------------------------------------------

/// Information about a connected monitor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenInfo {
    pub index: u32,
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    pub is_primary: bool,
}

// ---------------------------------------------------------------------------
// Visual geometry
// ---------------------------------------------------------------------------

/// How the source frame is mapped onto the output surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FitMode {
    /// Letterbox: keep aspect ratio, whole frame visible (mpv default).
    #[default]
    Fit,
    /// Cover the surface, cropping the overflow (`keepaspect` + `panscan=1`).
    Fill,
    /// Ignore aspect ratio and fill the surface exactly (`keepaspect=no`).
    Stretch,
}

fn geometry_default_scale() -> f64 {
    1.0
}

/// Per-cue visual geometry for Video and Image cues, applied to the output
/// surface via mpv properties (`video-zoom`, `video-pan-x/y`, `video-rotate`,
/// `keepaspect`, `panscan`, `video-crop`) on every load — and live when the
/// cue currently on screen is edited in the inspector.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VideoGeometry {
    #[serde(default)]
    pub fit_mode: FitMode,
    /// Horizontal offset as a fraction of the scaled video width (−1 .. 1).
    #[serde(default)]
    pub pan_x: f64,
    /// Vertical offset as a fraction of the scaled video height (−1 .. 1).
    #[serde(default)]
    pub pan_y: f64,
    /// Linear scale factor (1.0 = 100 %).  mpv `video-zoom` is `log2(scale)`.
    #[serde(default = "geometry_default_scale")]
    pub scale: f64,
    /// Clockwise rotation in degrees (0–359).
    #[serde(default)]
    pub rotation: u32,
    /// Crop per edge as a fraction of the source size (0 .. 0.45 each).
    /// Converted to mpv's pixel-based `video-crop` once the source dimensions
    /// are known (the crop applies to the source rect, before rotation).
    #[serde(default)]
    pub crop_left: f64,
    #[serde(default)]
    pub crop_right: f64,
    #[serde(default)]
    pub crop_top: f64,
    #[serde(default)]
    pub crop_bottom: f64,
}

impl Default for VideoGeometry {
    fn default() -> Self {
        Self {
            fit_mode: FitMode::Fit,
            pan_x: 0.0,
            pan_y: 0.0,
            scale: 1.0,
            rotation: 0,
            crop_left: 0.0,
            crop_right: 0.0,
            crop_top: 0.0,
            crop_bottom: 0.0,
        }
    }
}

impl VideoGeometry {
    /// `true` when every field is at its neutral value (engine defaults).
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// `true` when at least one crop edge is set.
    pub fn has_crop(&self) -> bool {
        self.crop_left > 0.0 || self.crop_right > 0.0 || self.crop_top > 0.0 || self.crop_bottom > 0.0
    }

    /// mpv `video-zoom` value (log2 of the linear scale factor).
    pub fn mpv_video_zoom(&self) -> f64 {
        self.scale.max(0.01).log2()
    }

    /// mpv `keepaspect` / `panscan` property values for the fit mode.
    pub fn fit_props(&self) -> (&'static str, &'static str) {
        match self.fit_mode {
            FitMode::Fit => ("yes", "0"),
            FitMode::Fill => ("yes", "1"),
            FitMode::Stretch => ("no", "0"),
        }
    }

    /// Pixel rect `(w, h, x, y)` for mpv `video-crop`, computed against the
    /// actual source dimensions.  `None` = no crop configured.
    ///
    /// Edges are clamped to 0.45 each so the remaining rect is never empty.
    pub fn crop_rect_px(&self, src_w: u32, src_h: u32) -> Option<(u32, u32, u32, u32)> {
        if !self.has_crop() || src_w == 0 || src_h == 0 {
            return None;
        }
        let clamp = |v: f64| v.clamp(0.0, 0.45);
        let (l, r) = (clamp(self.crop_left), clamp(self.crop_right));
        let (t, b) = (clamp(self.crop_top), clamp(self.crop_bottom));
        let x = (src_w as f64 * l).round() as u32;
        let y = (src_h as f64 * t).round() as u32;
        let w = ((src_w as f64 * (1.0 - l - r)).round() as u32).max(2).min(src_w - x);
        let h = ((src_h as f64 * (1.0 - t - b)).round() as u32).max(2).min(src_h - y);
        Some((w, h, x, y))
    }
}

// ---------------------------------------------------------------------------
// OutputTransform — global projector alignment
// ---------------------------------------------------------------------------

fn transform_default_scale() -> f64 {
    1.0
}

/// Venue-level output transform applied to **everything** on the output
/// window (all Video/Image cues and test patterns), on top of each cue's own
/// [`VideoGeometry`].  Lets the operator re-frame a projector inside its
/// screen: shift / shrink / finely rotate the whole picture, plus a full
/// four-corner pin (perspective warp) for keystone-style correction.
///
/// On the GL output path the whole transform (including fractional rotation
/// and the corner pin) is a dedicated warp render pass; the legacy Win32 path
/// approximates pan/scale/rotation via mpv properties (rotation rounded to
/// whole degrees, corner pin unsupported).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OutputTransform {
    /// Horizontal offset as a fraction of the output width (−1 .. 1).
    #[serde(default)]
    pub pan_x: f64,
    /// Vertical offset as a fraction of the output height (−1 .. 1).
    #[serde(default)]
    pub pan_y: f64,
    /// Linear scale factor (1.0 = 100 %).
    #[serde(default = "transform_default_scale")]
    pub scale: f64,
    /// Clockwise rotation in degrees.  Fractional values are honoured on the
    /// GL path (e.g. 0.4° to square up a slightly tilted projector).
    #[serde(default)]
    pub rotation: f64,
    /// Per-corner offsets in fractions of the output size, storage order
    /// **TL, TR, BL, BR** (reading order, matching the editor UI); positive =
    /// right / down.  Applied after scale/rotation/pan.
    #[serde(default)]
    pub corners: [[f64; 2]; 4],
}

impl Default for OutputTransform {
    fn default() -> Self {
        Self {
            pan_x: 0.0,
            pan_y: 0.0,
            scale: 1.0,
            rotation: 0.0,
            corners: [[0.0; 2]; 4],
        }
    }
}

impl OutputTransform {
    /// `true` when the transform is the identity (engine defaults).
    pub fn is_identity(&self) -> bool {
        *self == Self::default()
    }
}

/// The final mpv display properties after composing a cue's [`VideoGeometry`]
/// with the global [`OutputTransform`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct EffectiveDisplayProps {
    /// mpv `video-zoom` (log2 of the combined linear scale).
    pub zoom_log2: f64,
    pub pan_x: f64,
    pub pan_y: f64,
    /// mpv `video-rotate` (0–359).
    pub rotation: u32,
}

/// Compose a cue's geometry with the global output transform into mpv
/// properties — **legacy Win32 path only** (the GL path applies the global
/// transform as a warp render pass instead, with fractional rotation and the
/// corner pin; mpv `video-rotate` only takes whole degrees, so the rotation
/// is rounded here and the corner pin cannot be represented at all).
///
/// Scales multiply (mpv `video-zoom` is log2, so the logs add), pans add
/// (both are fractions of the scaled video size), rotations add mod 360.
/// Fit mode and crop stay cue-only — the transform re-frames the *output*,
/// it does not re-crop the source.
pub(crate) fn compose_display_props(
    cue: &VideoGeometry,
    transform: &OutputTransform,
) -> EffectiveDisplayProps {
    let rotation = (cue.rotation.min(359) as f64 + transform.rotation).rem_euclid(360.0);
    EffectiveDisplayProps {
        zoom_log2: cue.mpv_video_zoom() + transform.scale.max(0.01).log2(),
        pan_x: cue.pan_x + transform.pan_x,
        pan_y: cue.pan_y + transform.pan_y,
        rotation: (rotation.round() as u32) % 360,
    }
}

// ---------------------------------------------------------------------------
// TestPattern — projector calibration sources
// ---------------------------------------------------------------------------

/// Built-in calibration patterns rendered by libavfilter (`av://lavfi:`),
/// sized to the target screen.  `CustomImage` shows an operator-supplied
/// file (e.g. a colorimetry reference chart).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "path")]
pub enum TestPattern {
    /// Alignment grid: fine cells + centre cross + frame border.
    Grid,
    /// SMPTE HD colour bars.
    SmpteBars,
    /// RGB test chart.
    RgbTest,
    /// FFmpeg `testsrc2` — fine detail, useful for focus.
    TestCard,
    /// Full white (light output / uniformity).
    White,
    /// 50 % grey.
    Gray,
    /// Full black (black-level / ambient light).
    Black,
    /// Operator-supplied image file.
    CustomImage(String),
}

impl TestPattern {
    /// The mpv URL that renders this pattern at `width`×`height`.
    pub fn mpv_url(&self, width: u32, height: u32) -> String {
        let w = width.max(2);
        let h = height.max(2);
        match self {
            Self::Grid => format!(
                "av://lavfi:color=c=black:s={w}x{h}:r=1,\
                 drawgrid=w=iw/16:h=ih/9:t=1:c=white@0.5,\
                 drawgrid=w=iw/2:h=ih/2:t=3:c=white@0.9,\
                 drawbox=x=0:y=0:w=iw:h=ih:t=6:c=white@0.9"
            ),
            Self::SmpteBars => format!("av://lavfi:smptehdbars=s={w}x{h}:r=1"),
            Self::RgbTest => format!("av://lavfi:rgbtestsrc=s={w}x{h}:r=1"),
            Self::TestCard => format!("av://lavfi:testsrc2=s={w}x{h}:r=1"),
            Self::White => format!("av://lavfi:color=c=white:s={w}x{h}:r=1"),
            Self::Gray => format!("av://lavfi:color=c=gray:s={w}x{h}:r=1"),
            Self::Black => format!("av://lavfi:color=c=black:s={w}x{h}:r=1"),
            Self::CustomImage(path) => path.replace('\\', "/"),
        }
    }

    /// `true` for the file-based pattern (loaded like an image, not lavfi).
    pub fn is_file(&self) -> bool {
        matches!(self, Self::CustomImage(_))
    }
}

// ---------------------------------------------------------------------------
// ContentRequest
// ---------------------------------------------------------------------------

/// Everything [`super::OutputEngine::show_content`] needs to display one piece
/// of content (video or image) on the output window.
pub struct ContentRequest<'a> {
    pub file_path: &'a Path,
    pub is_image: bool,
    /// Visual fade-from-black duration when the first frame is revealed.
    pub fade_in_ms: u32,
    /// Extra loop repetitions (0 = play once, `u32::MAX` = infinite).
    pub loop_count: u32,
    pub start_ms: Option<u64>,
    pub end_ms: Option<u64>,
    /// Screen to go fullscreen on.  `None` = floating window.
    pub screen_index: Option<u32>,
    /// The AudioEngine voice carrying this video's audio track, if any.
    pub audio_voice_id: Option<VoiceId>,
    /// Image cues: how long the image stays before auto-completing
    /// (`None` = hold until stopped).
    pub display_duration_ms: Option<u64>,
    /// Video cues: freeze on the last frame at natural EOF instead of
    /// cutting to black (mpv `keep-open`).
    pub hold_last_frame: bool,
    /// Per-cue visual geometry (fit / pan / scale / rotate / crop).
    pub geometry: VideoGeometry,
    /// Camera / network feeds: apply low-latency demuxer options (no cache,
    /// minimal analyzeduration) instead of the file-playback defaults.
    pub live_source: bool,
}

// ---------------------------------------------------------------------------
// OutputStatus
// ---------------------------------------------------------------------------

/// Status events produced by the mpv event thread.
#[derive(Debug, Clone)]
pub enum OutputStatus {
    /// Playback reached its natural end.
    Completed { voice_id: VoiceId },
    /// File metadata loaded; total duration is now known.
    Duration { voice_id: VoiceId, duration_ms: u64 },
    /// A playback error occurred inside mpv.
    Error { voice_id: VoiceId, message: String },
}

// ---------------------------------------------------------------------------
// OutputSurface
// ---------------------------------------------------------------------------

/// A named output surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputSurface {
    pub id: SurfaceId,
    pub name: String,
    pub label: String,
}

// ---------------------------------------------------------------------------
// OutputVoice
// ---------------------------------------------------------------------------

#[derive(Debug)]
#[allow(dead_code)]
pub(super) struct OutputVoice {
    pub id: VoiceId,
    pub started_at: Instant,
    pub duration: Option<Duration>,
}

// ---------------------------------------------------------------------------
// Fade overlay state
// ---------------------------------------------------------------------------

/// Parameters for a pending content load, passed directly to `execute_load_params`.
pub(crate) struct FadePendingParams {
    pub path: String,
    pub is_image: bool,
    #[allow(dead_code)]
    pub voice_id: Uuid,
    pub fade_in_ms: u32,
    pub loop_count: u32,
    pub start_ms: Option<u64>,
    pub end_ms: Option<u64>,
    /// For image cues: how long mpv holds the image before auto-completing.
    /// `None` = infinite (hold until explicitly stopped).
    pub display_duration_ms: Option<u64>,
    /// Video cues: freeze on the last frame at natural EOF (mpv `keep-open`).
    pub hold_last_frame: bool,
    /// Per-cue visual geometry applied alongside the load.
    pub geometry: VideoGeometry,
    /// Live source (camera / stream): apply low-latency demuxer options.
    pub live_source: bool,
}

pub(crate) enum FadePending {
    Stop,
}

/// State carried from a video `loadfile` (issued paused) to the
/// `MPV_EVENT_PLAYBACK_RESTART` that fires once frame 0 is decoded and on
/// screen.  At that point the engine reveals the overlay and unpauses, so
/// audio and video both start from frame 0 with no A/V offset and no
/// decoder-warmup freeze.
pub(crate) struct PendingVideoStart {
    /// Fade-from-black duration to run when the first frame is revealed
    /// (0 = hard cut).
    pub fade_in_ms: u32,
}

pub(crate) struct FadeAnimState {
    pub current_alpha: u8,
    pub target_alpha: u8,
    pub start_alpha: u8,
    pub duration_ms: u32,
    pub start_time: Instant,
    pub timer_active: bool,
    pub pending: Option<FadePending>,
}

impl FadeAnimState {
    /// Resting state at startup: opaque black, matching the convention used
    /// everywhere else (overlay stays at alpha=255 until content fades in).
    /// Also load-bearing on the GL path: the render loop only swaps a buffer
    /// when there's an mpv frame OR alpha > 0, so an idle alpha of 0 means the
    /// output window's surface never commits a single frame on Wayland — the
    /// compositor then refuses to map the window no matter what `set_visible`
    /// says, which is why toggling it manually used to show nothing until a
    /// video/image cue forced the first real frame.
    pub fn idle() -> Self {
        Self {
            current_alpha: 255,
            target_alpha: 255,
            start_alpha: 255,
            duration_ms: 0,
            start_time: Instant::now(),
            timer_active: false,
            pending: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Win32 window state
// ---------------------------------------------------------------------------

#[cfg(output_win32)]
pub(crate) struct OutputWndState {
    pub is_fullscreen: bool,
    pub saved_rect: (i32, i32, i32, i32),
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_default_is_neutral() {
        let g = VideoGeometry::default();
        assert!(g.is_default());
        assert!(!g.has_crop());
        assert_eq!(g.scale, 1.0);
        assert_eq!(g.fit_mode, FitMode::Fit);
        assert_eq!(g.mpv_video_zoom(), 0.0);
        assert_eq!(g.crop_rect_px(1920, 1080), None);
    }

    #[test]
    fn geometry_serde_roundtrip() {
        let g = VideoGeometry {
            fit_mode: FitMode::Fill,
            pan_x: -0.25,
            pan_y: 0.1,
            scale: 1.5,
            rotation: 90,
            crop_left: 0.1,
            crop_right: 0.2,
            crop_top: 0.0,
            crop_bottom: 0.05,
        };
        let json = serde_json::to_value(g).unwrap();
        let back: VideoGeometry = serde_json::from_value(json).unwrap();
        assert_eq!(g, back);
        assert!(!back.is_default());
    }

    #[test]
    fn geometry_deserializes_from_empty_object_with_defaults() {
        let g: VideoGeometry = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(g.is_default());
    }

    #[test]
    fn geometry_zoom_is_log2_of_scale() {
        let mut g = VideoGeometry { scale: 2.0, ..Default::default() };
        assert_eq!(g.mpv_video_zoom(), 1.0);
        g.scale = 0.5;
        assert_eq!(g.mpv_video_zoom(), -1.0);
        // Degenerate scale never produces -inf.
        g.scale = 0.0;
        assert!(g.mpv_video_zoom().is_finite());
    }

    #[test]
    fn geometry_fit_props_mapping() {
        let mut g = VideoGeometry::default();
        assert_eq!(g.fit_props(), ("yes", "0"));
        g.fit_mode = FitMode::Fill;
        assert_eq!(g.fit_props(), ("yes", "1"));
        g.fit_mode = FitMode::Stretch;
        assert_eq!(g.fit_props(), ("no", "0"));
    }

    #[test]
    fn geometry_crop_rect_px_math() {
        let g = VideoGeometry {
            crop_left: 0.1,
            crop_right: 0.1,
            crop_top: 0.25,
            crop_bottom: 0.25,
            ..Default::default()
        };
        // 1920x1080: 10% off each side, 25% off top/bottom.
        let (w, h, x, y) = g.crop_rect_px(1920, 1080).unwrap();
        assert_eq!((w, h, x, y), (1536, 540, 192, 270));
    }

    #[test]
    fn geometry_crop_clamps_excessive_edges() {
        let g = VideoGeometry {
            crop_left: 0.9,
            crop_right: 0.9,
            ..Default::default()
        };
        // 0.9 per edge clamps to 0.45 each — the rect never collapses.
        let (w, _h, x, _y) = g.crop_rect_px(1000, 1000).unwrap();
        assert!(w >= 2);
        assert!(x + w <= 1000);
    }

    #[test]
    fn geometry_crop_zero_source_is_none() {
        let g = VideoGeometry { crop_left: 0.1, ..Default::default() };
        assert_eq!(g.crop_rect_px(0, 1080), None);
    }

    #[test]
    fn transform_default_is_identity() {
        let t = OutputTransform::default();
        assert!(t.is_identity());
        assert_eq!(t.scale, 1.0);
    }

    #[test]
    fn transform_serde_roundtrip_and_empty_defaults() {
        let t = OutputTransform {
            pan_x: -0.1,
            pan_y: 0.05,
            scale: 0.8,
            rotation: 180.5,
            corners: [[0.01, 0.02], [0.0; 2], [0.0; 2], [-0.03, 0.0]],
        };
        let json = serde_json::to_value(t).unwrap();
        let back: OutputTransform = serde_json::from_value(json).unwrap();
        assert_eq!(t, back);

        let empty: OutputTransform = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(empty.is_identity());

        // A workspace saved when rotation was an integer still loads.
        let legacy: OutputTransform =
            serde_json::from_value(serde_json::json!({ "rotation": 90, "scale": 1.0 })).unwrap();
        assert_eq!(legacy.rotation, 90.0);
    }

    #[test]
    fn compose_identity_transform_keeps_cue_props() {
        let cue = VideoGeometry { pan_x: 0.2, scale: 2.0, rotation: 90, ..Default::default() };
        let props = compose_display_props(&cue, &OutputTransform::default());
        assert_eq!(props.zoom_log2, 1.0);
        assert_eq!(props.pan_x, 0.2);
        assert_eq!(props.pan_y, 0.0);
        assert_eq!(props.rotation, 90);
    }

    #[test]
    fn compose_scales_multiply_and_pans_add() {
        let cue = VideoGeometry { pan_x: 0.1, pan_y: -0.1, scale: 2.0, ..Default::default() };
        let t = OutputTransform { pan_x: 0.05, pan_y: 0.05, scale: 0.5, ..Default::default() };
        let props = compose_display_props(&cue, &t);
        // 2.0 × 0.5 = 1.0 → log2 = 0.
        assert_eq!(props.zoom_log2, 0.0);
        assert!((props.pan_x - 0.15).abs() < 1e-12);
        assert!((props.pan_y + 0.05).abs() < 1e-12);
    }

    #[test]
    fn compose_rotations_wrap_mod_360() {
        let cue = VideoGeometry { rotation: 270, ..Default::default() };
        let t = OutputTransform { rotation: 180.0, ..Default::default() };
        assert_eq!(compose_display_props(&cue, &t).rotation, 90);
    }

    #[test]
    fn compose_rounds_fractional_rotation_for_mpv() {
        // The win32 fallback rounds fine rotation to whole degrees.
        let t = OutputTransform { rotation: 45.6, ..Default::default() };
        assert_eq!(compose_display_props(&VideoGeometry::default(), &t).rotation, 46);
    }

    #[test]
    fn compose_degenerate_transform_scale_stays_finite() {
        let t = OutputTransform { scale: 0.0, ..Default::default() };
        let props = compose_display_props(&VideoGeometry::default(), &t);
        assert!(props.zoom_log2.is_finite());
    }

    #[test]
    fn test_pattern_urls() {
        assert!(TestPattern::Grid.mpv_url(1920, 1080).starts_with("av://lavfi:color=c=black:s=1920x1080"));
        assert_eq!(TestPattern::SmpteBars.mpv_url(1280, 720), "av://lavfi:smptehdbars=s=1280x720:r=1");
        assert_eq!(
            TestPattern::CustomImage("C:\\charts\\macbeth.png".into()).mpv_url(1920, 1080),
            "C:/charts/macbeth.png",
        );
        assert!(TestPattern::CustomImage("x".into()).is_file());
        assert!(!TestPattern::Grid.is_file());
        // Degenerate size never emits 0x0.
        assert!(TestPattern::White.mpv_url(0, 0).contains("s=2x2"));
    }

    #[test]
    fn test_pattern_serde_tagging() {
        let json = serde_json::to_value(TestPattern::Grid).unwrap();
        assert_eq!(json["kind"], "grid");
        let custom: TestPattern =
            serde_json::from_value(serde_json::json!({ "kind": "custom_image", "path": "a.png" }))
                .unwrap();
        assert_eq!(custom, TestPattern::CustomImage("a.png".into()));
    }
}
