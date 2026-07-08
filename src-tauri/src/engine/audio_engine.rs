//! [`AudioEngine`] — the top-level audio subsystem.
//!
//! **Real-time safety:** the audio callback (`fill_buffer`) must never
//! allocate, block, or do I/O.  All state mutations happen via the command ring
//! buffer; all outgoing data goes through the status ring buffer.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use arc_swap::ArcSwap;
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


/// Number of retired voice-list generations kept alive on the non-RT side so
/// the real-time callback never holds the last reference to one — and therefore
/// never runs a `Vec`/`Voice`/samples destructor.  A single output callback
/// (a few ms) can never outlive this many publishes (each driven by a GO / GC),
/// so 3 is a comfortable margin.
const VOICE_POOL_RETAIN: usize = 3;

/// A voice pool published **wait-free** to the real-time audio callback.
///
/// The RT callback reads a snapshot via [`ArcSwap::load`] — no lock, so it can
/// never be starved into emitting a block of silence (the old `try_lock`
/// failure mode).  Non-RT threads mutate `master` (a plain `Mutex`, only ever
/// contended among themselves) and then [`publish`](VoicePool::publish) a fresh
/// immutable snapshot.  Retired snapshots are held in `retained` for a few
/// generations so their destructors always run on a non-RT thread — the RT
/// thread allocates and frees nothing.
struct VoicePool {
    /// Editable master list — locked only by non-RT threads.
    master: Mutex<Vec<Arc<Voice>>>,
    /// Immutable snapshot the RT callback loads.
    rt: Arc<ArcSwap<Vec<Arc<Voice>>>>,
    /// Keep-alive ring of recent snapshots so the RT thread never deallocs one.
    retained: Mutex<VecDeque<Arc<Vec<Arc<Voice>>>>>,
}

impl VoicePool {
    fn new() -> Self {
        Self {
            master: Mutex::new(Vec::new()),
            rt: Arc::new(ArcSwap::from_pointee(Vec::new())),
            retained: Mutex::new(VecDeque::new()),
        }
    }

    /// The handle the RT callback reads from (`load()` each block).
    fn rt_handle(&self) -> Arc<ArcSwap<Vec<Arc<Voice>>>> {
        Arc::clone(&self.rt)
    }

    /// Publish the current master list as a new immutable snapshot, retaining a
    /// few prior generations so the RT thread never drops the last reference.
    fn publish(&self, master: &[Arc<Voice>]) {
        let snap = Arc::new(master.to_vec());
        self.rt.store(Arc::clone(&snap));
        if let Ok(mut ret) = self.retained.lock() {
            ret.push_back(snap);
            while ret.len() > VOICE_POOL_RETAIN {
                ret.pop_front(); // dropped here, on this non-RT thread
            }
        }
    }

    /// Append a voice and publish.  On a poisoned lock the voice is handed back
    /// (`Err`) so a caller can recover it — e.g. fall the cue back to the main
    /// output instead of losing it.
    fn push(&self, voice: Arc<Voice>) -> std::result::Result<(), Arc<Voice>> {
        match self.master.lock() {
            Ok(mut m) => {
                m.push(voice);
                self.publish(&m);
                Ok(())
            }
            Err(_) => Err(voice),
        }
    }

    /// Run `f` over the current voices (non-RT read; locks `master`).
    fn with<T>(&self, f: impl FnOnce(&[Arc<Voice>]) -> T) -> Option<T> {
        self.master.lock().ok().map(|m| f(&m))
    }

    /// Retain voices matching `keep`, then publish.  Removed `Arc<Voice>`s are
    /// dropped on this (non-RT) thread.
    fn retain(&self, keep: impl Fn(&Arc<Voice>) -> bool) {
        if let Ok(mut m) = self.master.lock() {
            m.retain(|v| keep(v));
            self.publish(&m);
        }
    }
}

/// One additional cpal output stream opened for an Output Patch that targets
/// a different device than the main stream.
///
/// Owns its own voice pool (mixed only by its own callback — per-stream pools
/// keep RT `try_lock` contention identical to the single-stream design) and
/// its own command/status rings.  Live (Mic) voices never land here: input
/// feeds are drained exclusively by the main stream's callback, so aux
/// streams get a permanently empty feed list.
struct AuxStream {
    /// OS device id this stream is open on.
    device_id: String,
    /// Output channel count of this stream (for channel-bounds alerts).
    channels: u32,
    voices: VoicePool,
    cmd_prod: ringbuf::HeapProd<AudioCommand>,
    status_cons: ringbuf::HeapCons<AudioStatus>,
    /// Set by the cpal error callback on DeviceNotAvailable.
    failed: Arc<std::sync::atomic::AtomicBool>,
    _stream: Stream,
}

/// The audio engine.
pub struct AudioEngine {
    pub device_manager: Mutex<DeviceManager>,
    cmd_prod: Mutex<ringbuf::HeapProd<AudioCommand>>,
    status_cons: Mutex<ringbuf::HeapCons<AudioStatus>>,
    voices: VoicePool,
    /// Additional per-device streams for Output Patches targeting a device
    /// other than the main one.  Opened lazily at the first GO that needs the
    /// device, kept open afterwards so later GOs start with zero device-open
    /// latency.  Never contains the main device.
    aux_streams: Mutex<Vec<AuxStream>>,
    /// Live input feeds (one per captured device), shared with the output
    /// callback which drains them each block.
    input_feeds: Arc<Mutex<Vec<InputFeed>>>,
    _stream: Mutex<Option<Stream>>,
    sample_rate: std::sync::atomic::AtomicU32,
    /// Actual output callback period in frames (updated on the first callback).
    /// Used by `mix_live` to set a tight `target_lag` instead of a fixed 25 ms.
    output_period: Arc<std::sync::atomic::AtomicU32>,
    /// Monotonic count of output callbacks fired.  Shared across restarts; the
    /// device watchdog treats a count that stops advancing for ~2 s as a dead
    /// stream (kind-agnostic device-loss detection, even if cpal reports no error).
    output_callbacks: Arc<std::sync::atomic::AtomicU64>,
    /// Total output channel count of the current stream (updated on restart).
    output_channels: std::sync::atomic::AtomicU32,
    /// Output-channel offset applied to unpatched voices at submission — the
    /// selected ASIO output pair (0 on other backends).
    default_out_offset: std::sync::atomic::AtomicU32,
    master_gain: Arc<std::sync::atomic::AtomicU32>,
    /// Failure flag of the **current** stream (replaced on every restart).  Set by
    /// the cpal error callback after repeated `DeviceNotAvailable`; read by the
    /// device watchdog to detect a lost output device mid-show.
    stream_failed: Mutex<Arc<std::sync::atomic::AtomicBool>>,
    /// The config the operator actually selected — kept across an automatic
    /// fallback so the watchdog can detect the desired device's return.
    desired_config: Mutex<MachineAudioConfig>,
    /// Device id the current stream is open on (`None` = system default).
    current_device_id: Mutex<Option<String>>,
    /// `true` while running on the default device after the desired one was lost.
    in_fallback: std::sync::atomic::AtomicBool,
}

/// Snapshot of the output device's health, read by the device watchdog.
#[derive(Debug, Clone)]
pub struct AudioHealth {
    /// The current stream is erroring (device pulled / grabbed exclusively).
    pub failed: bool,
    /// Running on the default device after an automatic fallback.
    pub in_fallback: bool,
    /// The operator-selected device's display label — friendly name when known,
    /// else its id (`None` = system default).
    pub desired_device: Option<String>,
    /// Whether the desired device is currently present among enumerated devices.
    pub desired_present: bool,
}

// SAFETY: cpal::Stream is not Send on Windows when using WASAPI.
unsafe impl Send for AudioEngine {}
unsafe impl Sync for AudioEngine {}

impl AudioEngine {
    /// Open an output device according to `config` and start the audio callback.
    pub fn new(config: &MachineAudioConfig) -> Result<Arc<Self>> {
        let master_gain = Arc::new(std::sync::atomic::AtomicU32::new(f32::to_bits(1.0_f32)));
        let voices = VoicePool::new();
        let voices_rt = voices.rt_handle();
        let input_feeds: Arc<Mutex<Vec<InputFeed>>> = Arc::new(Mutex::new(Vec::new()));
        let output_period = Arc::new(std::sync::atomic::AtomicU32::new(256));
        let output_callbacks = Arc::new(std::sync::atomic::AtomicU64::new(0));

        let stream_failed = Arc::new(std::sync::atomic::AtomicBool::new(false));

        let open = |cfg: &MachineAudioConfig| {
            open_stream_inner(
                cfg,
                Arc::clone(&voices_rt),
                Arc::clone(&input_feeds),
                Arc::clone(&master_gain),
                Arc::clone(&output_period),
                Arc::clone(&stream_failed),
                Arc::clone(&output_callbacks),
            )
        };

        // Resilient startup: if the configured device is absent (unplugged since
        // it was saved), fall back to the system default rather than crashing.
        // The device watchdog then raises the banner and offers a restore when it
        // returns.  `desired_config` keeps the operator's original choice.
        let (sr, started_in_fallback) = match open(config) {
            Ok(sr) => (sr, false),
            Err(e) if config.device_id.is_some() => {
                log::warn!(
                    "Configured audio device unavailable at startup ({e}); \
                     falling back to the system default"
                );
                let fallback = MachineAudioConfig { device_id: None, ..config.clone() };
                (open(&fallback)?, true)
            }
            Err(e) => return Err(e),
        };

        let engine = Arc::new(Self {
            device_manager: Mutex::new(DeviceManager::new()),
            cmd_prod: Mutex::new(sr.cmd_prod),
            status_cons: Mutex::new(sr.status_cons),
            voices,
            aux_streams: Mutex::new(Vec::new()),
            input_feeds,
            _stream: Mutex::new(Some(sr.stream)),
            sample_rate: std::sync::atomic::AtomicU32::new(sr.sample_rate),
            output_channels: std::sync::atomic::AtomicU32::new(sr.channels),
            default_out_offset: std::sync::atomic::AtomicU32::new(sr.default_out_offset),
            master_gain,
            output_period,
            output_callbacks,
            stream_failed: Mutex::new(stream_failed),
            // `desired_config` keeps the operator's choice even when we started on
            // the fallback, so the watchdog can offer a restore when it returns.
            desired_config: Mutex::new(config.clone()),
            current_device_id: Mutex::new(
                if started_in_fallback { None } else { config.device_id.clone() },
            ),
            in_fallback: std::sync::atomic::AtomicBool::new(started_in_fallback),
        });

        // A broken configured device (HDMI with no display, unplugged interface)
        // is now handled continuously by the device watchdog (lib.rs), which
        // falls back to the default device and surfaces a banner — no one-shot
        // startup watchdog needed.

        // Warm the device cache off the main thread.  `DeviceManager::new()` no
        // longer enumerates (that blocked app startup on a slow/hung Windows
        // audio driver, cpal #867); a one-shot bounded refresh fills it here
        // without stalling `.setup()`.
        {
            let engine_bg = Arc::clone(&engine);
            let _ = std::thread::Builder::new()
                .name("inkue-device-warmup".to_string())
                .spawn(move || {
                    let devices = crate::engine::device_manager::run_bounded(
                        crate::engine::device_manager::ENUM_TIMEOUT,
                        crate::engine::device_manager::enumerate_output_devices,
                    )
                    .unwrap_or_default();
                    if let Ok(mut mgr) = engine_bg.device_manager.lock() {
                        mgr.replace_cache(devices);
                    }
                });
        }

        Ok(engine)
    }

    // ── Device resilience ─────────────────────────────────────────────────────

    /// Monotonic output-callback count.  The watchdog flags a dead stream when
    /// this stops advancing — a device-loss signal that does not depend on cpal
    /// surfacing an error of a particular kind.
    pub fn callback_count(&self) -> u64 {
        self.output_callbacks.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Snapshot of the output device's current health for the watchdog.
    pub fn audio_health(&self) -> AudioHealth {
        let failed = self
            .stream_failed
            .lock()
            .map(|f| f.load(std::sync::atomic::Ordering::Relaxed))
            .unwrap_or(false);
        let in_fallback = self.in_fallback.load(std::sync::atomic::Ordering::Relaxed);
        let (desired_id, desired_label) = self
            .desired_config
            .lock()
            .ok()
            .map(|c| {
                // Presence is checked by id; the banner shows the friendly name
                // (falling back to the id for devices saved before names existed).
                (c.device_id.clone(), c.device_name.clone().or_else(|| c.device_id.clone()))
            })
            .unwrap_or((None, None));
        // Only enumerate devices while in fallback — that is the sole moment we
        // need to detect the desired device's return.  In the healthy steady
        // state the watchdog stays free (just an atomic read of `failed`).
        let desired_present = match (&in_fallback, &desired_id) {
            (true, Some(id)) => {
                // Enumerate off the manager lock (bounded) so a slow WASAPI query
                // never stalls a concurrent main-thread reader of the cache.
                let devices = crate::engine::device_manager::run_bounded(
                    crate::engine::device_manager::ENUM_TIMEOUT,
                    crate::engine::device_manager::enumerate_output_devices,
                )
                .unwrap_or_default();
                let present = devices.iter().any(|d| &d.id == id);
                if let Ok(mut mgr) = self.device_manager.lock() {
                    mgr.replace_cache(devices);
                }
                present
            }
            _ => true,
        };
        AudioHealth { failed, in_fallback, desired_device: desired_label, desired_present }
    }

    /// Open `config` as the operator's chosen device (an explicit settings change).
    /// Records it as the desired device and clears any fallback state.
    pub fn apply_user_config(&self, config: &MachineAudioConfig) -> Result<()> {
        if let Ok(mut d) = self.desired_config.lock() {
            *d = config.clone();
        }
        self.in_fallback.store(false, std::sync::atomic::Ordering::Relaxed);
        self.restart(config)
    }

    /// Automatically fall back to the system default after the desired device was
    /// lost, keeping the show audible.  Returns the lost device id for the banner.
    pub fn fall_back_to_default(&self) -> Option<String> {
        let desired = self.desired_config.lock().ok().map(|c| c.clone())?;
        let lost = desired.device_id.clone();
        let fallback = MachineAudioConfig { device_id: None, ..desired };
        self.in_fallback.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Err(e) = self.restart(&fallback) {
            log::error!("Audio fallback restart failed: {e}");
        }
        lost
    }

    /// Re-open the operator's chosen device after it returned (manual restore).
    pub fn restore_desired(&self) -> Result<()> {
        let desired = self
            .desired_config
            .lock()
            .map(|c| c.clone())
            .map_err(|_| anyhow!("desired_config poisoned"))?;
        self.in_fallback.store(false, std::sync::atomic::Ordering::Relaxed);
        self.restart(&desired)
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

    /// Shift an unpatched voice's L/R outputs to the default output offset
    /// (the selected ASIO pair).  Patched voices route exactly where their
    /// Output Patch says.
    fn apply_default_offset(&self, voice: &mut Voice) {
        if voice.patched {
            return;
        }
        let offset = self.default_out_offset.load(std::sync::atomic::Ordering::Relaxed) as usize;
        voice.out_l += offset;
        voice.out_r += offset;
    }

    /// Add a pre-decoded voice to the pool and issue a Play command.
    pub fn play_voice(&self, mut voice: Voice) -> Result<VoiceId> {
        self.apply_default_offset(&mut voice);
        self.check_channel_bounds(&voice, self.output_channels());
        let id = voice.id;
        let arc = Arc::new(voice);
        arc.set_playing();

        self.voices.push(Arc::clone(&arc)).map_err(|_| anyhow!("voices mutex poisoned"))?;

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
    pub fn play_voice_paused(&self, mut voice: Voice) -> Result<VoiceId> {
        self.apply_default_offset(&mut voice);
        self.check_channel_bounds(&voice, self.output_channels());
        let id = voice.id;
        let arc = Arc::new(voice);
        arc.set_paused();

        self.voices.push(Arc::clone(&arc)).map_err(|_| anyhow!("voices mutex poisoned"))?;

        // No Play command — the callback only mixes Playing/FadingOut voices, so
        // this stays silent until resume_voice() is called.
        Ok(id)
    }

    // ── Output Patch device routing ───────────────────────────────────────────

    /// `true` when `device_id` should play on the main stream: no device
    /// requested, or it is the device the main stream is already open on
    /// (including when "main" is the system default and the patch names that
    /// same device explicitly — no duplicate stream on one device).
    fn routes_to_main(&self, device_id: Option<&str>) -> bool {
        let Some(dev) = device_id.filter(|d| !d.is_empty()) else { return true };
        let main = self.current_device_id.lock().ok().and_then(|c| c.clone());
        let default_dev = match main {
            Some(_) => None,
            None => self
                .device_manager
                .lock()
                .ok()
                .and_then(|m| m.default_device().map(|d| d.id.clone())),
        };
        resolves_to_main_device(dev, main.as_deref(), default_dev.as_deref())
    }

    /// Play `voice` on the device an Output Patch routes to.
    ///
    /// `None` / empty / the main device → the normal main-stream path.  Any
    /// other device → a dedicated aux stream (opened on first use).  If the
    /// device cannot be opened the voice falls back to the main stream — a
    /// mis-patched cue must stay audible in a show — and a health alert tells
    /// the operator.
    pub fn play_voice_routed(&self, voice: Voice, device_id: Option<&str>) -> Result<VoiceId> {
        if self.routes_to_main(device_id) {
            // Routing is healthy again — retire any stale device alert so the
            // banner always matches the latest GO.
            crate::health::clear("output-patch-device");
            return self.play_voice(voice);
        }
        // routes_to_main returned false, so device_id is Some(non-empty).
        let dev = device_id.unwrap_or_default();
        match self.submit_to_aux(voice, dev, true) {
            Ok(id) => Ok(id),
            Err((voice, e)) => {
                log::warn!("[audio] patch device '{dev}' unavailable ({e}) — playing on main output");
                crate::health::set(crate::health::HealthAlert::new(
                    "output-patch-device",
                    crate::health::HealthLevel::Warning,
                    format!("Patch output device unavailable — audio routed to the main output ({e})"),
                ));
                self.play_voice(voice)
            }
        }
    }

    /// [`Self::play_voice_paused`] with Output Patch device routing.
    /// Used for a video's audio track when its patch targets another device.
    pub fn play_voice_paused_routed(&self, voice: Voice, device_id: Option<&str>) -> Result<VoiceId> {
        if self.routes_to_main(device_id) {
            crate::health::clear("output-patch-device");
            return self.play_voice_paused(voice);
        }
        let dev = device_id.unwrap_or_default();
        match self.submit_to_aux(voice, dev, false) {
            Ok(id) => Ok(id),
            Err((voice, e)) => {
                log::warn!("[audio] patch device '{dev}' unavailable ({e}) — playing on main output");
                crate::health::set(crate::health::HealthAlert::new(
                    "output-patch-device",
                    crate::health::HealthLevel::Warning,
                    format!("Patch output device unavailable — audio routed to the main output ({e})"),
                ));
                self.play_voice_paused(voice)
            }
        }
    }

    /// Push `voice` into the aux stream for `device_id` (opening it if
    /// needed).  `start` issues the Play command; `false` submits paused.
    /// On failure the voice is handed back so the caller can fall back to
    /// the main stream.
    fn submit_to_aux(
        &self,
        voice: Voice,
        device_id: &str,
        start: bool,
    ) -> std::result::Result<VoiceId, (Voice, anyhow::Error)> {
        let mut aux = match self.aux_streams.lock() {
            Ok(g) => g,
            Err(_) => return Err((voice, anyhow!("aux_streams mutex poisoned"))),
        };

        // Drop a dead stream for this device so it is reopened fresh below.
        aux.retain(|s| {
            !(s.device_id == device_id && s.failed.load(std::sync::atomic::Ordering::Relaxed))
        });

        if !aux.iter().any(|s| s.device_id == device_id) {
            let buffer_size = self
                .desired_config
                .lock()
                .map(|c| c.buffer_size)
                .unwrap_or(256);
            match open_aux_stream(device_id, buffer_size, Arc::clone(&self.master_gain)) {
                Ok(s) => {
                    log::info!("[audio] aux output stream opened on '{device_id}'");
                    crate::health::clear("output-patch-device");
                    aux.push(s);
                }
                Err(e) => return Err((voice, e)),
            }
        }

        // Both branches above guarantee the stream exists here.
        let Some(stream) = aux.iter_mut().find(|s| s.device_id == device_id) else {
            return Err((voice, anyhow!("aux stream vanished")));
        };

        self.check_channel_bounds(&voice, stream.channels);
        let id = voice.id;
        let arc = Arc::new(voice);
        if start { arc.set_playing(); } else { arc.set_paused(); }
        if let Err(arc) = stream.voices.push(arc) {
            // Poisoned aux pool — recover the (still sole-owned) Voice so the
            // caller can fall the cue back to the main output.
            let voice = Arc::try_unwrap(arc)
                .unwrap_or_else(|_| unreachable!("push failure keeps the only ref"));
            return Err((voice, anyhow!("aux voice pool poisoned")));
        }
        if start {
            let _ = stream.cmd_prod.try_push(AudioCommand::Play { voice_id: id });
        }
        Ok(id)
    }

    /// Send `cmd` to the main stream and every aux stream.  Streams that do
    /// not own the referenced voice ignore the command, so broadcasting keeps
    /// the public API device-agnostic.
    fn broadcast_command(&self, cmd: AudioCommand) -> Result<()> {
        let main = self.send_command(cmd.clone());
        if let Ok(mut aux) = self.aux_streams.lock() {
            for s in aux.iter_mut() {
                let _ = s.cmd_prod.try_push(cmd.clone());
            }
        }
        main
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

    /// Register a synthetic input feed not backed by any capture device, and
    /// return its id plus the ring producer to push samples into.
    ///
    /// Used by the Timecode Cue's LTC generator: a background thread fills the
    /// producer with encoded LTC audio, and a live voice reading this feed routes
    /// it to an Output Patch through the normal [`Self::play_mic_voice`] path.
    /// The feed is released by [`Self::gc_voices`] once its live voice stops.
    pub fn register_synthetic_feed(
        &self,
        channels: usize,
        sample_rate: u32,
    ) -> Result<(Uuid, ringbuf::HeapProd<f32>)> {
        let channels = channels.max(1);
        let (prod, cons) = HeapRb::<f32>::new(STAGING_FRAMES * channels).split();
        let id = Uuid::new_v4();
        let feed = InputFeed {
            id,
            device_id: format!("synthetic:{id}"),
            in_channels: channels,
            sample_rate,
            cons,
            staging: vec![0.0_f32; STAGING_FRAMES * channels].into_boxed_slice(),
            write_frame: 0,
            _capture: None,
        };
        self.input_feeds
            .lock()
            .map_err(|_| anyhow!("input_feeds poisoned"))?
            .push(feed);
        Ok((id, prod))
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
        // Mic routing is always explicit (Input Patch → Output Patch channels);
        // never shift it by the default ASIO pair offset.
        voice.patched = true;
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
        self.voices.push(Arc::clone(&arc)).map_err(|_| anyhow!("voices mutex poisoned"))?;
        self.send_command(AudioCommand::Play { voice_id: id })?;
        Ok(id)
    }

    pub fn stop_voice(&self, voice_id: VoiceId, fade_ms: u32, fade_curve: FadeCurve) -> Result<()> {
        self.broadcast_command(AudioCommand::Stop { voice_id, fade_ms, fade_curve })
    }

    /// Panic: silence every voice immediately, bypassing per-voice IDs.
    ///
    /// One ring-buffer command per stream stops its whole pool inside a
    /// single RT callback, so it works even for voices whose owning cue lost
    /// track of them (desynced bookkeeping) and cannot overflow the command
    /// ring the way N individual `Stop`s could.  Broadcast to every aux
    /// stream so patch-routed voices are silenced too.
    pub fn panic_stop_all(&self) -> Result<()> {
        self.broadcast_command(AudioCommand::StopAll)
    }

    pub fn pause_voice(&self, voice_id: VoiceId) -> Result<()> {
        self.send_command(AudioCommand::Pause { voice_id })
    }

    pub fn resume_voice(&self, voice_id: VoiceId) -> Result<()> {
        self.broadcast_command(AudioCommand::Resume { voice_id })
    }

    pub fn set_voice_gain(&self, voice_id: VoiceId, gain: f32) -> Result<()> {
        self.broadcast_command(AudioCommand::SetGain { voice_id, gain })
    }

    pub fn set_voice_pan(&self, voice_id: VoiceId, pan: f32) -> Result<()> {
        self.broadcast_command(AudioCommand::SetPan { voice_id, pan })
    }

    /// Seek a voice to the given decoded-audio frame position.
    pub fn seek_voice(&self, voice_id: VoiceId, frame_pos: u64) -> Result<()> {
        self.broadcast_command(AudioCommand::Seek { voice_id, frame_pos })
    }

    /// Live mixer fader: set the patch-gain multiplier on every playing voice
    /// routed through `patch_id` (all streams).  New voices pick the gain up
    /// from the patch table at GO.
    pub fn set_patch_gain(&self, patch_id: Uuid, gain: f32) -> Result<()> {
        self.broadcast_command(AudioCommand::SetPatchGain { patch_id, gain })
    }

    /// Close every aux stream and clear routing alerts.
    ///
    /// Called on any event that changes the routing universe — main device /
    /// backend change, Output Patch edits — so no stream keeps playing on an
    /// output the operator just re-configured away from, and no stale banner
    /// contradicts the new reality.  The next GO reopens what it needs.
    pub fn close_all_aux(&self) {
        if let Ok(mut aux) = self.aux_streams.lock() {
            if !aux.is_empty() {
                log::info!("[audio] closing {} aux output stream(s) (routing changed)", aux.len());
            }
            aux.clear();
        }
        crate::health::clear("output-patch-device");
        crate::health::clear("output-patch-channels");
    }

    /// Raise or clear the "patch routes to a channel the device does not
    /// have" alert for one submitted voice.  Reflects the **latest** GO, so
    /// the banner always matches what the operator just heard (or didn't).
    fn check_channel_bounds(&self, voice: &Voice, device_channels: u32) {
        let highest = voice.out_l.max(voice.out_r);
        if highest >= device_channels as usize {
            crate::health::set(crate::health::HealthAlert::new(
                "output-patch-channels",
                crate::health::HealthLevel::Warning,
                format!(
                    "Cue routed to output channel {} but the device has only {} — audio is dropped. Fix the Output Patch channels.",
                    highest + 1,
                    device_channels
                ),
            ));
        } else {
            crate::health::clear("output-patch-channels");
        }
    }

    /// Run `f` on the voice with `voice_id`, searching the main pool then
    /// every aux pool.  Returns `None` when the voice is not found.
    fn with_voice<T>(&self, voice_id: VoiceId, f: impl Fn(&Arc<Voice>) -> T) -> Option<T> {
        let find = |pool: &[Arc<Voice>]| pool.iter().find(|v| v.id == voice_id).map(&f);
        if let Some(Some(t)) = self.voices.with(find) {
            return Some(t);
        }
        let aux = self.aux_streams.lock().ok()?;
        for s in aux.iter() {
            if let Some(Some(t)) = s.voices.with(find) {
                return Some(t);
            }
        }
        None
    }

    /// Read the current linear gain of a voice.
    /// Returns 1.0 if the voice is not found.
    pub fn get_voice_gain(&self, voice_id: VoiceId) -> f32 {
        self.with_voice(voice_id, |v| v.inner.gain()).unwrap_or(1.0)
    }

    /// Seek a voice to the given position in milliseconds.
    ///
    /// Looks up the voice's decoded sample rate to convert `position_ms` into a
    /// frame position, then sends a [`AudioCommand::Seek`] to the RT callback.
    /// Used by [`OutputEngine::seek`] which only knows wall-clock position.
    pub fn seek_voice_ms(&self, voice_id: VoiceId, position_ms: u64) -> Result<()> {
        let frame_pos = self.with_voice(voice_id, |v| position_ms * v.sample_rate as u64 / 1000);
        if let Some(fp) = frame_pos {
            self.broadcast_command(AudioCommand::Seek { voice_id, frame_pos: fp })?;
        }
        Ok(())
    }

    /// Drain the status ring buffers (main + aux) and return all pending
    /// status messages.
    pub fn drain_status(&self) -> Vec<AudioStatus> {
        let mut out = Vec::new();
        if let Ok(mut cons) = self.status_cons.lock() {
            while let Some(s) = cons.try_pop() {
                out.push(s);
            }
        }
        if let Ok(mut aux) = self.aux_streams.lock() {
            for s in aux.iter_mut() {
                while let Some(msg) = s.status_cons.try_pop() {
                    // Aux master-level meters would fight the main stream's on
                    // the VU display; only voice-scoped statuses pass through.
                    if !matches!(msg, AudioStatus::MasterLevels { .. }) {
                        out.push(msg);
                    }
                }
            }
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
        self.voices
            .retain(|v| !matches!(v.voice_state(), VoiceState::Stopped | VoiceState::Idle));

        // Feed ids still referenced by a surviving live voice.
        let in_use: std::collections::HashSet<Uuid> = self
            .voices
            .with(|pool| {
                pool.iter().filter_map(|v| v.live.as_ref().map(|l| l.feed_id)).collect()
            })
            .unwrap_or_default();

        // Drop unreferenced feeds — dropping an InputFeed drops its cpal input
        // stream, releasing the device.
        if let Ok(mut feeds) = self.input_feeds.lock() {
            feeds.retain(|f| in_use.contains(&f.id));
        }

        // Aux pools: same sweep.  A healthy aux stream stays open even when
        // idle (zero device-open latency on the next GO); a failed one is
        // dropped once empty so the next GO retries a fresh open.
        if let Ok(mut aux) = self.aux_streams.lock() {
            for s in aux.iter() {
                s.voices
                    .retain(|v| !matches!(v.voice_state(), VoiceState::Stopped | VoiceState::Idle));
            }
            aux.retain(|s| {
                let empty = s.voices.with(|p| p.is_empty()).unwrap_or(true);
                let failed = s.failed.load(std::sync::atomic::Ordering::Relaxed);
                !(failed && empty)
            });
        }
    }

    /// Stop the current stream and re-open according to `config`.
    ///
    /// Playing voices are **preserved**: the voice pool is shared with the new
    /// stream's callback, so each voice resumes from its current `frame_pos` on
    /// the new device (the source-frame cursor is output-rate-independent, and
    /// channel routing is bounds-checked in `fill_buffer`).  A device switch
    /// therefore only drops audio for the brief teardown/re-open gap instead of
    /// killing the running cue — for both a planned change and an auto-fallback.
    pub fn restart(&self, config: &MachineAudioConfig) -> Result<()> {
        // Drop the old stream before opening the new one (exclusive backends
        // require the device to be released first).  Voices are intentionally
        // left intact so the new stream continues mixing them where they froze.
        {
            let mut sg = self._stream.lock().map_err(|_| anyhow!("stream mutex poisoned"))?;
            *sg = None;
        }

        // Close ALL aux streams: they belong to the previous device universe
        // (a WASAPI aux on the interface ASIO is about to grab exclusively
        // would make this restart fail), and stale ones would keep playing on
        // outputs the operator just re-configured away from.  The next GO
        // through a patch reopens exactly what the new universe needs.
        self.close_all_aux();

        let new_failed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let sr = open_stream_inner(
            config,
            self.voices.rt_handle(),
            Arc::clone(&self.input_feeds),
            Arc::clone(&self.master_gain),
            Arc::clone(&self.output_period),
            Arc::clone(&new_failed),
            Arc::clone(&self.output_callbacks),
        )?;

        *self.cmd_prod.lock().map_err(|_| anyhow!("cmd_prod poisoned"))? = sr.cmd_prod;
        *self.status_cons.lock().map_err(|_| anyhow!("status_cons poisoned"))? = sr.status_cons;
        *self._stream.lock().map_err(|_| anyhow!("stream poisoned"))? = Some(sr.stream);
        self.sample_rate.store(sr.sample_rate, std::sync::atomic::Ordering::Relaxed);
        self.output_channels.store(sr.channels, std::sync::atomic::Ordering::Relaxed);
        self.default_out_offset.store(sr.default_out_offset, std::sync::atomic::Ordering::Relaxed);
        if let Ok(mut f) = self.stream_failed.lock() { *f = new_failed; }
        if let Ok(mut d) = self.current_device_id.lock() { *d = config.device_id.clone(); }

        // The device set does not change on a restart (same hardware, different
        // selection), so we do not re-enumerate here — that would add a slow,
        // possibly hanging WASAPI query to this main-thread path.  The cache is
        // kept fresh by the startup warm-up and every Preferences enumeration.

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
    /// Default output-channel offset for unpatched voices (the selected ASIO
    /// pair × 2; always 0 on non-ASIO backends).
    default_out_offset: u32,
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
    cb_voices: Arc<ArcSwap<Vec<Arc<Voice>>>>,
    cb_feeds: Arc<Mutex<Vec<InputFeed>>>,
    cb_mg: Arc<std::sync::atomic::AtomicU32>,
    cb_period: Arc<std::sync::atomic::AtomicU32>,
    stream_failed: Arc<std::sync::atomic::AtomicBool>,
    cb_callbacks: Arc<std::sync::atomic::AtomicU64>,
) -> Result<StreamResult> {
    use crate::preferences::AudioBackend;

    let host = match config.backend {
        AudioBackend::WasapiShared | AudioBackend::WasapiExclusive | AudioBackend::SystemDefault => {
            cpal::default_host()
        }
        AudioBackend::Asio => {
            // Fall back to the default host when ASIO is not compiled in
            // (e.g. `pnpm tauri dev` without `--features asio-support`).
            // This lets developers iterate without ASIO while keeping their
            // saved machine config pointing at ASIO for production builds.
            match open_asio_host() {
                Ok(h) => h,
                Err(e) => {
                    log::warn!("ASIO unavailable ({e}), falling back to default host");
                    cpal::default_host()
                }
            }
        }
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

    // ASIO: default output pair for voices that have no Output Patch.
    // Applied at voice submission (play_voice), not in the callback.
    let pair_offset = if matches!(config.backend, AudioBackend::Asio) {
        (config.asio_out_pair as usize * 2).min(total_ch.saturating_sub(2))
    } else {
        0
    };

    let (cmd_prod, mut cmd_cons) = HeapRb::<AudioCommand>::new(RING_CAPACITY).split();
    let (mut status_prod, status_cons) = HeapRb::<AudioStatus>::new(RING_CAPACITY).split();

    // Throttled error callback: log at most once per second, signal stream_failed
    // on the first DeviceNotAvailable so the watchdog can switch devices quickly.
    let make_err_fn = {
        let last_log  = Arc::new(std::sync::atomic::AtomicU64::new(0));
        move || {
            let last_log2  = Arc::clone(&last_log);
            let failed2    = Arc::clone(&stream_failed);
            move |err: cpal::Error| {
                // Only DeviceNotAvailable is truly fatal (device pulled or
                // exclusively grabbed).  Other kinds cover recoverable ALSA
                // errors (e.g. Xrun, POLLERR) that should not trigger the
                // watchdog restart.
                if matches!(err.kind(), cpal::ErrorKind::DeviceNotAvailable) {
                    failed2.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let prev = last_log2.swap(now, std::sync::atomic::Ordering::Relaxed);
                if now > prev {
                    log::error!("cpal stream error ({:?}): {err}", err.kind());
                }
            }
        }
    };

    // Both formats mix **full-width** (all device channels) so Output Patch
    // channel routing reaches every physical output — essential on ASIO where
    // one driver exposes all of the interface's outs.  Unpatched voices are
    // shifted to `pair_offset` at submission (see `play_voice`), not here.
    let stream = match default_config.sample_format() {
        cpal::SampleFormat::F32 => device.build_output_stream(
            stream_cfg,
            move |data: &mut [f32], _| {
                cb_callbacks.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                cb_period.store((data.len() / total_ch) as u32, std::sync::atomic::Ordering::Relaxed);
                fill_buffer(data, total_ch, sample_rate, &cb_voices, &cb_feeds, &mut cmd_cons, &mut status_prod, &cb_mg, &cb_period);
            },
            make_err_fn(),
            None,
        )?,
        cpal::SampleFormat::I32 => {
            // Pre-allocate a full-width f32 scratch — no alloc inside the RT
            // callback (4096 frames covers every period cpal will request).
            let scratch_len = (config.buffer_size as usize).max(4096) * total_ch;
            let mut scratch = vec![0.0f32; scratch_len].into_boxed_slice();
            device.build_output_stream(
                stream_cfg,
                move |data: &mut [i32], _| {
                    cb_callbacks.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let frames = data.len() / total_ch;
                    cb_period.store(frames as u32, std::sync::atomic::Ordering::Relaxed);
                    let n = (frames * total_ch).min(scratch.len());
                    fill_buffer(&mut scratch[..n], total_ch, sample_rate, &cb_voices, &cb_feeds, &mut cmd_cons, &mut status_prod, &cb_mg, &cb_period);
                    for (dst, src) in data.iter_mut().zip(scratch[..n].iter()) {
                        *dst = (src.clamp(-1.0, 1.0) * i32::MAX as f32) as i32;
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

    Ok(StreamResult {
        stream,
        sample_rate,
        channels: channels as u32,
        default_out_offset: pair_offset as u32,
        cmd_prod,
        status_cons,
    })
}

/// Pure routing decision: does an explicitly-requested patch device match the
/// device the main stream is open on?  `main` is the main stream's device id
/// (`None` = system default, compared via `default_dev`).
fn resolves_to_main_device(requested: &str, main: Option<&str>, default_dev: Option<&str>) -> bool {
    match main {
        Some(cur) => cur == requested,
        None => default_dev == Some(requested),
    }
}

/// Open an additional output stream on `device_id` for Output Patch routing.
///
/// Reuses [`open_stream_inner`] with a synthetic config: generic default host
/// (WASAPI shared on Windows, CoreAudio on macOS, ALSA/PipeWire on Linux — no
/// ASIO, which is exclusive and single-device by design), fresh voice pool,
/// permanently empty input-feed list (live voices stay on the main stream),
/// and the shared master gain so the master fader applies everywhere.
fn open_aux_stream(
    device_id: &str,
    buffer_size: u32,
    master_gain: Arc<std::sync::atomic::AtomicU32>,
) -> Result<AuxStream> {
    use crate::preferences::AudioBackend;

    // WasapiShared on Windows selects BufferSize::Default (WASAPI shared mode
    // owns its engine period); SystemDefault elsewhere applies the configured
    // buffer like the main stream does on CoreAudio/ALSA.
    #[cfg(windows)]
    let backend = AudioBackend::WasapiShared;
    #[cfg(not(windows))]
    let backend = AudioBackend::SystemDefault;

    let config = MachineAudioConfig {
        backend,
        device_id: Some(device_id.to_string()),
        device_name: None,
        input_device_id: None,
        buffer_size,
        asio_out_pair: 0,
    };

    let voices = VoicePool::new();
    let feeds: Arc<Mutex<Vec<InputFeed>>> = Arc::new(Mutex::new(Vec::new()));
    let failed = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let sr = open_stream_inner(
        &config,
        voices.rt_handle(),
        feeds,
        master_gain,
        Arc::new(std::sync::atomic::AtomicU32::new(256)),
        Arc::clone(&failed),
        Arc::new(std::sync::atomic::AtomicU64::new(0)),
    )?;

    Ok(AuxStream {
        device_id: device_id.to_string(),
        channels: sr.channels,
        voices,
        cmd_prod: sr.cmd_prod,
        status_cons: sr.status_cons,
        failed,
        _stream: sr.stream,
    })
}

// ---------------------------------------------------------------------------
// Audio callback — real-time safe
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn fill_buffer(
    output: &mut [f32],
    channels: usize,
    output_sample_rate: u32,
    voices: &ArcSwap<Vec<Arc<Voice>>>,
    input_feeds: &Arc<Mutex<Vec<InputFeed>>>,
    cmd_cons: &mut ringbuf::HeapCons<AudioCommand>,
    status_prod: &mut ringbuf::HeapProd<AudioStatus>,
    master_gain: &Arc<std::sync::atomic::AtomicU32>,
    output_period: &Arc<std::sync::atomic::AtomicU32>,
) {
    output.fill(0.0);

    // Wait-free snapshot of the voice list — no lock, so the callback can never
    // be starved into emitting a block of silence (the old `try_lock` failure
    // mode).  The snapshot's producer (VoicePool) retains recent generations so
    // dropping this guard never runs a destructor on the RT thread.
    let voices_guard = voices.load();

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
    // Per-Output-Patch peaks, indexed by Voice::patch_slot (fixed-size — no
    // allocation in the RT callback).
    let mut patch_peaks = [0.0_f32; PATCH_VU_SLOTS * 2];

    for voice in voices_guard.iter() {
        let state = voice.voice_state();
        if state != VoiceState::Playing && state != VoiceState::FadingOut {
            continue;
        }

        let mut voice_peak_l = 0.0_f32;
        let mut voice_peak_r = 0.0_f32;

        // Live (Mic Cue) voice — resample from its input feed instead of samples.
        if voice.live.is_some() {
            if let Some(feeds) = feeds_guard.as_deref() {
                mix_live(output, channels, output_sample_rate, voice, feeds, status_prod, &mut voice_peak_l, &mut voice_peak_r, output_period);
            }
            accumulate_peaks(voice, voice_peak_l, voice_peak_r, &mut peak_l, &mut peak_r, &mut patch_peaks);
            continue;
        }

        let patch_gain = voice.inner.patch_gain();
        let (gain_l, gain_r) = voice.pan_gains();
        let (gain_l, gain_r) = (gain_l * patch_gain, gain_r * patch_gain);
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

            voice_peak_l = voice_peak_l.max(out_l.abs());
            voice_peak_r = voice_peak_r.max(out_r.abs());

            frame_pos_f += rate;
        }

        // Store integer floor; sub-frame precision is re-established each callback.
        voice.frame_pos.store(frame_pos_f as u64, std::sync::atomic::Ordering::Relaxed);

        accumulate_peaks(voice, voice_peak_l, voice_peak_r, &mut peak_l, &mut peak_r, &mut patch_peaks);
    }

    // Apply master gain.
    for s in output.iter_mut() { *s *= master; }

    let _ = status_prod.try_push(AudioStatus::MasterLevels {
        peak_l: peak_l * master,
        peak_r: peak_r * master,
    });
    for slot in 0..PATCH_VU_SLOTS {
        let (l, r) = (patch_peaks[slot * 2], patch_peaks[slot * 2 + 1]);
        if l > 0.0 || r > 0.0 {
            let _ = status_prod.try_push(AudioStatus::PatchLevels {
                slot: slot as u8,
                peak_l: l * master,
                peak_r: r * master,
            });
        }
    }
}

/// Number of Output Patches metered by the per-patch VU (fixed so the RT
/// callback can accumulate into a stack array — patches beyond this many are
/// simply unmetered in the mixer).
pub const PATCH_VU_SLOTS: usize = 16;

/// Fold one voice's block peaks into the master meters and, when the voice is
/// routed through a metered Output Patch, into that patch's VU slot.
fn accumulate_peaks(
    voice: &Arc<Voice>,
    voice_peak_l: f32,
    voice_peak_r: f32,
    peak_l: &mut f32,
    peak_r: &mut f32,
    patch_peaks: &mut [f32; PATCH_VU_SLOTS * 2],
) {
    *peak_l = (*peak_l).max(voice_peak_l);
    *peak_r = (*peak_r).max(voice_peak_r);
    if let Some(slot) = voice.patch_slot {
        let slot = slot as usize;
        if slot < PATCH_VU_SLOTS {
            patch_peaks[slot * 2] = patch_peaks[slot * 2].max(voice_peak_l);
            patch_peaks[slot * 2 + 1] = patch_peaks[slot * 2 + 1].max(voice_peak_r);
        }
    }
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

    let patch_gain = voice.inner.patch_gain();
    let (gain_l, gain_r) = voice.pan_gains();
    let (gain_l, gain_r) = (gain_l * patch_gain, gain_r * patch_gain);
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
        AudioCommand::SetPatchGain { patch_id, gain } => {
            for v in voices.iter().filter(|v| v.patch_id == Some(patch_id)) {
                v.inner.set_patch_gain(gain);
            }
        }
        AudioCommand::StopAll => {
            for v in voices {
                v.set_stopped();
            }
        }
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

    /// An RT voice-list handle (the ArcSwap the callback reads) holding `voices`.
    fn rt_pool(voices: Vec<Arc<Voice>>) -> Arc<ArcSwap<Vec<Arc<Voice>>>> {
        Arc::new(ArcSwap::from_pointee(voices))
    }

    /// Call fill_buffer for `output_frames` output frames and return the
    /// resulting frame_pos stored in the voice.
    fn run_fill(voice: Arc<Voice>, output_frames: usize, output_sr: u32) -> u64 {
        let pool = rt_pool(vec![Arc::clone(&voice)]);
        let feeds: Arc<Mutex<Vec<InputFeed>>> = Arc::new(Mutex::new(Vec::new()));
        let (_, mut cmd_cons) = HeapRb::<AudioCommand>::new(16).split();
        let (mut status_prod, _) = HeapRb::<AudioStatus>::new(16).split();
        let master  = Arc::new(std::sync::atomic::AtomicU32::new(f32::to_bits(1.0)));
        let period  = Arc::new(std::sync::atomic::AtomicU32::new(output_frames as u32));
        let mut output = vec![0.0f32; output_frames * 2];
        fill_buffer(&mut output, 2, output_sr, &pool, &feeds, &mut cmd_cons, &mut status_prod, &master, &period);
        voice.frame_pos.load(std::sync::atomic::Ordering::Relaxed)
    }

    // The SR ratio (e.g. 44100/48000) is not exactly representable in f64, so
    // frame_pos after N output frames may be off by ±1 source frame.  All
    // assertions below allow a tolerance of 1 frame.
    fn assert_frame_pos(actual: u64, expected: u64, msg: &str) {
        let diff = (actual as i64 - expected as i64).unsigned_abs();
        assert!(diff <= 1, "{msg}: expected {expected} ± 1, got {actual}");
    }

    #[test]
    fn routing_no_device_goes_to_main() {
        // Decision covered by routes_to_main's early return; the pure helper
        // only sees explicitly-requested devices.
        assert!(resolves_to_main_device("dev-a", Some("dev-a"), None));
    }

    #[test]
    fn routing_matching_main_device() {
        assert!(resolves_to_main_device("focusrite", Some("focusrite"), None));
        assert!(!resolves_to_main_device("hdmi-out", Some("focusrite"), None));
    }

    #[test]
    fn routing_matching_system_default() {
        // Main stream on the system default: a patch naming that same device
        // explicitly must not open a duplicate aux stream.
        assert!(resolves_to_main_device("speakers", None, Some("speakers")));
        assert!(!resolves_to_main_device("hdmi-out", None, Some("speakers")));
        // Default device unknown — be conservative and open the aux stream.
        assert!(!resolves_to_main_device("hdmi-out", None, None));
    }

    #[test]
    fn stop_all_command_silences_every_voice() {
        let v1 = make_voice(1000, 2, 48000, 1.0);
        let v2 = make_voice(1000, 2, 48000, 1.0);
        let pool = rt_pool(vec![Arc::clone(&v1), Arc::clone(&v2)]);
        let feeds: Arc<Mutex<Vec<InputFeed>>> = Arc::new(Mutex::new(Vec::new()));
        let (mut cmd_prod, mut cmd_cons) = HeapRb::<AudioCommand>::new(16).split();
        let (mut status_prod, _) = HeapRb::<AudioStatus>::new(16).split();
        let master = Arc::new(std::sync::atomic::AtomicU32::new(f32::to_bits(1.0)));
        let period = Arc::new(std::sync::atomic::AtomicU32::new(64));
        cmd_prod.try_push(AudioCommand::StopAll).unwrap();

        let mut output = vec![0.0f32; 64 * 2];
        fill_buffer(&mut output, 2, 48000, &pool, &feeds, &mut cmd_cons, &mut status_prod, &master, &period);

        for v in [&v1, &v2] {
            assert_eq!(v.voice_state(), VoiceState::Stopped);
        }
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

    // ── End-of-file / loop / simultaneous / seek (RT completion logic) ──────

    /// A voice whose PCM is a steady 0.5 tone (non-silent) for contrast tests.
    fn make_tone_voice(n_frames: usize, sr: u32) -> Arc<Voice> {
        let samples = Arc::new(vec![0.5f32; n_frames * 2]);
        let v = Voice::new(samples, 2, sr, 1.0, 0.0);
        v.set_playing();
        Arc::new(v)
    }

    /// Run one 256-frame callback over `pool`; return the (drained) statuses.
    fn run_block(pool: &Arc<ArcSwap<Vec<Arc<Voice>>>>, cmd: Option<AudioCommand>) -> Vec<AudioStatus> {
        use ringbuf::traits::Consumer;
        let feeds: Arc<Mutex<Vec<InputFeed>>> = Arc::new(Mutex::new(Vec::new()));
        let (mut cmd_prod, mut cmd_cons) = HeapRb::<AudioCommand>::new(16).split();
        if let Some(c) = cmd {
            cmd_prod.try_push(c).unwrap();
        }
        let (mut status_prod, mut status_cons) = HeapRb::<AudioStatus>::new(64).split();
        let master = Arc::new(std::sync::atomic::AtomicU32::new(f32::to_bits(1.0)));
        let period = Arc::new(std::sync::atomic::AtomicU32::new(256));
        let mut out = vec![0.0f32; 256 * 2];
        fill_buffer(&mut out, 2, 48_000, pool, &feeds, &mut cmd_cons, &mut status_prod, &master, &period);
        let mut statuses = Vec::new();
        while let Some(s) = status_cons.try_pop() {
            statuses.push(s);
        }
        statuses
    }

    #[test]
    fn eof_stops_voice_and_emits_completed() {
        // 100 source frames, 256-frame block at matched SR → runs past the end.
        let voice = make_voice(100, 2, 48_000, 1.0);
        let id = voice.id;
        let pool = rt_pool(vec![Arc::clone(&voice)]);
        let statuses = run_block(&pool, None);

        assert_eq!(voice.voice_state(), VoiceState::Stopped, "voice must stop at EOF");
        assert!(
            statuses.iter().any(|s| matches!(s, AudioStatus::Completed { voice_id } if *voice_id == id)),
            "EOF must emit exactly one Completed for the voice"
        );
    }

    #[test]
    fn finite_loop_wraps_and_decrements_without_stopping() {
        // 200 frames, loop once (loops_remaining=1); 256-frame block wraps once
        // (dec to 0) then keeps playing from the top — must NOT stop yet.
        let voice = make_voice(200, 2, 48_000, 1.0);
        voice.inner.loops_remaining.store(1, std::sync::atomic::Ordering::Relaxed);
        let pool = rt_pool(vec![Arc::clone(&voice)]);
        let statuses = run_block(&pool, None);

        assert_eq!(
            voice.inner.loops_remaining.load(std::sync::atomic::Ordering::Relaxed),
            0,
            "one wrap must decrement the finite loop counter"
        );
        assert_eq!(voice.voice_state(), VoiceState::Playing, "still playing mid-loop");
        assert!(
            !statuses.iter().any(|s| matches!(s, AudioStatus::Completed { .. })),
            "a wrap must not emit Completed"
        );
        let pos = voice.frame_pos.load(std::sync::atomic::Ordering::Relaxed);
        assert!(pos < 100, "after wrapping 200-frame file within 256 frames, pos≈56, got {pos}");
    }

    #[test]
    fn infinite_loop_never_completes() {
        // loops_remaining = u32::MAX; play 4000 frames (20× past a 200-frame file).
        let voice = make_voice(200, 2, 48_000, 1.0);
        voice.inner.loops_remaining.store(u32::MAX, std::sync::atomic::Ordering::Relaxed);
        let pool = rt_pool(vec![Arc::clone(&voice)]);
        for _ in 0..16 {
            let statuses = run_block(&pool, None);
            assert!(
                !statuses.iter().any(|s| matches!(s, AudioStatus::Completed { .. })),
                "infinite loop must never emit Completed"
            );
        }
        assert_eq!(
            voice.inner.loops_remaining.load(std::sync::atomic::Ordering::Relaxed),
            u32::MAX,
            "infinite loop counter must not decrement"
        );
        assert_eq!(voice.voice_state(), VoiceState::Playing);
    }

    #[test]
    fn simultaneous_voices_all_advance() {
        let v1 = make_voice(100_000, 2, 48_000, 1.0);
        let v2 = make_voice(100_000, 2, 48_000, 1.0);
        let pool = rt_pool(vec![Arc::clone(&v1), Arc::clone(&v2)]);
        run_block(&pool, None);
        for v in [&v1, &v2] {
            let pos = v.frame_pos.load(std::sync::atomic::Ordering::Relaxed);
            assert!((pos as i64 - 256).abs() <= 1, "each simultaneous voice advances ~256, got {pos}");
        }
    }

    #[test]
    fn seek_command_repositions_before_mixing() {
        // Seek is applied before the voice loop, so pos = seek_target + block.
        let voice = make_voice(100_000, 2, 48_000, 1.0);
        let id = voice.id;
        let pool = rt_pool(vec![Arc::clone(&voice)]);
        run_block(&pool, Some(AudioCommand::Seek { voice_id: id, frame_pos: 5_000 }));
        let pos = voice.frame_pos.load(std::sync::atomic::Ordering::Relaxed);
        assert!((pos as i64 - 5_256).abs() <= 1, "seek(5000)+256 frames → ~5256, got {pos}");
    }

    #[test]
    fn voice_pool_publishes_snapshots_and_bounds_retention() {
        // FINDING A1 (fixed): the RT callback reads a wait-free ArcSwap snapshot
        // instead of try_lock'ing the pool.  A publish is immediately visible to
        // the RT handle, and the keep-alive ring stays bounded so retired
        // generations are dropped on the non-RT side (never the RT thread).
        let pool = VoicePool::new();
        assert!(pool.rt.load().is_empty());

        assert!(pool.push(make_tone_voice(1000, 48_000)).is_ok(), "push");
        assert_eq!(pool.rt.load().len(), 1, "a published voice is visible to the RT handle");

        // Several publishes; the retained ring never exceeds its cap.
        for _ in 0..10 {
            assert!(pool.push(make_tone_voice(1000, 48_000)).is_ok(), "push");
        }
        assert!(pool.retained.lock().unwrap().len() <= VOICE_POOL_RETAIN);

        pool.retain(|_| false);
        assert!(pool.rt.load().is_empty(), "retain(none) republishes an empty snapshot");
    }

    #[test]
    fn callback_mixes_the_published_snapshot_with_no_lock() {
        // Replaces the old A1 proof: there is no longer a lock the callback can be
        // starved on, so a freshly published tone always reaches the output —
        // the "block of silence on contention" failure mode is gone.
        let pool = VoicePool::new();
        assert!(pool.push(make_tone_voice(100_000, 48_000)).is_ok(), "push");

        let feeds: Arc<Mutex<Vec<InputFeed>>> = Arc::new(Mutex::new(Vec::new()));
        let (_p, mut cmd_cons) = HeapRb::<AudioCommand>::new(16).split();
        let (mut status_prod, _sc) = HeapRb::<AudioStatus>::new(16).split();
        let master = Arc::new(std::sync::atomic::AtomicU32::new(f32::to_bits(1.0)));
        let period = Arc::new(std::sync::atomic::AtomicU32::new(256));
        let mut out = vec![0.0f32; 256 * 2];
        fill_buffer(&mut out, 2, 48_000, &pool.rt, &feeds, &mut cmd_cons, &mut status_prod, &master, &period);
        assert!(out.iter().any(|s| s.abs() > 0.1), "the published tone must reach the output");
    }
}
