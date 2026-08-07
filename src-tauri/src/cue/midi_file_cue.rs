//! [`MidiFileCue`] — plays a Standard MIDI File out to a MIDI port.
//!
//! QLab's MIDI File cue, with the same two knobs: a destination port and a
//! playback-rate multiplier applied to every tempo in the file.
//!
//! The file is parsed as soon as the cue knows its path (files are kilobytes;
//! there is nothing to background), which gives the cue a real duration up
//! front — so it completes on its own, drives the progress bar, and chains
//! Auto-Follow like an Audio Cue. Sending is done by
//! [`MidiFilePlayer`](crate::engine::midi_file::MidiFilePlayer) on its own
//! thread; the cue only owns the handle.
//!
//! Two behaviours are worth knowing:
//!
//! - **Stopping never leaves a note hanging.** The player releases every note
//!   it started and lifts the sustain pedal. A MIDI file cut mid-chord would
//!   otherwise drone until the instrument is power-cycled.
//! - **Editing a playing cue does not silence it.** Every inspector edit
//!   rebuilds the cue from JSON, which drops the player thread; the rebuilt cue
//!   restarts playback at the position the old one had reached, replaying the
//!   channel state the file had established by then.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::engine::midi_file::{parse_midi_file, MidiFilePlayer, MidiSequence};

use super::{
    context::{CueContext, CueEvent},
    traits::{Cue, CueFactory, RuntimeState},
    types::{ContinueMode, CueColor, CueId, CueState, CueType},
};

/// Playback-rate bounds, matching the engine's clamp.
const MIN_RATE: f64 = 0.05;
const MAX_RATE: f64 = 20.0;

/// A cue that plays a `.mid` file to a MIDI output port.
pub struct MidiFileCue {
    id: CueId,
    name: String,
    number: Option<String>,
    notes: String,
    color: CueColor,
    state: CueState,
    continue_mode: ContinueMode,
    pre_wait: Duration,
    post_wait: Duration,
    started_at: Option<Instant>,
    action_started_at: Option<Instant>,
    elapsed_before_pause: Duration,
    action_elapsed_before_pause: Duration,
    in_pre_wait: bool,
    auto_continue_fired: bool,
    is_disabled: bool,

    /// Path to the `.mid` file. `None` = nothing to play.
    pub file_path: Option<PathBuf>,
    /// Name of the MIDI output port every message is sent to. QLab sends a
    /// whole file to one destination; channels inside the file do the routing.
    pub port_name: String,
    /// Multiplier on every tempo in the file. 0.5 = half speed.
    pub playback_rate: f64,

    /// The parsed file, shared with the player thread.
    sequence: Option<Arc<MidiSequence>>,
    /// Why parsing failed, for preflight to report.
    parse_error: Option<String>,
    /// Live playback. Dropping it stops the thread and releases held notes.
    player: Option<MidiFilePlayer>,
}

impl MidiFileCue {
    /// Create a new, empty MIDI File Cue with a fresh UUID.
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::from("MIDI File"),
            number: None,
            notes: String::new(),
            color: CueColor::White,
            state: CueState::Standby,
            continue_mode: ContinueMode::DoNotContinue,
            pre_wait: Duration::ZERO,
            post_wait: Duration::ZERO,
            started_at: None,
            action_started_at: None,
            elapsed_before_pause: Duration::ZERO,
            action_elapsed_before_pause: Duration::ZERO,
            in_pre_wait: false,
            auto_continue_fired: false,
            is_disabled: false,
            file_path: None,
            port_name: String::new(),
            playback_rate: 1.0,
            sequence: None,
            parse_error: None,
            player: None,
        }
    }

    /// Parse the assigned file, replacing whatever was loaded before.
    ///
    /// A failure is remembered rather than returned: a workspace with a broken
    /// file must still open, with preflight pointing at the cue.
    pub fn reload_sequence(&mut self) {
        self.sequence = None;
        self.parse_error = None;
        let Some(path) = self.file_path.as_deref() else {
            return;
        };
        if path.as_os_str().is_empty() {
            return;
        }
        match parse_midi_file(path) {
            Ok(sequence) => self.sequence = Some(Arc::new(sequence)),
            Err(e) => {
                log::warn!("MidiFileCue '{}': {e}", self.name);
                self.parse_error = Some(e.to_string());
            }
        }
    }

    /// The rate actually used, guarded against a nonsensical stored value.
    fn effective_rate(&self) -> f64 {
        if self.playback_rate.is_finite() {
            self.playback_rate.clamp(MIN_RATE, MAX_RATE)
        } else {
            1.0
        }
    }

    /// Start the player thread `played_offset` into the cue.
    ///
    /// The offset is **played** time, not written time — the same clock as
    /// `action_elapsed` and [`duration`](Cue::duration) — so the rate must not
    /// be applied to it here; the player already scales the file against it.
    fn start_player(&mut self, played_offset: Duration) {
        let Some(sequence) = self.sequence.clone() else {
            return;
        };
        let alert_key = format!("midi-file:{}", self.id);
        match MidiFilePlayer::start(
            sequence,
            &self.port_name,
            self.effective_rate(),
            played_offset,
            self.name.clone(),
        ) {
            Ok(player) => {
                self.player = Some(player);
                crate::health::clear(&alert_key);
            }
            Err(e) => {
                // The cue still runs for its full duration so Auto-Follow
                // chains behave; the operator gets a banner saying why the
                // stage is silent.
                log::warn!("MidiFileCue '{}': {e}", self.name);
                crate::health::set(crate::health::HealthAlert::new(
                    &alert_key,
                    crate::health::HealthLevel::Error,
                    format!("MIDI File Cue \"{}\": {e}", self.name),
                ));
            }
        }
    }

    fn start_action(&mut self, context: &CueContext) {
        self.action_started_at = Some(Instant::now());
        self.in_pre_wait = false;
        self.start_player(Duration::ZERO);
        context.emit(CueEvent::ActionStarted { cue_id: self.id });
    }

    /// Clear all playback and timing state. Shared by stop / hard stop / reset.
    fn clear_runtime(&mut self) {
        self.player = None;
        self.state = CueState::Standby;
        self.started_at = None;
        self.action_started_at = None;
        self.elapsed_before_pause = Duration::ZERO;
        self.action_elapsed_before_pause = Duration::ZERO;
        self.in_pre_wait = false;
        self.auto_continue_fired = false;
    }
}

impl Default for MidiFileCue {
    fn default() -> Self {
        Self::new()
    }
}

impl Cue for MidiFileCue {
    fn id(&self) -> CueId { self.id }
    fn cue_type(&self) -> CueType { CueType::MidiFile }
    fn name(&self) -> &str { &self.name }
    fn set_name(&mut self, name: String) { self.name = name; }
    fn number(&self) -> Option<&str> { self.number.as_deref() }
    fn set_number(&mut self, number: Option<String>) { self.number = number; }
    fn notes(&self) -> &str { &self.notes }
    fn set_notes(&mut self, notes: String) { self.notes = notes; }
    fn color(&self) -> CueColor { self.color }
    fn set_color(&mut self, color: CueColor) { self.color = color; }
    fn is_disabled(&self) -> bool { self.is_disabled }
    fn set_disabled(&mut self, d: bool) { self.is_disabled = d; }
    fn state(&self) -> CueState { self.state }

    fn media_file_path(&self) -> Option<&Path> {
        self.file_path.as_deref()
    }

    fn load(&mut self, _context: &CueContext) -> Result<()> {
        if self.sequence.is_none() && self.parse_error.is_none() {
            self.reload_sequence();
        }
        Ok(())
    }

    fn go(&mut self, context: &CueContext) -> Result<()> {
        if self.state == CueState::Running {
            return Ok(());
        }
        self.player = None;
        self.auto_continue_fired = false;
        self.elapsed_before_pause = Duration::ZERO;
        self.action_elapsed_before_pause = Duration::ZERO;
        self.state = CueState::Running;
        self.started_at = Some(Instant::now());

        if !self.pre_wait.is_zero() {
            self.in_pre_wait = true;
            return Ok(());
        }
        self.start_action(context);
        Ok(())
    }

    fn stop(&mut self, context: &CueContext) -> Result<()> {
        // MIDI has no fade: soft and hard stop are the same cut, and the
        // player releases every note it started on the way out.
        self.clear_runtime();
        context.emit(CueEvent::Stopped { cue_id: self.id });
        Ok(())
    }

    fn hard_stop(&mut self, context: &CueContext) -> Result<()> {
        self.stop(context)
    }

    fn pause(&mut self, _context: &CueContext) -> Result<()> {
        if self.state != CueState::Running || self.in_pre_wait {
            return Ok(());
        }
        if let Some(player) = &self.player {
            player.set_paused(true);
        }
        if let Some(t) = self.started_at.take() {
            self.elapsed_before_pause = t.elapsed();
        }
        if let Some(t) = self.action_started_at.take() {
            self.action_elapsed_before_pause = t.elapsed();
        }
        self.state = CueState::Paused;
        Ok(())
    }

    fn resume(&mut self, _context: &CueContext) -> Result<()> {
        if self.state != CueState::Paused {
            return Ok(());
        }
        if let Some(player) = &self.player {
            player.set_paused(false);
        }
        self.started_at = Some(Instant::now() - self.elapsed_before_pause);
        self.action_started_at = Some(Instant::now() - self.action_elapsed_before_pause);
        self.state = CueState::Running;
        Ok(())
    }

    fn seek(&mut self, position_ms: u64, _ctx: &CueContext) {
        if self.action_started_at.is_none() && self.state != CueState::Paused {
            return;
        }
        let position = Duration::from_millis(position_ms);
        // Restarting the player is the seek: it replays the channel state the
        // file had reached and picks the timeline up from there.
        self.player = None;
        self.start_player(position);
        if self.state == CueState::Paused {
            if let Some(player) = &self.player {
                player.set_paused(true);
            }
            self.action_elapsed_before_pause = position;
            self.elapsed_before_pause = self.pre_wait + position;
        } else {
            self.action_started_at = Some(Instant::now() - position);
            self.started_at = Some(Instant::now() - self.pre_wait - position);
        }
    }

    fn reset(&mut self) -> Result<()> {
        self.clear_runtime();
        Ok(())
    }

    fn tick(&mut self, context: &CueContext) -> Result<()> {
        if self.in_pre_wait && self.elapsed() >= self.pre_wait {
            self.start_action(context);
        }
        Ok(())
    }

    fn is_action_started(&self) -> bool {
        !self.in_pre_wait
    }

    fn pre_wait(&self) -> Duration { self.pre_wait }
    fn set_pre_wait(&mut self, d: Duration) { self.pre_wait = d; }
    fn post_wait(&self) -> Duration { self.post_wait }
    fn set_post_wait(&mut self, d: Duration) { self.post_wait = d; }

    /// How long the cue runs: the file's own length, stretched by the rate.
    fn duration(&self) -> Option<Duration> {
        let sequence = self.sequence.as_ref()?;
        Some(sequence.duration.div_f64(self.effective_rate()))
    }

    /// The file's length as written, ignoring the playback rate.
    fn file_duration(&self) -> Option<Duration> {
        self.sequence.as_ref().map(|s| s.duration)
    }

    fn elapsed(&self) -> Duration {
        match self.state {
            CueState::Running => self
                .started_at
                .map(|t| t.elapsed())
                .unwrap_or(Duration::ZERO),
            CueState::Paused => self.elapsed_before_pause,
            _ => Duration::ZERO,
        }
    }

    fn action_elapsed(&self) -> Duration {
        match self.state {
            CueState::Running => self
                .action_started_at
                .map(|t| t.elapsed())
                .unwrap_or(Duration::ZERO),
            CueState::Paused => self.action_elapsed_before_pause,
            _ => Duration::ZERO,
        }
    }

    fn continue_mode(&self) -> ContinueMode { self.continue_mode }
    fn set_continue_mode(&mut self, mode: ContinueMode) { self.continue_mode = mode; }

    fn is_auto_continue_fired(&self) -> bool { self.auto_continue_fired }
    fn mark_auto_continue_fired(&mut self) { self.auto_continue_fired = true; }
    fn clear_auto_continue_fired(&mut self) { self.auto_continue_fired = false; }

    fn validate(
        &self,
        ctx: &crate::cue::validation::ValidationContext,
    ) -> Vec<crate::cue::validation::CueIssue> {
        use crate::cue::validation::CueIssue;
        let mut issues = Vec::new();

        match self.file_path.as_deref() {
            None => issues.push(CueIssue::warning("No MIDI file assigned")),
            Some(p) if p.as_os_str().is_empty() => {
                issues.push(CueIssue::warning("No MIDI file assigned"))
            }
            // A missing file is reported centrally via `media_file_path`; only
            // a file that exists but will not parse is ours to flag.
            Some(p) if p.exists() => {
                if let Some(e) = &self.parse_error {
                    issues.push(CueIssue::error(e.clone()));
                }
            }
            Some(_) => {}
        }

        if self.port_name.is_empty() {
            issues.push(CueIssue::warning("MIDI port not configured"));
        } else if !ctx.midi_ports.iter().any(|p| p == &self.port_name) {
            issues.push(CueIssue::error(format!(
                "MIDI port not available: \"{}\"",
                self.port_name
            )));
        }

        if !self.playback_rate.is_finite()
            || self.playback_rate < MIN_RATE
            || self.playback_rate > MAX_RATE
        {
            issues.push(CueIssue::warning(format!(
                "Playback rate outside {MIN_RATE}–{MAX_RATE}; it will be clamped"
            )));
        }

        issues
    }

    fn runtime_state(&self) -> RuntimeState {
        RuntimeState {
            state: self.state,
            voice_id: None,
            started_at: self.started_at,
            action_started_at: self.action_started_at,
        }
    }

    fn restore_runtime_state(&mut self, snap: RuntimeState) {
        self.state = snap.state;
        self.started_at = snap.started_at;
        self.action_started_at = snap.action_started_at;
        self.in_pre_wait = snap.state == CueState::Running && snap.action_started_at.is_none();

        // The player thread cannot be handed to the rebuilt cue, so it is
        // restarted where the old one had got to. Without this, renaming a cue
        // mid-show would silence it.
        if snap.state == CueState::Running && !self.in_pre_wait {
            let position = snap
                .action_started_at
                .map(|t| t.elapsed())
                .unwrap_or(Duration::ZERO);
            self.start_player(position);
        }
    }

    fn serialize(&self) -> Value {
        json!({
            "type": "midi_file",
            "cue_type": "midi_file",
            "id": self.id,
            "number": self.number,
            "name": self.name,
            "notes": self.notes,
            "color": self.color,
            "pre_wait_ms": self.pre_wait.as_millis() as u64,
            "post_wait_ms": self.post_wait.as_millis() as u64,
            "continue_mode": self.continue_mode,
            "is_disabled": self.is_disabled,
            // `file_path` is the key the workspace layer relativises on save
            // and resolves on load — the name matters.
            "file_path": self.file_path.as_ref().map(|p| p.to_string_lossy().into_owned()),
            "port_name": self.port_name,
            "playback_rate": self.playback_rate,
            // Read-only, for the inspector: proof of what was actually parsed.
            // `from_json` ignores them and re-derives them from the file.
            "sequence_duration_ms": self.sequence.as_ref().map(|s| s.duration.as_millis() as u64),
            "track_count": self.sequence.as_ref().map(|s| s.track_count),
            "channels": self.sequence.as_ref().map(|s| s.channel_numbers()),
            "parse_error": self.parse_error,
        })
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Factory for [`MidiFileCue`].
pub struct MidiFileCueFactory;

impl CueFactory for MidiFileCueFactory {
    fn create(&self) -> Box<dyn Cue> {
        Box::new(MidiFileCue::new())
    }

    fn from_json(&self, value: Value) -> Result<Box<dyn Cue>> {
        let mut cue = MidiFileCue::new();

        if let Some(s) = value.get("id").and_then(|v| v.as_str()) {
            cue.id = s.parse().unwrap_or_else(|_| Uuid::new_v4());
        }
        if let Some(s) = value.get("name").and_then(|v| v.as_str()) {
            cue.name = s.to_string();
        }
        if let Some(s) = value.get("number").and_then(|v| v.as_str()) {
            cue.number = Some(s.to_string());
        }
        if let Some(s) = value.get("notes").and_then(|v| v.as_str()) {
            cue.notes = s.to_string();
        }
        if let Some(ms) = value.get("pre_wait_ms").and_then(|v| v.as_u64()) {
            cue.pre_wait = Duration::from_millis(ms);
        }
        if let Some(ms) = value.get("post_wait_ms").and_then(|v| v.as_u64()) {
            cue.post_wait = Duration::from_millis(ms);
        }
        if let Some(cm) = value.get("continue_mode") {
            if let Ok(mode) = serde_json::from_value(cm.clone()) {
                cue.continue_mode = mode;
            }
        }
        if let Some(col) = value.get("color") {
            if let Ok(color) = serde_json::from_value(col.clone()) {
                cue.color = color;
            }
        }
        if let Some(b) = value.get("is_disabled").and_then(|v| v.as_bool()) {
            cue.is_disabled = b;
        }
        if let Some(s) = value.get("file_path").and_then(|v| v.as_str()) {
            if !s.is_empty() {
                cue.file_path = Some(PathBuf::from(s));
            }
        }
        if let Some(s) = value.get("port_name").and_then(|v| v.as_str()) {
            cue.port_name = s.to_string();
        }
        if let Some(r) = value.get("playback_rate").and_then(|v| v.as_f64()) {
            cue.playback_rate = r;
        }

        // Parse now: the cue list needs a duration for the row the moment the
        // workspace opens, and a MIDI file is small enough that there is
        // nothing to gain from deferring it.
        cue.reload_sequence();

        Ok(Box::new(cue))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cue::validation::{Severity, ValidationContext};
    use std::collections::HashSet;

    fn ctx_with_ports(ports: &[&str]) -> ValidationContext {
        ValidationContext {
            all_cue_ids: HashSet::new(),
            fixture_ids: HashSet::new(),
            fixture_group_ids: HashSet::new(),
            osc_patch_ids: HashSet::new(),
            output_patch_ids: HashSet::new(),
            midi_ports: ports.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// A one-second MIDI file: note on, note off two beats later at 120 BPM.
    fn one_second_file() -> Vec<u8> {
        let mut body = Vec::new();
        body.extend([0x00, 0x90, 60, 100]);
        body.extend([0x87, 0x40, 0x80, 60, 0]); // delta 960 ticks
        body.extend([0x00, 0xFF, 0x2F, 0x00]);

        let mut chunk = b"MTrk".to_vec();
        chunk.extend((body.len() as u32).to_be_bytes());
        chunk.extend(body);

        let mut out = b"MThd".to_vec();
        out.extend(6u32.to_be_bytes());
        out.extend(0u16.to_be_bytes()); // format 0
        out.extend(1u16.to_be_bytes()); // one track
        out.extend(480u16.to_be_bytes()); // ticks per beat
        out.extend(chunk);
        out
    }

    fn cue_with_file(dir: &std::path::Path, bytes: &[u8]) -> (MidiFileCue, PathBuf) {
        let path = dir.join(format!("{}.mid", Uuid::new_v4()));
        std::fs::write(&path, bytes).unwrap();
        let mut cue = MidiFileCue::new();
        cue.file_path = Some(path.clone());
        cue.reload_sequence();
        (cue, path)
    }

    #[test]
    fn cue_type_is_midi_file() {
        assert_eq!(MidiFileCue::new().cue_type(), CueType::MidiFile);
    }

    #[test]
    fn duration_comes_from_the_file() {
        let dir = std::env::temp_dir();
        let (cue, path) = cue_with_file(&dir, &one_second_file());
        assert_eq!(cue.duration(), Some(Duration::from_secs(1)));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn playback_rate_stretches_the_duration_but_not_the_file_length() {
        let dir = std::env::temp_dir();
        let (mut cue, path) = cue_with_file(&dir, &one_second_file());

        cue.playback_rate = 2.0;
        assert_eq!(cue.duration(), Some(Duration::from_millis(500)));
        cue.playback_rate = 0.5;
        assert_eq!(cue.duration(), Some(Duration::from_secs(2)));
        // The written length is what the inspector shows for the file itself.
        assert_eq!(cue.file_duration(), Some(Duration::from_secs(1)));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn an_absurd_rate_is_clamped_rather_than_dividing_by_zero() {
        let dir = std::env::temp_dir();
        let (mut cue, path) = cue_with_file(&dir, &one_second_file());

        cue.playback_rate = 0.0;
        assert_eq!(cue.duration(), Some(Duration::from_secs(20)), "clamped to 0.05x");
        cue.playback_rate = f64::NAN;
        assert_eq!(cue.duration(), Some(Duration::from_secs(1)), "NaN falls back to 1x");

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_cue_with_no_file_has_no_duration() {
        assert_eq!(MidiFileCue::new().duration(), None);
    }

    #[test]
    fn serialize_roundtrip_preserves_file_port_and_rate() {
        let dir = std::env::temp_dir();
        let (mut cue, path) = cue_with_file(&dir, &one_second_file());
        cue.set_name("Overture".into());
        cue.port_name = "Bus 1".into();
        cue.playback_rate = 0.75;
        cue.set_continue_mode(ContinueMode::AutoFollow);

        let rebuilt = MidiFileCueFactory.from_json(cue.serialize()).unwrap();
        let json = rebuilt.serialize();

        assert_eq!(json["name"], "Overture");
        assert_eq!(json["port_name"], "Bus 1");
        assert_eq!(json["playback_rate"], 0.75);
        assert_eq!(json["cue_type"], "midi_file");
        assert_eq!(rebuilt.cue_type(), CueType::MidiFile);
        // The rebuilt cue re-parsed the file, so it knows how long it runs.
        assert!(rebuilt.duration().is_some());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_file_that_will_not_parse_is_an_error_not_a_crash() {
        let dir = std::env::temp_dir();
        let (cue, path) = cue_with_file(&dir, b"nope");
        assert_eq!(cue.duration(), None);
        let issues = cue.validate(&ctx_with_ports(&["Bus 1"]));
        assert!(issues.iter().any(|i| i.severity == Severity::Error));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn validate_flags_a_missing_file_a_missing_port_and_a_bad_rate() {
        let cue = MidiFileCue::new();
        let issues = cue.validate(&ctx_with_ports(&[]));
        assert_eq!(issues.len(), 2, "no file and no port");
        assert!(issues.iter().all(|i| i.severity == Severity::Warning));

        let mut cue = MidiFileCue::new();
        cue.port_name = "Ghost Port".into();
        assert!(cue
            .validate(&ctx_with_ports(&["Real Port"]))
            .iter()
            .any(|i| i.severity == Severity::Error));

        let mut cue = MidiFileCue::new();
        cue.playback_rate = 500.0;
        assert!(cue
            .validate(&ctx_with_ports(&[]))
            .iter()
            .any(|i| i.message.contains("Playback rate")));
    }

    #[test]
    fn a_healthy_cue_reports_nothing() {
        let dir = std::env::temp_dir();
        let (mut cue, path) = cue_with_file(&dir, &one_second_file());
        cue.port_name = "Real Port".into();
        assert!(cue.validate(&ctx_with_ports(&["Real Port"])).is_empty());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn the_media_path_is_exposed_for_relink_and_preflight() {
        let mut cue = MidiFileCue::new();
        cue.file_path = Some(PathBuf::from("/shows/overture.mid"));
        assert_eq!(cue.media_file_path(), Some(Path::new("/shows/overture.mid")));
    }
}
