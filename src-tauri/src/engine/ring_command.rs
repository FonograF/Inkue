//! Commands and status messages exchanged between the application layer and the
//! real-time audio callback via lock-free ring buffers.
//!
//! **Real-time safety:** these types must be trivially constructible without
//! heap allocation so that the audio callback can read them from a ring buffer
//! without allocating.

use uuid::Uuid;

/// A unique identifier for an audio voice (a single playing stream).
pub type VoiceId = Uuid;

/// Fade curve shape used when applying soft fades.
///
/// Defined here (engine layer) so the audio callback has no dependency on
/// the cue layer.  [`crate::cue::types::FadeCurve`] has a matching variant
/// set; [`crate::cue::audio_cue`] performs the conversion at the boundary.
#[derive(Debug, Clone, Copy)]
pub enum FadeCurve {
    /// Constant-rate gain change.
    Linear,
    /// Smooth S-shaped curve (QLab default): 3t² − 2t³.
    SCurve,
    /// Exponential (logarithmic perception) curve.
    Exponential,
}

impl FadeCurve {
    /// Map a normalised progress `t ∈ [0, 1]` to a gain multiplier `[0, 1]`.
    pub fn apply(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            FadeCurve::Linear => t,
            // Smooth-step: 3t² − 2t³
            FadeCurve::SCurve => t * t * (3.0 - 2.0 * t),
            // e^(k·t) − 1) / (e^k − 1), k=5 gives a noticeable exponential shape.
            FadeCurve::Exponential => {
                const K: f64 = 5.0;
                (K * t).exp_m1() / K.exp_m1()
            }
        }
    }
}

/// Commands sent *to* the audio thread from the application layer.
/// All variants must be `Send + 'static` and must not contain heap-allocated
/// data that would require deallocation inside the audio callback.
#[derive(Debug, Clone)]
pub enum AudioCommand {
    /// Begin playing a voice that has been pre-loaded into the voice pool.
    Play { voice_id: VoiceId },
    /// Stop a playing voice.  If `fade_ms` is non-zero, apply a soft fade-out
    /// with the given curve before silencing.
    Stop { voice_id: VoiceId, fade_ms: u32, fade_curve: FadeCurve },
    /// Pause a playing voice.
    Pause { voice_id: VoiceId },
    /// Resume a paused voice.
    Resume { voice_id: VoiceId },
    /// Set the linear gain for a voice (0.0 = silence, 1.0 = unity).
    SetGain { voice_id: VoiceId, gain: f32 },
    /// Set the stereo pan for a voice (-1.0 = left, 0.0 = center, 1.0 = right).
    SetPan { voice_id: VoiceId, pan: f32 },
    /// Set the master output gain (linear).
    SetMasterGain { gain: f32 },
}

/// Status updates sent *from* the audio thread to the application layer.
#[derive(Debug, Clone)]
pub enum AudioStatus {
    /// A voice has naturally reached the end of its audio data and stopped.
    Completed { voice_id: VoiceId },
    /// Current playback position of a voice in samples (for UI time display).
    Position {
        voice_id: VoiceId,
        /// Sample index from the start of the decoded audio.
        sample_pos: u64,
        /// Sample rate of the audio, for converting to wall-clock time.
        sample_rate: u32,
    },
    /// Peak and RMS levels measured in the last callback block.
    Levels {
        voice_id: VoiceId,
        peak_l: f32,
        peak_r: f32,
        rms_l: f32,
        rms_r: f32,
    },
    /// Master output peak levels.
    MasterLevels { peak_l: f32, peak_r: f32 },
}
