//! Engine interface traits held by [`CueContext`](crate::cue::context::CueContext).
//!
//! The cue and transport layers drive playback through these traits rather than
//! the concrete engines. In production the real [`AudioEngine`], [`OutputEngine`]
//! and [`DmxEngine`] implement them (thin forwarding to the existing inherent
//! methods, so behaviour is unchanged). In tests, lightweight doubles implement
//! them, which is what lets `CueContext` — and therefore `Transport::go` — be
//! constructed and exercised without an audio device, a GL window, or libmpv.
//!
//! The trait surface is exactly the set of methods reached via
//! `context.{audio,output,dmx}_engine.*` from the cue and transport layers; the
//! event loop keeps its own concrete engine handles and is unaffected.

use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use uuid::Uuid;

use super::audio_engine::AudioEngine;
use super::dmx_engine::{ChannelWidth, DmxEngine};
use super::output_engine::OutputEngine;
use super::ring_command::{FadeCurve, VoiceId};
use super::voice::Voice;

// ---------------------------------------------------------------------------
// Audio engine
// ---------------------------------------------------------------------------

/// The audio-engine operations the cue and transport layers depend on.
pub trait AudioEngineApi: Send + Sync {
    fn play_voice_routed(&self, voice: Voice, device_id: Option<&str>) -> Result<VoiceId>;
    fn play_voice_paused_routed(&self, voice: Voice, device_id: Option<&str>) -> Result<VoiceId>;
    fn stop_voice(&self, voice_id: VoiceId, fade_ms: u32, fade_curve: FadeCurve) -> Result<()>;
    fn pause_voice(&self, voice_id: VoiceId) -> Result<()>;
    fn resume_voice(&self, voice_id: VoiceId) -> Result<()>;
    fn seek_voice(&self, voice_id: VoiceId, frame_pos: u64) -> Result<()>;
    fn set_voice_gain(&self, voice_id: VoiceId, gain: f32) -> Result<()>;
    fn get_voice_gain(&self, voice_id: VoiceId) -> f32;
    fn set_voice_pan(&self, voice_id: VoiceId, pan: f32) -> Result<()>;
    fn get_voice_pan(&self, voice_id: VoiceId) -> f32;
    fn sample_rate(&self) -> u32;
    fn ensure_input_feed(&self, device_id: Option<&str>, buffer_size: u32) -> Result<Uuid>;
    fn register_synthetic_feed(
        &self,
        channels: usize,
        sample_rate: u32,
    ) -> Result<(Uuid, ringbuf::HeapProd<f32>)>;
    #[allow(clippy::too_many_arguments)]
    fn play_mic_voice(
        &self,
        feed_id: Uuid,
        in_l: usize,
        in_r: usize,
        out_l: usize,
        out_r: usize,
        gain: f32,
        pan: f32,
        fade_in_ms: u32,
        fade_curve: FadeCurve,
    ) -> Result<VoiceId>;
    fn panic_stop_all(&self) -> Result<()>;
}

impl AudioEngineApi for AudioEngine {
    fn play_voice_routed(&self, voice: Voice, device_id: Option<&str>) -> Result<VoiceId> {
        AudioEngine::play_voice_routed(self, voice, device_id)
    }
    fn play_voice_paused_routed(&self, voice: Voice, device_id: Option<&str>) -> Result<VoiceId> {
        AudioEngine::play_voice_paused_routed(self, voice, device_id)
    }
    fn stop_voice(&self, voice_id: VoiceId, fade_ms: u32, fade_curve: FadeCurve) -> Result<()> {
        AudioEngine::stop_voice(self, voice_id, fade_ms, fade_curve)
    }
    fn pause_voice(&self, voice_id: VoiceId) -> Result<()> {
        AudioEngine::pause_voice(self, voice_id)
    }
    fn resume_voice(&self, voice_id: VoiceId) -> Result<()> {
        AudioEngine::resume_voice(self, voice_id)
    }
    fn seek_voice(&self, voice_id: VoiceId, frame_pos: u64) -> Result<()> {
        AudioEngine::seek_voice(self, voice_id, frame_pos)
    }
    fn set_voice_gain(&self, voice_id: VoiceId, gain: f32) -> Result<()> {
        AudioEngine::set_voice_gain(self, voice_id, gain)
    }
    fn get_voice_gain(&self, voice_id: VoiceId) -> f32 {
        AudioEngine::get_voice_gain(self, voice_id)
    }
    fn set_voice_pan(&self, voice_id: VoiceId, pan: f32) -> Result<()> {
        AudioEngine::set_voice_pan(self, voice_id, pan)
    }
    fn get_voice_pan(&self, voice_id: VoiceId) -> f32 {
        AudioEngine::get_voice_pan(self, voice_id)
    }
    fn sample_rate(&self) -> u32 {
        AudioEngine::sample_rate(self)
    }
    fn ensure_input_feed(&self, device_id: Option<&str>, buffer_size: u32) -> Result<Uuid> {
        AudioEngine::ensure_input_feed(self, device_id, buffer_size)
    }
    fn register_synthetic_feed(
        &self,
        channels: usize,
        sample_rate: u32,
    ) -> Result<(Uuid, ringbuf::HeapProd<f32>)> {
        AudioEngine::register_synthetic_feed(self, channels, sample_rate)
    }
    fn play_mic_voice(
        &self,
        feed_id: Uuid,
        in_l: usize,
        in_r: usize,
        out_l: usize,
        out_r: usize,
        gain: f32,
        pan: f32,
        fade_in_ms: u32,
        fade_curve: FadeCurve,
    ) -> Result<VoiceId> {
        AudioEngine::play_mic_voice(
            self, feed_id, in_l, in_r, out_l, out_r, gain, pan, fade_in_ms, fade_curve,
        )
    }
    fn panic_stop_all(&self) -> Result<()> {
        AudioEngine::panic_stop_all(self)
    }
}

// ---------------------------------------------------------------------------
// Output engine
// ---------------------------------------------------------------------------

/// The output-engine operations the cue and transport layers depend on.
pub trait OutputEngineApi: Send + Sync {
    #[allow(clippy::too_many_arguments)]
    fn show_content(
        &self,
        file_path: &Path,
        is_image: bool,
        fade_in_ms: u32,
        this_fade_out_ms: u32,
        loop_count: u32,
        start_ms: Option<u64>,
        end_ms: Option<u64>,
        screen_index: Option<u32>,
        audio_voice_id: Option<VoiceId>,
        display_duration_ms: Option<u64>,
    ) -> Result<VoiceId>;
    fn stop_content(&self, voice_id: VoiceId, visual_fade_ms: u32, audio_fade_ms: u32);
    fn hard_stop_current(&self);
    fn panic_stop(&self);
    fn get_overlay_alpha(&self) -> u8;
    fn set_overlay_alpha_direct(&self, alpha: u8);
    fn get_current_audio_voice(&self) -> Option<VoiceId>;
    fn resync_audio_to_video(&self);
    fn stop_voice(&self, voice_id: VoiceId, fade_ms: u32) -> Result<()>;
    fn pause_voice(&self, voice_id: VoiceId) -> Result<()>;
    fn resume_voice(&self, voice_id: VoiceId) -> Result<()>;
    fn seek(&self, position_ms: u64);
    fn show_text_overlay(&self, ass_text: &str, screen_index: Option<u32>);
    fn clear_text_overlay(&self);
}

impl OutputEngineApi for OutputEngine {
    fn show_content(
        &self,
        file_path: &Path,
        is_image: bool,
        fade_in_ms: u32,
        this_fade_out_ms: u32,
        loop_count: u32,
        start_ms: Option<u64>,
        end_ms: Option<u64>,
        screen_index: Option<u32>,
        audio_voice_id: Option<VoiceId>,
        display_duration_ms: Option<u64>,
    ) -> Result<VoiceId> {
        OutputEngine::show_content(
            self,
            file_path,
            is_image,
            fade_in_ms,
            this_fade_out_ms,
            loop_count,
            start_ms,
            end_ms,
            screen_index,
            audio_voice_id,
            display_duration_ms,
        )
    }
    fn stop_content(&self, voice_id: VoiceId, visual_fade_ms: u32, audio_fade_ms: u32) {
        OutputEngine::stop_content(self, voice_id, visual_fade_ms, audio_fade_ms)
    }
    fn hard_stop_current(&self) {
        OutputEngine::hard_stop_current(self)
    }
    fn panic_stop(&self) {
        OutputEngine::panic_stop(self)
    }
    fn get_overlay_alpha(&self) -> u8 {
        OutputEngine::get_overlay_alpha(self)
    }
    fn set_overlay_alpha_direct(&self, alpha: u8) {
        OutputEngine::set_overlay_alpha_direct(self, alpha)
    }
    fn get_current_audio_voice(&self) -> Option<VoiceId> {
        OutputEngine::get_current_audio_voice(self)
    }
    fn resync_audio_to_video(&self) {
        OutputEngine::resync_audio_to_video(self)
    }
    fn stop_voice(&self, voice_id: VoiceId, fade_ms: u32) -> Result<()> {
        OutputEngine::stop_voice(self, voice_id, fade_ms)
    }
    fn pause_voice(&self, voice_id: VoiceId) -> Result<()> {
        OutputEngine::pause_voice(self, voice_id)
    }
    fn resume_voice(&self, voice_id: VoiceId) -> Result<()> {
        OutputEngine::resume_voice(self, voice_id)
    }
    fn seek(&self, position_ms: u64) {
        OutputEngine::seek(self, position_ms)
    }
    fn show_text_overlay(&self, ass_text: &str, screen_index: Option<u32>) {
        OutputEngine::show_text_overlay(self, ass_text, screen_index)
    }
    fn clear_text_overlay(&self) {
        OutputEngine::clear_text_overlay(self)
    }
}

// ---------------------------------------------------------------------------
// DMX engine
// ---------------------------------------------------------------------------

/// The DMX-engine operations the cue layer depends on (Light Cue fades).
pub trait DmxEngineApi: Send + Sync {
    fn submit_fade(
        &self,
        universe: u16,
        channel: u16,
        width: ChannelWidth,
        target_norm: f64,
        dur: Duration,
        curve: FadeCurve,
    );
}

impl DmxEngineApi for DmxEngine {
    fn submit_fade(
        &self,
        universe: u16,
        channel: u16,
        width: ChannelWidth,
        target_norm: f64,
        dur: Duration,
        curve: FadeCurve,
    ) {
        DmxEngine::submit_fade(self, universe, channel, width, target_norm, dur, curve)
    }
}
