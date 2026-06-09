//! [`AudioCue`] — plays an audio file through the audio engine.
//!
//! This is the primary cue type for WinCue.  It decodes an audio file using
//! symphonia and submits a [`Voice`](crate::engine::voice::Voice) to the
//! [`AudioEngine`](crate::engine::AudioEngine) when triggered.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::engine::{
    ring_command::{FadeCurve as EngineFadeCurve, VoiceId},
    voice::{FadeDirection, FadeState, Voice},
};

use super::{
    context::{CueContext, CueEvent},
    traits::{Cue, CueFactory},
    types::{
        ContinueMode, CueColor, CueId, CueState, CueType, FadeCurve, FadeSpec,
    },
};

// ---------------------------------------------------------------------------
// AudioCue
// ---------------------------------------------------------------------------

/// A cue that plays an audio file through the audio engine.
pub struct AudioCue {
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

    // --- Audio-specific ---
    /// Path to the audio file, relative to the workspace directory.
    pub file_path: Option<PathBuf>,
    /// Volume in dB (-60 to +12).
    pub volume_db: f64,
    /// Stereo pan (-1.0 to +1.0).
    pub pan: f32,
    /// Optional fade in specification.
    pub fade_in: Option<FadeSpec>,
    /// Optional fade out specification (also used on soft stop).
    pub fade_out: Option<FadeSpec>,
    /// Start playback at this offset into the file.
    pub start_time: Option<Duration>,
    /// Stop playback at this offset into the file.
    pub end_time: Option<Duration>,
    /// Number of extra loop repetitions (0 = play once, u32::MAX = infinite).
    pub loop_count: u32,
    /// Output patch to route through.
    pub output_patch_id: Option<uuid::Uuid>,
    /// Playback rate multiplier (1.0 = normal speed).
    pub rate: f64,

    // --- Runtime ---
    /// Pre-decoded samples, loaded by `load()`.
    decoded_samples: Option<Arc<Vec<f32>>>,
    decoded_channels: u16,
    decoded_sample_rate: u32,
    /// The voice ID currently in use, if any.
    active_voice_id: Option<VoiceId>,
    /// Cached duration computed from the decoded samples.
    cached_duration: Option<Duration>,
    /// `true` between `go()` and the moment the audio action actually starts
    /// (i.e. while waiting for `pre_wait` to expire).
    in_pre_wait: bool,
    /// Incremented on every `go()` call.  Kept for diagnostics / future use.
    play_generation: u64,
    /// Set to `true` by [`Transport::go`] immediately after firing the
    /// Auto-Continue chain, so the event loop never double-fires it.
    auto_continue_fired: bool,
    /// Elapsed time accumulated before the most recent pause (mirrors WaitCue pattern).
    elapsed_before_pause: Duration,
    /// Action-elapsed time accumulated before the most recent pause.
    action_elapsed_before_pause: Duration,
}

impl AudioCue {
    /// Create a new, empty Audio Cue with a fresh UUID.
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::from("Audio Cue"),
            number: None,
            notes: String::new(),
            color: CueColor::Blue,
            state: CueState::Standby,
            pre_wait: Duration::ZERO,
            post_wait: Duration::ZERO,
            started_at: None,
            action_started_at: None,
            continue_mode: ContinueMode::DoNotContinue,
            file_path: None,
            volume_db: 0.0,
            pan: 0.0,
            fade_in: None,
            fade_out: None,
            start_time: None,
            end_time: None,
            loop_count: 0,
            output_patch_id: None,
            rate: 1.0,
            decoded_samples: None,
            decoded_channels: 2,
            decoded_sample_rate: 44100,
            active_voice_id: None,
            cached_duration: None,
            in_pre_wait: false,
            play_generation: 0,
            auto_continue_fired: false,
            elapsed_before_pause: Duration::ZERO,
            action_elapsed_before_pause: Duration::ZERO,
        }
    }

    /// Return the active voice ID if the cue is currently playing.
    pub fn voice_id(&self) -> Option<VoiceId> {
        self.active_voice_id
    }

    /// Convert a [`FadeCurve`] from the cue layer to the engine layer.
    fn engine_curve(c: FadeCurve) -> EngineFadeCurve {
        match c {
            FadeCurve::Linear => EngineFadeCurve::Linear,
            FadeCurve::SCurve => EngineFadeCurve::SCurve,
            FadeCurve::Exponential => EngineFadeCurve::Exponential,
        }
    }

    /// Start the audio action (submit a voice to the engine).
    ///
    /// Called either directly from `go()` when `pre_wait` is zero, or from
    /// `tick()` once the pre-wait timer has expired.
    fn start_audio_action(&mut self, context: &CueContext) -> Result<()> {
        let samples = match &self.decoded_samples {
            Some(s) => Arc::clone(s),
            None => return Err(anyhow!(
                "AudioCue '{}': audio not loaded — assign a file and try again",
                self.name
            )),
        };

        let gain = crate::cue::types::db_to_linear(self.volume_db) as f32;
        let mut voice = Voice::new(
            samples,
            self.decoded_channels,
            self.decoded_sample_rate,
            gain,
            self.pan,
        );

        voice.inner.loops_remaining.store(self.loop_count, std::sync::atomic::Ordering::Relaxed);
        // Combine the user-specified playback-rate with the ratio needed to
        // compensate for a sample-rate mismatch between the audio file and the
        // output device.  Without this correction a 44 100 Hz file played on a
        // 48 000 Hz device would be heard at 108.8% speed and wrong pitch.
        let device_sr = context.audio_engine.sample_rate();
        let sr_ratio = self.decoded_sample_rate as f64 / device_sr.max(1) as f64;
        voice.inner.set_rate((self.rate * sr_ratio) as f32);

        // Apply start/end time markers (written before play_voice; no RT thread yet).
        if let Some(end) = self.end_time {
            let end_frame = (end.as_secs_f64() * self.decoded_sample_rate as f64) as u64;
            // SAFETY: written once before play_voice(); RT thread has not started yet.
            unsafe { *voice.inner.end_frame.get() = Some(end_frame); }
        }
        if let Some(start) = self.start_time {
            let start_frame = (start.as_secs_f64() * self.decoded_sample_rate as f64) as u64;
            voice.frame_pos.store(start_frame, std::sync::atomic::Ordering::Relaxed);
        }

        // Apply fade-in if configured (written before play_voice; RT thread not running yet).
        if let Some(ref fi) = self.fade_in {
            let total = (fi.duration_ms * self.decoded_sample_rate as u64) / 1000;
            // SAFETY: same as above — single writer before submission.
            unsafe {
                *voice.inner.fade.get() = Some(FadeState {
                    direction: FadeDirection::In,
                    total_samples: total,
                    elapsed_samples: 0,
                    curve: Self::engine_curve(fi.curve),
                });
            }
        }

        // Apply Output Patch channel routing.  Look up the cue's assigned patch
        // (falling back to the workspace default); if found, map its first two
        // channel indices to the voice's L/R output slots.
        if let Some(patch) = context.resolve_patch(self.output_patch_id) {
            if let Some(&ch_l) = patch.channels.first() {
                voice.out_l = ch_l as usize;
            }
            if let Some(&ch_r) = patch.channels.get(1) {
                voice.out_r = ch_r as usize;
            } else if let Some(&ch_l) = patch.channels.first() {
                // Mono patch — route both L and R to the same channel.
                voice.out_r = ch_l as usize;
            }
        }

        let voice_id = context.audio_engine.play_voice(voice)?;
        self.active_voice_id = Some(voice_id);
        self.action_started_at = Some(Instant::now());
        self.in_pre_wait = false;

        context.emit(CueEvent::ActionStarted { cue_id: self.id });
        Ok(())
    }

    /// Decode an audio file to interleaved f32 samples.
    ///
    /// This is a pure function (no `self` mutation) and must be called on a
    /// non-RT thread without holding any workspace locks.  The result is
    /// pushed back into the cue via [`accept_preloaded_audio`].  Delegates to
    /// the shared [`media_decode::decode_audio_track`] so audio files and the
    /// audio track of video containers decode through one code path.
    pub fn decode_file(path: &Path) -> Result<(Vec<f32>, u16, u32)> {
        crate::cue::media_decode::decode_audio_track(path)?
            .ok_or_else(|| anyhow!("No audio track in file: {}", path.display()))
    }
}

impl Default for AudioCue {
    fn default() -> Self {
        Self::new()
    }
}

impl Cue for AudioCue {
    fn id(&self) -> CueId { self.id }
    fn cue_type(&self) -> CueType { CueType::Audio }
    fn name(&self) -> &str { &self.name }
    fn set_name(&mut self, name: String) { self.name = name; }
    fn number(&self) -> Option<&str> { self.number.as_deref() }
    fn set_number(&mut self, number: Option<String>) { self.number = number; }
    fn notes(&self) -> &str { &self.notes }
    fn set_notes(&mut self, notes: String) { self.notes = notes; }
    fn color(&self) -> CueColor { self.color }
    fn set_color(&mut self, color: CueColor) { self.color = color; }
    fn state(&self) -> CueState { self.state }

    fn load(&mut self, _context: &CueContext) -> Result<()> {
        let path = match &self.file_path {
            Some(p) => p.clone(),
            None => return Ok(()), // No file assigned; nothing to load.
        };

        let (samples, channels, sample_rate) = Self::decode_file(&path)?;
        self.cached_duration = Some(Duration::from_secs_f64(
            samples.len() as f64 / channels as f64 / sample_rate as f64,
        ));
        self.decoded_channels = channels;
        self.decoded_sample_rate = sample_rate;
        self.decoded_samples = Some(Arc::new(samples));
        Ok(())
    }

    fn accept_preloaded_audio(
        &mut self,
        samples: std::sync::Arc<Vec<f32>>,
        channels: u16,
        sample_rate: u32,
        duration: std::time::Duration,
    ) {
        self.decoded_samples = Some(samples);
        self.decoded_channels = channels;
        self.decoded_sample_rate = sample_rate;
        self.cached_duration = Some(duration);
    }

    fn go(&mut self, context: &CueContext) -> Result<()> {
        if self.state == CueState::Running {
            return Ok(()); // Already playing; ignore duplicate GO.
        }

        // New play: bump generation and clear the auto-continue flag so the
        // transport can fire the chain again for this play.
        self.play_generation = self.play_generation.wrapping_add(1);
        self.auto_continue_fired = false;

        self.state = CueState::Running;
        self.started_at = Some(Instant::now());

        if !self.pre_wait.is_zero() {
            // Pre-wait active: record the start time and defer the action.
            // tick() will call start_audio_action() once the timer expires.
            self.in_pre_wait = true;
            return Ok(());
        }

        // No pre-wait: start the audio action immediately.
        // On failure, roll back to Standby so callers (e.g. GroupCue) don't
        // see a permanently-Running cue that will never complete.
        if let Err(e) = self.start_audio_action(context) {
            self.state = CueState::Standby;
            self.started_at = None;
            return Err(e);
        }
        Ok(())
    }

    fn stop(&mut self, context: &CueContext) -> Result<()> {
        self.in_pre_wait = false; // Cancel any pending pre-wait.
        if let Some(vid) = self.active_voice_id.take() {
            let (fade_ms, fade_curve) = self.fade_out
                .as_ref()
                .map(|f| (f.duration_ms as u32, Self::engine_curve(f.curve)))
                .unwrap_or((context.stop_fade_ms, EngineFadeCurve::SCurve));
            context.audio_engine.stop_voice(vid, fade_ms, fade_curve)?;
        }
        self.state = CueState::Standby;
        self.started_at = None;
        self.action_started_at = None;
        self.elapsed_before_pause = Duration::ZERO;
        self.action_elapsed_before_pause = Duration::ZERO;
        self.auto_continue_fired = false;
        context.emit(CueEvent::Stopped { cue_id: self.id });
        Ok(())
    }

    fn pause(&mut self, context: &CueContext) -> Result<()> {
        if self.in_pre_wait {
            return Ok(());
        }
        if let Some(vid) = self.active_voice_id {
            context.audio_engine.pause_voice(vid)?;
        }
        // Snapshot elapsed times so they freeze while paused.
        if let Some(t) = self.started_at.take() {
            self.elapsed_before_pause += t.elapsed();
        }
        if let Some(t) = self.action_started_at.take() {
            self.action_elapsed_before_pause += t.elapsed();
        }
        self.state = CueState::Paused;
        Ok(())
    }

    fn resume(&mut self, context: &CueContext) -> Result<()> {
        if let Some(vid) = self.active_voice_id {
            context.audio_engine.resume_voice(vid)?;
        }
        // Re-anchor Instants so elapsed() resumes from the frozen position.
        self.started_at = Some(Instant::now() - self.elapsed_before_pause);
        self.action_started_at = Some(Instant::now() - self.action_elapsed_before_pause);
        self.state = CueState::Running;
        Ok(())
    }

    fn seek(&mut self, position_ms: u64, ctx: &CueContext) {
        if self.action_started_at.is_none() {
            return; // Not yet playing (still in pre-wait or standby).
        }
        if let Some(vid) = self.active_voice_id {
            // frame_pos accounts for the cue's start-time offset so that
            // position_ms = 0 always corresponds to start_time in the file.
            let start_ms = self.start_time.unwrap_or(Duration::ZERO).as_millis() as u64;
            let target_ms = start_ms + position_ms;
            let frame_pos = target_ms * self.decoded_sample_rate as u64 / 1000;
            let _ = ctx.audio_engine.seek_voice(vid, frame_pos);
        }
        // Re-anchor elapsed time so the event loop reports the correct position.
        self.action_started_at =
            Some(Instant::now() - Duration::from_millis(position_ms));
    }

    fn hard_stop(&mut self, context: &CueContext) -> Result<()> {
        self.in_pre_wait = false;
        if let Some(vid) = self.active_voice_id.take() {
            context.audio_engine.stop_voice(vid, 0, EngineFadeCurve::Linear)?;
        }
        self.state = CueState::Standby;
        self.started_at = None;
        self.action_started_at = None;
        self.elapsed_before_pause = Duration::ZERO;
        self.action_elapsed_before_pause = Duration::ZERO;
        self.auto_continue_fired = false;
        context.emit(CueEvent::Stopped { cue_id: self.id });
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        // Note: does not stop playback — call stop() first if needed.
        self.state = CueState::Standby;
        self.active_voice_id = None;
        self.started_at = None;
        self.action_started_at = None;
        self.elapsed_before_pause = Duration::ZERO;
        self.action_elapsed_before_pause = Duration::ZERO;
        self.in_pre_wait = false;
        self.auto_continue_fired = false;
        Ok(())
    }

    fn tick(&mut self, context: &CueContext) -> Result<()> {
        if self.in_pre_wait && self.elapsed() >= self.pre_wait {
            self.start_audio_action(context)?;
        }
        Ok(())
    }

    fn is_action_started(&self) -> bool {
        !self.in_pre_wait
    }

    fn play_generation(&self) -> u64 {
        self.play_generation
    }

    fn is_auto_continue_fired(&self) -> bool {
        self.auto_continue_fired
    }

    fn mark_auto_continue_fired(&mut self) {
        self.auto_continue_fired = true;
    }

    fn clear_auto_continue_fired(&mut self) {
        self.auto_continue_fired = false;
    }

    fn pre_wait(&self) -> Duration { self.pre_wait }
    fn set_pre_wait(&mut self, d: Duration) { self.pre_wait = d; }
    fn post_wait(&self) -> Duration { self.post_wait }
    fn set_post_wait(&mut self, d: Duration) { self.post_wait = d; }

    fn duration(&self) -> Option<Duration> {
        // Infinite loop — no fixed duration; rely on voice_done for completion.
        if self.loop_count == u32::MAX {
            return None;
        }
        self.cached_duration.map(|d| {
            // Adjust for start/end markers.
            let start = self.start_time.unwrap_or(Duration::ZERO);
            let end = self.end_time.unwrap_or(d);
            let base = end.saturating_sub(start);
            // Adjust for playback rate: rate > 1.0 shortens effective duration.
            let adjusted = if self.rate > 0.0 && (self.rate - 1.0).abs() > f64::EPSILON {
                Duration::from_secs_f64(base.as_secs_f64() / self.rate)
            } else {
                base
            };
            // Multiply by total number of plays: initial play + loop_count repeats.
            adjusted * (self.loop_count + 1)
        })
    }

    fn elapsed(&self) -> Duration {
        if self.state == CueState::Paused {
            return self.elapsed_before_pause;
        }
        self.started_at.map(|t| t.elapsed()).unwrap_or(Duration::ZERO)
    }

    fn action_elapsed(&self) -> Duration {
        if self.state == CueState::Paused {
            return self.action_elapsed_before_pause;
        }
        self.action_started_at.map(|t| t.elapsed()).unwrap_or(Duration::ZERO)
    }

    fn continue_mode(&self) -> ContinueMode { self.continue_mode }
    fn set_continue_mode(&mut self, mode: ContinueMode) { self.continue_mode = mode; }

    fn playing_voice_id(&self) -> Option<CueId> {
        self.active_voice_id
    }

    fn file_duration(&self) -> Option<Duration> {
        self.cached_duration
    }

    fn extract_decoded_audio(
        &self,
    ) -> Option<(std::sync::Arc<Vec<f32>>, u16, u32, Duration)> {
        let samples = self.decoded_samples.as_ref()?;
        let duration = self.cached_duration?;
        Some((Arc::clone(samples), self.decoded_channels, self.decoded_sample_rate, duration))
    }

    fn waveform_peaks(&self, bins: usize) -> Option<Vec<f32>> {
        let samples = self.decoded_samples.as_ref()?;
        let channels = self.decoded_channels as usize;
        if bins == 0 || channels == 0 { return Some(vec![]); }
        let total_frames = samples.len() / channels;
        if total_frames == 0 { return Some(vec![]); }

        let mut peaks = Vec::with_capacity(bins);
        for i in 0..bins {
            let start_frame = (i * total_frames) / bins;
            let end_frame = (((i + 1) * total_frames) / bins).max(start_frame + 1);
            let mut peak = 0.0f32;
            for frame in start_frame..end_frame.min(total_frames) {
                for ch in 0..channels {
                    let v = samples[frame * channels + ch].abs();
                    if v > peak { peak = v; }
                }
            }
            peaks.push(peak);
        }
        Some(peaks)
    }

    fn runtime_state(&self) -> crate::cue::traits::RuntimeState {
        crate::cue::traits::RuntimeState {
            state: self.state,
            voice_id: self.active_voice_id,
            started_at: self.started_at,
            action_started_at: self.action_started_at,
        }
    }

    fn restore_runtime_state(&mut self, snap: crate::cue::traits::RuntimeState) {
        self.state = snap.state;
        self.active_voice_id = snap.voice_id;
        self.started_at = snap.started_at;
        self.action_started_at = snap.action_started_at;
        // Infer pre-wait: Running but action not yet started.
        self.in_pre_wait = snap.state == CueState::Running && snap.action_started_at.is_none();
    }

    fn serialize(&self) -> Value {
        json!({
            "type": "audio",
            "cue_type": "audio",
            "id": self.id,
            "number": self.number,
            "name": self.name,
            "notes": self.notes,
            "color": self.color,
            "pre_wait_ms": self.pre_wait.as_millis() as u64,
            "post_wait_ms": self.post_wait.as_millis() as u64,
            "continue_mode": self.continue_mode,
            "file_path": self.file_path.as_ref().map(|p| p.to_string_lossy().to_string()),
            "volume_db": self.volume_db,
            "pan": self.pan,
            "fade_in_ms": self.fade_in.as_ref().map(|f| f.duration_ms),
            "fade_in_curve": self.fade_in.as_ref().map(|f| f.curve),
            "fade_out_ms": self.fade_out.as_ref().map(|f| f.duration_ms),
            "fade_out_curve": self.fade_out.as_ref().map(|f| f.curve),
            "start_time_ms": self.start_time.map(|d| d.as_millis() as u64),
            "end_time_ms": self.end_time.map(|d| d.as_millis() as u64),
            "loop_count": self.loop_count,
            "output_patch_id": self.output_patch_id,
            "rate": self.rate,
        })
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Factory for [`AudioCue`].
pub struct AudioCueFactory;

impl CueFactory for AudioCueFactory {
    fn create(&self) -> Box<dyn Cue> {
        Box::new(AudioCue::new())
    }

    fn from_json(&self, value: Value) -> Result<Box<dyn Cue>> {
        let mut cue = AudioCue::new();

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
        if let Some(db) = value.get("volume_db").and_then(|v| v.as_f64()) {
            cue.volume_db = db;
        }
        if let Some(pan) = value.get("pan").and_then(|v| v.as_f64()) {
            cue.pan = pan as f32;
        }
        if let Some(ms) = value.get("fade_in_ms").and_then(|v| v.as_u64()) {
            let curve = value.get("fade_in_curve")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or(FadeCurve::SCurve);
            cue.fade_in = Some(FadeSpec { duration_ms: ms, curve });
        }
        if let Some(ms) = value.get("fade_out_ms").and_then(|v| v.as_u64()) {
            let curve = value.get("fade_out_curve")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or(FadeCurve::SCurve);
            cue.fade_out = Some(FadeSpec { duration_ms: ms, curve });
        }
        if let Some(ms) = value.get("start_time_ms").and_then(|v| v.as_u64()) {
            cue.start_time = Some(Duration::from_millis(ms));
        }
        if let Some(ms) = value.get("end_time_ms").and_then(|v| v.as_u64()) {
            cue.end_time = Some(Duration::from_millis(ms));
        }
        if let Some(lc) = value.get("loop_count").and_then(|v| v.as_u64()) {
            cue.loop_count = lc as u32;
        }
        if let Some(patch_str) = value.get("output_patch_id").and_then(|v| v.as_str()) {
            cue.output_patch_id = patch_str.parse().ok();
        }
        if let Some(rate) = value.get("rate").and_then(|v| v.as_f64()) {
            cue.rate = rate;
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

    fn make_cue() -> AudioCue {
        let mut c = AudioCue::new();
        c.set_name("Test Audio".to_string());
        c.set_number(Some("1".to_string()));
        c.volume_db = -6.0;
        c.pan = 0.5;
        c.fade_in = Some(FadeSpec::new(500));
        c.pre_wait = Duration::from_millis(1000);
        c.post_wait = Duration::from_millis(200);
        c.continue_mode = ContinueMode::AutoContinue;
        c
    }

    #[test]
    fn serialize_roundtrip() {
        let cue = make_cue();
        let json = cue.serialize();

        let factory = AudioCueFactory;
        let restored = factory.from_json(json).expect("should deserialize");

        assert_eq!(restored.name(), "Test Audio");
        assert_eq!(restored.number(), Some("1"));
        assert_eq!(restored.continue_mode(), ContinueMode::AutoContinue);
    }

    #[test]
    fn initial_state_is_standby() {
        let cue = AudioCue::new();
        assert_eq!(cue.state(), CueState::Standby);
        assert!(!cue.is_running());
        assert!(!cue.is_paused());
    }

    #[test]
    fn elapsed_zero_before_go() {
        let cue = AudioCue::new();
        assert_eq!(cue.elapsed(), Duration::ZERO);
        assert_eq!(cue.action_elapsed(), Duration::ZERO);
    }

    #[test]
    fn cue_number_is_string() {
        let mut cue = AudioCue::new();
        cue.set_number(Some("1.5.1".to_string()));
        assert_eq!(cue.number(), Some("1.5.1"));
        cue.set_number(Some("Intro".to_string()));
        assert_eq!(cue.number(), Some("Intro"));
        cue.set_number(None);
        assert_eq!(cue.number(), None);
    }
}
