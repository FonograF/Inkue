//! [`AudioEngine`] — the top-level audio subsystem.
//!
//! **Real-time safety:** the audio callback (`fill_buffer`) must never
//! allocate, block, or do I/O.  All state mutations happen via the command ring
//! buffer; all outgoing data goes through the status ring buffer.

use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Stream, StreamConfig};
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::HeapRb;
use uuid::Uuid;

use crate::preferences::MachineAudioConfig;

use super::{
    audio_input::{open_input, InputCapture},
    device_manager::DeviceManager,
    ring_command::{AudioCommand, AudioStatus, FadeCurve, VoiceId},
    voice::{FadeDirection, FadeState, LiveSource, Voice, VoiceState},
};

const _MAX_VOICES: usize = 64;
const RING_CAPACITY: usize = 256;
pub const DEFAULT_FADE_OUT_MS: u32 = 500;

/// Circular staging buffer per input feed, in frames.  Large enough to absorb
/// two full input-callback periods at maximum buffer size (2 × 2048 @ 48 kHz ≈
/// 85 ms) while staying small so the target-lag calculation stays well inside.
const STAGING_FRAMES: usize = 8192;

// ---------------------------------------------------------------------------
// Live input feed — one per captured device, drained by the output callback
// ---------------------------------------------------------------------------

/// One live input device's capture: its ring consumer plus a circular staging
/// buffer the output callback keeps current and live voices resample from.
///
/// Drained every output block (`drain`) so the input stays "warm" and bounded
/// even when no Mic Cue is playing — a GO is then instant with no cold-start.
struct InputFeed {
    /// Stable id referenced by a [`LiveSource`].
    id: Uuid,
    /// OS device id this feed captures from (one feed per device).
    device_id: String,
    /// Interleaved channel count of the staging frames.
    in_channels: usize,
    /// Input device sample rate (Hz).
    sample_rate: u32,
    /// Ring consumer fed by the cpal input callback.
    cons: ringbuf::HeapCons<f32>,
    /// Circular interleaved staging, `STAGING_FRAMES * in_channels` long.
    staging: Box<[f32]>,
    /// Monotonic count of frames written into `staging`.
    write_frame: u64,
    /// Keeps the cpal input stream alive (`None` only in unit tests).
    _capture: Option<InputCapture>,
}

impl InputFeed {
    /// Pop every available frame from the ring into the circular staging buffer.
    /// RT-safe: bounded by what the input callback produced, no allocation.
    fn drain(&mut self) {
        let ch = self.in_channels;
        while self.cons.occupied_len() >= ch {
            let slot = (self.write_frame as usize % STAGING_FRAMES) * ch;
            for c in 0..ch {
                if let Some(s) = self.cons.try_pop() {
                    self.staging[slot + c] = s;
                }
            }
            self.write_frame += 1;
        }
    }

    /// Linear-interpolated sample for input channel `ch` at fractional frame `pos`.
    fn sample(&self, ch: usize, pos: f64) -> f32 {
        let i0 = pos.floor() as u64;
        let frac = (pos - i0 as f64) as f32;
        let a = self.staging[(i0 as usize % STAGING_FRAMES) * self.in_channels + ch];
        let b = self.staging[((i0 + 1) as usize % STAGING_FRAMES) * self.in_channels + ch];
        a + (b - a) * frac
    }
}


/// The audio engine.
pub struct AudioEngine {
    pub device_manager: Mutex<DeviceManager>,
    cmd_prod: Mutex<ringbuf::HeapProd<AudioCommand>>,
    status_cons: Mutex<ringbuf::HeapCons<AudioStatus>>,
    voices: Arc<Mutex<Vec<Arc<Voice>>>>,
    /// Live input feeds (one per captured device), shared with the output
    /// callback which drains them each block.
    input_feeds: Arc<Mutex<Vec<InputFeed>>>,
    _stream: Mutex<Option<Stream>>,
    sample_rate: std::sync::atomic::AtomicU32,
    /// Actual output callback period in frames (updated on the first callback).
    /// Used by `mix_live` to set a tight `target_lag` instead of a fixed 25 ms.
    output_period: Arc<std::sync::atomic::AtomicU32>,
    /// Total output channel count of the current stream (updated on restart).
    output_channels: std::sync::atomic::AtomicU32,
    master_gain: Arc<std::sync::atomic::AtomicU32>,
}

// SAFETY: cpal::Stream is not Send on Windows when using WASAPI.
unsafe impl Send for AudioEngine {}
unsafe impl Sync for AudioEngine {}

impl AudioEngine {
    /// Open an output device according to `config` and start the audio callback.
    pub fn new(config: &MachineAudioConfig) -> Result<Arc<Self>> {
        let master_gain = Arc::new(std::sync::atomic::AtomicU32::new(f32::to_bits(1.0_f32)));
        let shared_voices: Arc<Mutex<Vec<Arc<Voice>>>> = Arc::new(Mutex::new(Vec::new()));
        let input_feeds: Arc<Mutex<Vec<InputFeed>>> = Arc::new(Mutex::new(Vec::new()));
        let output_period = Arc::new(std::sync::atomic::AtomicU32::new(256));

        let stream_failed = Arc::new(std::sync::atomic::AtomicBool::new(false));

        let sr = open_stream_inner(
            config,
            Arc::clone(&shared_voices),
            Arc::clone(&input_feeds),
            Arc::clone(&master_gain),
            Arc::clone(&output_period),
            Arc::clone(&stream_failed),
        )?;

        let engine = Arc::new(Self {
            device_manager: Mutex::new(DeviceManager::new()),
            cmd_prod: Mutex::new(sr.cmd_prod),
            status_cons: Mutex::new(sr.status_cons),
            voices: shared_voices,
            input_feeds,
            _stream: Mutex::new(Some(sr.stream)),
            sample_rate: std::sync::atomic::AtomicU32::new(sr.sample_rate),
            output_channels: std::sync::atomic::AtomicU32::new(sr.channels),
            master_gain,
            output_period,
        });

        // If the configured device starts erroring immediately (e.g. HDMI with no
        // display, or an unplugged device), fall back to the system default after
        // 500 ms rather than flooding logs forever.
        if config.device_id.is_some() {
            let bad_device = config.device_id.clone().unwrap_or_default();
            let fallback = MachineAudioConfig { device_id: None, ..config.clone() };
            let engine2 = Arc::clone(&engine);
            std::thread::Builder::new()
                .name("wincue-audio-watchdog".into())
                .spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    if stream_failed.load(std::sync::atomic::Ordering::Relaxed) {
                        log::warn!(
                            "Audio device '{bad_device}' is broken (repeated errors), \
                             falling back to system default"
                        );
                        if let Err(e) = engine2.restart(&fallback) {
                            log::error!("Audio fallback restart failed: {e}");
                        }
                    }
                })
                .ok();
        }

        Ok(engine)
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Total output channel count of the currently open stream.
    pub fn output_channels(&self) -> u32 {
        self.output_channels.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Set the master output gain (real-time safe via atomic).
    pub fn set_master_gain(&self, gain: f32) {
        self.master_gain.store(f32::to_bits(gain), std::sync::atomic::Ordering::Relaxed);
    }

    /// Add a pre-decoded voice to the pool and issue a Play command.
    pub fn play_voice(&self, voice: Voice) -> Result<VoiceId> {
        let id = voice.id;
        let arc = Arc::new(voice);
        arc.set_playing();

        self.voices
            .lock()
            .map_err(|_| anyhow!("voices mutex poisoned"))?
            .push(Arc::clone(&arc));

        self.send_command(AudioCommand::Play { voice_id: id })?;
        Ok(id)
    }

    /// Add a pre-decoded voice to the pool in the **paused** state, returning
    /// its id without starting playback.
    ///
    /// Used to pair a video's audio track with its muted mpv video: the voice
    /// is submitted paused at GO, then resumed (via [`resume_voice`]) the moment
    /// the video's first frame is presented, so audio and video start together
    /// with no A/V offset.
    pub fn play_voice_paused(&self, voice: Voice) -> Result<VoiceId> {
        let id = voice.id;
        let arc = Arc::new(voice);
        arc.set_paused();

        self.voices
            .lock()
            .map_err(|_| anyhow!("voices mutex poisoned"))?
            .push(Arc::clone(&arc));

        // No Play command — the callback only mixes Playing/FadingOut voices, so
        // this stays silent until resume_voice() is called.
        Ok(id)
    }

    /// Ensure an input capture exists for `device_id` (or the default input when
    /// `None`/empty), returning the feed id to bind a Mic Cue to.
    ///
    /// Idempotent: a device is captured once and shared by all Mic Cues using it.
    /// The feed is released by [`gc_voices`](Self::gc_voices) once no live voice
    /// references it any more (so the OS mic indicator turns off after stop).
    /// Ensure a capture feed exists for `device_id`.
    ///
    /// `buffer_size` is passed to the cpal input stream (0 = OS default).
    /// Pass the same value as the output stream's configured buffer so both
    /// device clocks fire at the same period, minimising input ↔ output drift.
    pub fn ensure_input_feed(&self, device_id: Option<&str>, buffer_size: u32) -> Result<Uuid> {
        let key = device_id.unwrap_or_default().to_string();
        {
            let feeds = self.input_feeds.lock().map_err(|_| anyhow!("input_feeds poisoned"))?;
            if let Some(f) = feeds.iter().find(|f| f.device_id == key) {
                return Ok(f.id);
            }
        }
        log::info!("ensure_input_feed: opening device={device_id:?} buf={buffer_size}");
        let (capture, cons) = open_input(device_id, buffer_size).map_err(|e| {
            log::error!("ensure_input_feed failed for {device_id:?}: {e}");
            e
        })?;
        let in_channels = capture.channels.max(1) as usize;
        let id = Uuid::new_v4();
        let feed = InputFeed {
            id,
            device_id: key,
            in_channels,
            sample_rate: capture.sample_rate,
            cons,
            staging: vec![0.0_f32; STAGING_FRAMES * in_channels].into_boxed_slice(),
            write_frame: 0,
            _capture: Some(capture),
        };
        self.input_feeds
            .lock()
            .map_err(|_| anyhow!("input_feeds poisoned"))?
            .push(feed);
        Ok(id)
    }

    /// Channel count and sample rate of an existing input feed.
    fn feed_info(&self, feed_id: Uuid) -> Option<(usize, u32)> {
        self.input_feeds
            .lock()
            .ok()
            .and_then(|f| f.iter().find(|f| f.id == feed_id).map(|f| (f.in_channels, f.sample_rate)))
    }

    /// Start a live (Mic Cue) voice reading input channels `in_l`/`in_r` (equal
    /// for mono) from `feed_id`, routed to output channels `out_l`/`out_r`, with
    /// optional fade-in.  Returns the voice id (use [`stop_voice`] to stop it).
    #[allow(clippy::too_many_arguments)]
    pub fn play_mic_voice(
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
        let (in_ch, src_rate) = self
            .feed_info(feed_id)
            .ok_or_else(|| anyhow!("input feed {feed_id:?} not found"))?;
        // Clamp requested channels to what the device offers.
        let in_l = in_l.min(in_ch.saturating_sub(1));
        let in_r = in_r.min(in_ch.saturating_sub(1));

        let live = LiveSource::new(feed_id, in_l, in_r, src_rate);
        let mut voice = Voice::new_live(live, self.sample_rate(), gain, pan);
        voice.out_l = out_l;
        voice.out_r = out_r;
        if fade_in_ms > 0 {
            let total = fade_in_ms as u64 * self.sample_rate() as u64 / 1000;
            // SAFETY: written once before the voice is shared with the callback.
            unsafe {
                *voice.inner.fade.get() = Some(FadeState {
                    direction: FadeDirection::In,
                    total_samples: total,
                    elapsed_samples: 0,
                    curve: fade_curve,
                });
            }
        }

        let id = voice.id;
        let arc = Arc::new(voice);
        arc.set_playing();
        self.voices
            .lock()
            .map_err(|_| anyhow!("voices mutex poisoned"))?
            .push(Arc::clone(&arc));
        self.send_command(AudioCommand::Play { voice_id: id })?;
        Ok(id)
    }

    pub fn stop_voice(&self, voice_id: VoiceId, fade_ms: u32, fade_curve: FadeCurve) -> Result<()> {
        self.send_command(AudioCommand::Stop { voice_id, fade_ms, fade_curve })
    }

    pub fn pause_voice(&self, voice_id: VoiceId) -> Result<()> {
        self.send_command(AudioCommand::Pause { voice_id })
    }

    pub fn resume_voice(&self, voice_id: VoiceId) -> Result<()> {
        self.send_command(AudioCommand::Resume { voice_id })
    }

    pub fn set_voice_gain(&self, voice_id: VoiceId, gain: f32) -> Result<()> {
        self.send_command(AudioCommand::SetGain { voice_id, gain })
    }

    pub fn set_voice_pan(&self, voice_id: VoiceId, pan: f32) -> Result<()> {
        self.send_command(AudioCommand::SetPan { voice_id, pan })
    }

    /// Seek a voice to the given decoded-audio frame position.
    pub fn seek_voice(&self, voice_id: VoiceId, frame_pos: u64) -> Result<()> {
        self.send_command(AudioCommand::Seek { voice_id, frame_pos })
    }

    /// Read the current linear gain of a voice.
    /// Returns 1.0 if the voice is not found.
    pub fn get_voice_gain(&self, voice_id: VoiceId) -> f32 {
        self.voices
            .lock()
            .ok()
            .and_then(|g| g.iter().find(|v| v.id == voice_id).map(|v| v.inner.gain()))
            .unwrap_or(1.0)
    }

    /// Seek a voice to the given position in milliseconds.
    ///
    /// Looks up the voice's decoded sample rate to convert `position_ms` into a
    /// frame position, then sends a [`AudioCommand::Seek`] to the RT callback.
    /// Used by [`OutputEngine::seek`] which only knows wall-clock position.
    pub fn seek_voice_ms(&self, voice_id: VoiceId, position_ms: u64) -> Result<()> {
        let frame_pos = {
            let voices = self.voices.lock().map_err(|_| anyhow!("voices mutex poisoned"))?;
            voices
                .iter()
                .find(|v| v.id == voice_id)
                .map(|v| position_ms * v.sample_rate as u64 / 1000)
        };
        if let Some(fp) = frame_pos {
            self.send_command(AudioCommand::Seek { voice_id, frame_pos: fp })?;
        }
        Ok(())
    }

    /// Drain the status ring buffer and return all pending status messages.
    pub fn drain_status(&self) -> Vec<AudioStatus> {
        let mut cons = match self.status_cons.lock() {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        let mut out = Vec::new();
        while let Some(s) = cons.try_pop() {
            out.push(s);
        }
        out
    }

    /// Remove fully-stopped voices from the pool, and close any input capture
    /// device that no live voice still references (so the OS releases the mic
    /// once its Mic Cue stops — the device indicator turns off).
    ///
    /// Locks `voices` then `input_feeds`, matching the RT callback's order so the
    /// two never deadlock.
    pub fn gc_voices(&self) {
        let Ok(mut voices) = self.voices.lock() else { return };
        voices.retain(|v| !matches!(v.voice_state(), VoiceState::Stopped | VoiceState::Idle));

        // Feed ids still referenced by a surviving live voice.
        let in_use: std::collections::HashSet<Uuid> = voices
            .iter()
            .filter_map(|v| v.live.as_ref().map(|l| l.feed_id))
            .collect();

        // Drop unreferenced feeds — dropping an InputFeed drops its cpal input
        // stream, releasing the device.
        if let Ok(mut feeds) = self.input_feeds.lock() {
            feeds.retain(|f| in_use.contains(&f.id));
        }
    }

    /// Stop the current stream and re-open according to `config`.
    /// All active voices are killed; cues should be reset by the caller.
    pub fn restart(&self, config: &MachineAudioConfig) -> Result<()> {
        // Kill all active voices.
        if let Ok(mut voices) = self.voices.lock() {
            for v in voices.iter() { v.set_stopped(); }
            voices.clear();
        }

        // Drop the old stream before opening the new one (exclusive backends
        // require the device to be released first).
        {
            let mut sg = self._stream.lock().map_err(|_| anyhow!("stream mutex poisoned"))?;
            *sg = None;
        }

        let sr = open_stream_inner(
            config,
            Arc::clone(&self.voices),
            Arc::clone(&self.input_feeds),
            Arc::clone(&self.master_gain),
            Arc::clone(&self.output_period),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )?;

        *self.cmd_prod.lock().map_err(|_| anyhow!("cmd_prod poisoned"))? = sr.cmd_prod;
        *self.status_cons.lock().map_err(|_| anyhow!("status_cons poisoned"))? = sr.status_cons;
        *self._stream.lock().map_err(|_| anyhow!("stream poisoned"))? = Some(sr.stream);
        self.sample_rate.store(sr.sample_rate, std::sync::atomic::Ordering::Relaxed);
        self.output_channels.store(sr.channels, std::sync::atomic::Ordering::Relaxed);

        if let Ok(mut mgr) = self.device_manager.lock() { let _ = mgr.refresh_devices(); }

        Ok(())
    }

    fn send_command(&self, cmd: AudioCommand) -> Result<()> {
        self.cmd_prod
            .lock()
            .map_err(|_| anyhow!("cmd_prod mutex poisoned"))?
            .try_push(cmd)
            .map_err(|_| anyhow!("Audio command ring buffer full"))
    }
}

// ---------------------------------------------------------------------------
// Stream builder — shared between new() and restart()
// ---------------------------------------------------------------------------

struct StreamResult {
    stream: Stream,
    sample_rate: u32,
    channels: u32,
    cmd_prod: ringbuf::HeapProd<AudioCommand>,
    status_cons: ringbuf::HeapCons<AudioStatus>,
}

/// Select device, configure buffer, build and start the cpal stream.
///
/// Creates fresh ring buffers and returns them alongside the running stream so
/// the caller can wire them into the engine (either for initial construction or
/// after a restart).
fn open_stream_inner(
    config: &MachineAudioConfig,
    cb_voices: Arc<Mutex<Vec<Arc<Voice>>>>,
    cb_feeds: Arc<Mutex<Vec<InputFeed>>>,
    cb_mg: Arc<std::sync::atomic::AtomicU32>,
    cb_period: Arc<std::sync::atomic::AtomicU32>,
    stream_failed: Arc<std::sync::atomic::AtomicBool>,
) -> Result<StreamResult> {
    use crate::preferences::AudioBackend;

    let host = match config.backend {
        AudioBackend::WasapiShared | AudioBackend::WasapiExclusive | AudioBackend::SystemDefault => {
            cpal::default_host()
        }
        AudioBackend::Asio => open_asio_host()?,
    };

    let device_name = config.device_id.as_deref();

    // On Linux, `pw:<node_name>` IDs route through the `pipewire` ALSA device
    // with PIPEWIRE_NODE set.  The guard keeps the env var live until the
    // device is fully opened below.
    #[cfg(target_os = "linux")]
    let (_pw_guard, effective_name): (Option<crate::engine::device_manager::PwNodeGuard>, Option<&str>) = {
        use crate::engine::device_manager::pipewire_node_of;
        match device_name.filter(|s| !s.is_empty()).and_then(pipewire_node_of) {
            Some(node) => {
                let guard = crate::engine::device_manager::acquire_pw_node(node);
                (Some(guard), Some("pipewire"))
            }
            None => (None, device_name.filter(|s| !s.is_empty())),
        }
    };
    #[cfg(not(target_os = "linux"))]
    let effective_name = device_name.filter(|s| !s.is_empty());

    let device = if matches!(config.backend, AudioBackend::Asio) {
        let found = effective_name
            .and_then(|name| {
                host.output_devices().ok()
                    .and_then(|mut it| it.find(|d| d.id().ok().map(|id| id.id() == name).unwrap_or(false)))
            });
        found
            .or_else(|| host.default_output_device())
            .ok_or_else(|| anyhow!(
                "No ASIO device found. Make sure the driver is not already in use by another application."
            ))?
    } else if let Some(name) = effective_name {
        host.output_devices()
            .map_err(|e| anyhow!("Failed to enumerate devices: {e}"))?
            .find(|d| d.id().ok().map(|id| id.id() == name).unwrap_or(false))
            .ok_or_else(|| anyhow!("Audio device '{}' not found", name))?
    } else {
        host.default_output_device()
            .ok_or_else(|| anyhow!("No default audio output device found"))?
    };

    let default_config = device.default_output_config()
        .map_err(|e| anyhow!("Device config error: {e}"))?;
    let sample_rate = default_config.sample_rate();
    let channels = default_config.channels();
    let total_ch = channels as usize;

    // Buffer size: apply Fixed on all backends except ASIO (which uses its own
    // control panel) and WASAPI Shared (where Windows owns the engine period and
    // ignores the hint).  This makes the user-configured buffer_size effective on
    // macOS (CoreAudio) and Linux (ALSA/PipeWire) — previously they always got
    // the OS default (typically 256-1024 samples), causing high Mic Cue latency
    // even when the operator had set 64 samples in Preferences.
    let buf_size = match config.backend {
        AudioBackend::Asio | AudioBackend::WasapiShared => cpal::BufferSize::Default,
        _ => cpal::BufferSize::Fixed(config.buffer_size),
    };

    let stream_cfg = StreamConfig {
        channels,
        sample_rate,
        buffer_size: buf_size,
    };

    // ASIO: route the stereo mix to the selected output pair.
    let pair_offset = (config.asio_out_pair as usize * 2).min(total_ch.saturating_sub(2));

    let (cmd_prod, mut cmd_cons) = HeapRb::<AudioCommand>::new(RING_CAPACITY).split();
    let (mut status_prod, status_cons) = HeapRb::<AudioStatus>::new(RING_CAPACITY).split();

    // Throttled error callback: log at most once per second, signal stream_failed
    // after 50 rapid errors so the watchdog can switch to a working device.
    let make_err_fn = {
        let last_log  = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let err_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        move || {
            let last_log2  = Arc::clone(&last_log);
            let err_count2 = Arc::clone(&err_count);
            let failed2    = Arc::clone(&stream_failed);
            move |err: cpal::Error| {
                // Only DeviceNotAvailable is truly fatal (device pulled or
                // exclusively grabbed).  Other kinds cover recoverable ALSA
                // errors (e.g. Xrun, POLLERR) that should not trigger the
                // watchdog restart.
                if matches!(err.kind(), cpal::ErrorKind::DeviceNotAvailable) {
                    let n = err_count2.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if n == 50 {
                        failed2.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                }
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let prev = last_log2.swap(now, std::sync::atomic::Ordering::Relaxed);
                if now > prev {
                    log::error!("cpal stream error: {err}");
                }
            }
        }
    };

    let stream = match default_config.sample_format() {
        cpal::SampleFormat::F32 if pair_offset == 0 => device.build_output_stream(
            stream_cfg,
            move |data: &mut [f32], _| {
                cb_period.store((data.len() / total_ch) as u32, std::sync::atomic::Ordering::Relaxed);
                fill_buffer(data, total_ch, sample_rate, &cb_voices, &cb_feeds, &mut cmd_cons, &mut status_prod, &cb_mg, &cb_period);
            },
            make_err_fn(),
            None,
        )?,
        cpal::SampleFormat::F32 => {
            // Route stereo mix to the selected ASIO pair; zero the rest.
            let mut scratch = vec![0.0f32; 4096 * 2].into_boxed_slice();
            device.build_output_stream(
                stream_cfg,
                move |data: &mut [f32], _| {
                    let frames = data.len() / total_ch;
                    cb_period.store(frames as u32, std::sync::atomic::Ordering::Relaxed);
                    let n = (frames * 2).min(scratch.len());
                    fill_buffer(&mut scratch[..n], 2, sample_rate, &cb_voices, &cb_feeds, &mut cmd_cons, &mut status_prod, &cb_mg, &cb_period);
                    data.fill(0.0);
                    for f in 0..frames {
                        data[f * total_ch + pair_offset]     = scratch[f * 2];
                        data[f * total_ch + pair_offset + 1] = scratch[f * 2 + 1];
                    }
                },
                make_err_fn(),
                None,
            )?
        }
        cpal::SampleFormat::I32 => {
            // Pre-allocate stereo scratch — no alloc inside the RT callback.
            let scratch_len = (config.buffer_size as usize * 2).max(4096 * 2);
            let mut scratch = vec![0.0f32; scratch_len].into_boxed_slice();
            device.build_output_stream(
                stream_cfg,
                move |data: &mut [i32], _| {
                    let frames = data.len() / total_ch;
                    cb_period.store(frames as u32, std::sync::atomic::Ordering::Relaxed);
                    let n = (frames * 2).min(scratch.len());
                    fill_buffer(&mut scratch[..n], 2, sample_rate, &cb_voices, &cb_feeds, &mut cmd_cons, &mut status_prod, &cb_mg, &cb_period);
                    data.fill(0);
                    for f in 0..frames {
                        data[f * total_ch + pair_offset]     = (scratch[f * 2].clamp(-1.0, 1.0) * i32::MAX as f32) as i32;
                        data[f * total_ch + pair_offset + 1] = (scratch[f * 2 + 1].clamp(-1.0, 1.0) * i32::MAX as f32) as i32;
                    }
                },
                make_err_fn(),
                None,
            )?
        }
        fmt => return Err(anyhow!("Unsupported sample format: {fmt:?}")),
    };

    stream.play()?;
    log::info!(
        "Audio stream opened — backend={:?} device={:?} rate={}Hz channels={} buf={:?}",
        config.backend,
        config.device_id,
        sample_rate,
        channels,
        buf_size,
    );

    Ok(StreamResult { stream, sample_rate, channels: channels as u32, cmd_prod, status_cons })
}

// ---------------------------------------------------------------------------
// Audio callback — real-time safe
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn fill_buffer(
    output: &mut [f32],
    channels: usize,
    output_sample_rate: u32,
    voices: &Arc<Mutex<Vec<Arc<Voice>>>>,
    input_feeds: &Arc<Mutex<Vec<InputFeed>>>,
    cmd_cons: &mut ringbuf::HeapCons<AudioCommand>,
    status_prod: &mut ringbuf::HeapProd<AudioStatus>,
    master_gain: &Arc<std::sync::atomic::AtomicU32>,
    output_period: &Arc<std::sync::atomic::AtomicU32>,
) {
    output.fill(0.0);

    // FIXME: try_lock is acceptable for the prototype.  Replace with a
    // seqlock or atomic-swap voice list for true lock-free RT in production.
    let voices_guard = match voices.try_lock() {
        Ok(g) => g,
        Err(_) => return,
    };

    // Process incoming commands first.
    while let Some(cmd) = cmd_cons.try_pop() {
        apply_command(&voices_guard, cmd, status_prod);
    }

    // Keep live input feeds current: drain each device's ring into its staging
    // buffer (cheap, bounded) so live voices have fresh audio and the input
    // stays "warm" even with no Mic Cue playing.  Held for the voice loop.
    let mut feeds_guard = input_feeds.try_lock().ok();
    if let Some(feeds) = feeds_guard.as_deref_mut() {
        for feed in feeds.iter_mut() {
            feed.drain();
        }
    }

    let master = f32::from_bits(
        master_gain.load(std::sync::atomic::Ordering::Relaxed),
    );

    let frames = output.len() / channels;
    let mut peak_l = 0.0_f32;
    let mut peak_r = 0.0_f32;

    for voice in voices_guard.iter() {
        let state = voice.voice_state();
        if state != VoiceState::Playing && state != VoiceState::FadingOut {
            continue;
        }

        // Live (Mic Cue) voice — resample from its input feed instead of samples.
        if voice.live.is_some() {
            if let Some(feeds) = feeds_guard.as_deref() {
                mix_live(output, channels, output_sample_rate, voice, feeds, status_prod, &mut peak_l, &mut peak_r, output_period);
            }
            continue;
        }

        let (gain_l, gain_r) = voice.pan_gains();
        let voice_channels = voice.channels as usize;
        let total_frames = voice.total_frames();
        // Frame advance step: user rate × (source SR / output SR).
        // voice.inner.rate() is the pure user multiplier (1.0 = normal speed).
        let rate = voice.inner.rate() as f64
            * (voice.sample_rate as f64 / output_sample_rate as f64);

        // Maintain frame position as f64 for accurate sub-frame interpolation.
        // The fractional part is lost at callback boundaries (≤ 1 sample / ~22 µs
        // at 44 100 Hz), which is inaudible.
        let mut frame_pos_f = voice.frame_pos.load(std::sync::atomic::Ordering::Relaxed) as f64;

        // SAFETY: `fade` is only mutated from this callback; VoiceInner docs
        // establish the single-writer invariant.
        let fade_ptr = voice.inner.fade.get();

        // SAFETY: `end_frame` is written once before the voice is submitted.
        let end_frame_val: Option<u64> = unsafe { *voice.inner.end_frame.get() };
        let end = end_frame_val.unwrap_or(u64::MAX);

        let mut voice_stopped = false;

        for frame in 0..frames {
            // --- Per-frame fade gain -------------------------------------------
            let fade_gain: f32 = if let Some(fade) = unsafe { &mut *fade_ptr } {
                let g = fade.gain();
                let done = fade.advance(1);
                if done {
                    if fade.direction == FadeDirection::Out {
                        voice.set_stopped();
                        let _ = status_prod.try_push(AudioStatus::Completed { voice_id: voice.id });
                        unsafe { *fade_ptr = None };
                        voice_stopped = true;
                    } else {
                        // Fade-in complete — clear state, continue playing.
                        unsafe { *fade_ptr = None };
                    }
                }
                g
            } else {
                1.0_f32
            };

            if voice_stopped {
                break;
            }

            // --- Boundary / loop check ----------------------------------------
            let int_pos = frame_pos_f as u64;
            if int_pos >= end || int_pos >= total_frames {
                let loops = voice.inner.loops_remaining.load(std::sync::atomic::Ordering::Relaxed);
                if loops > 0 {
                    if loops != u32::MAX {
                        voice.inner.loops_remaining.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    frame_pos_f = 0.0;
                } else {
                    voice.set_stopped();
                    let _ = status_prod.try_push(AudioStatus::Completed { voice_id: voice.id });
                    break;
                }
            }

            // --- Sample with linear interpolation (handles rate != 1.0) --------
            let int_pos = frame_pos_f as u64;
            let frac = (frame_pos_f - int_pos as f64) as f32;
            let base = int_pos as usize * voice_channels;
            // Clamp next frame to last valid frame for interpolation at end.
            let next = (int_pos + 1).min(total_frames.saturating_sub(1)) as usize * voice_channels;

            let sample_l = voice.samples[base] + (voice.samples[next] - voice.samples[base]) * frac;
            let sample_r = if voice_channels > 1 {
                voice.samples[base + 1] + (voice.samples[next + 1] - voice.samples[base + 1]) * frac
            } else {
                sample_l
            };

            let out_base = frame * channels;
            let out_l = sample_l * gain_l * fade_gain;
            let out_r = sample_r * gain_r * fade_gain;

            // Route to the per-voice output channels (from Output Patch).
            // Bounds-check at the sample level keeps the RT callback safe even
            // if the patch references a channel the device does not have.
            if voice.out_l < channels { output[out_base + voice.out_l] += out_l; }
            if voice.out_r < channels { output[out_base + voice.out_r] += out_r; }

            peak_l = peak_l.max(out_l.abs());
            peak_r = peak_r.max(out_r.abs());

            frame_pos_f += rate;
        }

        // Store integer floor; sub-frame precision is re-established each callback.
        voice.frame_pos.store(frame_pos_f as u64, std::sync::atomic::Ordering::Relaxed);
    }

    // Apply master gain.
    for s in output.iter_mut() { *s *= master; }

    let _ = status_prod.try_push(AudioStatus::MasterLevels {
        peak_l: peak_l * master,
        peak_r: peak_r * master,
    });
}

/// Mix one live (Mic Cue) voice from its input feed into `output`.
///
/// Resamples the input device clock to the output clock with adaptive drift
/// compensation: the read cursor is kept ~25 ms behind the feed's write head,
/// and the resample ratio is nudged ±2 % to hold that lag, so slow clock drift
/// between the input and output devices never under/overruns.  Applies the
/// voice's pan/gain, soft fade, and Output-Patch channel routing exactly like
/// the file path.
#[allow(clippy::too_many_arguments)]
fn mix_live(
    output: &mut [f32],
    channels: usize,
    output_sample_rate: u32,
    voice: &Arc<Voice>,
    feeds: &[InputFeed],
    status_prod: &mut ringbuf::HeapProd<AudioStatus>,
    peak_l: &mut f32,
    peak_r: &mut f32,
    output_period: &Arc<std::sync::atomic::AtomicU32>,
) {
    let Some(live) = voice.live.as_ref() else { return };
    let Some(feed) = feeds.iter().find(|f| f.id == live.feed_id) else { return };
    // Need at least a couple of frames captured before we can interpolate.
    if feed.write_frame < 2 {
        return;
    }

    let frames = output.len() / channels;
    // Keep the read cursor 3 output periods behind the write head.  This gives
    // enough headroom to absorb one missed input callback without underrunning,
    // while minimising the imposed latency.  With 64-sample buffers at 48kHz
    // that is 3 × 64 / 48000 ≈ 4 ms — vs. the old fixed 1200 samples (25 ms).
    let period = output_period.load(std::sync::atomic::Ordering::Relaxed).max(64) as f64;
    let target_lag = (period * 3.0).max(192.0);

    if !live.is_started() {
        live.set_read_frame((feed.write_frame as f64 - target_lag).max(0.0));
        live.mark_started();
    }

    let base_ratio = live.src_rate as f64 / output_sample_rate as f64;
    let mut read = live.read_frame();
    // Resync on a gross lag (resume after pause, glitch, or runaway drift):
    // jump the cursor back to `target_lag` behind the write head.
    if !(0.0..=STAGING_FRAMES as f64).contains(&(feed.write_frame as f64 - read)) {
        read = (feed.write_frame as f64 - target_lag).max(0.0);
    }
    // Adaptive: nudge the ratio to hold the read cursor near `target_lag`.
    let lag = feed.write_frame as f64 - read;
    let correction = ((lag - target_lag) / target_lag).clamp(-0.02, 0.02);
    let ratio = base_ratio * (1.0 + correction);

    let (gain_l, gain_r) = voice.pan_gains();
    // SAFETY: `fade` is only mutated from this callback thread.
    let fade_ptr = voice.inner.fade.get();
    // Oldest frame still resident in the circular staging buffer.
    let oldest = feed.write_frame as f64 - (STAGING_FRAMES as f64 - 2.0);

    for frame in 0..frames {
        let fade_gain: f32 = if let Some(fade) = unsafe { &mut *fade_ptr } {
            let g = fade.gain();
            let done = fade.advance(1);
            if done {
                if fade.direction == FadeDirection::Out {
                    voice.set_stopped();
                    let _ = status_prod.try_push(AudioStatus::Completed { voice_id: voice.id });
                    unsafe { *fade_ptr = None };
                    break;
                } else {
                    unsafe { *fade_ptr = None };
                }
            }
            g
        } else {
            1.0
        };

        if read < oldest {
            read = oldest.max(0.0);
        }
        // Underrun: not enough fresh audio yet — stop here, resume next block.
        if read + 1.0 >= feed.write_frame as f64 {
            break;
        }

        let s_l = feed.sample(live.in_l, read);
        let s_r = if live.in_r == live.in_l { s_l } else { feed.sample(live.in_r, read) };
        let out_l = s_l * gain_l * fade_gain;
        let out_r = s_r * gain_r * fade_gain;

        let base = frame * channels;
        if voice.out_l < channels { output[base + voice.out_l] += out_l; }
        if voice.out_r < channels { output[base + voice.out_r] += out_r; }

        *peak_l = (*peak_l).max(out_l.abs());
        *peak_r = (*peak_r).max(out_r.abs());

        read += ratio;
    }

    live.set_read_frame(read);
}

fn apply_command(
    voices: &[Arc<Voice>],
    cmd: AudioCommand,
    _status_prod: &mut ringbuf::HeapProd<AudioStatus>,
) {
    match cmd {
        AudioCommand::Play { voice_id } => {
            if let Some(v) = voices.iter().find(|v| v.id == voice_id) {
                v.set_playing();
            }
        }
        AudioCommand::Stop { voice_id, fade_ms, fade_curve } => {
            if let Some(v) = voices.iter().find(|v| v.id == voice_id) {
                // A paused voice (e.g. a video's audio that was never resumed
                // because the video was replaced before its first frame) must
                // hard-stop: fading it would set it Playing and make it audible.
                if fade_ms == 0 || v.voice_state() == VoiceState::Paused {
                    v.set_stopped();
                } else {
                    let total = (fade_ms as u64 * v.sample_rate as u64) / 1000;
                    // SAFETY: Only written from this callback.
                    unsafe {
                        *v.inner.fade.get() = Some(FadeState {
                            direction: FadeDirection::Out,
                            total_samples: total,
                            elapsed_samples: 0,
                            curve: fade_curve,
                        });
                    }
                    v.state.store(VoiceState::FadingOut as u8, std::sync::atomic::Ordering::Release);
                }
            }
        }
        AudioCommand::Pause { voice_id } => {
            if let Some(v) = voices.iter().find(|v| v.id == voice_id) {
                v.set_paused();
            }
        }
        AudioCommand::Resume { voice_id } => {
            if let Some(v) = voices.iter().find(|v| v.id == voice_id) {
                v.set_playing();
            }
        }
        AudioCommand::SetGain { voice_id, gain } => {
            if let Some(v) = voices.iter().find(|v| v.id == voice_id) {
                v.inner.set_gain(gain);
            }
        }
        AudioCommand::SetPan { voice_id, pan } => {
            if let Some(v) = voices.iter().find(|v| v.id == voice_id) {
                v.inner.set_pan(pan);
            }
        }
        AudioCommand::SetMasterGain { .. } => {}
        AudioCommand::Seek { voice_id, frame_pos } => {
            if let Some(v) = voices.iter().find(|v| v.id == voice_id) {
                v.frame_pos.store(frame_pos, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
}


/// Open and return the ASIO cpal host.
///
/// Requires the `asio-support` Cargo feature. Returns an error when the
/// feature is absent or no ASIO host is detected at runtime.
fn open_asio_host() -> Result<cpal::Host> {
    #[cfg(all(windows, feature = "asio-support"))]
    {
        let asio = cpal::available_hosts()
            .into_iter()
            .filter(|id| *id != cpal::default_host().id())
            .find_map(|id| cpal::host_from_id(id).ok());
        asio.ok_or_else(|| anyhow!("No ASIO host found. Ensure your ASIO drivers are installed."))
    }
    #[cfg(not(all(windows, feature = "asio-support")))]
    {
        Err(anyhow!(
            "ASIO support is not compiled in. \
             Install the Steinberg ASIO SDK, set CPAL_ASIO_DIR, \
             then build with: pnpm tauri build -- --features asio-support"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ringbuf::HeapRb;
    use ringbuf::traits::{Producer, Split};

    /// Build a minimal Voice with `n_frames` of silence at the given sample rate.
    fn make_voice(n_frames: usize, channels: u16, sample_rate: u32, rate: f32) -> Arc<Voice> {
        let samples = Arc::new(vec![0.0f32; n_frames * channels as usize]);
        let v = Voice::new(samples, channels, sample_rate, 1.0, 0.0);
        v.inner.set_rate(rate);
        v.set_playing();
        Arc::new(v)
    }

    /// Call fill_buffer for `output_frames` output frames and return the
    /// resulting frame_pos stored in the voice.
    fn run_fill(voice: Arc<Voice>, output_frames: usize, output_sr: u32) -> u64 {
        let pool: Arc<Mutex<Vec<Arc<Voice>>>> = Arc::new(Mutex::new(vec![voice]));
        let feeds: Arc<Mutex<Vec<InputFeed>>> = Arc::new(Mutex::new(Vec::new()));
        let (_, mut cmd_cons) = HeapRb::<AudioCommand>::new(16).split();
        let (mut status_prod, _) = HeapRb::<AudioStatus>::new(16).split();
        let master  = Arc::new(std::sync::atomic::AtomicU32::new(f32::to_bits(1.0)));
        let period  = Arc::new(std::sync::atomic::AtomicU32::new(output_frames as u32));
        let mut output = vec![0.0f32; output_frames * 2];
        fill_buffer(&mut output, 2, output_sr, &pool, &feeds, &mut cmd_cons, &mut status_prod, &master, &period);
        let pos = pool.lock().unwrap().first().map(|v| v.frame_pos.load(std::sync::atomic::Ordering::Relaxed)).unwrap_or(0);
        pos
    }

    // The SR ratio (e.g. 44100/48000) is not exactly representable in f64, so
    // frame_pos after N output frames may be off by ±1 source frame.  All
    // assertions below allow a tolerance of 1 frame.
    fn assert_frame_pos(actual: u64, expected: u64, msg: &str) {
        let diff = (actual as i64 - expected as i64).unsigned_abs();
        assert!(diff <= 1, "{msg}: expected {expected} ± 1, got {actual}");
    }

    #[test]
    fn sr_ratio_44100_on_48000() {
        // 1 s of 48 kHz output = 48 000 frames → should consume 44 100 source frames.
        let voice = make_voice(220_500, 2, 44_100, 1.0);
        let pos = run_fill(voice, 48_000, 48_000);
        assert_frame_pos(pos, 44_100, "44.1 kHz file on 48 kHz output");
    }

    #[test]
    fn sr_ratio_48000_on_48000() {
        // Same SR: 1 output frame = 1 source frame exactly.
        let voice = make_voice(96_000, 2, 48_000, 1.0);
        let pos = run_fill(voice, 48_000, 48_000);
        assert_frame_pos(pos, 48_000, "48 kHz file on 48 kHz output");
    }

    #[test]
    fn sr_ratio_48000_on_44100() {
        // 1 s of 44.1 kHz output = 44 100 frames → should consume 48 000 source frames.
        let voice = make_voice(96_000, 2, 48_000, 1.0);
        let pos = run_fill(voice, 44_100, 44_100);
        assert_frame_pos(pos, 48_000, "48 kHz file on 44.1 kHz output");
    }

    #[test]
    fn user_rate_2x_on_matching_sr() {
        // rate=2.0 on matching SR: 2 source frames per output frame.
        let voice = make_voice(96_000, 2, 48_000, 2.0);
        let pos = run_fill(voice, 48_000, 48_000);
        assert_frame_pos(pos, 96_000, "rate=2.0 should consume 96 000 frames in 1 s");
    }

    #[test]
    fn sr_ratio_96000_on_48000() {
        // 96 kHz file on 48 kHz output: rate step = 2.0, correct duration.
        // Note: no anti-aliasing filter — content above 24 kHz may alias, but
        // in practice 96 kHz files are already band-limited below 20 kHz.
        let voice = make_voice(192_000, 2, 96_000, 1.0);
        let pos = run_fill(voice, 48_000, 48_000);
        assert_frame_pos(pos, 96_000, "96 kHz file on 48 kHz output: 1 s = 96 000 source frames");
    }

    // ── Live input feed / resampler ────────────────────────────────────────

    /// Build a test feed (no real device) plus its ring producer.
    fn make_feed(in_channels: usize) -> (InputFeed, ringbuf::HeapProd<f32>) {
        let (prod, cons) = HeapRb::<f32>::new(STAGING_FRAMES * in_channels).split();
        let feed = InputFeed {
            id: Uuid::new_v4(),
            device_id: "test".into(),
            in_channels,
            sample_rate: 48_000,
            cons,
            staging: vec![0.0_f32; STAGING_FRAMES * in_channels].into_boxed_slice(),
            write_frame: 0,
            _capture: None,
        };
        (feed, prod)
    }

    #[test]
    fn feed_drain_advances_write_and_interpolates() {
        let (mut feed, mut prod) = make_feed(1);
        for i in 0..4 {
            let _ = prod.try_push(i as f32); // ramp 0,1,2,3
        }
        feed.drain();
        assert_eq!(feed.write_frame, 4);
        assert_eq!(feed.sample(0, 0.0), 0.0);
        assert_eq!(feed.sample(0, 2.0), 2.0);
        assert!((feed.sample(0, 1.5) - 1.5).abs() < 1e-6, "linear interp between frames");
    }

    #[test]
    fn mix_live_unity_ratio_routes_input() {
        let (mut feed, mut prod) = make_feed(2);
        for _ in 0..2000 {
            let _ = prod.try_push(0.5); // L
            let _ = prod.try_push(0.25); // R
        }
        feed.drain();
        let feeds = vec![feed];

        let live = LiveSource::new(feeds[0].id, 0, 1, 48_000);
        let voice = Arc::new(Voice::new_live(live, 48_000, 1.0, 0.0)); // unity gain, center pan
        let (mut status_prod, _) = HeapRb::<AudioStatus>::new(64).split();
        let mut out = vec![0.0_f32; 256 * 2];
        let (mut pl, mut pr) = (0.0_f32, 0.0_f32);
        let period = Arc::new(std::sync::atomic::AtomicU32::new(256));

        mix_live(&mut out, 2, 48_000, &voice, &feeds, &mut status_prod, &mut pl, &mut pr, &period);

        // Center pan → both gains = sqrt(0.5) ≈ 0.707; L = 0.5·0.707 ≈ 0.354.
        assert!(out[0] > 0.34 && out[0] < 0.37, "L sample was {}", out[0]);
        assert!(out[1] > 0.16 && out[1] < 0.19, "R sample was {}", out[1]);

        // in_sr == out_sr, target_lag = 3 × period = 768, read advances 256.
        let read = voice.live.as_ref().unwrap().read_frame();
        let target_lag = 3.0 * 256.0_f64;
        let expected = (feeds[0].write_frame as f64 - target_lag) + 256.0;
        assert!((read - expected).abs() < 5.0, "read {read} vs expected {expected}");
    }
}
