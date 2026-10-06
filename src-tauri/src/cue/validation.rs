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
    /// IDs of the show's extra video outputs (the main output needs no id: a
    /// cue without `output_id` plays on it).
    pub video_output_ids: HashSet<Uuid>,
    /// IDs of the cues that put a picture on an output (Video, Image, Camera)
    /// — what a crossfade can dissolve into.
    pub visual_cue_ids: HashSet<CueId>,
}

impl ValidationContext {
    /// The problem to report for a visual cue pointing at `output_id`, if any:
    /// an output the show no longer has.
    pub fn missing_output_issue(&self, output_id: Option<Uuid>) -> Option<CueIssue> {
        let id = output_id?;
        (!self.video_output_ids.contains(&id)).then(|| {
            CueIssue::error("Video output not found — pick another output in the Inspector")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context_with_outputs(ids: &[Uuid]) -> ValidationContext {
        ValidationContext {
            all_cue_ids: HashSet::new(),
            fixture_ids: HashSet::new(),
            fixture_group_ids: HashSet::new(),
            osc_patch_ids: HashSet::new(),
            output_patch_ids: HashSet::new(),
            midi_ports: Vec::new(),
            video_output_ids: ids.iter().copied().collect(),
            visual_cue_ids: HashSet::new(),
        }
    }

    #[test]
    fn a_cue_on_the_main_output_is_never_flagged() {
        assert!(context_with_outputs(&[]).missing_output_issue(None).is_none());
    }

    #[test]
    fn a_cue_on_an_existing_output_is_fine() {
        let id = Uuid::new_v4();
        assert!(context_with_outputs(&[id]).missing_output_issue(Some(id)).is_none());
    }

    #[test]
    fn a_cue_on_a_deleted_output_is_an_error() {
        let issue = context_with_outputs(&[]).missing_output_issue(Some(Uuid::new_v4())).unwrap();
        assert_eq!(issue.severity, Severity::Error);
    }
}
