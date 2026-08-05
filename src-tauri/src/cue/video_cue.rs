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

use crate::engine::output_engine::{ContentRequest, LayerStyle, SurfaceId, VideoGeometry, VoiceId};
use crate::engine::ring_command::FadeCurve as EngineFadeCurve;
use crate::engine::voice::{FadeDirection, FadeState, Voice};

use super::{
    context::{CueContext, CueEvent},
    traits::{Cue, CueFactory, RuntimeState},
    types::{
        db_to_linear, eof_fade_remaining_ms, ContinueMode, CueColor, CueId, CueState, CueType,
        FadeCurve, FadeSpec,
    },
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
    /// Audio fade-in applied to the decoded audio voice.
    pub fade_in: Option<FadeSpec>,
    /// Audio fade-out applied to the decoded audio voice on stop.
    pub fade_out: Option<FadeSpec>,
    /// Visual (GL overlay) fade-in — independent from audio.
    pub video_fade_in: Option<FadeSpec>,
    /// Visual (GL overlay) fade-out — independent from audio.
    pub video_fade_out: Option<FadeSpec>,
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
    /// Freeze on the last frame at natural EOF instead of cutting to black.
    pub hold_last_frame: bool,
    /// Visual geometry (fit / position / scale / rotation / crop).
    pub geometry: VideoGeometry,
    /// Compositing (stacking layer, base opacity, blend mode).
    pub layer_style: LayerStyle,
    /// QLab-style slices (markers + per-segment play counts).  Empty = plain
    /// playback.  When present, `loop_count` is ignored.
    pub slices: crate::cue::types::SliceList,

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
    /// `true` once the natural-end visual fade-out has been triggered for the
    /// current play, so `tick()` fires it exactly once.
    eof_fade_started: bool,
    /// Same, for the natural-end fade-out of the paired audio voice — the two
    /// fades have their own spec, so they arm independently.
    eof_audio_fade_started: bool,
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
            video_fade_in: None,
            video_fade_out: None,
            start_time: None,
            end_time: None,
            loop_count: 0,
            output_surface_id: None,
            output_patch_id: None,
            hold_last_frame: false,
            geometry: VideoGeometry::default(),
            layer_style: LayerStyle::default(),
            slices: crate::cue::types::SliceList::default(),
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
            eof_fade_started: false,
            eof_audio_fade_started: false,
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

    /// Resolve the slice segments within the clip window as
    /// `(start_ms, end_ms, play_count)`.  Empty = no slicing.
    fn slice_segments_ms(&self) -> Vec<(u64, u64, u32)> {
        if self.slices.is_empty() {
            return Vec::new();
        }
        let file_ms = self
            .cached_duration
            .map(|d| d.as_millis() as u64)
            .unwrap_or(u64::MAX);
        let clip_start = self.start_time.map(|d| d.as_millis() as u64).unwrap_or(0);
        let clip_end = self
            .end_time
            .map(|d| d.as_millis() as u64)
            .unwrap_or(file_ms)
            .min(file_ms);
        self.slices.segments(clip_start, clip_end)
    }

    /// Trigger the fade-outs that land on the cue's natural end, once the
    /// remaining action time drops inside each fade-out window.
    ///
    /// Without this, `video_fade_out` / `fade_out` only ever applied to
    /// *manual* stops — a video reaching EOF hard-cut to black
    /// (`mpv_events` forces the overlay opaque on END_FILE) and its sound cut
    /// off abruptly.  Picture and sound have their own spec and their own
    /// window, so they are armed independently.  Skipped for infinite loops
    /// (no natural end).
    fn tick_eof_fade(&mut self, context: &CueContext) {
        if self.in_pre_wait || self.loop_count == u32::MAX {
            return;
        }
        let (Some(voice_id), Some(total)) = (self.active_voice_id, self.duration()) else {
            return;
        };
        let action_elapsed = self.action_elapsed();

        // Picture — skipped for hold-last-frame (nothing to fade to).
        if !self.eof_fade_started && !self.hold_last_frame {
            let fade_ms = self.video_fade_out.as_ref().map(|f| f.duration_ms).unwrap_or(0);
            if let Some(remaining_ms) = eof_fade_remaining_ms(action_elapsed, total, fade_ms) {
                // Fire exactly once per play, whether or not the engine accepted
                // it (a `false` return means another cue took over the output).
                self.eof_fade_started = true;
                context.output_engine.begin_eof_fade_out(voice_id, remaining_ms);
            }
        }

        // Sound — the video's audio track is a normal AudioEngine voice, so it
        // fades exactly as an Audio Cue would.  Held last frames still fade:
        // the sound does reach its end even when the picture freezes.
        if !self.eof_audio_fade_started {
            let Some(fade) = self.fade_out.as_ref() else { return };
            let (fade_ms, curve) = (fade.duration_ms, Self::engine_curve(fade.curve));
            let Some(remaining_ms) = eof_fade_remaining_ms(action_elapsed, total, fade_ms) else {
                return;
            };
            self.eof_audio_fade_started = true;
            if let Some(audio_voice) = context.output_engine.video_audio_voice(voice_id) {
                let _ = context.audio_engine.stop_voice(audio_voice, remaining_ms, curve);
            }
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

        // Slice program: the paired audio follows the same segments as the
        // mpv side (which drives them via ab-loop), sample-resolved here.
        {
            let sr = self.decoded_sample_rate as u64;
            let segments: Vec<crate::engine::voice::SliceSegment> = self
                .slice_segments_ms()
                .into_iter()
                .map(|(s, e, count)| crate::engine::voice::SliceSegment {
                    start_frame: s * sr / 1000,
                    end_frame: e * sr / 1000,
                    play_count: count,
                })
                .collect();
            if let Some(program) = crate::engine::voice::SliceProgram::new(segments) {
                voice.frame_pos.store(
                    program.segments[0].start_frame,
                    std::sync::atomic::Ordering::Relaxed,
                );
                // SAFETY: written once before submission.
                unsafe { *voice.inner.slices.get() = Some(program) };
            }
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

        let mut patch_device: Option<String> = None;
        if let Some(patch) = context.resolve_patch(self.output_patch_id) {
            if let Some(&ch_l) = patch.channels.first() {
                voice.out_l = ch_l as usize;
            }
            if let Some(&ch_r) = patch.channels.get(1) {
                voice.out_r = ch_r as usize;
            } else if let Some(&ch_l) = patch.channels.first() {
                voice.out_r = ch_l as usize;
            }
            voice.patched = true;
            voice.patch_id = Some(patch.id);
            voice.patch_slot = context
                .output_patches
                .iter()
                .position(|p| p.id == patch.id)
                .map(|i| i as u8);
            voice.inner.set_patch_gain(crate::cue::types::db_to_linear(patch.gain_db as f64) as f32);
            patch_device = Some(patch.device_id.clone());
        }

        Ok(Some(context.audio_engine.play_voice_paused_routed(voice, patch_device.as_deref())?))
    }

    /// Kick off video playback.  Called directly from `go()` when there is no
    /// pre-wait, or from `tick()` once the pre-wait timer has elapsed.
    fn start_video_action(&mut self, context: &CueContext) -> Result<()> {
        let start_ms = self.start_time.map(|d| d.as_millis() as u64);
        let end_ms = self.end_time.map(|d| d.as_millis() as u64);
        let fade_in_ms = self.video_fade_in.as_ref().map(|f| f.duration_ms as u32).unwrap_or(0);

        // Submit the audio voice (paused) first so it is ready to resume the
        // instant the video's first frame is presented.
        let audio_voice_id = self.submit_paused_audio(context)?;

        let path = self.file_path.as_ref().ok_or_else(|| {
            anyhow!("VideoCue '{}': no file assigned — set a file in the inspector", self.name)
        })?;

        let slices: Vec<(f64, f64, u32)> = self
            .slice_segments_ms()
            .into_iter()
            .map(|(s, e, count)| (s as f64 / 1000.0, e as f64 / 1000.0, count))
            .collect();

        let voice_id = context.output_engine.show_content(ContentRequest {
            file_path: path,
            is_image: false,
            fade_in_ms,
            loop_count: self.loop_count,
            start_ms,
            end_ms,
            screen_index: context.output_screen,
            audio_voice_id,
            display_duration_ms: None,
            hold_last_frame: self.hold_last_frame,
            geometry: self.geometry,
            live_source: false,
            layer_style: self.layer_style,
            slices,
        })?;

        self.active_voice_id = Some(voice_id);
        self.action_started_at = Some(Instant::now());
        self.in_pre_wait = false;
        self.eof_fade_started = false;
        self.eof_audio_fade_started = false;

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
    fn output_patch_id(&self) -> Option<uuid::Uuid> { self.output_patch_id }
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

        let has_file = self.file_path.as_ref().is_some_and(|p| !p.as_os_str().is_empty());
        if !has_file {
            // No file assigned — nothing to play. Complete instantly (same
            // pattern as MemoCue) so Auto-Continue/Auto-Follow can advance
            // past it instead of getting stuck "running" an empty cue.
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

        self.start_video_action(context)
    }

    fn stop(&mut self, context: &CueContext) -> Result<()> {
        self.in_pre_wait = false;

        if let Some(vid) = self.active_voice_id.take() {
            let visual_fade_ms = self.video_fade_out.as_ref().map(|f| f.duration_ms as u32).unwrap_or(0);
            let audio_fade_ms  = self.fade_out.as_ref().map(|f| f.duration_ms as u32).unwrap_or(0);
            context.output_engine.stop_content(vid, visual_fade_ms, audio_fade_ms);
        }

        self.state = CueState::Standby;
        self.started_at = None;
        self.action_started_at = None;
        self.elapsed_before_pause = Duration::ZERO;
        self.action_elapsed_before_pause = Duration::ZERO;
        self.auto_continue_fired = false;
        self.eof_fade_started = false;
        self.eof_audio_fade_started = false;
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
        let Some(voice_id) = self.active_voice_id else { return };
        ctx.output_engine.seek_voice_ms(voice_id, position_ms);
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
        self.eof_fade_started = false;
        self.eof_audio_fade_started = false;
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
        self.eof_fade_started = false;
        self.eof_audio_fade_started = false;
        Ok(())
    }

    fn tick(&mut self, context: &CueContext) -> Result<()> {
        // Once the pre-wait timer expires, start the video action.
        if self.in_pre_wait && self.elapsed() >= self.pre_wait {
            self.start_video_action(context)?;
        }
        self.tick_eof_fade(context);
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
        if !self.slices.is_empty() {
            // Sliced playback: a vamp has no fixed duration; finite counts sum.
            let segments = self.slice_segments_ms();
            if segments.is_empty() {
                // Markers all fall outside the clip window — plain playback.
            } else if segments.iter().any(|&(_, _, c)| c == u32::MAX) {
                return None;
            } else {
                let total_ms: u64 = segments
                    .iter()
                    .map(|&(s, e, c)| (e - s) * c as u64)
                    .sum();
                return Some(Duration::from_millis(total_ms));
            }
        }
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

    fn live_audio_params(&self) -> Option<crate::cue::traits::LiveAudioParams> {
        let voice_id = self.active_voice_id?;
        Some(crate::cue::traits::LiveAudioParams {
            voice_id,
            gain: db_to_linear(self.volume_db) as f32,
            pan: 0.0,
        })
    }

    fn visual_geometry(&self) -> Option<VideoGeometry> {
        Some(self.geometry)
    }

    fn layer_style(&self) -> Option<LayerStyle> {
        Some(self.layer_style)
    }

    fn uses_sliced_playback(&self) -> bool {
        !self.slices.is_empty()
    }

    fn is_visual(&self) -> bool {
        true
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
            "video_fade_in_ms": self.video_fade_in.as_ref().map(|f| f.duration_ms),
            "video_fade_in_curve": self.video_fade_in.as_ref().map(|f| f.curve),
            "video_fade_out_ms": self.video_fade_out.as_ref().map(|f| f.duration_ms),
            "video_fade_out_curve": self.video_fade_out.as_ref().map(|f| f.curve),
            "start_time_ms": self.start_time.map(|d| d.as_millis() as u64),
            "end_time_ms": self.end_time.map(|d| d.as_millis() as u64),
            "loop_count": self.loop_count,
            "output_surface_id": self.output_surface_id,
            "output_patch_id": self.output_patch_id,
            "hold_last_frame": self.hold_last_frame,
            "geometry": self.geometry,
            "layer_style": self.layer_style,
            "slices": self.slices,
            "is_disabled": self.is_disabled,
            "cached_duration_ms": self.cached_duration.map(|d| d.as_millis() as u64),
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
        if let Some(b) = value.get("hold_last_frame").and_then(|v| v.as_bool()) {
            cue.hold_last_frame = b;
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
        if let Some(s) = value.get("slices") {
            if let Ok(mut slices) = serde_json::from_value::<crate::cue::types::SliceList>(s.clone()) {
                slices.normalize();
                cue.slices = slices;
            }
        }
        if let Some(b) = value.get("is_disabled").and_then(|v| v.as_bool()) {
            cue.is_disabled = b;
        }
        if let Some(ms) = value.get("cached_duration_ms").and_then(|v| v.as_u64()) {
            cue.cached_duration = Some(Duration::from_millis(ms));
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
    use crate::engine::output_engine::FitMode;

    #[test]
    fn serialize_roundtrip_geometry_and_hold() {
        let mut cue = VideoCue::new();
        cue.hold_last_frame = true;
        cue.geometry = VideoGeometry {
            fit_mode: FitMode::Fill,
            pan_x: 0.2,
            pan_y: -0.1,
            scale: 1.25,
            rotation: 180,
            crop_left: 0.05,
            crop_right: 0.0,
            crop_top: 0.1,
            crop_bottom: 0.0,
        };

        let json = cue.serialize();
        assert_eq!(json["hold_last_frame"], true);
        assert_eq!(json["geometry"]["fit_mode"], "fill");

        let rebuilt = VideoCueFactory.from_json(json).expect("roundtrip");
        assert_eq!(rebuilt.visual_geometry().unwrap(), cue.geometry);
        let rebuilt_json = rebuilt.serialize();
        assert_eq!(rebuilt_json["hold_last_frame"], true);
    }

    #[test]
    fn from_json_without_geometry_uses_defaults() {
        let json = serde_json::json!({ "type": "video", "name": "Legacy" });
        let cue = VideoCueFactory.from_json(json).expect("legacy load");
        assert!(cue.visual_geometry().unwrap().is_default());
        let json = cue.serialize();
        assert_eq!(json["hold_last_frame"], false);
    }

    #[test]
    fn layer_style_roundtrip() {
        use crate::engine::output_engine::BlendMode;
        let mut cue = VideoCue::new();
        assert!(cue.layer_style().unwrap().is_default());
        // Visual cues never auto-stop each other: launching a visual cue
        // stacks it as a new layer, only Stop/Fade cues remove one.
        assert!(!cue.stop_on_next_go());

        cue.layer_style = LayerStyle {
            layer: Some(3),
            opacity: 0.5,
            blend_mode: BlendMode::Multiply,
        };

        let json = cue.serialize();
        assert_eq!(json["layer_style"]["blend_mode"], "multiply");

        let rebuilt = VideoCueFactory.from_json(json).expect("roundtrip");
        assert_eq!(rebuilt.layer_style().unwrap(), cue.layer_style);
    }

    #[test]
    fn legacy_json_loads_with_default_layer_style() {
        // Pre-1.3 workspaces have no layer_style (and may carry the removed
        // stop_on_next_visual flag, which must be ignored).
        let json =
            serde_json::json!({ "type": "video", "name": "Old", "stop_on_next_visual": true });
        let cue = VideoCueFactory.from_json(json).expect("legacy load");
        assert!(!cue.stop_on_next_go());
        assert!(cue.layer_style().unwrap().is_default());
    }
}
