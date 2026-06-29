//! DMX-over-IP lighting engine.
//!
//! Owns one buffer per universe, interpolates timed fades, and streams frames
//! over sACN / Art-Net at a fixed rate.  It knows nothing about cues — the cue
//! layer submits fades through [`DmxEngine::submit_fade`].
//!
//! Lighting is **stateful and tracking**: the universe buffers persist between
//! cues, a cue only touches the channels it changes (the rest keep their value),
//! and the latest fade on a channel wins (LTP).  This mirrors how a lighting
//! console / QLab behaves, and is why a Light Cue stores only deltas.
//!
//! ## Threading
//!
//! [`DmxState`] is pure (no sockets, no thread) so the fade maths and LTP/tracking
//! behaviour are unit-tested directly.  [`DmxEngine`] wraps it in a `inkue-dmx`
//! thread that ticks at [`REFRESH_HZ`], publishes a snapshot for the UI monitor,
//! and transmits each enabled universe (send-on-change + periodic keepalive).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{Sender, TryRecvError};
use serde::{Deserialize, Serialize};

use super::dmx_sink::{DmxSink, UniverseOutput, DMX_UNIVERSE_SIZE};
use super::ring_command::FadeCurve;

/// Output frame rate (Hz). DMX hardware refreshes at ~44 Hz; 40 is plenty.
const REFRESH_HZ: u64 = 40;
/// Re-send an unchanged universe at least this often (receivers may time out).
const KEEPALIVE: Duration = Duration::from_millis(800);

/// Resolution of one fixture parameter on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelWidth {
    /// One DMX channel (0–255).
    Bit8,
    /// Two adjacent channels, coarse then fine (0–65535).
    Bit16,
}

/// Convert a normalised value `[0, 1]` to its coarse (+ optional fine) DMX byte.
fn norm_to_bytes(norm: f64, width: ChannelWidth) -> (u8, Option<u8>) {
    let n = norm.clamp(0.0, 1.0);
    match width {
        ChannelWidth::Bit8 => ((n * 255.0).round() as u8, None),
        ChannelWidth::Bit16 => {
            let v = (n * 65535.0).round() as u16;
            ((v >> 8) as u8, Some((v & 0xff) as u8))
        }
    }
}

/// A single in-progress fade on one channel.
struct ChannelFade {
    width: ChannelWidth,
    start_norm: f64,
    target_norm: f64,
    t0: Instant,
    dur: Duration,
    curve: FadeCurve,
}

/// Pure DMX render state: universe buffers + active fades + blackout.
pub struct DmxState {
    universes: HashMap<u16, [u8; DMX_UNIVERSE_SIZE]>,
    /// Keyed by `(universe, coarse_channel)` so a new fade on a channel replaces
    /// the previous one (LTP).
    fades: HashMap<(u16, u16), ChannelFade>,
    blackout: bool,
}

impl Default for DmxState {
    fn default() -> Self {
        Self::new()
    }
}

impl DmxState {
    pub fn new() -> Self {
        Self { universes: HashMap::new(), fades: HashMap::new(), blackout: false }
    }

    fn buffer_mut(&mut self, universe: u16) -> &mut [u8; DMX_UNIVERSE_SIZE] {
        self.universes.entry(universe).or_insert([0; DMX_UNIVERSE_SIZE])
    }

    /// Current normalised value of a channel (0.0 if the universe is untouched).
    pub fn read_norm(&self, universe: u16, channel: u16, width: ChannelWidth) -> f64 {
        let Some(buf) = self.universes.get(&universe) else { return 0.0 };
        let i = channel as usize;
        match width {
            ChannelWidth::Bit8 => *buf.get(i).unwrap_or(&0) as f64 / 255.0,
            ChannelWidth::Bit16 => {
                let hi = *buf.get(i).unwrap_or(&0) as u16;
                let lo = *buf.get(i + 1).unwrap_or(&0) as u16;
                ((hi << 8) | lo) as f64 / 65535.0
            }
        }
    }

    fn write_norm(&mut self, universe: u16, channel: u16, width: ChannelWidth, norm: f64) {
        let (hi, lo) = norm_to_bytes(norm, width);
        let i = channel as usize;
        let buf = self.buffer_mut(universe);
        if i < DMX_UNIVERSE_SIZE {
            buf[i] = hi;
        }
        if let Some(lo) = lo {
            if i + 1 < DMX_UNIVERSE_SIZE {
                buf[i + 1] = lo;
            }
        }
    }

    /// Fade a channel toward `target_norm` over `dur` (LTP — supersedes any fade
    /// already running on that channel, starting from its current value).
    /// `dur == 0` sets the value immediately.
    #[allow(clippy::too_many_arguments)]
    pub fn submit_fade(
        &mut self,
        universe: u16,
        channel: u16,
        width: ChannelWidth,
        target_norm: f64,
        dur: Duration,
        curve: FadeCurve,
        now: Instant,
    ) {
        if dur.is_zero() {
            self.fades.remove(&(universe, channel));
            self.write_norm(universe, channel, width, target_norm);
            return;
        }
        let start_norm = self.read_norm(universe, channel, width);
        self.fades.insert(
            (universe, channel),
            ChannelFade { width, start_norm, target_norm, t0: now, dur, curve },
        );
    }

    pub fn set_blackout(&mut self, blackout: bool) {
        self.blackout = blackout;
    }

    /// Advance every fade to `now`, writing interpolated values into the buffers.
    /// Completed fades are removed, leaving their final value in place (tracking).
    pub fn tick(&mut self, now: Instant) {
        let mut writes: Vec<(u16, u16, ChannelWidth, f64)> = Vec::new();
        let mut done: Vec<(u16, u16)> = Vec::new();
        for (&(universe, channel), fade) in self.fades.iter() {
            let elapsed = now.saturating_duration_since(fade.t0).as_secs_f64();
            let t = (elapsed / fade.dur.as_secs_f64()).clamp(0.0, 1.0);
            let k = fade.curve.apply(t);
            let norm = fade.start_norm + (fade.target_norm - fade.start_norm) * k;
            writes.push((universe, channel, fade.width, norm));
            if t >= 1.0 {
                done.push((universe, channel));
            }
        }
        for (universe, channel, width, norm) in writes {
            self.write_norm(universe, channel, width, norm);
        }
        for key in done {
            self.fades.remove(&key);
        }
    }

    /// The bytes to transmit for a universe (all-zero while blacked out).
    pub fn rendered(&self, universe: u16) -> [u8; DMX_UNIVERSE_SIZE] {
        if self.blackout {
            return [0; DMX_UNIVERSE_SIZE];
        }
        self.universes.get(&universe).copied().unwrap_or([0; DMX_UNIVERSE_SIZE])
    }

    /// Universes that have ever been written (for snapshot / send iteration).
    pub fn active_universes(&self) -> Vec<u16> {
        self.universes.keys().copied().collect()
    }
}

// ---------------------------------------------------------------------------
// Engine handle + thread
// ---------------------------------------------------------------------------

/// A live snapshot of every universe's output bytes, for the DMX monitor UI.
pub type DmxSnapshot = HashMap<u16, [u8; DMX_UNIVERSE_SIZE]>;

enum DmxCommand {
    SubmitFade {
        universe: u16,
        channel: u16,
        width: ChannelWidth,
        target_norm: f64,
        dur: Duration,
        curve: FadeCurve,
    },
    SetBlackout(bool),
    SetOutputs(Vec<UniverseOutput>),
    Shutdown,
}

/// Handle to the DMX engine. Cheap to clone-share via `Arc`.
pub struct DmxEngine {
    cmd_tx: Sender<DmxCommand>,
    snapshot: Arc<Mutex<DmxSnapshot>>,
    blackout: Arc<AtomicBool>,
}

impl DmxEngine {
    /// Spawn the DMX engine thread.
    pub fn new() -> Self {
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<DmxCommand>();
        let snapshot = Arc::new(Mutex::new(DmxSnapshot::new()));
        let blackout = Arc::new(AtomicBool::new(false));

        let snapshot_thread = Arc::clone(&snapshot);
        std::thread::Builder::new()
            .name("inkue-dmx".into())
            .spawn(move || dmx_thread(cmd_rx, snapshot_thread))
            .expect("failed to spawn DMX engine thread");

        Self { cmd_tx, snapshot, blackout }
    }

    /// Fade a channel toward `target_norm` (0.0–1.0) over `dur`.
    pub fn submit_fade(
        &self,
        universe: u16,
        channel: u16,
        width: ChannelWidth,
        target_norm: f64,
        dur: Duration,
        curve: FadeCurve,
    ) {
        let _ = self.cmd_tx.send(DmxCommand::SubmitFade {
            universe,
            channel,
            width,
            target_norm,
            dur,
            curve,
        });
    }

    /// Set a channel immediately (no fade).
    pub fn set_channel(&self, universe: u16, channel: u16, width: ChannelWidth, value_norm: f64) {
        self.submit_fade(universe, channel, width, value_norm, Duration::ZERO, FadeCurve::Linear);
    }

    /// Toggle the global blackout override.
    pub fn set_blackout(&self, blackout: bool) {
        self.blackout.store(blackout, Ordering::Relaxed);
        let _ = self.cmd_tx.send(DmxCommand::SetBlackout(blackout));
    }

    pub fn is_blackout(&self) -> bool {
        self.blackout.load(Ordering::Relaxed)
    }

    /// Replace the set of transmitted universes (rebinds sockets).
    pub fn set_outputs(&self, outputs: Vec<UniverseOutput>) {
        let _ = self.cmd_tx.send(DmxCommand::SetOutputs(outputs));
    }

    /// Current output bytes per universe (for the monitor view).
    pub fn snapshot(&self) -> DmxSnapshot {
        self.snapshot.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

impl Default for DmxEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for DmxEngine {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(DmxCommand::Shutdown);
    }
}

/// One configured output universe plus its rolling send state.
struct ActiveSink {
    universe: u16,
    sink: DmxSink,
    sequence: u8,
    last: Option<(Instant, [u8; DMX_UNIVERSE_SIZE])>,
}

fn dmx_thread(cmd_rx: crossbeam_channel::Receiver<DmxCommand>, snapshot: Arc<Mutex<DmxSnapshot>>) {
    let cid = *uuid::Uuid::new_v4().as_bytes();
    let source_name = "Inkue".to_string();
    let frame = Duration::from_millis(1000 / REFRESH_HZ);

    let mut state = DmxState::new();
    let mut sinks: Vec<ActiveSink> = Vec::new();

    loop {
        // Drain all pending commands without blocking.
        loop {
            match cmd_rx.try_recv() {
                Ok(DmxCommand::Shutdown) | Err(TryRecvError::Disconnected) => return,
                Ok(DmxCommand::SubmitFade { universe, channel, width, target_norm, dur, curve }) => {
                    state.submit_fade(universe, channel, width, target_norm, dur, curve, Instant::now());
                }
                Ok(DmxCommand::SetBlackout(b)) => state.set_blackout(b),
                Ok(DmxCommand::SetOutputs(outputs)) => {
                    sinks = build_sinks(&outputs, cid, &source_name);
                }
                Err(TryRecvError::Empty) => break,
            }
        }

        let now = Instant::now();
        state.tick(now);

        // Publish snapshot for the monitor UI.
        if let Ok(mut snap) = snapshot.lock() {
            snap.clear();
            for universe in state.active_universes() {
                snap.insert(universe, state.rendered(universe));
            }
        }

        // Transmit each universe (send-on-change + keepalive).
        for active in sinks.iter_mut() {
            let data = state.rendered(active.universe);
            let changed = active.last.map(|(_, prev)| prev != data).unwrap_or(true);
            let due = active.last.map(|(t, _)| now.duration_since(t) >= KEEPALIVE).unwrap_or(true);
            if changed || due {
                active.sequence = active.sequence.wrapping_add(1);
                let _ = active.sink.send(active.sequence, &data);
                active.last = Some((now, data));
            }
        }

        std::thread::sleep(frame);
    }
}

fn build_sinks(outputs: &[UniverseOutput], cid: [u8; 16], source_name: &str) -> Vec<ActiveSink> {
    outputs
        .iter()
        .filter(|o| o.enabled)
        .filter_map(|o| match DmxSink::new(o, cid, source_name.to_string()) {
            Ok(sink) => Some(ActiveSink { universe: o.universe, sink, sequence: 0, last: None }),
            Err(e) => {
                log::warn!("[dmx] universe {} disabled: {e}", o.universe);
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn fade_interpolates_linearly() {
        let mut s = DmxState::new();
        let start = t0();
        s.submit_fade(1, 0, ChannelWidth::Bit8, 1.0, Duration::from_secs(1), FadeCurve::Linear, start);

        s.tick(start);
        assert_eq!(s.rendered(1)[0], 0);

        s.tick(start + Duration::from_millis(500));
        let mid = s.rendered(1)[0];
        assert!((127..=128).contains(&mid), "mid was {mid}");

        s.tick(start + Duration::from_millis(1000));
        assert_eq!(s.rendered(1)[0], 255);
    }

    #[test]
    fn fade_completes_and_is_removed() {
        let mut s = DmxState::new();
        let start = t0();
        s.submit_fade(1, 0, ChannelWidth::Bit8, 1.0, Duration::from_millis(100), FadeCurve::Linear, start);
        s.tick(start + Duration::from_millis(200));
        assert_eq!(s.rendered(1)[0], 255);
        // No fade left → a later tick keeps the held value (tracking).
        s.tick(start + Duration::from_secs(10));
        assert_eq!(s.rendered(1)[0], 255);
    }

    #[test]
    fn immediate_set_when_duration_zero() {
        let mut s = DmxState::new();
        s.submit_fade(2, 5, ChannelWidth::Bit8, 0.5, Duration::ZERO, FadeCurve::Linear, t0());
        assert_eq!(s.rendered(2)[5], 128); // round(0.5*255)=128
    }

    #[test]
    fn sixteen_bit_split() {
        let mut s = DmxState::new();
        s.submit_fade(1, 0, ChannelWidth::Bit16, 0.5, Duration::ZERO, FadeCurve::Linear, t0());
        let out = s.rendered(1);
        // round(0.5 * 65535) = 32768 = 0x8000
        assert_eq!(out[0], 0x80); // coarse
        assert_eq!(out[1], 0x00); // fine
    }

    #[test]
    fn ltp_supersedes_and_starts_from_current() {
        let mut s = DmxState::new();
        let start = t0();
        // Fade up over 1s, advance to ~50%.
        s.submit_fade(1, 0, ChannelWidth::Bit8, 1.0, Duration::from_secs(1), FadeCurve::Linear, start);
        s.tick(start + Duration::from_millis(500));
        let mid = s.rendered(1)[0];
        assert!((127..=128).contains(&mid));

        // New fade to 0 should start from the current ~50%, not from 0 or 1.
        let t1 = start + Duration::from_millis(500);
        s.submit_fade(1, 0, ChannelWidth::Bit8, 0.0, Duration::from_secs(1), FadeCurve::Linear, t1);
        s.tick(t1); // immediately: still ~mid
        assert!((126..=129).contains(&s.rendered(1)[0]));
        s.tick(t1 + Duration::from_millis(1000));
        assert_eq!(s.rendered(1)[0], 0);
        // Only one fade per channel.
        assert_eq!(s.fades.len(), 0);
    }

    #[test]
    fn tracking_leaves_untouched_channels_alone() {
        let mut s = DmxState::new();
        let start = t0();
        s.set_channel_now(1, 10, 200);
        s.submit_fade(1, 0, ChannelWidth::Bit8, 1.0, Duration::from_millis(100), FadeCurve::Linear, start);
        s.tick(start + Duration::from_millis(200));
        assert_eq!(s.rendered(1)[0], 255); // faded channel
        assert_eq!(s.rendered(1)[10], 200); // untouched channel preserved
    }

    #[test]
    fn blackout_zeros_output_but_keeps_state() {
        let mut s = DmxState::new();
        s.set_channel_now(1, 0, 255);
        s.set_blackout(true);
        assert_eq!(s.rendered(1)[0], 0);
        s.set_blackout(false);
        assert_eq!(s.rendered(1)[0], 255);
    }

    // Small test helper: write a raw 8-bit channel value directly.
    impl DmxState {
        fn set_channel_now(&mut self, universe: u16, channel: u16, value: u8) {
            self.write_norm(universe, channel, ChannelWidth::Bit8, value as f64 / 255.0);
        }
    }
}
