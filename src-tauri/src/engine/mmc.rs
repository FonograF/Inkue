//! MMC (MIDI Machine Control) decoder for the TC/MMC MIDI port.
//!
//! The MPC acts as a transport master and speaks standard MMC SysEx:
//! `F0 7F <device_id> 06 <command> [<data>] F7`, where `<device_id>` is `7F`
//! for "all devices" (the MPC actually sends `00`; we accept any value).
//!
//! This module is a **pure decoder**: it turns a raw MIDI byte slice into an
//! [`MmcCommand`] and performs no I/O of its own. It lives in the engine layer
//! so it is trivially unit-testable with the real MPC byte captures.
//!
//! Only the subset of commands the MPC actually sends is modelled:
//! Stop (`01`), Play (`02`), Deferred Play (`03`), and Locate (`44`).
//! Reset (`06`) and the generic record/stop variants are normalised to Stop
//! (an explicit halt); anything unrecognised returns `None` so the caller can
//! ignore it silently (clock/active-sensing are not SysEx anyway).

use crate::engine::timecode_types::{TcPosition, TcRate};

/// The handful of MMC commands we react to from the MPC transport master.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MmcCommand {
    /// Stop playback / halt transport.
    Stop,
    /// Immediate play ("Play").
    Play,
    /// "Deferred Play" — armed play from the next beat/bar.
    DeferredPlay,
    /// Seek to an absolute SMPTE position.
    Locate { position: TcPosition },
}

/// Decode a raw MIDI byte slice into an [`MmcCommand`].
///
/// Expected layout: `F0 7F <device_id> 06 <command> [<data>] F7`.
pub fn decode_mmc(msg: &[u8]) -> Option<MmcCommand> {
    // SysEx must be at least `F0 7F dev 06 cmd F7` (6 bytes), start with F0,
    // end with F7, and carry the Universal Non-Real-Time id (7F) + MMC sub-id (06).
    if msg.len() < 6 {
        return None;
    }
    if msg[0] != 0xF0 || msg[msg.len() - 1] != 0xF7 {
        return None;
    }
    if msg[1] != 0x7F || msg[3] != 0x06 {
        return None;
    }
    // Any device id is accepted (MPC sends broadcast-style 00; lenient here).

    let cmd = msg[4];
    match cmd {
        // Command messages with no payload.
        0x01 => Some(MmcCommand::Stop), // Stop
        0x02 => Some(MmcCommand::Play), // Play
        0x03 => Some(MmcCommand::DeferredPlay), // Deferred Play (armed)
        // Reset/record/stop-ish variants → treat as an explicit halt.
        0x06 | 0x09 | 0x0A | 0x0B | 0x0D => Some(MmcCommand::Stop),
        // Locate: F0 7F <dev> 06 44 06 01 hh mm ss ff F7
        // The payload after the 0x44 command byte is `06 01 hh mm ss ff`
        // (the final byte is a subframe, which we ignore — we quantise to frame).
        0x44 => {
            if msg.len() < 11 || msg[5] != 0x06 || msg[6] != 0x01 {
                return None;
            }
            let (hh, mm, ss, ff) = (msg[7], msg[8], msg[9], msg[10]);
            let rate = rate_from_mmc_hours(hh);
            Some(MmcCommand::Locate {
                position: TcPosition::new(hh, mm, ss, ff, rate),
            })
        }
        // Ignore everything else (timing clock, active sensing are not SysEx;
        // MMC go/rewrite/etc. are not sent by the MPC).
        _ => None,
    }
}

/// Derive the SMPTE `TcRate` from the high nibble of the MMC Locate hours byte.
///
/// SMPTE rate bits are carried in bits 7–5 of the hours byte:
/// `000`=24fps, `001`=25fps, `010`=29.97 drop, `011`=30fps. Unknown → 24fps.
fn rate_from_mmc_hours(hours: u8) -> TcRate {
    match (hours >> 5) & 0x07 {
        0b000 => TcRate::Fps24,
        0b001 => TcRate::Fps25,
        0b010 => TcRate::Fps2997Df,
        0b011 => TcRate::Fps30,
        _ => TcRate::Fps24,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::timecode_types::TcRate;

    /// Reproduces the exact capture from the MPC: `F0 7F 00 06 44 06 01 00 00
    /// 02 0B F7` → target `00:00:02:11` (~2458 ms at 24fps).
    #[test]
    fn decodes_mpc_locate_to_2_11_at_24fps() {
        let bytes = [0xF0, 0x7F, 0x00, 0x06, 0x44, 0x06, 0x01, 0x00, 0x00, 0x02, 0x0B, 0xF7];
        let cmd = decode_mmc(&bytes).expect("must decode MPC Locate");
        assert_eq!(
            cmd,
            MmcCommand::Locate {
                position: TcPosition::new(0, 0, 2, 11, TcRate::Fps24)
            }
        );
        // 00:00:02:11 @24fps → 59 frames → 59 * 41667 / 1000 = 2458 ms.
        assert_eq!(position_of(&cmd).to_millis(), 2458);
    }

    #[test]
    fn locate_carries_rate_bits_in_hours() {
        // hours byte 0xE0 → rate bits 111 (unknown) → falls back to 24fps.
        let bytes = [0xF0, 0x7F, 0x00, 0x06, 0x44, 0x06, 0x01, 0xE0, 0x00, 0x05, 0x00, 0xF7];
        assert_eq!(
            decode_mmc(&bytes).unwrap(),
            MmcCommand::Locate {
                position: TcPosition::new(0, 0, 5, 0, TcRate::Fps24)
            }
        );
                        // 25fps: rate bits 001 → hours high nibble 0x20 → 25fps.
        let bytes = [0xF0, 0x7F, 0x00, 0x06, 0x44, 0x06, 0x01, 0x20, 0x00, 0x01, 0x00, 0xF7];
        assert_eq!(position_of(&decode_mmc(&bytes).unwrap()).rate, TcRate::Fps25);
    }

    #[test]
    fn decodes_transport_commands() {
        let stop = [0xF0, 0x7F, 0x00, 0x06, 0x01, 0xF7];
        assert_eq!(decode_mmc(&stop), Some(MmcCommand::Stop));
        let play = [0xF0, 0x7F, 0x00, 0x06, 0x02, 0xF7];
        assert_eq!(decode_mmc(&play), Some(MmcCommand::Play));
        let deferred = [0xF0, 0x7F, 0x00, 0x06, 0x03, 0xF7];
        assert_eq!(decode_mmc(&deferred), Some(MmcCommand::DeferredPlay));
        let reset = [0xF0, 0x7F, 0x00, 0x06, 0x06, 0xF7];
        assert_eq!(decode_mmc(&reset), Some(MmcCommand::Stop));
    }

    #[test]
    fn rejects_non_mmc_sysex_and_malformed() {
        // Active sensing / system start are not SysEx — ignored.
        assert_eq!(decode_mmc(&[0xF2]), None);
        assert_eq!(decode_mmc(&[0xF2, 0x00, 0x01]), None);
        // SysEx but not Universal Non-Real-Time (1st data byte != 0x7F).
        assert_eq!(decode_mmc(&[0xF0, 0x41, 0x00, 0x06, 0x01, 0xF7]), None);
        // SysEx 7F but wrong sub-id (not MMC 06).
        assert_eq!(decode_mmc(&[0xF0, 0x7F, 0x00, 0x7F, 0x01, 0xF7]), None);
        // Play with wrong sub-id.
        assert_eq!(decode_mmc(&[0xF0, 0x7F, 0x00, 0x05, 0x02, 0xF7]), None);
        // Locate missing payload bytes.
        assert_eq!(decode_mmc(&[0xF0, 0x7F, 0x00, 0x06, 0x44, 0xF7]), None);
        // Locate with malformed payload header.
        assert_eq!(
            decode_mmc(&[0xF0, 0x7F, 0x00, 0x06, 0x44, 0x05, 0x01, 0x00, 0x00, 0x02, 0x0B, 0xF7]),
            None
        );
        // Unterminated SysEx.
        assert_eq!(decode_mmc(&[0xF0, 0x7F, 0x00, 0x06, 0x02]), None);
    }

    #[test]
    fn ignores_unrecognised_commands() {
        // MMC "Go" (0x05) etc. are not handled — return None.
        let unknown = [0xF0, 0x7F, 0x00, 0x06, 0x05, 0xF7];
        assert_eq!(decode_mmc(&unknown), None);
    }

    /// Helper: borrow the position out of a Locate command.
    fn position_of(cmd: &MmcCommand) -> &TcPosition {
        match cmd {
            MmcCommand::Locate { position } => position,
            _ => panic!("not a Locate"),
        }
    }
}
