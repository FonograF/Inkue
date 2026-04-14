//! [`AudioEngine`] — the top-level audio subsystem.
//!
//! **Real-time safety:** the audio callback (`fill_buffer`) must never
//! allocate, block, or do I/O.  All state mutations happen via the command ring
//! buffer; all outgoing data goes through the status ring buffer.

use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Stream, StreamConfig};
use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::HeapRb;

use super::{
    device_manager::DeviceManager,
    ring_command::{AudioCommand, AudioStatus, FadeCurve, VoiceId},
    voice::{FadeDirection, FadeState, Voice, VoiceState},
};

const _MAX_VOICES: usize = 64;
const RING_CAPACITY: usize = 256;
pub const DEFAULT_FADE_OUT_MS: u32 = 500;

/// The audio engine.
pub struct AudioEngine {
    pub device_manager: Mutex<DeviceManager>,
    cmd_prod: Mutex<ringbuf::HeapProd<AudioCommand>>,
    status_cons: Mutex<ringbuf::HeapCons<AudioStatus>>,
    voices: Arc<Mutex<Vec<Arc<Voice>>>>,
    _stream: Mutex<Option<Stream>>,
    sample_rate: std::sync::atomic::AtomicU32,
    /// Total output channel count of the current stream (updated on restart).
    output_channels: std::sync::atomic::AtomicU32,
    master_gain: Arc<std::sync::atomic::AtomicU32>,
}

// SAFETY: cpal::Stream is not Send on Windows when using WASAPI.
unsafe impl Send for AudioEngine {}
unsafe impl Sync for AudioEngine {}

impl AudioEngine {
    /// Open the default output device and start the audio callback.
    pub fn new() -> Result<Arc<Self>> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| anyhow!("No default audio output device found"))?;

        let config = device.default_output_config()?;
        let sample_rate = config.sample_rate().0;
        let channels = config.channels();

        let (cmd_prod, mut cmd_cons) = HeapRb::<AudioCommand>::new(RING_CAPACITY).split();
        let (mut status_prod, status_cons) = HeapRb::<AudioStatus>::new(RING_CAPACITY).split();

        let shared_voices: Arc<Mutex<Vec<Arc<Voice>>>> = Arc::new(Mutex::new(Vec::new()));
        let cb_voices = Arc::clone(&shared_voices);

        let master_gain = Arc::new(std::sync::atomic::AtomicU32::new(f32::to_bits(1.0_f32)));
        let cb_master_gain = Arc::clone(&master_gain);
        let engine_master_gain = Arc::clone(&master_gain);

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_output_stream(
                &StreamConfig {
                    channels,
                    sample_rate: cpal::SampleRate(sample_rate),
                    buffer_size: cpal::BufferSize::Default,
                },
                move |data: &mut [f32], _| {
                    fill_buffer(data, channels as usize, &cb_voices, &mut cmd_cons, &mut status_prod, &cb_master_gain);
                },
                |err| log::error!("cpal stream error: {err}"),
                None,
            )?,
            cpal::SampleFormat::I32 => {
                // Pre-allocate scratch buffer once — no allocation inside the RT callback.
                let mut scratch = vec![0.0f32; 4096 * channels as usize].into_boxed_slice();
                device.build_output_stream(
                    &StreamConfig {
                        channels,
                        sample_rate: cpal::SampleRate(sample_rate),
                        buffer_size: cpal::BufferSize::Default,
                    },
                    move |data: &mut [i32], _| {
                        let n = data.len().min(scratch.len());
                        fill_buffer(&mut scratch[..n], channels as usize, &cb_voices, &mut cmd_cons, &mut status_prod, &cb_master_gain);
                        for (out, &s) in data.iter_mut().zip(scratch[..n].iter()) {
                            *out = (s.clamp(-1.0, 1.0) * i32::MAX as f32) as i32;
                        }
                    },
                    |err| log::error!("cpal stream error: {err}"),
                    None,
                )?
            }
            fmt => return Err(anyhow!("Unsupported sample format: {fmt:?}")),
        };

        stream.play()?;

        Ok(Arc::new(Self {
            device_manager: Mutex::new(DeviceManager::new()),
            cmd_prod: Mutex::new(cmd_prod),
            status_cons: Mutex::new(status_cons),
            voices: shared_voices,
            _stream: Mutex::new(Some(stream)),
            sample_rate: std::sync::atomic::AtomicU32::new(sample_rate),
            output_channels: std::sync::atomic::AtomicU32::new(channels as u32),
            master_gain: engine_master_gain,
        }))
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

    /// Remove fully-stopped voices from the pool.
    pub fn gc_voices(&self) {
        if let Ok(mut voices) = self.voices.lock() {
            voices.retain(|v| {
                !matches!(v.voice_state(), VoiceState::Stopped | VoiceState::Idle)
            });
        }
    }

    /// Stop the current stream and re-open on the specified device/backend.
    /// All active voices are killed; cues should be reset by the caller.
    pub fn restart(
        &self,
        device_name: Option<&str>,
        backend: &crate::preferences::AudioBackend,
        buffer_size: u32,
        asio_out_pair: u32,
    ) -> Result<()> {
        use crate::preferences::AudioBackend;

        // Kill all active voices.
        if let Ok(mut voices) = self.voices.lock() {
            for v in voices.iter() { v.set_stopped(); }
            voices.clear();
        }

        // Drop the old stream.
        {
            let mut sg = self._stream.lock().map_err(|_| anyhow!("stream mutex poisoned"))?;
            *sg = None;
        }

        // Select host.
        let host = match backend {
            AudioBackend::WasapiShared | AudioBackend::WasapiExclusive => cpal::default_host(),
            AudioBackend::Asio => open_asio_host()?,
        };

        // Select device.
        let device = if matches!(backend, AudioBackend::Asio) {
            // Try to find the ASIO driver by name; fall back to the first available.
            let found = device_name
                .filter(|s| !s.is_empty())
                .and_then(|name| {
                    host.output_devices().ok()
                        .and_then(|mut it| it.find(|d| d.name().ok().as_deref() == Some(name)))
                });
            found
                .or_else(|| host.default_output_device())
                .ok_or_else(|| anyhow!(
                    "No ASIO device found. Make sure the driver is not \
                     already in use by another application."
                ))?
        } else if let Some(name) = device_name.filter(|s| !s.is_empty()) {
            host.output_devices()
                .map_err(|e| anyhow!("Failed to enumerate devices: {e}"))?
                .find(|d| d.name().ok().as_deref() == Some(name))
                .ok_or_else(|| anyhow!("Audio device '{}' not found", name))?
        } else {
            host.default_output_device()
                .ok_or_else(|| anyhow!("No default audio output device found"))?
        };

        let default_config = device.default_output_config()
            .map_err(|e| anyhow!("Device config error: {e}"))?;
        let new_sample_rate = default_config.sample_rate().0;
        let channels = default_config.channels();

        let buf_size = match backend {
            // ASIO drivers have a preferred buffer size set in their own control panel.
            // Forcing a Fixed size often causes the stream to fail or underrun.
            AudioBackend::WasapiExclusive => cpal::BufferSize::Fixed(buffer_size),
            AudioBackend::Asio | AudioBackend::WasapiShared => cpal::BufferSize::Default,
        };

        // New ring buffers.
        let (new_cmd_prod, mut new_cmd_cons) = HeapRb::<AudioCommand>::new(RING_CAPACITY).split();
        let (mut new_status_prod, new_status_cons) = HeapRb::<AudioStatus>::new(RING_CAPACITY).split();

        // Build new stream — reuse same voices and master_gain.
        let cb_voices = Arc::clone(&self.voices);
        let cb_mg = Arc::clone(&self.master_gain);

        // For ASIO: route the internal stereo mix to the selected output pair.
        // pair_offset = first channel index of the selected pair (e.g. pair 1 → offset 2).
        let total_ch = channels as usize;
        let pair_offset = (asio_out_pair as usize * 2).min(total_ch.saturating_sub(2));

        let stream_cfg = cpal::StreamConfig {
            channels,
            sample_rate: cpal::SampleRate(new_sample_rate),
            buffer_size: buf_size,
        };

        let new_stream = match default_config.sample_format() {
            cpal::SampleFormat::F32 if pair_offset == 0 => device.build_output_stream(
                &stream_cfg,
                move |data: &mut [f32], _| {
                    fill_buffer(data, total_ch, &cb_voices, &mut new_cmd_cons, &mut new_status_prod, &cb_mg);
                },
                |err| log::error!("cpal stream error: {err}"),
                None,
            )?,
            cpal::SampleFormat::F32 => {
                // Route stereo mix to the selected pair; zero the rest.
                let mut scratch = vec![0.0f32; 4096 * 2].into_boxed_slice();
                device.build_output_stream(
                    &stream_cfg,
                    move |data: &mut [f32], _| {
                        let frames = data.len() / total_ch;
                        let n = (frames * 2).min(scratch.len());
                        fill_buffer(&mut scratch[..n], 2, &cb_voices, &mut new_cmd_cons, &mut new_status_prod, &cb_mg);
                        data.fill(0.0);
                        for f in 0..frames {
                            data[f * total_ch + pair_offset]     = scratch[f * 2];
                            data[f * total_ch + pair_offset + 1] = scratch[f * 2 + 1];
                        }
                    },
                    |err| log::error!("cpal stream error: {err}"),
                    None,
                )?
            }
            cpal::SampleFormat::I32 => {
                // Pre-allocate stereo scratch — no alloc inside the RT callback.
                let scratch_len = (buffer_size as usize * 2).max(4096 * 2);
                let mut scratch = vec![0.0f32; scratch_len].into_boxed_slice();
                device.build_output_stream(
                    &stream_cfg,
                    move |data: &mut [i32], _| {
                        let frames = data.len() / total_ch;
                        let n = (frames * 2).min(scratch.len());
                        fill_buffer(&mut scratch[..n], 2, &cb_voices, &mut new_cmd_cons, &mut new_status_prod, &cb_mg);
                        data.fill(0);
                        for f in 0..frames {
                            data[f * total_ch + pair_offset]     = (scratch[f * 2].clamp(-1.0, 1.0) * i32::MAX as f32) as i32;
                            data[f * total_ch + pair_offset + 1] = (scratch[f * 2 + 1].clamp(-1.0, 1.0) * i32::MAX as f32) as i32;
                        }
                    },
                    |err| log::error!("cpal stream error: {err}"),
                    None,
                )?
            }
            fmt => return Err(anyhow!("Unsupported sample format: {fmt:?}")),
        };
        new_stream.play()?;

        // Swap ring buffers and stream.
        *self.cmd_prod.lock().map_err(|_| anyhow!("cmd_prod poisoned"))? = new_cmd_prod;
        *self.status_cons.lock().map_err(|_| anyhow!("status_cons poisoned"))? = new_status_cons;
        *self._stream.lock().map_err(|_| anyhow!("stream poisoned"))? = Some(new_stream);
        self.sample_rate.store(new_sample_rate, std::sync::atomic::Ordering::Relaxed);
        self.output_channels.store(channels as u32, std::sync::atomic::Ordering::Relaxed);

        // Refresh device manager.
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
// Audio callback — real-time safe
// ---------------------------------------------------------------------------

fn fill_buffer(
    output: &mut [f32],
    channels: usize,
    voices: &Arc<Mutex<Vec<Arc<Voice>>>>,
    cmd_cons: &mut ringbuf::HeapCons<AudioCommand>,
    status_prod: &mut ringbuf::HeapProd<AudioStatus>,
    master_gain: &Arc<std::sync::atomic::AtomicU32>,
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

        let (gain_l, gain_r) = voice.pan_gains();
        let voice_channels = voice.channels as usize;
        let total_frames = voice.total_frames();
        // Rate as f64 for sub-frame accumulation.  Default 1.0.
        let rate = voice.inner.rate() as f64;

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
                if fade_ms == 0 {
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
    }
}


/// Open and return the ASIO cpal host.
///
/// Requires the `asio-support` Cargo feature. Returns an error when the
/// feature is absent or no ASIO host is detected at runtime.
fn open_asio_host() -> Result<cpal::Host> {
    #[cfg(feature = "asio-support")]
    {
        let asio = cpal::available_hosts()
            .into_iter()
            .filter(|id| *id != cpal::default_host().id())
            .find_map(|id| cpal::host_from_id(id).ok());
        asio.ok_or_else(|| anyhow!("No ASIO host found. Ensure your ASIO drivers are installed."))
    }
    #[cfg(not(feature = "asio-support"))]
    {
        Err(anyhow!(
            "ASIO support is not compiled in. \
             Install the Steinberg ASIO SDK, set CPAL_ASIO_DIR, \
             then build with: pnpm tauri build -- --features asio-support"
        ))
    }
}
