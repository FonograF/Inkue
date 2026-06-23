//! Timecode receiver — decodes incoming MTC (MIDI Timecode) and LTC
//! (Linear/Longitudinal Timecode from an audio input) into [`TcPosition`]
//! events and distributes them to the dispatcher.
//!
//! **MTC** — spawns a `midir::MidiInput` connection on a background thread.
//! Handles both quarter-frame messages (8 messages → advance 2 frames) and
//! full-frame SysEx.  A **flywheel** (software PLL) interpolates the position
//! between quarter-frame pairs so the dispatcher always has a current value.
//!
//! **LTC** — reads f32 samples from an audio [`InputFeed`] ring and passes
//! them through the biphase-mark decoder in [`crate::engine::ltc`].
//!
//! Both paths emit [`TcEvent`]s via a crossbeam channel consumed by the
//! show event loop plus a Tauri `timecode` event for the UI.
//!
//! The receiver is reconfigurable at runtime (like `OscServer`) via
//! [`TimecodeReceiver::reconfigure`].

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};

use super::timecode_types::{TcEvent, TcPosition, TcRate};

// ---------------------------------------------------------------------------
// Public configuration
// ---------------------------------------------------------------------------

/// Which physical source feeds this receiver.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TcSource {
    /// MTC via a MIDI input port.
    #[default]
    Mtc,
    /// LTC decoded from an audio input (requires an Input Patch).
    Ltc,
}

/// Machine-level timecode receiver configuration (stored in machine-config,
/// like the audio device — source/port are hardware-specific).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TcReceiverConfig {
    /// Which physical source to listen to.
    #[serde(default)]
    pub source: TcSource,
    /// MIDI input port name (for MTC).  `None` = first available port.
    #[serde(default)]
    pub midi_port: Option<String>,
    /// Input patch device id (for LTC).  `None` = default input.
    #[serde(default)]
    pub ltc_device_id: Option<String>,
}

impl Default for TcReceiverConfig {
    fn default() -> Self {
        Self { source: TcSource::Mtc, midi_port: None, ltc_device_id: None }
    }
}

// ---------------------------------------------------------------------------
// MTC quarter-frame decoder
// ---------------------------------------------------------------------------

/// Assembles 8 MTC quarter-frame messages into a complete [`TcPosition`].
///
/// MTC sends one nibble per quarter-frame (2 frames = 8 messages).
/// The full frame becomes valid after message 7 arrives.
#[derive(Default)]
pub(crate) struct MtcAssembler {
    nibbles: [u8; 8],
    count:   u8,
}

impl MtcAssembler {
    /// Feed one quarter-frame byte (the data byte of a 0xF1 message).
    /// Returns `Some(TcPosition)` when a complete frame has been assembled
    /// (every 8th quarter-frame message).
    pub fn push_quarter_frame(&mut self, data: u8) -> Option<TcPosition> {
        let piece = (data >> 4) & 0x07;
        let value = data & 0x0F;
        self.nibbles[piece as usize] = value;
        self.count += 1;

        // A complete frame is ready after we've seen all 8 pieces.
        // In practice reassemble on piece 7 (the last one) to minimise latency.
        if piece == 7 || self.count >= 8 {
            self.count = 0;
            return Some(self.assemble());
        }
        None
    }

    /// Feed a full-frame SysEx payload (F0 7F 7F 01 01 hh mm ss ff F7).
    /// The four data bytes are the last four bytes before F7.
    pub fn push_full_frame(&mut self, hh: u8, mm: u8, ss: u8, ff: u8) -> TcPosition {
        let rate = match (hh >> 5) & 0x03 {
            0 => TcRate::Fps24,
            1 => TcRate::Fps25,
            2 => TcRate::Fps2997Df,
            _ => TcRate::Fps30,
        };
        self.count = 8; // reset QF accumulator
        TcPosition::new(hh & 0x1F, mm & 0x3F, ss & 0x3F, ff & 0x1F, rate)
    }

    fn assemble(&self) -> TcPosition {
        let n = &self.nibbles;
        let frames  =  n[0] | ((n[1] & 0x01) << 4);
        let seconds =  n[2] | ((n[3] & 0x03) << 4);
        let minutes =  n[4] | ((n[5] & 0x03) << 4);
        let hours   =  n[6] | ((n[7] & 0x01) << 4);
        let rate = match (n[7] >> 1) & 0x03 {
            0 => TcRate::Fps24,
            1 => TcRate::Fps25,
            2 => TcRate::Fps2997Df,
            _ => TcRate::Fps30,
        };
        TcPosition::new(hours, minutes, seconds, frames, rate)
    }
}

// ---------------------------------------------------------------------------
// Flywheel / interpolator
// ---------------------------------------------------------------------------

/// Tracks the last received `TcPosition` and interpolates the current
/// position from the wall-clock elapsed time since it arrived.
pub(crate) struct TcFlywheel {
    last_pos:  Option<TcPosition>,
    last_wall: Option<Instant>,
    /// How long we coast before declaring TC stopped.
    freewheel: Duration,
    running:   bool,
}

impl TcFlywheel {
    pub fn new(freewheel_ms: u32) -> Self {
        Self {
            last_pos:  None,
            last_wall: Some(Instant::now()),
            freewheel: Duration::from_millis(freewheel_ms as u64),
            running:   false,
        }
    }

    /// Update with a newly received position.  Returns `true` if this is
    /// a TC-started event (first frame after silence).
    pub fn update(&mut self, pos: TcPosition) -> bool {
        let was_running = self.running;
        self.last_pos  = Some(pos);
        self.last_wall = Some(Instant::now());
        self.running   = true;
        !was_running
    }

    /// Current interpolated position, or `None` if the freewheel has expired.
    pub fn current(&mut self) -> Option<TcPosition> {
        let (pos, wall) = self.last_pos.zip(self.last_wall)?;
        let elapsed = wall.elapsed();
        if elapsed > self.freewheel {
            if self.running {
                self.running = false;
                // Return None to signal TC stopped.
            }
            return None;
        }
        // Interpolate: advance by elapsed time.
        let extra_frames = (elapsed.as_micros() as u64) / pos.rate.frame_us();
        let advanced = TcPosition::from_frame_number(
            pos.to_frame_number().saturating_add(extra_frames),
            pos.rate,
        );
        Some(advanced)
    }

    pub fn is_running(&self) -> bool { self.running }

    #[allow(dead_code)]
    pub fn set_freewheel(&mut self, ms: u32) {
        self.freewheel = Duration::from_millis(ms as u64);
    }
}

// ---------------------------------------------------------------------------
// Receiver handle
// ---------------------------------------------------------------------------

/// Enumerate available MIDI input port names (for UI dropdowns).
pub fn list_midi_input_ports() -> Vec<String> {
    match midir::MidiInput::new("WinCue-tc-list") {
        Ok(inp) => inp
            .ports()
            .iter()
            .filter_map(|p| inp.port_name(p).ok())
            .collect(),
        Err(e) => {
            log::warn!("TC: failed to enumerate MIDI input ports: {e}");
            Vec::new()
        }
    }
}

/// Handle to the timecode receiver subsystem.
///
/// Internally spawns a background thread that owns the `midir` connection
/// (or the LTC audio decode loop) and publishes [`TcEvent`]s to the channel
/// returned by [`TimecodeReceiver::subscribe`].
pub struct TimecodeReceiver {
    event_tx:   Sender<TcEvent>,
    event_rx:   Receiver<TcEvent>,
    config:     Mutex<TcReceiverConfig>,
    /// Set to `true` to ask the receiver thread to shut down.
    shutdown:   Arc<AtomicBool>,
    flywheel:   Arc<Mutex<TcFlywheel>>,
}

impl TimecodeReceiver {
    /// Create a receiver from `config` and start the background thread.
    pub fn new(config: TcReceiverConfig) -> Arc<Self> {
        let (tx, rx) = crossbeam_channel::unbounded::<TcEvent>();
        let shutdown = Arc::new(AtomicBool::new(false));
        let flywheel = Arc::new(Mutex::new(TcFlywheel::new(500)));

        let receiver = Arc::new(Self {
            event_tx: tx.clone(),
            event_rx: rx,
            config: Mutex::new(config.clone()),
            shutdown: Arc::clone(&shutdown),
            flywheel: Arc::clone(&flywheel),
        });

        Self::start_thread(config, tx, Arc::clone(&shutdown), Arc::clone(&flywheel));
        receiver
    }

    /// Subscribe to TC events (clones the channel receiver — multiple
    /// consumers are supported via the unbounded channel).
    pub fn subscribe(&self) -> Receiver<TcEvent> {
        self.event_rx.clone()
    }

    /// Replace the configuration and restart the background thread.
    pub fn reconfigure(&self, config: TcReceiverConfig) {
        self.shutdown.store(true, Ordering::Relaxed);
        // Give the old thread a moment to notice the shutdown flag.
        std::thread::sleep(Duration::from_millis(50));
        self.shutdown.store(false, Ordering::Relaxed);
        if let Ok(mut c) = self.config.lock() {
            *c = config.clone();
        }
        let tx       = self.event_tx.clone();
        let shutdown = Arc::clone(&self.shutdown);
        let flywheel = Arc::clone(&self.flywheel);
        Self::start_thread(config, tx, shutdown, flywheel);
    }

    /// Current interpolated timecode position (for the UI status indicator).
    /// Returns `None` when TC is not running / freewheel expired.
    pub fn current_position(&self) -> Option<TcPosition> {
        self.flywheel.lock().ok()?.current()
    }

    pub fn is_running(&self) -> bool {
        self.flywheel.lock().map(|f| f.is_running()).unwrap_or(false)
    }

    // ── Internal ─────────────────────────────────────────────────────────

    fn start_thread(
        config:   TcReceiverConfig,
        tx:       Sender<TcEvent>,
        shutdown: Arc<AtomicBool>,
        flywheel: Arc<Mutex<TcFlywheel>>,
    ) {
        match config.source {
            TcSource::Mtc => {
                std::thread::Builder::new()
                    .name("wincue-tc-mtc".into())
                    .spawn(move || mtc_thread(config.midi_port, tx, shutdown, flywheel))
                    .expect("failed to spawn MTC receiver thread");
            }
            TcSource::Ltc => {
                // LTC receive is handled by the show event loop which polls
                // the audio input ring via ltc::Decoder — no separate thread needed.
                log::info!("TC: LTC receive mode (polled by event loop)");
            }
        }
    }
}

impl Drop for TimecodeReceiver {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// MTC receive thread
// ---------------------------------------------------------------------------

fn mtc_thread(
    port_name: Option<String>,
    tx:        Sender<TcEvent>,
    shutdown:  Arc<AtomicBool>,
    flywheel:  Arc<Mutex<TcFlywheel>>,
) {
    let assembler = MtcAssembler::default();

    let Ok(midi_in) = midir::MidiInput::new("WinCue-tc") else {
        log::error!("TC: failed to create MIDI input");
        return;
    };

    let ports = midi_in.ports();
    if ports.is_empty() {
        log::warn!("TC: no MIDI input ports found");
        return;
    }

    let port = if let Some(name) = &port_name {
        ports.iter().find(|p| midi_in.port_name(p).ok().as_deref() == Some(name))
            .or_else(|| ports.first())
    } else {
        ports.first()
    };

    let Some(port) = port else {
        log::warn!("TC: MTC port '{:?}' not found", port_name);
        return;
    };

    let port_display = midi_in.port_name(port).unwrap_or_default();
    log::info!("TC: MTC listening on '{port_display}'");

    // The midir callback is `Fn` (not `FnMut`) so we share state via Arc.
    let assembler_shared = Arc::new(Mutex::new(assembler));
    let tx2       = tx.clone();
    let flywheel2 = Arc::clone(&flywheel);
    let shutdown2 = Arc::clone(&shutdown);
    let asm2      = Arc::clone(&assembler_shared);

    let _conn = midi_in.connect(
        port,
        "wincue-tc",
        move |_stamp, message, _| {
            if shutdown2.load(Ordering::Relaxed) { return; }
            let Some(pos) = decode_mtc_message(message, &asm2) else { return };
            let started = flywheel2.lock().map(|mut f| f.update(pos)).unwrap_or(false);
            if started { let _ = tx2.send(TcEvent::Started(pos)); }
            let _ = tx2.send(TcEvent::Position(pos));
        },
        (),
    );

    // Keep the connection alive until shutdown is requested, polling every 50ms.
    loop {
        if shutdown.load(Ordering::Relaxed) { break; }
        std::thread::sleep(Duration::from_millis(50));

        // Check for freewheel expiry.
        let expired = flywheel.lock().map(|mut f| {
            let was = f.is_running();
            f.current(); // advances the freewheel check
            was && !f.is_running()
        }).unwrap_or(false);
        if expired { let _ = tx.send(TcEvent::Stopped); }
    }
    log::info!("TC: MTC thread stopped");
}

fn decode_mtc_message(
    msg: &[u8],
    asm: &Arc<Mutex<MtcAssembler>>,
) -> Option<TcPosition> {
    match msg {
        // Quarter-frame: F1 dd
        [0xF1, data] => {
            asm.lock().ok()?.push_quarter_frame(*data)
        }
        // Full-frame SysEx: F0 7F 7F 01 01 hh mm ss ff F7
        [0xF0, 0x7F, 0x7F, 0x01, 0x01, hh, mm, ss, ff, 0xF7] => {
            Some(asm.lock().ok()?.push_full_frame(*hh, *mm, *ss, *ff))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::timecode_types::TcRate;

    // Build an assembler with all 8 nibbles pre-loaded for a known position.
    fn make_assembler_for(pos: TcPosition) -> MtcAssembler {
        let mut asm = MtcAssembler::default();
        let f = pos.frames;
        let s = pos.seconds;
        let m = pos.minutes;
        let h = pos.hours;
        let rate_bits: u8 = match pos.rate {
            TcRate::Fps24    => 0,
            TcRate::Fps25    => 1,
            TcRate::Fps2997Df => 2,
            _                => 3,
        };
        // Quarter frames: piece | (value << 4)
        let qf: [u8; 8] = [
            (0 << 4) | (f & 0x0F),
            (1 << 4) | ((f >> 4) & 0x01),
            (2 << 4) | (s & 0x0F),
            (3 << 4) | ((s >> 4) & 0x03),
            (4 << 4) | (m & 0x0F),
            (5 << 4) | ((m >> 4) & 0x03),
            (6 << 4) | (h & 0x0F),
            (7 << 4) | (((h >> 4) & 0x01) | (rate_bits << 1)),
        ];
        for byte in qf {
            asm.push_quarter_frame(byte);
        }
        asm
    }

    #[test]
    fn quarter_frame_assembles_known_position() {
        let expected = TcPosition::new(1, 2, 3, 4, TcRate::Fps30);
        let mut asm = MtcAssembler::default();
        let f = expected.frames;
        let s = expected.seconds;
        let m = expected.minutes;
        let h = expected.hours;
        let rate_bits: u8 = 3; // Fps30
        let qf: [u8; 8] = [
            (0 << 4) | (f & 0x0F),
            (1 << 4) | ((f >> 4) & 0x01),
            (2 << 4) | (s & 0x0F),
            (3 << 4) | ((s >> 4) & 0x03),
            (4 << 4) | (m & 0x0F),
            (5 << 4) | ((m >> 4) & 0x03),
            (6 << 4) | (h & 0x0F),
            (7 << 4) | (((h >> 4) & 0x01) | (rate_bits << 1)),
        ];
        let mut result = None;
        for byte in qf {
            result = asm.push_quarter_frame(byte);
        }
        let got = result.expect("should assemble on 8th QF");
        assert_eq!(got.hours,   expected.hours);
        assert_eq!(got.minutes, expected.minutes);
        assert_eq!(got.seconds, expected.seconds);
        assert_eq!(got.frames,  expected.frames);
        assert_eq!(got.rate,    expected.rate);
    }

    #[test]
    fn full_frame_sysex_parses_correctly() {
        let mut asm = MtcAssembler::default();
        // 01:02:03:04 @ 29.97df (rate_bits=2 → hh = 2<<5 | 1 = 0x41)
        let pos = asm.push_full_frame(0x41, 0x02, 0x03, 0x04);
        assert_eq!(pos.hours,   1);
        assert_eq!(pos.minutes, 2);
        assert_eq!(pos.seconds, 3);
        assert_eq!(pos.frames,  4);
        assert_eq!(pos.rate, TcRate::Fps2997Df);
    }

    #[test]
    fn flywheel_starts_after_first_update() {
        let mut fw = TcFlywheel::new(500);
        assert!(!fw.is_running());
        let pos = TcPosition::new(0, 0, 0, 0, TcRate::Fps30);
        let started = fw.update(pos);
        assert!(started, "first update should signal started");
        assert!(fw.is_running());
        assert!(fw.current().is_some());
    }

    #[test]
    fn flywheel_not_started_twice() {
        let mut fw = TcFlywheel::new(500);
        let pos = TcPosition::new(0, 0, 0, 0, TcRate::Fps30);
        assert!(fw.update(pos),  "first update = started");
        assert!(!fw.update(pos), "second update = not started again");
    }
}
