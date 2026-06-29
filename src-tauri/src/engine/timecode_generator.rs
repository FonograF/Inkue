//! Timecode generator — produces an MTC or LTC output stream.
//!
//! **MTC** sends a continuous stream of MIDI quarter-frame messages (one per
//! output-frame quarter, i.e., 4 × fps messages/s) to a MIDI output port.
//! A dedicated `inkue-tc-gen` thread owns the MIDI connection and times
//! quarter-frame messages using `std::thread::sleep`.
//!
//! **LTC** fills a `Vec<f32>` ring via [`super::ltc::LtcEncoder`] and feeds
//! it to the audio engine as a continuous live voice on an Output Patch (same
//! pipeline as the Mic Cue audio path).  This is handled by the
//! [`TimecodeCue`](crate::cue::timecode_cue::TimecodeCue) at GO time.
//!
//! The generator is designed to be held by a `TimecodeCue` instance and
//! dropped when the cue stops.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use super::timecode_types::{TcPosition, TcRate};

// ---------------------------------------------------------------------------
// MTC generator (background thread)
// ---------------------------------------------------------------------------

/// MTC quarter-frame message bytes for a given `TcPosition`.
///
/// Returns the 8 quarter-frame data bytes (the data byte of each 0xF1 message).
pub fn mtc_quarter_frames(pos: TcPosition) -> [u8; 8] {
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
    [
        (f & 0x0F),
        (1 << 4) | ((f >> 4) & 0x01),
        (2 << 4) | (s & 0x0F),
        (3 << 4) | ((s >> 4) & 0x03),
        (4 << 4) | (m & 0x0F),
        (5 << 4) | ((m >> 4) & 0x03),
        (6 << 4) | (h & 0x0F),
        (7 << 4) | (((h >> 4) & 0x01) | (rate_bits << 1)),
    ]
}

/// MTC full-frame SysEx for an immediate jam-sync.
pub fn mtc_full_frame(pos: TcPosition) -> Vec<u8> {
    let rate_bits: u8 = match pos.rate {
        TcRate::Fps24    => 0,
        TcRate::Fps25    => 1,
        TcRate::Fps2997Df => 2,
        _                => 3,
    };
    vec![
        0xF0, 0x7F, 0x7F, 0x01, 0x01,
        ((rate_bits & 0x03) << 5) | (pos.hours   & 0x1F),
        pos.minutes & 0x3F,
        pos.seconds & 0x3F,
        pos.frames  & 0x1F,
        0xF7,
    ]
}

/// Handle to a running MTC generator thread.  Dropping it stops generation.
pub struct MtcGenerator {
    stop: Arc<AtomicBool>,
}

impl MtcGenerator {
    /// Start generating MTC from `start_pos` on `port_name`, advancing in
    /// real time.  Returns `None` if the port cannot be opened.
    pub fn start(start_pos: TcPosition, port_name: Option<String>) -> Option<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = Arc::clone(&stop);

        std::thread::Builder::new()
            .name("inkue-tc-gen".into())
            .spawn(move || mtc_gen_thread(start_pos, port_name, stop2))
            .ok()?;

        Some(Self { stop })
    }
}

impl Drop for MtcGenerator {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn mtc_gen_thread(
    start: TcPosition,
    port_name: Option<String>,
    stop: Arc<AtomicBool>,
) {
    // Open the MIDI output connection.
    let midi_out = match midir::MidiOutput::new("Inkue-TC-Gen") {
        Ok(m) => m,
        Err(e) => { log::error!("TC gen: failed to create MIDI output: {e}"); return; }
    };

    let ports = midi_out.ports();
    let port = if let Some(name) = &port_name {
        ports.iter().find(|p| midi_out.port_name(p).ok().as_deref() == Some(name))
            .or_else(|| ports.first())
            .cloned()
    } else {
        ports.into_iter().next()
    };

    let Some(port) = port else {
        log::warn!("TC gen: MIDI output port '{:?}' not found", port_name);
        return;
    };

    let port_name_display = midi_out.port_name(&port).unwrap_or_default();
    let mut conn = match midi_out.connect(&port, "inkue-tc") {
        Ok(c) => c,
        Err(e) => { log::error!("TC gen: failed to connect to '{port_name_display}': {:?}", e.kind()); return; }
    };

    log::info!("TC gen: MTC → '{port_name_display}' @ {start}");

    // Send a full-frame first so receivers jam-sync immediately.
    let _ = conn.send(&mtc_full_frame(start));

    let fps = start.rate.fps();
    // Quarter-frame interval: 1/(4 × fps) seconds.
    let qf_us = 1_000_000u64 / (4 * fps as u64);
    let qf_dur = Duration::from_micros(qf_us);

    let mut frame = start.to_frame_number();
    let mut qf_idx: u8 = 0; // 0..7
    let mut tick = Instant::now();

    loop {
        if stop.load(Ordering::Relaxed) { break; }

        let pos = TcPosition::from_frame_number(frame, start.rate);
        let qfs = mtc_quarter_frames(pos);

        for &qf_data in &qfs {
            if stop.load(Ordering::Relaxed) { break; }
            let _ = conn.send(&[0xF1, qf_data]);

            qf_idx = (qf_idx + 1) % 8;
            tick += qf_dur;

            // Compensated sleep: sleep only what's left.
            let now = Instant::now();
            if tick > now {
                std::thread::sleep(tick - now);
            }
        }
        frame = frame.wrapping_add(2); // 8 QFs advance 2 frames
    }

    log::info!("TC gen: MTC generation stopped");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quarter_frames_encode_known_position() {
        // 01:02:03:04 @ 30fps (rate_bits=3)
        let pos = TcPosition::new(1, 2, 3, 4, TcRate::Fps30);
        let qf = mtc_quarter_frames(pos);
        // Piece 0: frame units = 4 → 0x04 | (0<<4) = 0x04
        assert_eq!(qf[0] & 0x0F, 4, "frame units");
        // Piece 2: sec units = 3 → 0x03
        assert_eq!(qf[2] & 0x0F, 3, "sec units");
        // Piece 4: min units = 2 → 0x02
        assert_eq!(qf[4] & 0x0F, 2, "min units");
        // Piece 6: hr units = 1 → 0x01
        assert_eq!(qf[6] & 0x0F, 1, "hr units");
        // Piece 7: rate_bits for Fps30 = 3 → bits 5-6 of qf[7] = 3
        assert_eq!((qf[7] >> 1) & 0x03, 3, "rate bits");
    }

    #[test]
    fn full_frame_sysex_layout() {
        let pos = TcPosition::new(1, 2, 3, 4, TcRate::Fps2997Df);
        let ff = mtc_full_frame(pos);
        assert_eq!(ff[0], 0xF0, "SysEx start");
        assert_eq!(*ff.last().unwrap(), 0xF7, "SysEx end");
        // hh byte: rate_bits(2) << 5 | hours(1) = 0x41
        assert_eq!(ff[5], 0x41, "hh byte");
        assert_eq!(ff[6], 0x02, "mm");
        assert_eq!(ff[7], 0x03, "ss");
        assert_eq!(ff[8], 0x04, "ff");
    }

    #[test]
    fn quarter_frame_roundtrips_via_assembler() {
        use crate::engine::timecode_receiver::MtcAssembler;
        let pos_in = TcPosition::new(0, 30, 15, 12, TcRate::Fps25);
        let qfs = mtc_quarter_frames(pos_in);
        let mut asm = MtcAssembler::default();
        let mut got = None;
        for qf in qfs {
            got = asm.push_quarter_frame(qf);
        }
        let decoded = got.expect("should decode after 8 QFs");
        assert_eq!(decoded.hours,   pos_in.hours);
        assert_eq!(decoded.minutes, pos_in.minutes);
        assert_eq!(decoded.seconds, pos_in.seconds);
        assert_eq!(decoded.frames,  pos_in.frames);
        assert_eq!(decoded.rate,    pos_in.rate);
    }
}
