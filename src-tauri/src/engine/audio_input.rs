//! Live audio input capture for Mic Cues.
//!
//! Provides the [`InputPatch`] model (a named device + channel selection,
//! mirror of [`OutputPatch`](super::device_manager::OutputPatch)), input-device
//! enumeration, and a **persistent** cpal input stream that streams interleaved
//! `f32` frames into a lock-free [`ringbuf`] ring shared with the output
//! callback.
//!
//! The stream is opened when a Mic Cue fires (`ensure_input_feed`) and closed
//! once no live voice references it any more (`gc_voices`), so the OS releases
//! the capture device — and turns off its indicator — when the Mic Cue stops.
//! The output callback drains the ring; drift between the input and output
//! device clocks is absorbed there (see the live voice path in
//! [`audio_engine`](super::audio_engine)).
//!
//! Cross-platform: uses the generic cpal host (WASAPI/CoreAudio/ALSA) — no
//! per-OS API.

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::Stream;
use ringbuf::traits::{Producer, Split};
use ringbuf::{HeapCons, HeapRb};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::device_manager::DeviceInfo;

/// Unique identifier for an Input Patch.
pub type InputPatchId = Uuid;

/// A named mapping from a label to a specific audio **input** device and a set
/// of channels — the input-side mirror of
/// [`OutputPatch`](super::device_manager::OutputPatch).
///
/// A [`MicCue`](crate::cue::mic_cue::MicCue) references an `InputPatch` (plus the
/// channels it wants from it) rather than a device directly, so re-patching a
/// show changes only one place.  Stored in the workspace (`.wincue`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputPatch {
    pub id: InputPatchId,
    /// Display label shown in the UI (e.g. "Stage mics", "Presenter").
    pub name: String,
    /// The OS device identifier (name) this patch captures from.
    pub device_id: String,
    /// Zero-based channel indices on the input device this patch exposes.
    pub channels: Vec<u16>,
}

impl InputPatch {
    /// Create a new patch with a fresh UUID.
    pub fn new(name: impl Into<String>, device_id: impl Into<String>, channels: Vec<u16>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            device_id: device_id.into(),
            channels,
        }
    }
}

/// Enumerate available audio **input** devices.
///
/// Mirrors [`DeviceManager::refresh_devices`](super::device_manager::DeviceManager::refresh_devices)
/// but for the input side.  Best-effort: devices whose config can't be read are
/// reported with conservative defaults.
pub fn list_input_devices() -> Vec<DeviceInfo> {
    let host = cpal::default_host();
    let mut devices = Vec::new();
    let Ok(iter) = host.input_devices() else { return devices };
    for device in iter {
        let name = device.name().unwrap_or_else(|_| "Unknown".to_string());
        let (channels, sample_rate) = device
            .default_input_config()
            .map(|c| (c.channels(), c.sample_rate().0))
            .unwrap_or((2, 48_000));
        devices.push(DeviceInfo { id: name.clone(), name, channels, sample_rate });
    }
    devices
}

/// A running input capture: the persistent cpal stream plus the metadata the
/// output callback needs to consume and resample its frames.
///
/// The [`Stream`] is kept alive for as long as the capture is needed; dropping
/// it stops the device.  The matching ring **consumer** is handed to the caller
/// (the [`AudioEngine`](super::audio_engine::AudioEngine)) at construction.
pub struct InputCapture {
    /// OS device id (name) this capture reads from.
    pub device_id: String,
    /// Number of interleaved channels in each frame written to the ring.
    pub channels: u16,
    /// Sample rate of the captured audio (the input device clock).
    pub sample_rate: u32,
    /// The live cpal input stream (held to keep capture running).
    _stream: Stream,
}

// SAFETY: cpal::Stream is not Send on Windows (WASAPI), mirroring the assertion
// made for the output stream in `audio_engine`.
unsafe impl Send for InputCapture {}
unsafe impl Sync for InputCapture {}

/// Ring capacity in frames-worth of samples. ~0.5 s of stereo @ 48 kHz; large
/// enough to never overflow between output callbacks, small enough that a flush
/// at tap-open is cheap.
const RING_FRAMES: usize = 48_000 / 2;

/// Open a persistent input stream for `device_id` (or the default input device
/// when `None`/empty), returning the running [`InputCapture`] and the ring
/// **consumer** the output callback reads from.
///
/// The input callback only ever calls `try_push` (no allocation, no lock); when
/// the ring is full it drops the oldest-unconsumed samples by failing the push,
/// which is harmless because the output side always wants the most recent audio.
/// Open a persistent input stream.
/// `buffer_size`: target period in frames (`0` = OS default).
pub fn open_input(device_id: Option<&str>, buffer_size: u32) -> Result<(InputCapture, HeapCons<f32>)> {
    let host = cpal::default_host();

    let device = match device_id.filter(|s| !s.is_empty()) {
        Some(name) => host
            .input_devices()
            .map_err(|e| anyhow!("Failed to enumerate input devices: {e}"))?
            .find(|d| d.name().ok().as_deref() == Some(name))
            .ok_or_else(|| anyhow!("Audio input device '{}' not found", name))?,
        None => host
            .default_input_device()
            .ok_or_else(|| anyhow!("No default audio input device found"))?,
    };

    let default_config = device
        .default_input_config()
        .map_err(|e| anyhow!("Input device config error: {e}"))?;
    let sample_rate = default_config.sample_rate().0;
    let channels = default_config.channels();
    let sample_format = default_config.sample_format();
    // Apply the same buffer size as the output stream so both device clocks
    // fire at the same cadence.  Use Default when buffer_size == 0 (ASIO mode).
    let buf = if buffer_size > 0 {
        cpal::BufferSize::Fixed(buffer_size)
    } else {
        cpal::BufferSize::Default
    };
    let mut cfg: cpal::StreamConfig = default_config.into();
    cfg.buffer_size = buf;

    let capacity = (RING_FRAMES * channels as usize).max(4096);
    let (mut prod, cons) = HeapRb::<f32>::new(capacity).split();

    let err_fn = |err| log::error!("cpal input stream error: {err}");

    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &cfg,
            move |data: &[f32], _| {
                for &s in data {
                    let _ = prod.try_push(s);
                }
            },
            err_fn,
            None,
        )?,
        cpal::SampleFormat::I16 => device.build_input_stream(
            &cfg,
            move |data: &[i16], _| {
                for &s in data {
                    let _ = prod.try_push(s as f32 / i16::MAX as f32);
                }
            },
            err_fn,
            None,
        )?,
        cpal::SampleFormat::I32 => device.build_input_stream(
            &cfg,
            move |data: &[i32], _| {
                for &s in data {
                    let _ = prod.try_push(s as f32 / i32::MAX as f32);
                }
            },
            err_fn,
            None,
        )?,
        fmt => return Err(anyhow!("Unsupported input sample format: {fmt:?}")),
    };

    stream.play()?;
    log::info!(
        "Audio input opened — device={:?} rate={}Hz channels={}",
        device_id,
        sample_rate,
        channels,
    );

    let capture = InputCapture { device_id: device_id.unwrap_or_default().to_string(), channels, sample_rate, _stream: stream };
    Ok((capture, cons))
}
