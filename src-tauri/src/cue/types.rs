//! Core types shared across the cue system.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for a cue, backed by a UUID v4.
pub type CueId = Uuid;

/// All supported cue types. New types can be added here and registered in [`super::registry::CueRegistry`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CueType {
    Audio,
    Memo,
    Wait,
    Group,
    Fade,
    /// Stops all currently-running cues when triggered.
    Stop,
    /// Plays a video file on a video output surface window.
    Video,
    /// Displays a static or animated image on an output surface window.
    Image,
    /// Sends one or more OSC messages over UDP when triggered.
    Osc,
    /// Sends one or more MIDI messages when triggered.
    Midi,
    /// Fades patched DMX fixtures to a target look (DMX-over-IP).
    Light,
    /// Routes a live audio input through the engine (mic / line).
    Mic,
    /// Generates a SMPTE timecode stream (MTC or LTC).
    Timecode,
    /// Displays formatted text on the output surface via the mpv subtitle layer.
    Text,
    /// Shows a live camera / capture / network video feed on the output surface.
    Camera,
    /// Releases a vamping (infinitely-looping) slice on its target cues.
    Devamp,

    // --- Command cues -------------------------------------------------------
    // Cues whose action is performed *on other cues*. They share one
    // implementation ([`super::control_cue::ControlCue`]) and differ only by
    // the action they carry, but stay distinct types so each keeps its own
    // colour, its own row label, and a 1:1 mapping when importing from QLab.
    /// Triggers its target cues (QLab Start).
    Start,
    /// Pauses its target cues.
    Pause,
    /// Resumes its paused target cues.
    Resume,
    /// Brings its target cues up paused at their start position (QLab Load).
    Load,
    /// Returns its target cues to Standby.
    Reset,
    /// Moves the Playhead to its target cue.
    Goto,
    /// Enables its target cues.
    Arm,
    /// Disables its target cues.
    Disarm,

    /// Runs an external command or script (QLab's Script Cue, made portable).
    Script,
}

impl std::fmt::Display for CueType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CueType::Audio    => write!(f, "audio"),
            CueType::Memo     => write!(f, "memo"),
            CueType::Wait     => write!(f, "wait"),
            CueType::Group    => write!(f, "group"),
            CueType::Fade     => write!(f, "fade"),
            CueType::Stop     => write!(f, "stop"),
            CueType::Video    => write!(f, "video"),
            CueType::Image    => write!(f, "image"),
            CueType::Osc      => write!(f, "osc"),
            CueType::Midi     => write!(f, "midi"),
            CueType::Light    => write!(f, "light"),
            CueType::Mic      => write!(f, "mic"),
            CueType::Timecode => write!(f, "timecode"),
            CueType::Text     => write!(f, "text"),
            CueType::Camera   => write!(f, "camera"),
            CueType::Devamp   => write!(f, "devamp"),
            CueType::Start    => write!(f, "start"),
            CueType::Pause    => write!(f, "pause"),
            CueType::Resume   => write!(f, "resume"),
            CueType::Load     => write!(f, "load"),
            CueType::Reset    => write!(f, "reset"),
            CueType::Goto     => write!(f, "goto"),
            CueType::Arm      => write!(f, "arm"),
            CueType::Disarm   => write!(f, "disarm"),
            CueType::Script   => write!(f, "script"),
        }
    }
}

/// Lifecycle state of a cue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CueState {
    /// Ready to be triggered; not currently playing.
    #[default]
    Standby,
    /// Currently executing its action (pre-wait, action, or post-wait phase).
    Running,
    /// Execution has been suspended mid-action.
    Paused,
    /// Execution has finished naturally.
    Completed,
}

/// Determines what happens after the Post-Wait expires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContinueMode {
    /// Wait for manual GO before the next cue fires.
    #[default]
    DoNotContinue,
    /// Automatically GO the next cue after this cue's Post-Wait expires.
    AutoContinue,
    /// Automatically GO the next cue as soon as this cue's action starts (after Pre-Wait).
    AutoFollow,
}

/// Color label displayed on a cue row in the Cue List, matching QLab's palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CueColor {
    #[default]
    None,
    Red,
    Orange,
    Yellow,
    Green,
    Cyan,
    Blue,
    Purple,
    Pink,
    White,
    Black,
}

/// Play count meaning "loop this slice forever" (a *vamp*).
pub const PLAY_COUNT_INFINITE: u32 = u32::MAX;

/// QLab-style slices on a media cue's timeline.
///
/// `markers` split the clip into `markers.len() + 1` segments; segment *i*
/// plays `play_counts[i]` times ([`PLAY_COUNT_INFINITE`] = vamp until a Devamp
/// Cue releases it). Empty markers = no slicing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SliceList {
    /// Marker positions in ms from **file** start, sorted ascending.
    pub markers: Vec<u64>,
    /// Play count per segment — always `markers.len() + 1` entries after
    /// [`Self::normalize`]; `u32::MAX` = infinite (vamp).
    pub play_counts: Vec<u32>,
}

impl SliceList {
    /// `true` when the cue has no slice markers (plain linear playback).
    pub fn is_empty(&self) -> bool {
        self.markers.is_empty()
    }

    /// Sort markers, drop duplicates and resize `play_counts` to
    /// `markers.len() + 1` (new segments default to 1 play).
    pub fn normalize(&mut self) {
        self.markers.sort_unstable();
        self.markers.dedup();
        self.play_counts.resize(self.markers.len() + 1, 1);
        for c in &mut self.play_counts {
            if *c == 0 {
                *c = 1;
            }
        }
    }

    /// Resolve the slice segments within the clip window
    /// `[clip_start_ms, clip_end_ms)` as `(start_ms, end_ms, play_count)`.
    ///
    /// Markers outside the window are ignored; segment edges are clamped so
    /// the result always tiles the window exactly. Returns an empty vec when
    /// there is no marker inside the window (plain playback).
    pub fn segments(&self, clip_start_ms: u64, clip_end_ms: u64) -> Vec<(u64, u64, u32)> {
        if self.is_empty() || clip_end_ms <= clip_start_ms {
            return Vec::new();
        }
        let mut normalized = self.clone();
        normalized.normalize();

        let mut segments = Vec::with_capacity(normalized.markers.len() + 1);
        let mut cursor = clip_start_ms;
        for (i, &m) in normalized.markers.iter().enumerate() {
            if m <= clip_start_ms || m >= clip_end_ms {
                continue;
            }
            segments.push((cursor, m, normalized.play_counts[i]));
            cursor = m;
        }
        if segments.is_empty() {
            return Vec::new();
        }
        let last_count = *normalized.play_counts.last().unwrap_or(&1);
        segments.push((cursor, clip_end_ms, last_count));
        segments
    }
}

/// Available fade curve shapes, matching QLab's options.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FadeCurve {
    Linear,
    /// Smooth S-shaped curve (QLab default).
    #[default]
    SCurve,
    Exponential,
}

impl FadeCurve {
    /// Compute gain multiplier [0.0, 1.0] for a normalized progress `t` in [0.0, 1.0].
    pub fn apply(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            FadeCurve::Linear => t,
            FadeCurve::SCurve => {
                // Smooth-step: 3t² - 2t³
                t * t * (3.0 - 2.0 * t)
            }
            FadeCurve::Exponential => {
                if t == 0.0 {
                    0.0
                } else {
                    // Map [0,1] to an exponential curve that starts near 0 and ends at 1.
                    (10.0_f64.powf(t) - 1.0) / 9.0
                }
            }
        }
    }
}

/// How a Group Cue triggers its children.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GroupMode {
    /// All children fire at the same time. The Group completes when every child has finished.
    #[default]
    Simultaneous,
    /// Children fire one after another. Each child's Continue Mode is respected:
    /// Auto-Continue chains after Post-Wait, Auto-Follow chains at action start,
    /// Do Not Continue stops the sequence.
    Sequential,
    /// Like Sequential, but starting a child stops any other still-playing child
    /// in the group (exclusive — never two children audible at once). With group
    /// loop enabled it wraps from the last child back to the first instead of
    /// ending. QLab's "Playlist" mode.
    Playlist,
    /// Each GO fires one randomly-chosen child. Every child plays once before any
    /// child repeats (shuffle-bag; refilled + reshuffled when it empties). QLab's
    /// "Start random" mode.
    StartRandom,
}

/// Parameters passed from a Fade Cue to the transport so it can resolve
/// target voices and inject them back via [`super::traits::Cue::set_fade_voices`].
pub struct FadeAction {
    /// UUIDs of cues to fade (empty = no-op).
    pub target_cue_ids: Vec<CueId>,
    /// Target linear gain for audio (0.0 = silence, 1.0 = unity).
    pub target_gain_linear: f32,
    /// Explicit visual (GL overlay) target alpha.  `None` = derive from
    /// `target_gain_linear` (legacy behaviour); `Some(alpha)` = independent
    /// from the audio target so brightness and volume can be set separately.
    pub target_visual_alpha: Option<u8>,
    /// Fade duration in milliseconds.
    pub duration_ms: u64,
    /// Curve shape.
    pub curve: FadeCurve,
    /// Whether to stop the target cue after the fade completes.
    pub stop_at_end: bool,
}

/// Specification for a single fade (in or out).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FadeSpec {
    /// Duration of the fade in milliseconds.
    pub duration_ms: u64,
    /// Shape of the fade curve.
    pub curve: FadeCurve,
}

impl FadeSpec {
    /// Create a new fade spec with the given duration and default S-curve.
    pub fn new(duration_ms: u64) -> Self {
        Self {
            duration_ms,
            curve: FadeCurve::default(),
        }
    }

    /// Convert to [`std::time::Duration`].
    pub fn duration(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.duration_ms)
    }
}

/// Convert a dB value to a linear gain multiplier.
/// Values below -60 dB are treated as silence (0.0).
pub fn db_to_linear(db: f64) -> f64 {
    if db <= -60.0 {
        0.0
    } else {
        10.0_f64.powf(db / 20.0)
    }
}

/// Convert a linear gain multiplier to dB.
/// A gain of 0.0 returns -60.0 (silence floor).
pub fn linear_to_db(gain: f64) -> f64 {
    if gain <= 0.0 {
        -60.0
    } else {
        20.0 * gain.log10()
    }
}

/// How long a natural-end (EOF) fade should run, once due.
///
/// Returns `Some(remaining_ms)` when the remaining action time has dropped
/// inside the configured fade-out window — the fade then lands exactly on the
/// cue's natural end — and `None` while it is still too early, or when no fade
/// is configured.  Shared by every cue that fades itself out at EOF instead of
/// hard-cutting: [`AudioCue`](crate::cue::audio_cue::AudioCue) (sound),
/// [`VideoCue`](crate::cue::video_cue::VideoCue) (picture *and* sound) and
/// [`ImageCue`](crate::cue::image_cue::ImageCue) (picture).
pub fn eof_fade_remaining_ms(
    action_elapsed: std::time::Duration,
    total: std::time::Duration,
    fade_ms: u64,
) -> Option<u32> {
    if fade_ms == 0 {
        return None;
    }
    let remaining = total.checked_sub(action_elapsed)?;
    let remaining_ms = remaining.as_millis() as u64;
    if remaining_ms <= fade_ms {
        Some(remaining_ms.max(1).min(u32::MAX as u64) as u32)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn eof_fade_not_due_early() {
        assert_eq!(
            eof_fade_remaining_ms(Duration::from_secs(1), Duration::from_secs(10), 2000),
            None,
        );
    }

    #[test]
    fn eof_fade_due_inside_window() {
        // 10s cue, 2s fade, 8.5s elapsed → 1500ms remaining.
        assert_eq!(
            eof_fade_remaining_ms(Duration::from_millis(8500), Duration::from_secs(10), 2000),
            Some(1500),
        );
    }

    #[test]
    fn eof_fade_none_without_fade_configured() {
        assert_eq!(
            eof_fade_remaining_ms(Duration::from_secs(9), Duration::from_secs(10), 0),
            None,
        );
    }

    #[test]
    fn eof_fade_none_past_the_end() {
        assert_eq!(
            eof_fade_remaining_ms(Duration::from_secs(11), Duration::from_secs(10), 2000),
            None,
        );
    }

    #[test]
    fn eof_fade_clamps_to_at_least_one_ms() {
        assert_eq!(
            eof_fade_remaining_ms(Duration::from_secs(10), Duration::from_secs(10), 2000),
            Some(1),
        );
    }

    #[test]
    fn db_to_linear_unity() {
        let gain = db_to_linear(0.0);
        assert!((gain - 1.0).abs() < 1e-9, "0 dB should be unity gain 1.0, got {gain}");
    }

    #[test]
    fn db_to_linear_silence() {
        assert_eq!(db_to_linear(-60.0), 0.0);
        assert_eq!(db_to_linear(-100.0), 0.0);
    }

    #[test]
    fn db_linear_roundtrip() {
        for db in [-12.0_f64, -6.0, -3.0, 0.0, 3.0, 6.0, 12.0] {
            let roundtrip = linear_to_db(db_to_linear(db));
            assert!(
                (roundtrip - db).abs() < 1e-9,
                "Roundtrip failed for {db} dB: got {roundtrip}"
            );
        }
    }

    #[test]
    fn fade_curve_boundaries() {
        for curve in [FadeCurve::Linear, FadeCurve::SCurve, FadeCurve::Exponential] {
            let start = curve.apply(0.0);
            let end = curve.apply(1.0);
            assert!(start.abs() < 1e-9, "{curve:?} at t=0 should be 0, got {start}");
            assert!((end - 1.0).abs() < 1e-9, "{curve:?} at t=1 should be 1, got {end}");
        }
    }

    #[test]
    fn slice_list_normalize_sorts_and_pads_counts() {
        let mut s = SliceList { markers: vec![5000, 2000, 5000], play_counts: vec![0] };
        s.normalize();
        assert_eq!(s.markers, vec![2000, 5000]);
        assert_eq!(s.play_counts, vec![1, 1, 1]);
    }

    #[test]
    fn slice_list_segments_tile_the_clip_window() {
        let s = SliceList { markers: vec![2000, 6000], play_counts: vec![1, PLAY_COUNT_INFINITE, 2] };
        let segs = s.segments(0, 10000);
        assert_eq!(segs, vec![
            (0, 2000, 1),
            (2000, 6000, PLAY_COUNT_INFINITE),
            (6000, 10000, 2),
        ]);
    }

    #[test]
    fn slice_list_segments_ignore_markers_outside_clip() {
        let s = SliceList { markers: vec![500, 4000, 9500], play_counts: vec![1, 3, 1, 1] };
        // Clip trimmed to [1000, 8000): only the 4000 marker survives.
        let segs = s.segments(1000, 8000);
        assert_eq!(segs, vec![(1000, 4000, 3), (4000, 8000, 1)]);
    }

    #[test]
    fn slice_list_segments_empty_without_markers_in_window() {
        let s = SliceList { markers: vec![9000], play_counts: vec![1, 1] };
        assert!(s.segments(0, 5000).is_empty());
        assert!(SliceList::default().segments(0, 5000).is_empty());
    }

    #[test]
    fn fade_curve_midpoint() {
        // Linear must be exactly 0.5 at t=0.5
        assert!((FadeCurve::Linear.apply(0.5) - 0.5).abs() < 1e-9);
        // S-curve must be exactly 0.5 at t=0.5 (symmetric)
        assert!((FadeCurve::SCurve.apply(0.5) - 0.5).abs() < 1e-9);
        // Exponential must be strictly between 0 and 0.5 at t=0.5 (slower start)
        let exp_mid = FadeCurve::Exponential.apply(0.5);
        assert!(exp_mid > 0.0 && exp_mid < 0.5, "Exponential midpoint {exp_mid}");
    }
}
