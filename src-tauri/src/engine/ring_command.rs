//! Commands and status messages exchanged between the application layer and the
//! real-time audio callback via lock-free ring buffers.
//!
//! **Real-time safety:** these types must be trivially constructible without
//! heap allocation so that the audio callback can read them from a ring buffer
//! without allocating.

use uuid::Uuid;

/// A unique identifier for an audio voice (a single playing stream).
pub type VoiceId = Uuid;

/// Number of samples in a baked [`CurveTable`].
///
/// 32 segments. The audio callback interpolates between samples, so the error
/// against a smooth analytic curve is under 0.1 % — inaudible on a gain
/// envelope — while keeping the table at 132 bytes.
pub const CURVE_TABLE_POINTS: usize = 33;

/// An arbitrary fade envelope, pre-sampled into a fixed-size table.
///
/// This is how a **custom** curve reaches the audio callback at all: control
/// points live in a `Vec` at the cue layer, which cannot be evaluated on the
/// RT thread (no allocation, and no deallocation either — dropping an `Arc`
/// there would be a bug). Baking to a plain `Copy` array sidesteps both.
/// QLab does the same thing; its `resolution` field is the giveaway.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurveTable {
    samples: [f32; CURVE_TABLE_POINTS],
}

impl CurveTable {
    /// Bake a table by sampling `shape` at evenly spaced points.
    pub fn from_fn(shape: impl Fn(f64) -> f64) -> Self {
        let mut samples = [0.0_f32; CURVE_TABLE_POINTS];
        for (index, sample) in samples.iter_mut().enumerate() {
            let t = index as f64 / (CURVE_TABLE_POINTS - 1) as f64;
            *sample = shape(t).clamp(0.0, 1.0) as f32;
        }
        Self { samples }
    }

    /// Read the envelope at `t ∈ [0, 1]`, interpolating between samples.
    /// Called from the audio callback: no branches beyond the bounds check,
    /// no allocation.
    pub fn eval(&self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        let scaled = t * (CURVE_TABLE_POINTS - 1) as f64;
        let index = scaled as usize;
        if index >= CURVE_TABLE_POINTS - 1 {
            return self.samples[CURVE_TABLE_POINTS - 1] as f64;
        }
        let frac = scaled - index as f64;
        let a = self.samples[index] as f64;
        let b = self.samples[index + 1] as f64;
        a + (b - a) * frac
    }

    /// The raw samples, for tests and for drawing the curve in the inspector.
    pub fn samples(&self) -> &[f32; CURVE_TABLE_POINTS] {
        &self.samples
    }
}

/// Fade curve shape used when applying soft fades.
///
/// Defined here (engine layer) so the audio callback has no dependency on
/// the cue layer.  [`crate::cue::curve::CurveShape`] is the authoring model and
/// bakes into [`FadeCurve::Table`] at the boundary.
///
/// The three analytic variants stay as variants rather than being baked too:
/// they cost one byte, they are what the vast majority of fades use, and
/// keeping them exact avoids any question about sampling error. Only a custom
/// shape pays for the table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FadeCurve {
    /// Constant-rate gain change.
    Linear,
    /// Smooth S-shaped curve (QLab default): 3t² − 2t³.
    SCurve,
    /// Exponential (logarithmic perception) curve.
    Exponential,
    /// A baked custom envelope — any shape the operator drew.
    Table(CurveTable),
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
            FadeCurve::Table(table) => table.eval(t),
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
    /// Instantly seek a voice to the given frame position.
    Seek { voice_id: VoiceId, frame_pos: u64 },
    /// Set the patch-gain multiplier (mixer fader) on every voice routed
    /// through the given Output Patch.
    SetPatchGain { patch_id: uuid::Uuid, gain: f32 },
    /// Release the target voice's current slice loop (QLab Devamp): the pass
    /// in progress finishes, then playback continues into the next slice —
    /// or stops at the slice boundary when `stop_at_end` is set.
    Devamp { voice_id: VoiceId, stop_at_end: bool },
    /// Set one crosspoint of a voice's level matrix, for live matrix editing.
    /// Creates the matrix on first use — [`LevelMatrix`](crate::engine::voice::LevelMatrix)
    /// is a fixed array, so this allocates nothing in the callback.  One
    /// command per cell keeps this enum (and the ring buffer) small.
    SetCrosspoint { voice_id: VoiceId, input: u8, output: u8, gain: f32 },
    /// Drop a voice's level matrix, returning it to pan + Output Patch routing.
    ClearLevelMatrix { voice_id: VoiceId },
    /// Panic: immediately silence every voice in the pool, whatever its state.
    /// Backstop for desynced cue bookkeeping — must never depend on voice IDs.
    StopAll,
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
    /// Peak levels of all voices routed through one Output Patch slot
    /// (`Voice::patch_slot`) during the last callback block.
    PatchLevels { slot: u8, peak_l: f32, peak_r: f32 },
}
