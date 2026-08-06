//! [`ControlCue`] — the family of cues whose action is performed **on other
//! cues**: Start, Pause, Resume, Load, Reset, Goto, Arm and Disarm.
//!
//! QLab exposes each of these as its own cue type, and so does Inkue — they
//! keep their own colour, their own row label and a 1:1 mapping when importing
//! a QLab workspace. What they do *not* need is eight near-identical files:
//! they differ only by the action they carry, so one struct implements all of
//! them and the registry binds it once per [`CueType`].
//!
//! Like [`StopCue`](super::stop_cue::StopCue), a Command cue does not act by
//! itself — it declares what it wants through [`Cue::control_specification`]
//! and the transport, which owns the cue list and the playhead, carries it out.

use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use uuid::Uuid;

use super::{
    context::CueContext,
    traits::{Cue, CueFactory, RuntimeState},
    types::{ContinueMode, CueColor, CueId, CueState, CueType},
};

/// What a Command cue does to each of its targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlAction {
    Start,
    Pause,
    Resume,
    Load,
    Reset,
    /// Moves the Playhead rather than touching the target's playback.
    Goto,
    Arm,
    Disarm,
}

impl ControlAction {
    /// The cue type that carries this action.
    pub fn cue_type(self) -> CueType {
        match self {
            ControlAction::Start => CueType::Start,
            ControlAction::Pause => CueType::Pause,
            ControlAction::Resume => CueType::Resume,
            ControlAction::Load => CueType::Load,
            ControlAction::Reset => CueType::Reset,
            ControlAction::Goto => CueType::Goto,
            ControlAction::Arm => CueType::Arm,
            ControlAction::Disarm => CueType::Disarm,
        }
    }

    /// The action a cue type carries, or `None` for every non-Command type.
    pub fn from_cue_type(cue_type: &CueType) -> Option<Self> {
        match cue_type {
            CueType::Start => Some(ControlAction::Start),
            CueType::Pause => Some(ControlAction::Pause),
            CueType::Resume => Some(ControlAction::Resume),
            CueType::Load => Some(ControlAction::Load),
            CueType::Reset => Some(ControlAction::Reset),
            CueType::Goto => Some(ControlAction::Goto),
            CueType::Arm => Some(ControlAction::Arm),
            CueType::Disarm => Some(ControlAction::Disarm),
            _ => None,
        }
    }

    /// Default cue name, matching the type ("Start Cue", "Goto Cue"…).
    fn default_name(self) -> String {
        format!("{} Cue", self.label())
    }

    /// Display label used for the name and the cue-list row.
    pub fn label(self) -> &'static str {
        match self {
            ControlAction::Start => "Start",
            ControlAction::Pause => "Pause",
            ControlAction::Resume => "Resume",
            ControlAction::Load => "Load",
            ControlAction::Reset => "Reset",
            ControlAction::Goto => "Goto",
            ControlAction::Arm => "Arm",
            ControlAction::Disarm => "Disarm",
        }
    }

    /// Only one cue can hold the Playhead, so Goto ignores extra targets.
    pub fn is_single_target(self) -> bool {
        self == ControlAction::Goto
    }

    fn default_color(self) -> CueColor {
        match self {
            ControlAction::Start | ControlAction::Resume => CueColor::Green,
            ControlAction::Pause | ControlAction::Load => CueColor::Yellow,
            ControlAction::Reset | ControlAction::Goto => CueColor::Cyan,
            ControlAction::Arm | ControlAction::Disarm => CueColor::Orange,
        }
    }
}

/// A cue that performs [`ControlAction`] on its target cues.
pub struct ControlCue {
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

    /// The action this cue performs — fixed by its cue type, never edited.
    action: ControlAction,
    /// UUIDs of the cues acted upon. Empty = nothing happens (unlike Stop,
    /// where empty deliberately means "everything": starting or resetting
    /// every cue in the show by accident is not a mistake worth enabling).
    pub target_cue_ids: Vec<CueId>,
    /// Human-readable numbers kept alongside the ids for display, and used to
    /// resolve targets when loading a workspace that only carried numbers.
    pub target_cue_numbers: Vec<String>,
    is_disabled: bool,
}

impl ControlCue {
    /// Create a Command cue carrying `action`.
    pub fn new(action: ControlAction) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: action.default_name(),
            number: None,
            notes: String::new(),
            color: action.default_color(),
            state: CueState::Standby,
            continue_mode: ContinueMode::DoNotContinue,
            pre_wait: Duration::ZERO,
            post_wait: Duration::ZERO,
            started_at: None,
            action,
            target_cue_ids: Vec::new(),
            target_cue_numbers: Vec::new(),
            is_disabled: false,
        }
    }

    /// The action this cue performs.
    pub fn action(&self) -> ControlAction {
        self.action
    }
}

impl Cue for ControlCue {
    fn id(&self) -> CueId { self.id }
    fn cue_type(&self) -> CueType { self.action.cue_type() }
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

    fn load(&mut self, _context: &CueContext) -> Result<()> { Ok(()) }

    fn go(&mut self, _context: &CueContext) -> Result<()> {
        // The action itself is executed by the transport, which owns the cue
        // list and the playhead; the cue completes as soon as it has been read.
        self.state = CueState::Completed;
        self.started_at = Some(Instant::now());
        Ok(())
    }

    fn stop(&mut self, _context: &CueContext) -> Result<()> {
        self.state = CueState::Standby;
        self.started_at = None;
        Ok(())
    }

    fn pause(&mut self, _context: &CueContext) -> Result<()> { Ok(()) }
    fn resume(&mut self, _context: &CueContext) -> Result<()> { Ok(()) }

    fn hard_stop(&mut self, context: &CueContext) -> Result<()> {
        self.stop(context)
    }

    fn reset(&mut self) -> Result<()> {
        self.state = CueState::Standby;
        self.started_at = None;
        Ok(())
    }

    fn pre_wait(&self) -> Duration { self.pre_wait }
    fn set_pre_wait(&mut self, d: Duration) { self.pre_wait = d; }
    fn post_wait(&self) -> Duration { self.post_wait }
    fn set_post_wait(&mut self, d: Duration) { self.post_wait = d; }

    fn duration(&self) -> Option<Duration> { None }

    fn elapsed(&self) -> Duration {
        self.started_at.map(|t| t.elapsed()).unwrap_or(Duration::ZERO)
    }

    fn action_elapsed(&self) -> Duration { self.elapsed() }

    fn continue_mode(&self) -> ContinueMode { self.continue_mode }
    fn set_continue_mode(&mut self, mode: ContinueMode) { self.continue_mode = mode; }

    fn control_specification(&self) -> Option<(ControlAction, Vec<CueId>)> {
        if self.target_cue_ids.is_empty() {
            return None;
        }
        let targets = if self.action.is_single_target() {
            self.target_cue_ids[..1].to_vec()
        } else {
            self.target_cue_ids.clone()
        };
        Some((self.action, targets))
    }

    fn validate(
        &self,
        ctx: &crate::cue::validation::ValidationContext,
    ) -> Vec<crate::cue::validation::CueIssue> {
        use crate::cue::validation::CueIssue;
        if self.target_cue_ids.is_empty() {
            return vec![CueIssue::warning(format!(
                "{} Cue has no target — it will do nothing",
                self.action.label()
            ))];
        }
        self.target_cue_ids
            .iter()
            .filter(|id| !ctx.all_cue_ids.contains(id))
            .map(|_| {
                CueIssue::warning(format!("{} target not found (cue deleted)", self.action.label()))
            })
            .collect()
    }

    fn resolve_stop_target(&mut self, number_to_id: &std::collections::HashMap<String, CueId>) {
        // Shares Stop's resolution hook: it runs for every cue once the whole
        // list is loaded, which is exactly when a number can become an id.
        if self.target_cue_ids.is_empty() {
            for num in &self.target_cue_numbers {
                if let Some(&id) = number_to_id.get(num) {
                    if !self.target_cue_ids.contains(&id) {
                        self.target_cue_ids.push(id);
                    }
                }
            }
        }
    }

    fn runtime_state(&self) -> RuntimeState {
        RuntimeState {
            state: self.state,
            voice_id: None,
            started_at: self.started_at,
            action_started_at: self.started_at,
        }
    }

    fn restore_runtime_state(&mut self, snap: RuntimeState) {
        self.state = snap.state;
        self.started_at = snap.started_at;
    }

    fn serialize(&self) -> Value {
        let type_name = self.action.cue_type().to_string();
        json!({
            "type": type_name,
            "cue_type": type_name,
            "id": self.id,
            "number": self.number,
            "name": self.name,
            "notes": self.notes,
            "color": self.color,
            "pre_wait_ms": self.pre_wait.as_millis() as u64,
            "post_wait_ms": self.post_wait.as_millis() as u64,
            "continue_mode": self.continue_mode,
            "target_cue_ids": self.target_cue_ids.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
            "target_cue_numbers": self.target_cue_numbers,
            "is_disabled": self.is_disabled,
        })
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Factory for one Command cue type; the registry binds one per action.
pub struct ControlCueFactory(pub ControlAction);

impl CueFactory for ControlCueFactory {
    fn create(&self) -> Box<dyn Cue> {
        Box::new(ControlCue::new(self.0))
    }

    fn from_json(&self, value: Value) -> Result<Box<dyn Cue>> {
        // Trust the serialised type over the factory it was routed to, so a
        // hand-edited workspace cannot produce a cue whose action and type
        // disagree.
        let action = value
            .get("type")
            .and_then(|v| serde_json::from_value::<CueType>(v.clone()).ok())
            .and_then(|t| ControlAction::from_cue_type(&t))
            .unwrap_or(self.0);
        let mut cue = ControlCue::new(action);

        if let Some(id_str) = value.get("id").and_then(|v| v.as_str()) {
            cue.id = id_str.parse().unwrap_or_else(|_| Uuid::new_v4());
        }
        if let Some(name) = value.get("name").and_then(|v| v.as_str()) {
            cue.name = name.to_string();
        }
        if let Some(num) = value.get("number").and_then(|v| v.as_str()) {
            cue.number = Some(num.to_string());
        }
        if let Some(notes) = value.get("notes").and_then(|v| v.as_str()) {
            cue.notes = notes.to_string();
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
        if let Some(arr) = value.get("target_cue_ids").and_then(|v| v.as_array()) {
            cue.target_cue_ids = arr.iter().filter_map(|v| v.as_str()?.parse().ok()).collect();
        }
        if let Some(arr) = value.get("target_cue_numbers").and_then(|v| v.as_array()) {
            cue.target_cue_numbers = arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
        }
        if let Some(b) = value.get("is_disabled").and_then(|v| v.as_bool()) {
            cue.is_disabled = b;
        }

        Ok(Box::new(cue))
    }
}

/// Every Command action, for registry wiring and UI listings.
pub const ALL_CONTROL_ACTIONS: [ControlAction; 8] = [
    ControlAction::Start,
    ControlAction::Pause,
    ControlAction::Resume,
    ControlAction::Load,
    ControlAction::Reset,
    ControlAction::Goto,
    ControlAction::Arm,
    ControlAction::Disarm,
];

/// Parse a Command cue type from its serialised name — used by the frontend
/// command layer, which addresses cue types by string.
pub fn action_from_str(name: &str) -> Result<ControlAction> {
    ALL_CONTROL_ACTIONS
        .iter()
        .copied()
        .find(|a| a.cue_type().to_string() == name)
        .ok_or_else(|| anyhow!("'{name}' is not a Command cue type"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_maps_to_its_own_cue_type_and_back() {
        for action in ALL_CONTROL_ACTIONS {
            let cue_type = action.cue_type();
            assert_eq!(
                ControlAction::from_cue_type(&cue_type),
                Some(action),
                "{action:?} must round-trip through its cue type",
            );
            assert_eq!(ControlCue::new(action).cue_type(), cue_type);
        }
    }

    #[test]
    fn non_command_types_carry_no_action() {
        assert_eq!(ControlAction::from_cue_type(&CueType::Audio), None);
        assert_eq!(ControlAction::from_cue_type(&CueType::Stop), None);
    }

    #[test]
    fn a_command_cue_without_targets_does_nothing() {
        // Deliberately unlike Stop, where empty means "all cues".
        let cue = ControlCue::new(ControlAction::Reset);
        assert!(cue.control_specification().is_none());
        assert_eq!(cue.validate(&empty_ctx()).len(), 1, "and it says so in preflight");
    }

    #[test]
    fn goto_keeps_only_its_first_target() {
        let mut cue = ControlCue::new(ControlAction::Goto);
        cue.target_cue_ids = vec![Uuid::new_v4(), Uuid::new_v4()];
        let (_, targets) = cue.control_specification().unwrap();
        assert_eq!(targets.len(), 1, "only one cue can hold the Playhead");
    }

    #[test]
    fn other_actions_keep_every_target() {
        let mut cue = ControlCue::new(ControlAction::Start);
        cue.target_cue_ids = vec![Uuid::new_v4(), Uuid::new_v4()];
        let (action, targets) = cue.control_specification().unwrap();
        assert_eq!(action, ControlAction::Start);
        assert_eq!(targets.len(), 2);
    }

    #[test]
    fn serialize_roundtrip_preserves_action_and_targets() {
        for action in ALL_CONTROL_ACTIONS {
            let mut cue = ControlCue::new(action);
            let target = Uuid::new_v4();
            cue.target_cue_ids = vec![target];
            cue.target_cue_numbers = vec!["12".into()];
            cue.set_name("Custom".into());

            let json = cue.serialize();
            assert_eq!(json["type"], action.cue_type().to_string());

            let rebuilt = ControlCueFactory(action).from_json(json).unwrap();
            assert_eq!(rebuilt.cue_type(), action.cue_type());
            assert_eq!(rebuilt.name(), "Custom");
            let (rebuilt_action, targets) = rebuilt.control_specification().unwrap();
            assert_eq!(rebuilt_action, action);
            assert_eq!(targets, vec![target]);
        }
    }

    #[test]
    fn the_serialised_type_wins_over_the_factory_it_is_routed_to() {
        let json = ControlCue::new(ControlAction::Goto).serialize();
        let rebuilt = ControlCueFactory(ControlAction::Start).from_json(json).unwrap();
        assert_eq!(rebuilt.cue_type(), CueType::Goto);
    }

    #[test]
    fn targets_resolve_from_numbers_after_load() {
        let mut cue = ControlCue::new(ControlAction::Start);
        cue.target_cue_numbers = vec!["7".into()];
        let id = Uuid::new_v4();
        let mut map = std::collections::HashMap::new();
        map.insert("7".to_string(), id);

        cue.resolve_stop_target(&map);

        assert_eq!(cue.target_cue_ids, vec![id]);
    }

    #[test]
    fn action_parses_from_its_type_name() {
        assert_eq!(action_from_str("goto").unwrap(), ControlAction::Goto);
        assert!(action_from_str("audio").is_err());
    }

    fn empty_ctx() -> crate::cue::validation::ValidationContext {
        use std::collections::HashSet;
        crate::cue::validation::ValidationContext {
            all_cue_ids: HashSet::new(),
            fixture_ids: HashSet::new(),
            fixture_group_ids: HashSet::new(),
            osc_patch_ids: HashSet::new(),
            output_patch_ids: HashSet::new(),
            midi_ports: Vec::new(),
        }
    }
}
