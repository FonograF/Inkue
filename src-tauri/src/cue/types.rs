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
}

impl std::fmt::Display for CueType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CueType::Audio => write!(f, "audio"),
            CueType::Memo => write!(f, "memo"),
            CueType::Wait => write!(f, "wait"),
            CueType::Group => write!(f, "group"),
            CueType::Fade => write!(f, "fade"),
            CueType::Stop => write!(f, "stop"),
            CueType::Video => write!(f, "video"),
            CueType::Image => write!(f, "image"),
            CueType::Osc   => write!(f, "osc"),
            CueType::Midi  => write!(f, "midi"),
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
    Blue,
    Purple,
    Pink,
    White,
    Black,
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
}

/// Parameters passed from a Fade Cue to the transport so it can resolve the
/// target voice and inject it back via [`super::traits::Cue::set_fade_voice`].
pub struct FadeAction {
    /// Cue number to fade (`None` = no target resolved yet; transport fills it in).
    pub target_cue_number: Option<String>,
    /// Target linear gain (0.0 = silence, 1.0 = unity).
    pub target_gain_linear: f32,
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

#[cfg(test)]
mod tests {
    use super::*;

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
