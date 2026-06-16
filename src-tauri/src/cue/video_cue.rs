//! [`VideoCue`] — plays a video file on the unified [`OutputEngine`] window.
//!
//! The cue delegates actual playback to the [`OutputEngine`], which manages
//! the persistent Win32 + libmpv output window.
//! The lifecycle (go / stop / pause / resume / pre-wait) mirrors [`AudioCue`]
//! exactly, so the Transport and event loop need no special-casing.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::engine::output_engine::{SurfaceId, VoiceId};
use crate::engine::ring_command::FadeCurve as EngineFadeCurve;
use crate::engine::voice::{FadeDirection, FadeState, Voice};

use super::{
    context::{CueContext, CueEvent},
    traits::{Cue, CueFactory, RuntimeState},
    types::{db_to_linear, ContinueMode, CueColor, CueId, CueState, CueType, FadeCurve, FadeSpec},
};

// ---------------------------------------------------------------------------
// VideoCue
// ---------------------------------------------------------------------------

/// A cue that plays a video file on the unified [`OutputEngine`] output window.
pub struct VideoCue {
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

    // --- Video-specific ---
    /// Path to the video file (relative to the workspace directory).
    pub file_path: Option<PathBuf>,
    /// Playback volume in dB (−60 to +12).
    pub volume_db: f64,
    /// Optional fade-in (controls HTML5 video element opacity via JS).
    pub fade_in: Option<FadeSpec>,
    /// Optional fade-out (applied on soft stop; default 500 ms).
    pub fade_out: Option<FadeSpec>,
    /// Start playback at this offset into the file.
    pub start_time: Option<Duration>,
    /// Stop playback at this offset into the file.
    pub end_time: Option<Duration>,
    /// Extra loop repetitions (0 = play once, `u32::MAX` = infinite).
    pub loop_count: u32,
    /// Output surface to render on.  `None` uses the default surface.
    pub output_surface_id: Option<SurfaceId>,
    /// Output Patch to route video audio through.  `None` uses the workspace
    /// default patch (or system default if none is configured).
    pub output_patch_id: Option<uuid::Uuid>,

    is_disabled: bool,

    // --- Runtime ---
    /// The video voice ID currently in use, if any.
    active_voice_id: Option<VoiceId>,
    /// The video's audio track, decoded to interleaved f32 by `load()` /
    /// background preload.  `None` when the file has no audio track.
    decoded_samples: Option<Arc<Vec<f32>>>,
    decoded_channels: u16,
    decoded_sample_rate: u32,
    /// Total media duration — set by [`Cue::set_runtime_duration`] when the
    /// surface reports its `loadedmetadata` event.
    cached_duration: Option<Duration>,
    /// `true` between `go()` and the moment the action starts after pre-wait.
    in_pre_wait: bool,
    /// Incremented on every `go()` call.
    play_generation: u64,
    /// Prevents double-firing of Auto-Continue.
    auto_continue_fired: bool,
    /// Elapsed time accumulated before the most recent pause.
    elapsed_before_pause: Duration,
    /// Action-elapsed time accumulated before the most recent pause.
    action_elapsed_before_pause: Duration,
}

impl VideoCue {
    /// Create a new, empty Video Cue with a fresh UUID.
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::from("Video Cue"),
            number: None,
            notes: String::new(),
            color: CueColor::Purple,
            state: CueState::Standby,
            pre_wait: Duration::ZERO,
            post_wait: Duration::ZERO,
            started_at: None,
            action_started_at: None,
            continue_mode: ContinueMode::DoNotContinue,
            file_path: None,
            volume_db: 0.0,
            fade_in: None,
            fade_out: None,
            start_time: None,
            end_time: None,
            loop_count: 0,
            output_surface_id: None,
            output_patch_id: None,
            is_disabled: false,
            active_voice_id: None,
            decoded_samples: None,
            decoded_channels: 2,
            decoded_sample_rate: 44100,
            cached_duration: None,
            in_pre_wait: false,
            play_generation: 0,
            auto_continue_fired: false,
            elapsed_before_pause: Duration::ZERO,
            action_elapsed_before_pause: Duration::ZERO,
        }
    }

    /// Convert a [`FadeCurve`] from the cue layer to the engine layer.
    fn engine_curve(c: FadeCurve) -> EngineFadeCurve {
        match c {
            FadeCurve::Linear => EngineFadeCurve::Linear,
            FadeCurve::SCurve => EngineFadeCurve::SCurve,
            FadeCurve::Exponential => EngineFadeCurve::Exponential,
        }
    }

    /// Build the audio voice for this video's audio track and submit it to the
    /// AudioEngine in the **paused** state, returning its id.
    ///
    /// The voice carries the cue's volume, fade-in, loop, start/end markers and
    /// Output Patch routing — exactly like an Audio Cue — so video audio gets
    /// the full professional signal path (routing, master volume, VU, fades).
    /// Returns `Ok(None)` when the video has no audio track.
    fn submit_paused_audio(&self, context: &CueContext) -> Result<Option<VoiceId>> {
        let samples = match &self.decoded_samples {
            Some(s) => Arc::clone(s),
            None => return Ok(None), // Silent video — no audio voice.
        };

        let gain = db_to_linear(self.volume_db) as f32;
        let mut voice = Voice::new(samples, self.decoded_channels, self.decoded_sample_rate, gain, 0.0);

        voice
            .inner
            .loops_remaining
            .store(self.loop_count, std::sync::atomic::Ordering::Relaxed);

        // Rate defaults to 1.0; SR mismatch is corrected in fill_buffer.

        if let Some(end) = self.end_time {
            let end_frame = (end.as_secs_f64() * self.decoded_sample_rate as f64) as u64;
            // SAFETY: written once before submission; the RT thread never sees
            // this voice until play_voice_paused pushes it.
            unsafe { *voice.inner.end_frame.get() = Some(end_frame); }
        }
        if let Some(start) = self.start_time {
            let start_frame = (start.as_secs_f64() * self.decoded_sample_rate as f64) as u64;
            voice.frame_pos.store(start_frame, std::sync::atomic::Ordering::Relaxed);
        }

        if let Some(ref fi) = self.fade_in {
            let total = (fi.duration_ms * self.decoded_sample_rate as u64) / 1000;
            // SAFETY: single writer before submission.
            unsafe {
                *voice.inner.fade.get() = Some(FadeState {
                    direction: FadeDirection::In,
                    total_samples: total,
                    elapsed_samples: 0,
                    curve: Self::engine_curve(fi.curve),
                });
            }
        }

        if let Some(patch) = context.resolve_patch(self.output_patch_id) {
            if let Some(&ch_l) = patch.channels.first() {
                voice.out_l = ch_l as usize;
            }
            if let Some(&ch_r) = patch.channels.get(1) {
                voice.out_r = ch_r as usize;
            } else if let Some(&ch_l) = patch.channels.first() {
                voice.out_r = ch_l as usize;
            }
        }

        Ok(Some(context.audio_engine.play_voice_paused(voice)?))
    }

    /// Kick off video playback.  Called directly from `go()` when there is no
    /// pre-wait, or from `tick()` once the pre-wait timer has elapsed.
    fn start_video_action(&mut self, context: &CueContext) -> Result<()> {
        let start_ms = self.start_time.map(|d| d.as_millis() as u64);
        let end_ms = self.end_time.map(|d| d.as_millis() as u64);
        let fade_in_ms: u32 = 0;
        let fade_out_ms: u32 = 0;

        // Submit the audio voice (paused) first so it is ready to resume the
        // instant the video's first frame is presented.
        let audio_voice_id = self.submit_paused_audio(context)?;

        let path = self.file_path.as_ref().ok_or_else(|| {
            anyhow!("VideoCue '{}': no file assigned — set a file in the inspector", self.name)
        })?;

        let voice_id = context.output_engine.show_content(
            path,
            false,
            fade_in_ms,
            fade_out_ms,
            self.loop_count,
            start_ms,
            end_ms,
            context.output_screen,
            audio_voice_id,
            None,
        )?;

        self.active_voice_id = Some(voice_id);
        self.action_started_at = Some(Instant::now());
        self.in_pre_wait = false;

        context.emit(CueEvent::ActionStarted { cue_id: self.id });
        Ok(())
    }
}

impl Default for VideoCue {
    fn default() -> Self {
        Self::new()
    }
}

impl Cue for VideoCue {
    // -----------------------------------------------------------------------
    // Identity
    // -----------------------------------------------------------------------

    fn id(&self) -> CueId { self.id }
    fn cue_type(&self) -> CueType { CueType::Video }
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
        // The video frames stream directly from disk via the OutputEngine, but
        // the audio track must be decoded so it can play as an AudioEngine voice
        // in sync with the (muted) video.
        let path = match &self.file_path {
            Some(p) => p.clone(),
            None => return Ok(()),
        };
        if let Some((samples, channels, sample_rate)) =
            crate::cue::media_decode::decode_audio_track(&path)?
        {
            self.decoded_channels = channels;
            self.decoded_sample_rate = sample_rate;
            self.decoded_samples = Some(Arc::new(samples));
        }
        Ok(())
    }

    fn accept_preloaded_audio(
        &mut self,
        samples: Arc<Vec<f32>>,
        channels: u16,
        sample_rate: u32,
        _duration: Duration,
    ) {
        // Store the decoded audio track.  The video's own duration comes from
        // the mpv probe (set_runtime_duration), so the decoded length is ignored.
        self.decoded_channels = channels;
        self.decoded_sample_rate = sample_rate;
        self.decoded_samples = Some(samples);
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

        self.start_video_action(context)
    }

    fn stop(&mut self, context: &CueContext) -> Result<()> {
        self.in_pre_wait = false;

        if let Some(vid) = self.active_voice_id.take() {
            let fade_ms = self
                .fade_out
                .as_ref()
                .map(|_| 0_u32)
                .unwrap_or(0);
            let _ = context.output_engine.stop_voice(vid, fade_ms);
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
            context.output_engine.pause_voice(vid)?;
        }
        if let Some(t) = self.started_at.take() {
            self.elapsed_before_pause = t.elapsed();
        }
        if let Some(t) = self.action_started_at.take() {
            self.action_elapsed_before_pause = t.elapsed();
        }
        self.state = CueState::Paused;
        Ok(())
    }

    fn resume(&mut self, context: &CueContext) -> Result<()> {
        if let Some(vid) = self.active_voice_id {
            context.output_engine.resume_voice(vid)?;
        }
        self.started_at = Some(Instant::now() - self.elapsed_before_pause);
        self.action_started_at = Some(Instant::now() - self.action_elapsed_before_pause);
        self.state = CueState::Running;
        Ok(())
    }

    fn seek(&mut self, position_ms: u64, ctx: &CueContext) {
        if self.action_started_at.is_none() && self.state != CueState::Paused {
            return;
        }
        ctx.output_engine.seek(position_ms);
        if self.state == CueState::Paused {
            self.action_elapsed_before_pause = Duration::from_millis(position_ms);
            self.elapsed_before_pause = self.pre_wait + Duration::from_millis(position_ms);
        } else {
            self.action_started_at =
                Some(Instant::now() - Duration::from_millis(position_ms));
        }
    }

    fn hard_stop(&mut self, context: &CueContext) -> Result<()> {
        self.in_pre_wait = false;

        if let Some(vid) = self.active_voice_id.take() {
            let _ = context.output_engine.stop_voice(vid, 0);
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
        // Once the pre-wait timer expires, start the video action.
        if self.in_pre_wait && self.elapsed() >= self.pre_wait {
            self.start_video_action(context)?;
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
        if self.loop_count == u32::MAX {
            return None; // Infinite loop — no fixed duration.
        }
        self.cached_duration.map(|d| {
            let start = self.start_time.unwrap_or(Duration::ZERO);
            let end = self.end_time.unwrap_or(d);
            let base = end.saturating_sub(start);
            base * (self.loop_count + 1)
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

    fn extract_decoded_audio(&self) -> Option<(Arc<Vec<f32>>, u16, u32, Duration)> {
        let samples = self.decoded_samples.as_ref()?;
        let duration = self.cached_duration?;
        Some((Arc::clone(samples), self.decoded_channels, self.decoded_sample_rate, duration))
    }

    fn play_generation(&self) -> u64 { self.play_generation }
    fn is_auto_continue_fired(&self) -> bool { self.auto_continue_fired }
    fn mark_auto_continue_fired(&mut self) { self.auto_continue_fired = true; }
    fn clear_auto_continue_fired(&mut self) { self.auto_continue_fired = false; }

    fn media_file_path(&self) -> Option<&std::path::Path> {
        self.file_path.as_deref()
    }

    fn set_runtime_duration(&mut self, duration: Duration) {
        self.cached_duration = Some(duration);
    }

    fn file_duration(&self) -> Option<Duration> {
        self.cached_duration
    }

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
            "type": "video",
            "cue_type": "video",
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
            "fade_in_ms": self.fade_in.as_ref().map(|f| f.duration_ms),
            "fade_in_curve": self.fade_in.as_ref().map(|f| f.curve),
            "fade_out_ms": self.fade_out.as_ref().map(|f| f.duration_ms),
            "fade_out_curve": self.fade_out.as_ref().map(|f| f.curve),
            "start_time_ms": self.start_time.map(|d| d.as_millis() as u64),
            "end_time_ms": self.end_time.map(|d| d.as_millis() as u64),
            "loop_count": self.loop_count,
            "output_surface_id": self.output_surface_id,
            "output_patch_id": self.output_patch_id,
            "is_disabled": self.is_disabled,
        })
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Factory for [`VideoCue`].
pub struct VideoCueFactory;

impl CueFactory for VideoCueFactory {
    fn create(&self) -> Box<dyn Cue> {
        Box::new(VideoCue::new())
    }

    fn from_json(&self, value: Value) -> Result<Box<dyn Cue>> {
        let mut cue = VideoCue::new();

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
        if let Some(ms) = value.get("start_time_ms").and_then(|v| v.as_u64()) {
            cue.start_time = Some(Duration::from_millis(ms));
        }
        if let Some(ms) = value.get("end_time_ms").and_then(|v| v.as_u64()) {
            cue.end_time = Some(Duration::from_millis(ms));
        }
        if let Some(lc) = value.get("loop_count").and_then(|v| v.as_u64()) {
            cue.loop_count = lc as u32;
        }
        if let Some(sid_str) = value.get("output_surface_id").and_then(|v| v.as_str()) {
            cue.output_surface_id = sid_str.parse().ok();
        }
        // "screen_index" was a per-cue field in older workspaces; it is now a
        // global preference (DisplayPreferences::output_screen) and is ignored here.
        if let Some(pid_str) = value.get("output_patch_id").and_then(|v| v.as_str()) {
            cue.output_patch_id = pid_str.parse().ok();
        }
        if let Some(b) = value.get("is_disabled").and_then(|v| v.as_bool()) {
            cue.is_disabled = b;
        }

        Ok(Box::new(cue))
    }
}
