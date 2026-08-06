//! [`Voice`] — a single active audio stream in the voice pool.
//!
//! Mutable fields that are modified inside the audio callback use
//! [`std::cell::UnsafeCell`] (for fade state and loop counter, which are only
//! ever mutated from the callback), or [`std::sync::atomic`] types (for fields
//! that may also be written via ring-buffer commands processed inside the same
//! callback).

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::Arc;

use uuid::Uuid;

use super::ring_command::{FadeCurve, VoiceId};

// ---------------------------------------------------------------------------
// Voice state (atomic, shared with the audio callback)
// ---------------------------------------------------------------------------

/// Playback state, stored as u8 for atomic access.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceState {
    Idle = 0,
    Playing = 1,
    Paused = 2,
    FadingOut = 3,
    Stopped = 4,
}

impl From<u8> for VoiceState {
    fn from(v: u8) -> Self {
        match v {
            1 => VoiceState::Playing,
            2 => VoiceState::Paused,
            3 => VoiceState::FadingOut,
            4 => VoiceState::Stopped,
            _ => VoiceState::Idle,
        }
    }
}

// ---------------------------------------------------------------------------
// Fade state (used inside the audio callback — no allocations)
// ---------------------------------------------------------------------------

/// Direction of an active fade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FadeDirection {
    In,
    Out,
}

/// Fade progress tracked entirely with primitives so the audio callback can
/// update it without locks.
pub struct FadeState {
    pub direction: FadeDirection,
    /// Total fade duration in samples.
    pub total_samples: u64,
    /// Samples elapsed so far.
    pub elapsed_samples: u64,
    /// Shape of the fade curve.
    pub curve: FadeCurve,
}

impl FadeState {
    /// Compute the current gain multiplier [0.0, 1.0] from elapsed progress
    /// using the configured curve.
    pub fn gain(&self) -> f32 {
        if self.total_samples == 0 {
            return match self.direction {
                FadeDirection::In => 1.0,
                FadeDirection::Out => 0.0,
            };
        }
        let t = (self.elapsed_samples as f64 / self.total_samples as f64).clamp(0.0, 1.0);
        let s = self.curve.apply(t);
        match self.direction {
            FadeDirection::In => s as f32,
            FadeDirection::Out => (1.0 - s) as f32,
        }
    }

    /// Advance by `n` samples.  Returns `true` when the fade is complete.
    pub fn advance(&mut self, n: u64) -> bool {
        self.elapsed_samples = (self.elapsed_samples + n).min(self.total_samples);
        self.elapsed_samples >= self.total_samples
    }
}

// ---------------------------------------------------------------------------
// Slice program (QLab-style slices — used inside the audio callback)
// ---------------------------------------------------------------------------

/// Devamp request values for [`VoiceInner::devamp_request`].
pub const DEVAMP_NONE: u8 = 0;
/// Release the current slice's loop: finish the pass, then continue.
pub const DEVAMP_CONTINUE: u8 = 1;
/// Release the loop and stop the voice at the end of the current slice.
pub const DEVAMP_STOP: u8 = 2;

/// One slice segment, resolved to frames at GO time.
#[derive(Debug, Clone, Copy)]
pub struct SliceSegment {
    pub start_frame: u64,
    pub end_frame: u64,
    /// Times to play this segment; `u32::MAX` = vamp (infinite).
    pub play_count: u32,
}

/// Playback program for a sliced voice. Built once before `play_voice()`;
/// `current`/`remaining` are mutated **only** inside the audio callback.
/// The `segments` Vec is never resized after submission (no RT allocation).
#[derive(Debug, Clone)]
pub struct SliceProgram {
    pub segments: Vec<SliceSegment>,
    /// Index of the segment being played.
    pub current: usize,
    /// Remaining *additional* passes of the current segment
    /// (`u32::MAX` = vamp).
    pub remaining: u32,
}

impl SliceProgram {
    /// Build a program from `(start_frame, end_frame, play_count)` triples.
    /// Returns `None` for an empty list (plain linear playback).
    pub fn new(segments: Vec<SliceSegment>) -> Option<Self> {
        let first = segments.first()?;
        let remaining = first.play_count.saturating_sub(1);
        let remaining = if first.play_count == u32::MAX { u32::MAX } else { remaining };
        Some(Self { segments, current: 0, remaining })
    }
}

// ---------------------------------------------------------------------------
// Level matrix
// ---------------------------------------------------------------------------

/// Input channels a level matrix can address.  The mixer collapses every
/// source to stereo before routing (a mono file feeds both), so two is not a
/// limitation — it is what the engine actually carries.
pub const MATRIX_INPUTS: usize = 2;

/// Output channels a level matrix can address.  Fixed so the matrix is a
/// plain array the RT callback can read without allocating or bounds-guessing;
/// matches the per-patch VU slot count.
pub const MATRIX_OUTPUTS: usize = 16;

/// QLab-style crosspoint levels: how much of each input channel goes to each
/// output channel.
///
/// When a voice carries one, it **replaces** pan and the `out_l`/`out_r` pair —
/// those are the stereo special case of exactly this. Voice gain, patch gain
/// and fades still apply on top, so a Fade Cue keeps working unchanged.
#[derive(Debug, Clone)]
pub struct LevelMatrix {
    /// Linear gain per `[input][output]`.
    pub gains: [[f32; MATRIX_OUTPUTS]; MATRIX_INPUTS],
    /// One past the highest output index that carries anything, so the
    /// callback iterates over the used width instead of all 16.
    pub width: usize,
}

impl LevelMatrix {
    /// All crosspoints silent.  Used when a live edit creates the matrix on a
    /// voice that had none — it is a plain array, so building one inside the
    /// audio callback allocates nothing.
    pub fn silent() -> Self {
        Self { gains: [[0.0; MATRIX_OUTPUTS]; MATRIX_INPUTS], width: 0 }
    }

    /// Build from `[input][output]` linear gains, ignoring anything past the
    /// fixed capacity.
    ///
    /// Returns `None` only for an **empty** spec (no rows, or no columns).  A
    /// matrix whose crosspoints are all silent is kept: the operator asked for
    /// silence, and turning that back into pan routing would make the cue
    /// audible against their instruction.
    pub fn new(gains: &[Vec<f32>]) -> Option<Self> {
        if gains.iter().all(|row| row.is_empty()) {
            return None;
        }
        let mut m = Self::silent();
        for (i, row) in gains.iter().take(MATRIX_INPUTS).enumerate() {
            for (o, &g) in row.iter().take(MATRIX_OUTPUTS).enumerate() {
                m.gains[i][o] = g;
            }
        }
        m.recompute_width();
        Some(m)
    }

    /// Refresh the used width after a crosspoint changed.  Called from the RT
    /// callback, so it stays a bounded scan over the fixed array.
    pub fn recompute_width(&mut self) {
        self.width = 0;
        for row in &self.gains {
            for (o, &g) in row.iter().enumerate() {
                if g != 0.0 {
                    self.width = self.width.max(o + 1);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// VoiceInner — wraps all RT-mutable fields in UnsafeCell / atomics
// ---------------------------------------------------------------------------

/// All fields of a `Voice` that may be mutated from the audio callback.
///
/// # Safety contract
/// `VoiceInner` is shared between the non-RT thread (via `Arc`) and the RT
/// audio callback.  The contract is:
/// - `gain` and `pan` are `AtomicU32` (f32-bits) — safe for concurrent
///   read/write from both the RT and non-RT sides.
/// - `loops_remaining` is `AtomicU32` — only *decremented* inside the callback,
///   but can be *read* by the non-RT side.
/// - `fade` and `end_frame` are `UnsafeCell` — **only ever written from the
///   audio callback after the voice has been submitted to the pool**.  The
///   non-RT side writes these fields once, *before* calling `play_voice()`, so
///   there is no data race.
pub struct VoiceInner {
    /// Linear gain (f32 bits in an AtomicU32).  Written by `SetGain` commands
    /// processed inside the callback, and by the non-RT `play_voice` path.
    pub gain_bits: AtomicU32,
    /// Stereo pan (f32 bits).
    pub pan_bits: AtomicU32,
    /// Remaining loop repetitions.  `0` = play once.  `u32::MAX` = infinite.
    pub loops_remaining: AtomicU32,
    /// Playback rate multiplier (f32 bits).  1.0 = normal speed.
    /// Written once before play_voice(); read by the RT callback.
    pub rate_bits: AtomicU32,
    /// Linear gain multiplier of the voice's Output Patch (f32 bits).  1.0
    /// when the voice has no patch.  Written by `SetPatchGain` commands
    /// (mixer fader) and at submission; multiplied into the mix on top of
    /// the voice's own gain.
    pub patch_gain_bits: AtomicU32,
    /// Active fade, if any.  Mutated exclusively from inside the callback.
    /// SAFETY: only ever accessed from the single RT callback thread.
    pub fade: UnsafeCell<Option<FadeState>>,
    /// Optional end-frame marker.  Written once before voice is submitted.
    pub end_frame: UnsafeCell<Option<u64>>,
    /// Optional slice program (QLab slices).  Written once before the voice is
    /// submitted; `current`/`remaining` mutated only inside the callback.
    /// When present, `loops_remaining` and `end_frame` are ignored — the
    /// program owns the playback boundaries.
    pub slices: UnsafeCell<Option<SliceProgram>>,
    /// Pending devamp request ([`DEVAMP_NONE`] / [`DEVAMP_CONTINUE`] /
    /// [`DEVAMP_STOP`]).  Written from the non-RT side, consumed by the
    /// callback at the next slice boundary.
    pub devamp_request: AtomicU8,
    /// Optional crosspoint routing ([`LevelMatrix`]).  Written once before the
    /// voice is submitted, read-only in the callback.  `None` — the default —
    /// leaves the voice on the original pan + `out_l`/`out_r` mix path.
    pub level_matrix: UnsafeCell<Option<LevelMatrix>>,
}

// SAFETY: `VoiceInner` is never accessed from two threads simultaneously
// except via the documented atomic fields.
unsafe impl Sync for VoiceInner {}
unsafe impl Send for VoiceInner {}

impl VoiceInner {
    pub fn gain(&self) -> f32 {
        f32::from_bits(self.gain_bits.load(Ordering::Relaxed))
    }
    pub fn set_gain(&self, g: f32) {
        self.gain_bits.store(f32::to_bits(g), Ordering::Relaxed);
    }
    pub fn pan(&self) -> f32 {
        f32::from_bits(self.pan_bits.load(Ordering::Relaxed))
    }
    pub fn set_pan(&self, p: f32) {
        self.pan_bits.store(f32::to_bits(p), Ordering::Relaxed);
    }
    pub fn rate(&self) -> f32 {
        f32::from_bits(self.rate_bits.load(Ordering::Relaxed))
    }
    pub fn set_rate(&self, r: f32) {
        self.rate_bits.store(f32::to_bits(r), Ordering::Relaxed);
    }
    pub fn patch_gain(&self) -> f32 {
        f32::from_bits(self.patch_gain_bits.load(Ordering::Relaxed))
    }
    pub fn set_patch_gain(&self, g: f32) {
        self.patch_gain_bits.store(f32::to_bits(g), Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// LiveSource — live audio input read state (Mic Cue)
// ---------------------------------------------------------------------------

/// Read state for a voice fed by a live audio **input** instead of a fixed
/// sample buffer.
///
/// A live voice has no `samples`; it reads from the shared input feed identified
/// by `feed_id` (owned by the [`AudioEngine`](super::audio_engine::AudioEngine)),
/// resampling the input device clock to the output clock with adaptive drift
/// compensation.  The read cursor is mutated **only inside the audio callback**.
pub struct LiveSource {
    /// The input feed this voice reads from.
    pub feed_id: Uuid,
    /// Device input channel feeding the Left output (0-based).
    pub in_l: usize,
    /// Device input channel feeding the Right output (`== in_l` for a mono source).
    pub in_r: usize,
    /// Input device sample rate (Hz) — numerator of the base resample ratio.
    pub src_rate: u32,
    /// Absolute fractional read cursor, in input frames.  Callback-only.
    read_frame: UnsafeCell<f64>,
    /// `false` until the first callback initialises the read cursor.
    started: AtomicBool,
}

// SAFETY: `read_frame` is only ever written from the single RT callback thread.
unsafe impl Sync for LiveSource {}
unsafe impl Send for LiveSource {}

impl LiveSource {
    /// Create a live source bound to `feed_id`, taking input channels `in_l` /
    /// `in_r` (equal for mono) from a device running at `src_rate`.
    pub fn new(feed_id: Uuid, in_l: usize, in_r: usize, src_rate: u32) -> Self {
        Self {
            feed_id,
            in_l,
            in_r,
            src_rate,
            read_frame: UnsafeCell::new(0.0),
            started: AtomicBool::new(false),
        }
    }
    /// Current read cursor. SAFETY: only called from the audio callback.
    pub fn read_frame(&self) -> f64 {
        unsafe { *self.read_frame.get() }
    }
    /// Set the read cursor. SAFETY: only called from the audio callback.
    pub fn set_read_frame(&self, v: f64) {
        unsafe {
            *self.read_frame.get() = v;
        }
    }
    pub fn is_started(&self) -> bool {
        self.started.load(Ordering::Relaxed)
    }
    pub fn mark_started(&self) {
        self.started.store(true, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// Voice
// ---------------------------------------------------------------------------

/// Shared, thread-safe audio voice.
pub struct Voice {
    pub id: VoiceId,

    /// Fully decoded PCM samples, interleaved (L, R, L, R, …).
    pub samples: Arc<Vec<f32>>,

    /// Number of channels in `samples` (1 = mono, 2 = stereo).
    pub channels: u16,

    /// Sample rate of the decoded audio (e.g., 44100, 48000).
    pub sample_rate: u32,

    /// Current read position in frames (atomic).
    pub frame_pos: AtomicU64,

    /// [`VoiceState`] encoded as u8.
    pub state: AtomicU8,

    /// Interior-mutable fields modified from the RT callback.
    pub inner: Arc<VoiceInner>,

    /// Zero-based index of the output channel that receives the Left mix.
    /// Defaults to 0.  Set from the cue's Output Patch before submitting.
    pub out_l: usize,
    /// Zero-based index of the output channel that receives the Right mix.
    /// Defaults to 1.  Set from the cue's Output Patch before submitting.
    pub out_r: usize,

    /// When `Some`, this is a live (Mic Cue) voice that reads from an input feed
    /// instead of `samples`.  `None` for ordinary file/decoded voices.
    pub live: Option<Arc<LiveSource>>,

    /// `true` when an Output Patch set `out_l`/`out_r` explicitly.  Unpatched
    /// voices are shifted to the engine's default output offset (the selected
    /// ASIO pair) at submission.  Written once before `play_voice()`.
    pub patched: bool,
    /// Output Patch this voice was routed through, for live mixer-fader
    /// updates (`SetPatchGain`).  Written once before `play_voice()`.
    pub patch_id: Option<Uuid>,
    /// Index of the patch in the workspace table at GO time — the per-patch
    /// VU accumulator slot (`PATCH_VU_SLOTS`-bounded).  `None` = unmetered.
    pub patch_slot: Option<u8>,
}

// AtomicU64 is not in std for 32-bit targets, but for our Windows x64 target it is fine.
use std::sync::atomic::AtomicU64;

impl Voice {
    /// Create a new idle voice from pre-decoded samples.
    pub fn new(
        samples: Arc<Vec<f32>>,
        channels: u16,
        sample_rate: u32,
        gain: f32,
        pan: f32,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            samples,
            channels,
            sample_rate,
            frame_pos: AtomicU64::new(0),
            state: AtomicU8::new(VoiceState::Idle as u8),
            inner: Arc::new(VoiceInner {
                gain_bits: AtomicU32::new(f32::to_bits(gain)),
                pan_bits: AtomicU32::new(f32::to_bits(pan)),
                loops_remaining: AtomicU32::new(0),
                rate_bits: AtomicU32::new(f32::to_bits(1.0_f32)),
                patch_gain_bits: AtomicU32::new(f32::to_bits(1.0_f32)),
                fade: UnsafeCell::new(None),
                end_frame: UnsafeCell::new(None),
                slices: UnsafeCell::new(None),
                devamp_request: AtomicU8::new(DEVAMP_NONE),
                level_matrix: UnsafeCell::new(None),
            }),
            out_l: 0,
            out_r: 1,
            live: None,
            patched: false,
            patch_id: None,
            patch_slot: None,
        }
    }

    /// Create a live (Mic Cue) voice that reads from an input feed instead of a
    /// fixed sample buffer.  `sample_rate` is the **output** rate so soft-fade
    /// durations (computed in [`crate::engine::audio_engine`]) land in
    /// wall-clock milliseconds.
    pub fn new_live(live: LiveSource, sample_rate: u32, gain: f32, pan: f32) -> Self {
        Self {
            id: Uuid::new_v4(),
            samples: Arc::new(Vec::new()),
            channels: 2,
            sample_rate,
            frame_pos: AtomicU64::new(0),
            state: AtomicU8::new(VoiceState::Idle as u8),
            inner: Arc::new(VoiceInner {
                gain_bits: AtomicU32::new(f32::to_bits(gain)),
                pan_bits: AtomicU32::new(f32::to_bits(pan)),
                loops_remaining: AtomicU32::new(0),
                rate_bits: AtomicU32::new(f32::to_bits(1.0_f32)),
                patch_gain_bits: AtomicU32::new(f32::to_bits(1.0_f32)),
                fade: UnsafeCell::new(None),
                end_frame: UnsafeCell::new(None),
                slices: UnsafeCell::new(None),
                devamp_request: AtomicU8::new(DEVAMP_NONE),
                level_matrix: UnsafeCell::new(None),
            }),
            out_l: 0,
            out_r: 1,
            live: Some(Arc::new(live)),
            patched: false,
            patch_id: None,
            patch_slot: None,
        }
    }

    /// Return the total number of frames in the voice.
    pub fn total_frames(&self) -> u64 {
        if self.channels == 0 { 0 } else { self.samples.len() as u64 / self.channels as u64 }
    }

    /// Current frame position.
    pub fn current_frame(&self) -> u64 {
        self.frame_pos.load(Ordering::Relaxed)
    }

    pub fn set_playing(&self) {
        self.state.store(VoiceState::Playing as u8, Ordering::Release);
    }
    pub fn set_paused(&self) {
        self.state.store(VoiceState::Paused as u8, Ordering::Release);
    }
    pub fn set_stopped(&self) {
        self.state.store(VoiceState::Stopped as u8, Ordering::Release);
    }

    pub fn voice_state(&self) -> VoiceState {
        VoiceState::from(self.state.load(Ordering::Acquire))
    }

    /// Compute pan gains `(left, right)`.
    pub fn pan_gains(&self) -> (f32, f32) {
        let pan = self.inner.pan();
        let gain = self.inner.gain();
        let left = ((1.0 - pan) * 0.5).clamp(0.0, 1.0).sqrt();
        let right = ((1.0 + pan) * 0.5).clamp(0.0, 1.0).sqrt();
        (left * gain, right * gain)
    }
}
