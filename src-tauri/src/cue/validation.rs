//! Preflight validation — detect cues whose external dependencies do not resolve
//! (missing media file, dangling Stop/Fade target, unpatched fixture, absent MIDI
//! port, …) so the operator sees them *before* the show rather than at GO time.
//!
//! Each cue type reports its own problems via [`Cue::validate`](super::traits::Cue::validate),
//! keeping cue-specific knowledge in the cue (a new cue type validates itself and
//! needs no change to the walker).  Media-file existence is checked centrally by
//! the command layer via [`Cue::media_file_path`](super::traits::Cue::media_file_path).

use std::collections::HashSet;

use serde::Serialize;
use uuid::Uuid;

use super::types::CueId;

/// How serious a validation problem is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// The cue cannot perform its action (missing file, dangling target).
    Error,
    /// The cue will run but something is off (no target selected, fallback used).
    Warning,
}

/// A single problem found on a cue.
#[derive(Debug, Clone, Serialize)]
pub struct CueIssue {
    pub severity: Severity,
    pub message: String,
}

impl CueIssue {
    pub fn error(message: impl Into<String>) -> Self {
        Self { severity: Severity::Error, message: message.into() }
    }
    pub fn warning(message: impl Into<String>) -> Self {
        Self { severity: Severity::Warning, message: message.into() }
    }
}

/// Read-only snapshot of the workspace's resolvable resources, built once per
/// preflight pass and shared by every cue's [`Cue::validate`](super::traits::Cue::validate).
pub struct ValidationContext {
    /// Every cue ID in the workspace (all lists, nested groups included).
    pub all_cue_ids: HashSet<CueId>,
    /// IDs of patched lighting fixtures.
    pub fixture_ids: HashSet<Uuid>,
    /// IDs of fixture groups.
    pub fixture_group_ids: HashSet<Uuid>,
    /// IDs of configured OSC send patches.
    pub osc_patch_ids: HashSet<Uuid>,
    /// IDs of configured audio output patches.
    pub output_patch_ids: HashSet<Uuid>,
    /// Names of MIDI output ports currently available on this machine.
    pub midi_ports: Vec<String>,
}
