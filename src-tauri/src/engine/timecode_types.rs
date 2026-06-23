//! Core SMPTE timecode types and arithmetic.
//!
//! A [`TcPosition`] is an `HH:MM:SS:FF` position at a given [`TcRate`].
//! The module provides lossless conversions between positions and absolute
//! frame counts, including **29.97 drop-frame** (DF), plus Real-Time (ms) ↔
//! frame helpers used by the dispatcher and the cue trigger editor.

use serde::{Deserialize, Serialize};
use std::fmt;

// ---------------------------------------------------------------------------
// TcRate
// ---------------------------------------------------------------------------

/// SMPTE frame rate — the four standard rates plus 29.97 drop-frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TcRate {
    /// 24 fps — cinema.
    #[serde(rename = "24")]
    Fps24,
    /// 25 fps — PAL / EBU.
    #[serde(rename = "25")]
    Fps25,
    /// 29.97 non-drop (rarely used; included for completeness).
    #[serde(rename = "29.97")]
    Fps2997,
    /// 29.97 drop-frame — NTSC broadcast standard.
    #[serde(rename = "29.97df")]
    #[default]
    Fps2997Df,
    /// 30 fps — film / DAW.
    #[serde(rename = "30")]
    Fps30,
}

impl TcRate {
    /// Nominal frames per second (integer for DF, actual for the rest).
    pub const fn fps(self) -> u32 {
        match self {
            TcRate::Fps24    => 24,
            TcRate::Fps25    => 25,
            TcRate::Fps2997  => 30,
            TcRate::Fps2997Df => 30,
            TcRate::Fps30    => 30,
        }
    }

    /// Whether this rate uses drop-frame numbering.
    pub const fn is_drop_frame(self) -> bool {
        matches!(self, TcRate::Fps2997Df)
    }

    /// Real-time duration of one frame in microseconds, for wall-clock math.
    pub const fn frame_us(self) -> u64 {
        match self {
            TcRate::Fps24    => 41_667,          // 1/24 s
            TcRate::Fps25    => 40_000,          // 1/25 s
            TcRate::Fps2997  => 33_367,          // ≈1/29.97 s
            TcRate::Fps2997Df => 33_367,
            TcRate::Fps30    => 33_333,          // 1/30 s
        }
    }

    /// All display variants in UI order.
    pub const ALL: &'static [TcRate] = &[
        TcRate::Fps24, TcRate::Fps25, TcRate::Fps2997, TcRate::Fps2997Df, TcRate::Fps30,
    ];
}

impl fmt::Display for TcRate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TcRate::Fps24     => write!(f, "24"),
            TcRate::Fps25     => write!(f, "25"),
            TcRate::Fps2997   => write!(f, "29.97"),
            TcRate::Fps2997Df => write!(f, "29.97df"),
            TcRate::Fps30     => write!(f, "30"),
        }
    }
}

// ---------------------------------------------------------------------------
// TcPosition
// ---------------------------------------------------------------------------

/// An HH:MM:SS:FF SMPTE timecode position at a given rate.
///
/// This struct stores the *display* fields, which can differ from the
/// corresponding absolute-frame count for drop-frame rates (frames 00 and 01
/// of every non-multiple-of-10 minute are skipped in DF numbering).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TcPosition {
    pub hours:   u8,
    pub minutes: u8,
    pub seconds: u8,
    pub frames:  u8,
    pub rate:    TcRate,
}

impl TcPosition {
    pub fn new(hours: u8, minutes: u8, seconds: u8, frames: u8, rate: TcRate) -> Self {
        Self { hours, minutes, seconds, frames, rate }
    }

    /// Convert to an absolute frame number (monotone, accounts for DF gaps).
    pub fn to_frame_number(self) -> u64 {
        let fps = self.rate.fps() as u64;
        let h = self.hours as u64;
        let m = self.minutes as u64;
        let s = self.seconds as u64;
        let f = self.frames as u64;

        let nominal = h * 3600 * fps + m * 60 * fps + s * fps + f;

        if !self.rate.is_drop_frame() {
            return nominal;
        }

        // Drop-frame 29.97: skip frames 0 and 1 at the start of every minute
        // except multiples of 10.
        //   drop_count = 2 * (total_minutes - total_minutes / 10)
        let total_minutes = h * 60 + m;
        let drops = 2 * (total_minutes - total_minutes / 10);
        nominal - drops
    }

    /// Reconstruct a `TcPosition` from an absolute frame number + rate.
    pub fn from_frame_number(mut frame: u64, rate: TcRate) -> Self {
        let fps = rate.fps() as u64;

        let (h, m, s, f) = if rate.is_drop_frame() {
            // Undo the drop-frame mapping.
            //
            // A 10-minute block has:
            //   - minute 0 (multiple of 10): 1800 frames (no drops)
            //   - minutes 1..9: 1798 frames each (frames 00 and 01 dropped)
            //   - total: 1800 + 9*1798 = 17_982  (D)
            const D: u64 = 17_982;
            let d      = frame / D;       // which 10-min block
            let rem_10 = frame % D;       // position within that block

            let (mm, fr_in_min) = if rem_10 < 1800 {
                // First minute of the block — no frame drops.
                (0u64, rem_10)
            } else {
                // Minutes 1..9: frames 00 and 01 are skipped, so the nominal
                // frame offset within the minute starts at 2.
                let adj = rem_10 - 1800;
                let mm  = adj / 1798 + 1;
                let fr  = adj % 1798 + 2;  // re-insert the two dropped frames
                (mm, fr)
            };

            let total_m   = d * 10 + mm;
            let total_h   = total_m / 60;
            let total_m_r = total_m % 60;
            let s  = fr_in_min / 30;
            let fr = fr_in_min % 30;
            (total_h, total_m_r, s, fr)
        } else {
            let fr      = frame % fps;
            let s_total = frame / fps;
            let s       = s_total % 60;
            let m_total = s_total / 60;
            let m       = m_total % 60;
            let h       = m_total / 60;
            (h, m, s, fr)
        };

        Self {
            hours:   (h % 24) as u8,
            minutes: m as u8,
            seconds: s as u8,
            frames:  f as u8,
            rate,
        }
    }

    /// Wall-clock position in milliseconds (approximate for 29.97).
    pub fn to_millis(self) -> u64 {
        self.to_frame_number() * self.rate.frame_us() / 1_000
    }

    /// Build a position from a wall-clock millisecond offset.
    pub fn from_millis(ms: u64, rate: TcRate) -> Self {
        let frame = ms * 1_000 / rate.frame_us();
        Self::from_frame_number(frame, rate)
    }
}

impl fmt::Display for TcPosition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sep = if self.rate.is_drop_frame() { ';' } else { ':' };
        write!(f, "{:02}:{:02}:{:02}{}{:02}",
               self.hours, self.minutes, self.seconds, sep, self.frames)
    }
}

/// Parse `HH:MM:SS:FF` or `HH:MM:SS;FF` (DF separator).
impl std::str::FromStr for TcPosition {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let df = s.contains(';');
        let s2 = s.replace(';', ":");
        let parts: Vec<&str> = s2.split(':').collect();
        if parts.len() != 4 {
            return Err(format!("expected HH:MM:SS:FF, got '{s}'"));
        }
        let parse = |p: &str| p.parse::<u8>().map_err(|e| e.to_string());
        Ok(Self {
            hours:   parse(parts[0])?,
            minutes: parse(parts[1])?,
            seconds: parse(parts[2])?,
            frames:  parse(parts[3])?,
            rate: if df { TcRate::Fps2997Df } else { TcRate::Fps30 },
        })
    }
}

// ---------------------------------------------------------------------------
// TcTrigger
// ---------------------------------------------------------------------------

/// A timecode trigger attached to a cue.  The cue fires when incoming TC
/// crosses this position (fire-and-play — not chase).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TcTrigger {
    /// The timecode position at which the cue fires.
    pub position: TcPosition,
    /// `true` when the user entered the position as Real-Time (ms),
    /// `false` for SMPTE H:M:S:F.  Preserved so the inspector can display
    /// it in the same format the operator set it.
    pub real_time: bool,
}

// ---------------------------------------------------------------------------
// TcEvent
// ---------------------------------------------------------------------------

/// Events emitted by the timecode receiver engine.
#[derive(Debug, Clone)]
pub enum TcEvent {
    /// A new position was decoded (fires ~25 fps for MTC, per-frame for LTC).
    Position(TcPosition),
    /// The TC stream started (first frame after silence / freewheel expiry).
    Started(TcPosition),
    /// The TC stream stopped (freewheel window expired).
    Stopped,
}

// ---------------------------------------------------------------------------
// On-Start / On-Stop policy (QLab)
// ---------------------------------------------------------------------------

/// What happens to running cues when TC starts or stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TcOnStop {
    /// Leave running cues untouched when TC stops.
    #[default]
    Continue,
    /// Pause all running cues when TC stops; resume when TC restarts.
    Pause,
    /// Stop all running cues when TC stops.
    Stop,
}

// ---------------------------------------------------------------------------
// CueList TC configuration
// ---------------------------------------------------------------------------

/// Per-CueList timecode synchronisation settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CueListTcConfig {
    /// Enable TC-triggered cue firing for this list.
    #[serde(default)]
    pub enabled: bool,
    /// Rate expected from the incoming stream.
    #[serde(default)]
    pub rate: TcRate,
    /// Dropout tolerance before declaring TC stopped (ms, 0–2000).
    #[serde(default = "CueListTcConfig::default_freewheel_ms")]
    pub freewheel_ms: u32,
    /// Behaviour when the TC stream stops.
    #[serde(default)]
    pub on_stop: TcOnStop,
}

impl CueListTcConfig {
    fn default_freewheel_ms() -> u32 { 500 }
}

impl Default for CueListTcConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            rate: TcRate::default(),
            freewheel_ms: Self::default_freewheel_ms(),
            on_stop: TcOnStop::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── Non-drop frame conversions ────────────────────────────────────────

    #[test]
    fn ndf_30_roundtrip() {
        for frame in [0u64, 1, 29, 30, 1799, 1800, 107_999] {
            let pos = TcPosition::from_frame_number(frame, TcRate::Fps30);
            assert_eq!(pos.to_frame_number(), frame, "frame {frame}");
        }
    }

    #[test]
    fn ndf_25_roundtrip() {
        for frame in [0u64, 24, 25, 1499, 1500, 89_999] {
            let pos = TcPosition::from_frame_number(frame, TcRate::Fps25);
            assert_eq!(pos.to_frame_number(), frame, "frame {frame}");
        }
    }

    #[test]
    fn ndf_24_roundtrip() {
        for frame in [0u64, 23, 24, 1439, 1440, 86_399] {
            let pos = TcPosition::from_frame_number(frame, TcRate::Fps24);
            assert_eq!(pos.to_frame_number(), frame, "frame {frame}");
        }
    }

    #[test]
    fn ndf_30_known_values() {
        // 01:00:00:00 = 108 000 frames at 30fps.
        let one_hour = TcPosition::new(1, 0, 0, 0, TcRate::Fps30);
        assert_eq!(one_hour.to_frame_number(), 108_000);

        // 00:01:00:00 = 1 800 frames.
        let one_min = TcPosition::new(0, 1, 0, 0, TcRate::Fps30);
        assert_eq!(one_min.to_frame_number(), 1_800);
    }

    // ── Drop-frame 29.97 ─────────────────────────────────────────────────

    #[test]
    fn df_skips_frames_0_1_at_non_multiple_of_10_minutes() {
        // 00:00:59:29 is the last frame before the minute boundary.
        let before = TcPosition::new(0, 0, 59, 29, TcRate::Fps2997Df);
        // 00:01:00:02 comes right after (00 and 01 are skipped).
        let after  = TcPosition::new(0, 1, 0,  2, TcRate::Fps2997Df);
        assert_eq!(after.to_frame_number(), before.to_frame_number() + 1,
            "DF: frame 0 and 1 skipped at 1-minute boundary");
    }

    #[test]
    fn df_does_not_skip_at_multiple_of_10_minutes() {
        // 00:09:59:29 → 00:10:00:00 : no skip.
        let before = TcPosition::new(0, 9, 59, 29, TcRate::Fps2997Df);
        let after  = TcPosition::new(0, 10, 0,  0, TcRate::Fps2997Df);
        assert_eq!(after.to_frame_number(), before.to_frame_number() + 1,
            "DF: no skip at 10-minute boundary");
    }

    #[test]
    fn df_one_hour_frame_count() {
        // 29.97 DF: 1 hour = 107 892 frames (standard).
        let one_hour = TcPosition::new(1, 0, 0, 0, TcRate::Fps2997Df);
        assert_eq!(one_hour.to_frame_number(), 107_892);
    }

    #[test]
    fn df_roundtrip_exhaustive_first_minute() {
        // Every frame in the first minute should round-trip cleanly.
        for frame in 0u64..1800 {
            let pos = TcPosition::from_frame_number(frame, TcRate::Fps2997Df);
            assert_eq!(pos.to_frame_number(), frame,
                "DF roundtrip failed at frame {frame} → {pos}");
        }
    }

    #[test]
    fn df_roundtrip_across_minute_boundaries() {
        // Sample frames spanning several minute boundaries.
        for frame in [1798u64, 1799, 1800, 1801, 3596, 3597, 3598, 5394, 17_982, 35_964, 107_892] {
            let pos = TcPosition::from_frame_number(frame, TcRate::Fps2997Df);
            assert_eq!(pos.to_frame_number(), frame,
                "DF roundtrip failed at frame {frame}");
        }
    }

    // ── Display / parse ───────────────────────────────────────────────────

    #[test]
    fn display_ndf_uses_colon_separator() {
        let p = TcPosition::new(1, 2, 3, 4, TcRate::Fps30);
        assert_eq!(p.to_string(), "01:02:03:04");
    }

    #[test]
    fn display_df_uses_semicolon_separator() {
        let p = TcPosition::new(1, 2, 3, 4, TcRate::Fps2997Df);
        assert_eq!(p.to_string(), "01:02:03;04");
    }

    // ── Real-time conversion ──────────────────────────────────────────────

    #[test]
    fn realtime_30fps_one_hour() {
        // 30 fps: frame_us = 33_333 µs.  1 hour = 108 000 frames.
        // 108 000 * 33_333 / 1_000 = 3_599_964 ms ≈ 3_600 s.
        let ms = TcPosition::new(1, 0, 0, 0, TcRate::Fps30).to_millis();
        // Allow ±100 ms for integer arithmetic.
        assert!((3_599_800..=3_600_100).contains(&ms), "ms={ms}");
    }

    #[test]
    fn realtime_roundtrip_10_seconds() {
        let rate = TcRate::Fps25;
        let start_ms = 10_000u64;
        let frame = TcPosition::from_millis(start_ms, rate).to_frame_number();
        // 10 s × 25 fps = 250 frames exactly.
        assert_eq!(frame, 250);
    }
}
