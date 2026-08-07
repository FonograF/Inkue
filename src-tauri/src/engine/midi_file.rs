//! Standard MIDI File parsing and playback.
//!
//! Two halves, deliberately separate:
//!
//! - [`parse_midi_bytes`] flattens an SMF into a list of [`TimedEvent`]s whose
//!   times are absolute [`Duration`]s. This is where the **tempo map** is
//!   resolved: a Set Tempo meta event can appear anywhere in the file, so a
//!   tick has no fixed duration and times must be accumulated as the merged
//!   track is walked. Getting this wrong is the classic MIDI-file bug — the
//!   file plays at the right speed until the first tempo change and then
//!   drifts. Parsing is pure and takes bytes, so it is fully unit-testable.
//!
//! - [`MidiFilePlayer`] owns a background thread that sends those events to a
//!   `midir` output port at the right moments. It supports pause/resume and
//!   starting at an offset, and it always leaves the instrument quiet: every
//!   note it turned on is turned off again when it stops.
//!
//! The module knows nothing about cues — it is engine-layer, like the timecode
//! generator next to it.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use midly::{Format, MetaMessage, MidiMessage, Smf, Timing, TrackEventKind};
use thiserror::Error;

/// Sleep granularity is coarse on Windows; the last stretch before an event is
/// spent yielding instead so note timing is not quantised to the OS tick.
const SPIN_MARGIN: Duration = Duration::from_millis(1);

/// Longest single sleep. Caps how long a stop or pause takes to be noticed.
const MAX_SLEEP: Duration = Duration::from_millis(20);

/// Tempo assumed until the file says otherwise: 120 BPM, per the SMF spec.
const DEFAULT_US_PER_BEAT: f64 = 500_000.0;

/// Playback rate bounds. A rate of zero would stall the cue forever.
const MIN_RATE: f64 = 0.05;
const MAX_RATE: f64 = 20.0;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Everything that can go wrong loading or playing a MIDI file.
#[derive(Debug, Error)]
pub enum MidiFileError {
    #[error("cannot read MIDI file: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a valid MIDI file: {0}")]
    Parse(String),
    #[error("MIDI output port \"{0}\" not found")]
    PortNotFound(String),
    #[error("cannot open MIDI output port \"{0}\"")]
    PortUnavailable(String),
    #[error("cannot create a MIDI output client: {0}")]
    ClientUnavailable(String),
}

// ---------------------------------------------------------------------------
// Parsed sequence
// ---------------------------------------------------------------------------

/// What an event does, as far as playback control is concerned.
///
/// Used for two things: knowing which notes are sounding (so they can be
/// released on stop), and knowing which events must be replayed when playback
/// starts partway through the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    /// Starts a note sounding on `channel`.
    NoteOn { channel: u8, key: u8 },
    /// Releases a note (including a Note On with velocity 0).
    NoteOff { channel: u8, key: u8 },
    /// Sets persistent channel state — controller, program, pitch bend,
    /// channel pressure. Replayed when starting mid-file so the instrument is
    /// configured the way the file left it.
    ChannelState,
    /// Anything else: SysEx, polyphonic aftertouch, escaped bytes. Sent only
    /// when its own moment arrives.
    Other,
}

/// One MIDI message with the moment it is due, measured from the start of the
/// file at playback rate 1.0.
#[derive(Debug, Clone)]
pub struct TimedEvent {
    /// Offset from the start of the sequence.
    pub at: Duration,
    /// Raw bytes, ready to hand to `midir`.
    pub data: Vec<u8>,
    /// Classification used by note tracking and mid-file starts.
    pub kind: EventKind,
}

/// A whole MIDI file, flattened to a single time-ordered event list.
#[derive(Debug, Clone, Default)]
pub struct MidiSequence {
    /// Every playable event, ordered by [`TimedEvent::at`].
    pub events: Vec<TimedEvent>,
    /// Length of the file, including any silent tail before End of Track.
    pub duration: Duration,
    /// How many tracks the file contained.
    pub track_count: usize,
    /// Bit *n* set = channel *n* (0-based) carries at least one message.
    pub channels_used: u16,
}

impl MidiSequence {
    /// Channel numbers used by the file, 1-based for display.
    pub fn channel_numbers(&self) -> Vec<u8> {
        (0..16u8)
            .filter(|c| self.channels_used & (1 << c) != 0)
            .map(|c| c + 1)
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Read and parse a Standard MIDI File from disk.
pub fn parse_midi_file(path: &Path) -> Result<MidiSequence, MidiFileError> {
    let bytes = std::fs::read(path)?;
    parse_midi_bytes(&bytes)
}

/// Parse a Standard MIDI File held in memory.
///
/// Times are resolved against the file's tempo map, so the returned events are
/// wall-clock accurate even when the file changes tempo partway through.
pub fn parse_midi_bytes(bytes: &[u8]) -> Result<MidiSequence, MidiFileError> {
    let smf = Smf::parse(bytes).map_err(|e| MidiFileError::Parse(e.to_string()))?;
    let timing = smf.header.timing;

    // Format 2 tracks are independent sequences played one after another;
    // formats 0 and 1 share one timeline. Each format-2 track carries its own
    // tempo map, so it is converted on its own and then offset.
    let (events, duration) = match smf.header.format {
        Format::Sequential => {
            let mut events = Vec::new();
            let mut offset = Duration::ZERO;
            for track in &smf.tracks {
                let (mut track_events, track_duration) =
                    flatten_tracks(std::slice::from_ref(track), timing);
                for event in &mut track_events {
                    event.at += offset;
                }
                events.append(&mut track_events);
                offset += track_duration;
            }
            (events, offset)
        }
        _ => flatten_tracks(&smf.tracks, timing),
    };

    let channels_used = events.iter().fold(0u16, |acc, e| match e.kind {
        EventKind::NoteOn { channel, .. } | EventKind::NoteOff { channel, .. } => {
            acc | (1 << channel)
        }
        _ => acc,
    }) | channel_mask_from_status(&events);

    Ok(MidiSequence {
        events,
        duration,
        track_count: smf.tracks.len(),
        channels_used,
    })
}

/// Channels addressed by any channel-voice message, read back off the status
/// byte so controller-only channels are counted too.
fn channel_mask_from_status(events: &[TimedEvent]) -> u16 {
    events.iter().fold(0u16, |acc, e| {
        match e.data.first() {
            Some(status) if (0x80..0xF0).contains(status) => acc | (1 << (status & 0x0F)),
            _ => acc,
        }
    })
}

/// Merge `tracks` onto one timeline and convert ticks to wall-clock time.
///
/// Returns the playable events and the total length, which includes the
/// trailing End of Track delta — a file may hold silence after its last note
/// and that silence is part of the cue's duration.
fn flatten_tracks(tracks: &[midly::Track<'_>], timing: Timing) -> (Vec<TimedEvent>, Duration) {
    // Merge first: a tempo change in track 0 must apply to the events of every
    // other track that follow it in tick order, so time can only be
    // accumulated over the merged stream.
    let mut merged: Vec<(u64, usize, TrackEventKind<'_>)> = Vec::new();
    for (track_index, track) in tracks.iter().enumerate() {
        let mut tick = 0u64;
        for event in track.iter() {
            tick += u64::from(event.delta.as_int());
            merged.push((tick, track_index, event.kind));
        }
    }
    // Stable, so events sharing a tick keep their in-track order.
    merged.sort_by_key(|(tick, track_index, _)| (*tick, *track_index));

    let mut events = Vec::with_capacity(merged.len());
    let mut microseconds = 0.0_f64;
    let mut us_per_beat = DEFAULT_US_PER_BEAT;
    let mut previous_tick = 0u64;

    for (tick, _, kind) in merged {
        let delta_ticks = (tick - previous_tick) as f64;
        previous_tick = tick;
        microseconds += tick_span_us(delta_ticks, timing, us_per_beat);

        if let TrackEventKind::Meta(MetaMessage::Tempo(tempo)) = kind {
            // Only metrical files have a tempo map; SMPTE timing is absolute.
            if matches!(timing, Timing::Metrical(_)) {
                us_per_beat = f64::from(tempo.as_int());
            }
            continue;
        }

        if let Some((data, event_kind)) = encode_event(kind) {
            events.push(TimedEvent {
                at: Duration::from_micros(microseconds as u64),
                data,
                kind: event_kind,
            });
        }
    }

    (events, Duration::from_micros(microseconds as u64))
}

/// Wall-clock span of `delta_ticks` under the file's timing scheme.
fn tick_span_us(delta_ticks: f64, timing: Timing, us_per_beat: f64) -> f64 {
    match timing {
        Timing::Metrical(ticks_per_beat) => {
            let ticks_per_beat = f64::from(ticks_per_beat.as_int()).max(1.0);
            delta_ticks / ticks_per_beat * us_per_beat
        }
        // SMPTE timing: ticks are a fixed subdivision of a frame, so tempo
        // events have no effect on them at all.
        Timing::Timecode(fps, subframes_per_frame) => {
            let ticks_per_second = f64::from(fps.as_f32()) * f64::from(subframes_per_frame).max(1.0);
            if ticks_per_second <= 0.0 {
                0.0
            } else {
                delta_ticks / ticks_per_second * 1_000_000.0
            }
        }
    }
}

/// Turn a track event into wire bytes, or `None` if it is not transmitted
/// (meta events other than tempo, which the caller has already consumed).
fn encode_event(kind: TrackEventKind<'_>) -> Option<(Vec<u8>, EventKind)> {
    match kind {
        TrackEventKind::Midi { channel, message } => {
            let channel = channel.as_int();
            Some(encode_channel_message(channel, message))
        }
        // midly hands over the payload without the leading 0xF0; the trailing
        // 0xF7 is included.
        TrackEventKind::SysEx(payload) => {
            let mut data = Vec::with_capacity(payload.len() + 1);
            data.push(0xF0);
            data.extend_from_slice(payload);
            Some((data, EventKind::Other))
        }
        // An escape sequence is raw bytes the file wants sent verbatim.
        TrackEventKind::Escape(payload) => Some((payload.to_vec(), EventKind::Other)),
        TrackEventKind::Meta(_) => None,
    }
}

/// Wire bytes for one channel-voice message.
fn encode_channel_message(channel: u8, message: MidiMessage) -> (Vec<u8>, EventKind) {
    let channel = channel & 0x0F;
    match message {
        MidiMessage::NoteOff { key, vel } => (
            vec![0x80 | channel, key.as_int(), vel.as_int()],
            EventKind::NoteOff { channel, key: key.as_int() },
        ),
        MidiMessage::NoteOn { key, vel } => {
            // Note On with velocity 0 is the running-status way of writing
            // Note Off, and must be tracked as a release.
            let velocity = vel.as_int();
            let kind = if velocity == 0 {
                EventKind::NoteOff { channel, key: key.as_int() }
            } else {
                EventKind::NoteOn { channel, key: key.as_int() }
            };
            (vec![0x90 | channel, key.as_int(), velocity], kind)
        }
        MidiMessage::Aftertouch { key, vel } => (
            vec![0xA0 | channel, key.as_int(), vel.as_int()],
            EventKind::Other,
        ),
        MidiMessage::Controller { controller, value } => (
            vec![0xB0 | channel, controller.as_int(), value.as_int()],
            EventKind::ChannelState,
        ),
        MidiMessage::ProgramChange { program } => (
            vec![0xC0 | channel, program.as_int()],
            EventKind::ChannelState,
        ),
        MidiMessage::ChannelAftertouch { vel } => (
            vec![0xD0 | channel, vel.as_int()],
            EventKind::ChannelState,
        ),
        MidiMessage::PitchBend { bend } => {
            let value = bend.0.as_int();
            (
                vec![0xE0 | channel, (value & 0x7F) as u8, (value >> 7) as u8],
                EventKind::ChannelState,
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Port helpers
// ---------------------------------------------------------------------------

/// Open a `midir` connection to the output port called `port_name`.
fn connect(port_name: &str) -> Result<midir::MidiOutputConnection, MidiFileError> {
    let output = midir::MidiOutput::new("Inkue-MIDI-File")
        .map_err(|e| MidiFileError::ClientUnavailable(e.to_string()))?;
    let port = output
        .ports()
        .into_iter()
        .find(|p| output.port_name(p).ok().as_deref() == Some(port_name))
        .ok_or_else(|| MidiFileError::PortNotFound(port_name.to_string()))?;
    output
        .connect(&port, "inkue-midi-file")
        .map_err(|_| MidiFileError::PortUnavailable(port_name.to_string()))
}

// ---------------------------------------------------------------------------
// Playback
// ---------------------------------------------------------------------------

/// Where the playback thread sends its bytes.
///
/// A trait rather than a concrete `midir` connection so the scheduler — event
/// order, timing, what a stop leaves behind — can be tested without a MIDI
/// port, which no CI machine has and not every developer machine can loop back.
trait MidiSink {
    fn send(&mut self, data: &[u8]) -> Result<(), String>;
}

impl MidiSink for midir::MidiOutputConnection {
    fn send(&mut self, data: &[u8]) -> Result<(), String> {
        midir::MidiOutputConnection::send(self, data).map_err(|e| e.to_string())
    }
}

/// Shared flags the playback thread polls.
#[derive(Debug, Default)]
struct PlayerControl {
    stop: AtomicBool,
    paused: AtomicBool,
}

/// Handle to a MIDI file playing on a background thread.
///
/// Dropping the handle stops playback and releases every note the player had
/// turned on.
pub struct MidiFilePlayer {
    control: Arc<PlayerControl>,
}

impl MidiFilePlayer {
    /// Start playing `sequence` on `port_name` at `rate` times its written
    /// tempo, beginning `played_offset` in.
    ///
    /// `played_offset` is **played** time, not written time: at rate 2.0, an
    /// offset of 250 ms starts 500 ms into the file. Callers pass the position
    /// on the cue's own clock and let the rate apply once, here.
    ///
    /// Starting at a non-zero offset replays the channel state (controllers,
    /// programs, pitch bend) the file had established by that point, so the
    /// instrument sounds the way it should rather than reverting to defaults.
    /// Notes that were already sounding are not re-attacked.
    pub fn start(
        sequence: Arc<MidiSequence>,
        port_name: &str,
        rate: f64,
        played_offset: Duration,
        label: String,
    ) -> Result<Self, MidiFileError> {
        let connection = connect(port_name)?;
        let control = Arc::new(PlayerControl::default());
        let thread_control = Arc::clone(&control);
        let rate = rate.clamp(MIN_RATE, MAX_RATE);

        std::thread::Builder::new()
            .name("inkue-midi-file".into())
            .spawn(move || {
                play(sequence, connection, rate, played_offset, thread_control, label);
            })
            .map_err(MidiFileError::Io)?;

        Ok(Self { control })
    }

    /// Freeze or resume playback. Time spent paused does not advance the file.
    pub fn set_paused(&self, paused: bool) {
        self.control.paused.store(paused, Ordering::Relaxed);
    }
}

impl Drop for MidiFilePlayer {
    fn drop(&mut self) {
        self.control.stop.store(true, Ordering::Relaxed);
    }
}

/// Notes currently sounding, one 128-bit key mask per channel.
#[derive(Default)]
struct ActiveNotes([u128; 16]);

impl ActiveNotes {
    fn apply(&mut self, kind: EventKind) {
        match kind {
            EventKind::NoteOn { channel, key } => {
                self.0[channel as usize] |= 1u128 << key;
            }
            EventKind::NoteOff { channel, key } => {
                self.0[channel as usize] &= !(1u128 << key);
            }
            _ => {}
        }
    }

    /// `(channel, key)` for every note still down.
    fn sounding(&self) -> Vec<(u8, u8)> {
        let mut out = Vec::new();
        for (channel, mask) in self.0.iter().enumerate() {
            let mut bits = *mask;
            while bits != 0 {
                let key = bits.trailing_zeros() as u8;
                bits &= bits - 1;
                out.push((channel as u8, key));
            }
        }
        out
    }
}

/// The playback thread body.
fn play<S: MidiSink>(
    sequence: Arc<MidiSequence>,
    mut sink: S,
    rate: f64,
    played_offset: Duration,
    control: Arc<PlayerControl>,
    label: String,
) {
    // Windows' default 15.6 ms scheduler tick would quantise every note; ask
    // for 1 ms while this thread is alive.
    let _timer_resolution = TimerResolution::acquire();

    let mut active = ActiveNotes::default();

    // Catch up on the channel state the file had established before the
    // offset, then start the clock at the first event still due. Both sides of
    // the comparison are played time, so the rate applies exactly once.
    let start_index = sequence
        .events
        .partition_point(|e| scale(e.at, rate) < played_offset);
    for event in &sequence.events[..start_index] {
        if event.kind == EventKind::ChannelState {
            let _ = sink.send(&event.data);
        }
    }

    let mut origin = Instant::now() - played_offset;
    log::info!(
        "[midi-file] {label}: playing {} events at {rate}x from {played_offset:?}",
        sequence.events.len() - start_index,
    );

    for event in &sequence.events[start_index..] {
        if !wait_until(&mut origin, scale(event.at, rate), &control) {
            break;
        }
        if let Err(e) = sink.send(&event.data) {
            log::warn!("[midi-file] {label}: send failed: {e}");
            break;
        }
        active.apply(event.kind);
    }

    // Let the tail of the file play out: a note released on the last tick
    // still needs its silence honoured before the cue reports done.
    if !control.stop.load(Ordering::Relaxed) {
        wait_until(&mut origin, scale(sequence.duration, rate), &control);
    }

    silence(&mut sink, &active, sequence.channels_used);
    log::info!("[midi-file] {label}: stopped");
}

/// Convert a written offset to a played offset at `rate`.
fn scale(at: Duration, rate: f64) -> Duration {
    at.div_f64(rate)
}

/// Block until `origin + due`, absorbing pauses by pushing `origin` forward.
///
/// Returns `false` if the player was stopped while waiting.
fn wait_until(origin: &mut Instant, due: Duration, control: &PlayerControl) -> bool {
    loop {
        if control.stop.load(Ordering::Relaxed) {
            return false;
        }
        if control.paused.load(Ordering::Relaxed) {
            let paused_at = Instant::now();
            while control.paused.load(Ordering::Relaxed) {
                if control.stop.load(Ordering::Relaxed) {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            // The file did not advance while paused.
            *origin += paused_at.elapsed();
            continue;
        }

        let deadline = *origin + due;
        let now = Instant::now();
        if now >= deadline {
            return true;
        }
        let remaining = deadline - now;
        if remaining > SPIN_MARGIN {
            std::thread::sleep((remaining - SPIN_MARGIN).min(MAX_SLEEP));
        } else {
            // Inside the margin the OS timer is too coarse to help.
            std::thread::yield_now();
        }
    }
}

/// Leave the instrument quiet: release every note this player started, lift
/// the sustain pedal, and follow up with All Notes Off on the channels the
/// file touched, for gear that missed a release.
fn silence<S: MidiSink>(sink: &mut S, active: &ActiveNotes, channels_used: u16) {
    for (channel, key) in active.sounding() {
        let _ = sink.send(&[0x80 | channel, key, 0]);
    }
    for channel in 0..16u8 {
        if channels_used & (1 << channel) == 0 {
            continue;
        }
        let _ = sink.send(&[0xB0 | channel, 64, 0]); // sustain pedal up
        let _ = sink.send(&[0xB0 | channel, 123, 0]); // all notes off
    }
}

// ---------------------------------------------------------------------------
// Windows timer resolution
// ---------------------------------------------------------------------------

/// Raises the process timer resolution to 1 ms for as long as it is held.
///
/// Without this, `thread::sleep` on Windows rounds up to the 15.6 ms scheduler
/// tick and note timing audibly stutters. `winmm` is already linked (midir's
/// Windows backend uses it), so this costs nothing at build time. On other
/// platforms nanosleep is already fine-grained and the guard does nothing.
struct TimerResolution;

#[cfg(target_os = "windows")]
#[link(name = "winmm")]
extern "system" {
    fn timeBeginPeriod(period: u32) -> u32;
    fn timeEndPeriod(period: u32) -> u32;
}

impl TimerResolution {
    fn acquire() -> Self {
        #[cfg(target_os = "windows")]
        // SAFETY: timeBeginPeriod only adjusts this process's timer period and
        // is paired with timeEndPeriod in `Drop`.
        unsafe {
            timeBeginPeriod(1);
        }
        Self
    }
}

impl Drop for TimerResolution {
    fn drop(&mut self) {
        #[cfg(target_os = "windows")]
        // SAFETY: matches the timeBeginPeriod call in `acquire`.
        unsafe {
            timeEndPeriod(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Minimal SMF builder, so the tempo-map tests need no fixture files. --

    fn varlen(value: u32) -> Vec<u8> {
        let mut out = vec![(value & 0x7F) as u8];
        let mut rest = value >> 7;
        while rest > 0 {
            out.push(((rest & 0x7F) as u8) | 0x80);
            rest >>= 7;
        }
        out.reverse();
        out
    }

    /// `(delta_ticks, event_bytes)` pairs → a complete MTrk chunk.
    fn track(events: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let mut body = Vec::new();
        for (delta, bytes) in events {
            body.extend(varlen(*delta));
            body.extend_from_slice(bytes);
        }
        body.extend(varlen(0));
        body.extend_from_slice(&[0xFF, 0x2F, 0x00]); // End of Track

        let mut chunk = b"MTrk".to_vec();
        chunk.extend((body.len() as u32).to_be_bytes());
        chunk.extend(body);
        chunk
    }

    fn smf(format: u16, division: u16, tracks: &[Vec<u8>]) -> Vec<u8> {
        let mut out = b"MThd".to_vec();
        out.extend(6u32.to_be_bytes());
        out.extend(format.to_be_bytes());
        out.extend((tracks.len() as u16).to_be_bytes());
        out.extend(division.to_be_bytes());
        for t in tracks {
            out.extend_from_slice(t);
        }
        out
    }

    fn tempo(us_per_beat: u32) -> Vec<u8> {
        let b = us_per_beat.to_be_bytes();
        vec![0xFF, 0x51, 0x03, b[1], b[2], b[3]]
    }

    fn note_on(channel: u8, key: u8, velocity: u8) -> Vec<u8> {
        vec![0x90 | channel, key, velocity]
    }

    fn note_off(channel: u8, key: u8) -> Vec<u8> {
        vec![0x80 | channel, key, 0]
    }

    fn ms(duration: Duration) -> u64 {
        duration.as_millis() as u64
    }

    // -- Timing -----------------------------------------------------------

    #[test]
    fn absent_tempo_means_120_bpm() {
        // 480 ticks per beat, one beat between the two notes.
        let bytes = smf(
            0,
            480,
            &[track(&[(0, note_on(0, 60, 100)), (480, note_off(0, 60))])],
        );
        let seq = parse_midi_bytes(&bytes).unwrap();
        assert_eq!(ms(seq.events[0].at), 0);
        assert_eq!(ms(seq.events[1].at), 500, "one beat at 120 BPM is 500 ms");
    }

    #[test]
    fn a_tempo_change_midway_moves_every_later_event() {
        // This is the trap the whole module exists for: a tick is not a fixed
        // amount of time. Beat 1 at 120 BPM (500 ms), then 60 BPM for beat 2
        // (1000 ms) — the third note lands at 1500 ms, not 1000 ms.
        let bytes = smf(
            0,
            480,
            &[track(&[
                (0, note_on(0, 60, 100)),
                (480, note_on(0, 62, 100)),
                (0, tempo(1_000_000)),
                (480, note_on(0, 64, 100)),
            ])],
        );
        let seq = parse_midi_bytes(&bytes).unwrap();
        assert_eq!(ms(seq.events[0].at), 0);
        assert_eq!(ms(seq.events[1].at), 500);
        assert_eq!(ms(seq.events[2].at), 1500);
    }

    #[test]
    fn a_tempo_map_in_track_zero_applies_to_the_other_tracks() {
        // Format 1: conductor track holds the tempo, the music is elsewhere.
        let conductor = track(&[(0, tempo(1_000_000))]);
        let music = track(&[(0, note_on(0, 60, 100)), (480, note_on(0, 62, 100))]);
        let bytes = smf(1, 480, &[conductor, music]);
        let seq = parse_midi_bytes(&bytes).unwrap();
        assert_eq!(ms(seq.events[1].at), 1000, "60 BPM from the conductor track");
    }

    #[test]
    fn smpte_timing_ignores_tempo_events() {
        // Division 0xE728 = -25 fps, 40 subframes → 1000 ticks per second.
        let bytes = smf(
            0,
            0xE728,
            &[track(&[
                (0, tempo(1_000_000)),
                (1000, note_on(0, 60, 100)),
            ])],
        );
        let seq = parse_midi_bytes(&bytes).unwrap();
        assert_eq!(ms(seq.events[0].at), 1000, "SMPTE ticks are absolute");
    }

    #[test]
    fn format_2_tracks_play_one_after_another() {
        let first = track(&[(0, note_on(0, 60, 100)), (480, note_off(0, 60))]);
        let second = track(&[(0, note_on(0, 62, 100))]);
        let bytes = smf(2, 480, &[first, second]);
        let seq = parse_midi_bytes(&bytes).unwrap();
        // Track 1 is 500 ms long, so track 2 opens there.
        assert_eq!(ms(seq.events[2].at), 500);
        assert_eq!(seq.track_count, 2);
    }

    #[test]
    fn duration_includes_a_silent_tail() {
        // Two beats of nothing after the last note still belong to the cue.
        let bytes = smf(0, 480, &[track(&[(0, note_on(0, 60, 100)), (960, note_off(0, 60))])]);
        let seq = parse_midi_bytes(&bytes).unwrap();
        assert_eq!(ms(seq.duration), 1000);
    }

    #[test]
    fn events_from_parallel_tracks_come_out_in_time_order() {
        let a = track(&[(240, note_on(0, 60, 100))]);
        let b = track(&[(0, note_on(1, 62, 100)), (480, note_on(1, 64, 100))]);
        let bytes = smf(1, 480, &[a, b]);
        let seq = parse_midi_bytes(&bytes).unwrap();
        let times: Vec<u64> = seq.events.iter().map(|e| ms(e.at)).collect();
        assert_eq!(times, vec![0, 250, 500]);
    }

    // -- Encoding ---------------------------------------------------------

    #[test]
    fn note_on_with_zero_velocity_counts_as_a_release() {
        let bytes = smf(0, 480, &[track(&[(0, note_on(3, 60, 100)), (10, note_on(3, 60, 0))])]);
        let seq = parse_midi_bytes(&bytes).unwrap();
        assert_eq!(seq.events[0].kind, EventKind::NoteOn { channel: 3, key: 60 });
        assert_eq!(seq.events[1].kind, EventKind::NoteOff { channel: 3, key: 60 });
    }

    #[test]
    fn controllers_and_programs_are_channel_state() {
        let bytes = smf(
            0,
            480,
            &[track(&[
                (0, vec![0xB0, 7, 100]),  // CC volume
                (0, vec![0xC1, 42]),      // program change
                (0, vec![0xE0, 0x00, 0x40]), // pitch bend centre
                (0, note_on(0, 60, 100)),
            ])],
        );
        let seq = parse_midi_bytes(&bytes).unwrap();
        assert_eq!(seq.events[0].kind, EventKind::ChannelState);
        assert_eq!(seq.events[1].kind, EventKind::ChannelState);
        assert_eq!(seq.events[2].kind, EventKind::ChannelState);
        assert_eq!(seq.events[3].kind, EventKind::NoteOn { channel: 0, key: 60 });
        assert_eq!(seq.events[1].data, vec![0xC1, 42], "program change is two bytes");
    }

    #[test]
    fn sysex_is_reframed_with_its_leading_status_byte() {
        // 0xF0 <varlen len> payload… — midly strips the 0xF0, we put it back.
        let bytes = smf(0, 480, &[track(&[(0, vec![0xF0, 0x03, 0x7E, 0x7F, 0xF7])])]);
        let seq = parse_midi_bytes(&bytes).unwrap();
        assert_eq!(seq.events[0].data, vec![0xF0, 0x7E, 0x7F, 0xF7]);
        assert_eq!(seq.events[0].kind, EventKind::Other);
    }

    #[test]
    fn channels_used_covers_notes_and_controllers() {
        let bytes = smf(
            0,
            480,
            &[track(&[(0, note_on(0, 60, 100)), (0, vec![0xB9, 7, 90])])],
        );
        let seq = parse_midi_bytes(&bytes).unwrap();
        assert_eq!(seq.channel_numbers(), vec![1, 10]);
    }

    #[test]
    fn a_file_that_is_not_midi_is_rejected() {
        assert!(parse_midi_bytes(b"this is not a MIDI file").is_err());
    }

    // -- Runtime helpers ---------------------------------------------------

    #[test]
    fn active_notes_track_what_is_sounding() {
        let mut active = ActiveNotes::default();
        active.apply(EventKind::NoteOn { channel: 0, key: 60 });
        active.apply(EventKind::NoteOn { channel: 9, key: 36 });
        active.apply(EventKind::NoteOn { channel: 0, key: 64 });
        active.apply(EventKind::NoteOff { channel: 0, key: 60 });
        assert_eq!(active.sounding(), vec![(0, 64), (9, 36)]);
    }

    #[test]
    fn playback_rate_scales_the_timeline() {
        assert_eq!(ms(scale(Duration::from_millis(1000), 2.0)), 500);
        assert_eq!(ms(scale(Duration::from_millis(1000), 0.5)), 2000);
    }

    // -- The waiting clock -------------------------------------------------

    #[test]
    fn waiting_returns_once_the_moment_arrives() {
        let control = PlayerControl::default();
        let mut origin = Instant::now();
        let started = Instant::now();
        assert!(wait_until(&mut origin, Duration::from_millis(30), &control));
        assert!(started.elapsed() >= Duration::from_millis(28), "did not actually wait");
    }

    #[test]
    fn a_stop_cuts_the_wait_short() {
        let control = Arc::new(PlayerControl::default());
        let stopper = Arc::clone(&control);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            stopper.stop.store(true, Ordering::Relaxed);
        });

        let mut origin = Instant::now();
        let started = Instant::now();
        // An hour away: only the stop can end this wait.
        assert!(!wait_until(&mut origin, Duration::from_secs(3600), &control));
        assert!(started.elapsed() < Duration::from_secs(2), "stop was not noticed");
    }

    #[test]
    fn a_pause_pushes_the_deadline_back_by_the_time_spent_paused() {
        // The file must not advance while paused: a 40 ms pause on a 30 ms
        // wait has to come out at ~70 ms, not 40.
        let control = Arc::new(PlayerControl::default());
        control.paused.store(true, Ordering::Relaxed);
        let resumer = Arc::clone(&control);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(40));
            resumer.paused.store(false, Ordering::Relaxed);
        });

        let mut origin = Instant::now();
        let started = Instant::now();
        assert!(wait_until(&mut origin, Duration::from_millis(30), &control));
        let waited = started.elapsed();
        assert!(
            waited >= Duration::from_millis(65),
            "paused time was counted against the file: waited {waited:?}"
        );
    }

    #[test]
    fn an_event_already_past_due_is_sent_at_once() {
        let control = PlayerControl::default();
        // Origin an hour ago: everything in the file is overdue.
        let mut origin = Instant::now() - Duration::from_secs(3600);
        let started = Instant::now();
        assert!(wait_until(&mut origin, Duration::from_millis(500), &control));
        assert!(started.elapsed() < Duration::from_millis(50));
    }

    // -- The scheduler, end to end ----------------------------------------
    //
    // These drive the real `play` loop against a recording sink, so ordering,
    // timing, mid-file starts and what a stop leaves behind are covered
    // without a MIDI port.

    use std::sync::Mutex;

    /// `(milliseconds since the player started, message bytes)`.
    type SentLog = Arc<Mutex<Vec<(u64, Vec<u8>)>>>;

    #[derive(Clone)]
    struct RecordingSink {
        started: Instant,
        log: SentLog,
    }

    impl MidiSink for RecordingSink {
        fn send(&mut self, data: &[u8]) -> Result<(), String> {
            self.log
                .lock()
                .unwrap()
                .push((self.started.elapsed().as_millis() as u64, data.to_vec()));
            Ok(())
        }
    }

    struct Running {
        control: Arc<PlayerControl>,
        log: SentLog,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl Running {
        fn messages(&self) -> Vec<Vec<u8>> {
            self.log.lock().unwrap().iter().map(|(_, m)| m.clone()).collect()
        }
        fn timed(&self) -> Vec<(u64, Vec<u8>)> {
            self.log.lock().unwrap().clone()
        }
        fn stop_and_join(&mut self) {
            self.control.stop.store(true, Ordering::Relaxed);
            if let Some(h) = self.handle.take() {
                h.join().unwrap();
            }
        }
    }

    fn run(bytes: &[u8], rate: f64, offset: Duration) -> Running {
        let sequence = Arc::new(parse_midi_bytes(bytes).unwrap());
        let log = Arc::new(Mutex::new(Vec::new()));
        let sink = RecordingSink { started: Instant::now(), log: Arc::clone(&log) };
        let control = Arc::new(PlayerControl::default());
        let thread_control = Arc::clone(&control);
        let handle = std::thread::spawn(move || {
            play(sequence, sink, rate, offset, thread_control, "test".into());
        });
        Running { control, log, handle: Some(handle) }
    }

    /// Timing assertions have to survive a loaded CI box; ±70 ms is loose
    /// enough not to flake and tight enough to catch a real scheduling bug.
    fn assert_near(actual: u64, expected: u64) {
        let drift = actual.abs_diff(expected);
        assert!(drift <= 70, "expected ~{expected} ms, got {actual} ms");
    }

    #[test]
    fn events_are_sent_in_order_at_their_written_times() {
        // 480 tpqn at 120 BPM → 96 ticks = 100 ms.
        let bytes = smf(
            0,
            480,
            &[track(&[
                (0, note_on(0, 60, 100)),
                (96, note_on(0, 62, 100)),
                (96, note_on(0, 64, 100)),
            ])],
        );
        let mut running = run(&bytes, 1.0, Duration::ZERO);
        std::thread::sleep(Duration::from_millis(350));
        running.stop_and_join();

        let timed = running.timed();
        assert_eq!(timed[0].1, vec![0x90, 60, 100]);
        assert_eq!(timed[1].1, vec![0x90, 62, 100]);
        assert_eq!(timed[2].1, vec![0x90, 64, 100]);
        assert_near(timed[0].0, 0);
        assert_near(timed[1].0, 100);
        assert_near(timed[2].0, 200);
    }

    #[test]
    fn the_playback_rate_compresses_the_wall_clock() {
        let bytes = smf(0, 480, &[track(&[(0, note_on(0, 60, 100)), (192, note_on(0, 62, 100))])]);
        let mut running = run(&bytes, 2.0, Duration::ZERO);
        std::thread::sleep(Duration::from_millis(300));
        running.stop_and_join();

        // 200 ms as written, 100 ms at 2×.
        assert_near(running.timed()[1].0, 100);
    }

    #[test]
    fn stopping_mid_file_releases_every_note_it_had_started() {
        // A note and a sustain pedal that the file only lifts after 5 s.
        let bytes = smf(
            0,
            480,
            &[track(&[
                (0, note_on(0, 60, 100)),
                (0, vec![0xB0, 64, 127]),
                (4800, note_off(0, 60)),
            ])],
        );
        let mut running = run(&bytes, 1.0, Duration::ZERO);
        std::thread::sleep(Duration::from_millis(120));
        running.stop_and_join();

        let messages = running.messages();
        assert_eq!(messages[0], vec![0x90, 60, 100]);
        assert_eq!(messages[1], vec![0xB0, 64, 127]);
        // On the way out: the sounding note released, pedal up, all notes off.
        assert_eq!(messages[2], vec![0x80, 60, 0], "the held note must be released");
        assert_eq!(messages[3], vec![0xB0, 64, 0], "the sustain pedal must come up");
        assert_eq!(messages[4], vec![0xB0, 123, 0]);
        assert_eq!(messages.len(), 5, "untouched channels are left alone");
    }

    #[test]
    fn a_finished_file_still_gets_its_notes_released() {
        // Nothing releases note 60 before End of Track — the player must.
        let bytes = smf(0, 480, &[track(&[(0, note_on(2, 60, 100)), (48, note_on(2, 62, 100))])]);
        let mut running = run(&bytes, 1.0, Duration::ZERO);
        std::thread::sleep(Duration::from_millis(250));

        let messages = running.messages();
        assert!(messages.contains(&vec![0x82, 60, 0]));
        assert!(messages.contains(&vec![0x82, 62, 0]));
        running.stop_and_join();
    }

    #[test]
    fn starting_partway_replays_channel_state_but_not_past_notes() {
        // Editing a playing cue restarts the player at its current position.
        // The instrument must keep its program and volume; the note that was
        // already sounding is not re-attacked.
        let bytes = smf(
            0,
            480,
            &[track(&[
                (0, vec![0xC0, 42]),        // program change
                (0, vec![0xB0, 7, 90]),     // volume
                (0, note_on(0, 60, 100)),   // already sounding at the offset
                (192, note_on(0, 67, 100)), // 200 ms — still to come
            ])],
        );
        let mut running = run(&bytes, 1.0, Duration::from_millis(100));
        std::thread::sleep(Duration::from_millis(250));
        running.stop_and_join();

        let messages = running.messages();
        assert_eq!(messages[0], vec![0xC0, 42], "program is restored");
        assert_eq!(messages[1], vec![0xB0, 7, 90], "volume is restored");
        assert_eq!(messages[2], vec![0x90, 67, 100], "playback resumes at the offset");
        assert!(
            !messages[..3].contains(&vec![0x90, 60, 100]),
            "a note already sounding is not re-attacked"
        );
    }

    #[test]
    fn the_start_offset_is_played_time_so_the_rate_applies_once() {
        // Regression: the offset was being pre-multiplied by the rate before
        // reaching the player, which then scaled it again — at 2x, seeking to
        // 250 ms landed 1000 ms into the file instead of 500 ms.
        //
        // Notes every 250 written ms; at 2x those are 0, 125, 250, 375 played.
        // Starting 250 ms in must therefore open on the third note.
        let bytes = smf(
            0,
            480,
            &[track(&[
                (0, note_on(0, 60, 100)),
                (240, note_on(0, 62, 100)),
                (240, note_on(0, 64, 100)),
                (240, note_on(0, 66, 100)),
            ])],
        );
        let mut running = run(&bytes, 2.0, Duration::from_millis(250));
        std::thread::sleep(Duration::from_millis(250));
        running.stop_and_join();

        let messages = running.messages();
        assert_eq!(messages[0], vec![0x90, 64, 100], "resumed at the third note");
        assert_eq!(messages[1], vec![0x90, 66, 100]);
    }

    #[test]
    fn pausing_holds_the_file_where_it_is() {
        let bytes = smf(0, 480, &[track(&[(0, note_on(0, 60, 100)), (96, note_on(0, 62, 100))])]);
        let mut running = run(&bytes, 1.0, Duration::ZERO);
        // Let the event due at 0 go out, then freeze 40 ms into the file.
        std::thread::sleep(Duration::from_millis(40));
        running.control.paused.store(true, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(running.messages().len(), 1, "the file must not advance while paused");

        running.control.paused.store(false, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(200));
        // 100 ms of file remained, so the second note lands ~300 ms in.
        assert_near(running.timed()[1].0, 300);
        running.stop_and_join();
    }

    // -- Port errors -------------------------------------------------------

    #[test]
    fn starting_on_a_port_that_is_not_there_is_an_error_not_a_panic() {
        let result = MidiFilePlayer::start(
            Arc::new(MidiSequence::default()),
            "No Such Port 8f3a",
            1.0,
            Duration::ZERO,
            "test".into(),
        );
        // Headless CI may have no MIDI client at all; either way it must fail
        // cleanly rather than spawn a thread that goes nowhere.
        assert!(result.is_err());
    }
}
