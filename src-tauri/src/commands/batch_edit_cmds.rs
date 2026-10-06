//! Tauri commands that act on a multi-cue selection in one undo step: editing
//! many cues at once (the cue list's right-click menu) and creating a cue that
//! targets the selection (Fade / Stop / Devamp / Start … "for these cues").

use serde::Deserialize;
use tauri::{Emitter, State};
use uuid::Uuid;

use super::cue_cmds::{apply_cue_properties, merge_properties, MergePolicy};
use crate::{
    cue::{registry::CueRegistry, traits::Cue, types::CueType},
    show::cue_list::CueList,
    state::AppState,
};

/// One cue's share of a batch edit: the properties to merge into it.
///
/// Each cue carries its own patch so relative edits ("+3 dB") and
/// type-dependent keys (a Video's picture fade vs an Audio's fade) resolve per
/// cue on the frontend.
#[derive(Debug, Clone, Deserialize)]
pub struct CueEdit {
    /// Target cue (top level or nested in a group).
    pub cue_id: String,
    /// Partial JSON merged into the cue — only keys the cue already has.
    pub properties: serde_json::Value,
}

/// Apply a batch of edits as a single undo step.
///
/// Keys a cue does not serialise are skipped, so one patch can be sent to a
/// mixed selection.  Returns how many cues actually changed.
#[tauri::command]
pub fn update_cues(
    edits: Vec<CueEdit>,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<usize, String> {
    let parsed = parse_edits(edits)?;
    if parsed.is_empty() {
        return Ok(0);
    }
    super::undo_cmds::push_current_snapshot(&state)?;

    // Lock order: registry first, then workspace (matches update_cue).
    let registry = state.registry.lock().map_err(|e| e.to_string())?;
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let mut changed = 0;
    for (id, properties) in &parsed {
        if apply_cue_properties(&state, &registry, &mut ws, *id, properties, MergePolicy::KnownKeysOnly)? {
            changed += 1;
        }
    }
    if changed > 0 {
        ws.mark_modified();
    }
    drop(ws);
    drop(registry);

    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(changed)
}

/// Create a cue of `cue_type` aimed at `target_ids` — e.g. a Fade Cue that
/// fades the selection out — and insert it at the top level right after the
/// last target.  `properties` is merged over the new cue's defaults.
///
/// Returns the new cue's id.  Fails for a cue type that has no targets.
#[tauri::command]
pub fn add_targeting_cue(
    cue_type: CueType,
    target_ids: Vec<String>,
    properties: serde_json::Value,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<String, String> {
    let targets = parse_ids(&target_ids)?;
    if targets.is_empty() {
        return Err("No target cues".to_string());
    }
    super::undo_cmds::push_current_snapshot(&state)?;

    let registry = state.registry.lock().map_err(|e| e.to_string())?;
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let cue_list = ws.active_cue_list_mut().ok_or("No active cue list")?;
    let (cue, position) = build_targeting_cue(&registry, cue_list, &cue_type, &targets, &properties)?;
    let id = cue.id().to_string();
    insert_new_cue(cue_list, cue, position);
    ws.mark_modified();
    drop(ws);
    drop(registry);

    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(id)
}

fn parse_ids(ids: &[String]) -> Result<Vec<Uuid>, String> {
    ids.iter()
        .map(|s| s.parse::<Uuid>().map_err(|e| e.to_string()))
        .collect()
}

fn parse_edits(edits: Vec<CueEdit>) -> Result<Vec<(Uuid, serde_json::Value)>, String> {
    edits
        .into_iter()
        .map(|edit| {
            let id = edit.cue_id.parse::<Uuid>().map_err(|e| e.to_string())?;
            Ok((id, edit.properties))
        })
        .collect()
}

/// Build the targeting cue and the top-level index it belongs at, without
/// touching the list — kept apart from the command so it is testable.
pub fn build_targeting_cue(
    registry: &CueRegistry,
    cue_list: &CueList,
    cue_type: &CueType,
    targets: &[Uuid],
    properties: &serde_json::Value,
) -> Result<(Box<dyn Cue>, usize), String> {
    let target_numbers = targets
        .iter()
        .map(|id| {
            let cue = cue_list.get_recursive(id).ok_or_else(|| format!("Cue {id} not found"))?;
            Ok(cue.number().map(str::to_string))
        })
        .collect::<Result<Vec<_>, String>>()?;

    let mut json = registry.create(cue_type).map_err(|e| e.to_string())?.serialize();
    set_targets(&mut json, targets, &target_numbers)?;
    merge_properties(&mut json, properties, MergePolicy::KnownKeysOnly);
    let cue = registry.from_json(json).map_err(|e| e.to_string())?;

    let position = insertion_index(cue_list, targets).unwrap_or(cue_list.cues.len());
    Ok((cue, position))
}

/// Write `targets` (and their display numbers) into a cue's serialised form.
fn set_targets(json: &mut serde_json::Value, targets: &[Uuid], numbers: &[Option<String>]) -> Result<(), String> {
    let object = json.as_object_mut().ok_or("Cue did not serialise to an object")?;
    if !object.contains_key("target_cue_ids") {
        return Err("This cue type has no targets".to_string());
    }
    let ids: Vec<String> = targets.iter().map(Uuid::to_string).collect();
    let numbers: Vec<&String> = numbers.iter().flatten().collect();
    object.insert("target_cue_ids".into(), serde_json::json!(ids));
    object.insert("target_cue_numbers".into(), serde_json::json!(numbers));
    Ok(())
}

/// The top-level index just after the last top-level cue holding a target
/// (the target itself, or the Group it is nested in).
fn insertion_index(cue_list: &CueList, targets: &[Uuid]) -> Option<usize> {
    cue_list
        .cues
        .iter()
        .rposition(|cue| targets.iter().any(|id| contains_cue(cue.as_ref(), id)))
        .map(|index| index + 1)
}

fn contains_cue(cue: &dyn Cue, id: &Uuid) -> bool {
    cue.id() == *id
        || cue
            .child_cues()
            .is_some_and(|children| children.iter().any(|child| contains_cue(child.as_ref(), id)))
}

/// Insert a freshly built cue the way `add_cue` does: numbered when
/// auto-renumber is off, resequenced by the list otherwise.
fn insert_new_cue(cue_list: &mut CueList, mut cue: Box<dyn Cue>, position: usize) {
    if !cue_list.auto_renumber {
        cue.set_number(Some(cue_list.next_available_number()));
    }
    if position >= cue_list.cues.len() {
        cue_list.push(cue);
    } else {
        cue_list.insert(position, cue);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::cue::{
        audio_cue::{AudioCue, AudioCueFactory},
        fade_cue::FadeCueFactory,
        group_cue::GroupCueFactory,
        memo_cue::{MemoCue, MemoCueFactory},
        stop_cue::StopCueFactory,
    };

    fn registry() -> CueRegistry {
        let mut r = CueRegistry::new();
        r.register(CueType::Audio, Box::new(AudioCueFactory));
        r.register(CueType::Memo, Box::new(MemoCueFactory));
        r.register(CueType::Fade, Box::new(FadeCueFactory));
        r.register(CueType::Stop, Box::new(StopCueFactory));
        r.register(CueType::Group, Box::new(GroupCueFactory));
        r
    }

    fn numbered(mut cue: Box<dyn Cue>, number: &str) -> Box<dyn Cue> {
        cue.set_number(Some(number.to_string()));
        cue
    }

    #[test]
    fn known_keys_only_skips_fields_the_cue_does_not_have() {
        let mut memo = MemoCue::new().serialize();
        let merged = merge_properties(&mut memo, &json!({ "volume_db": -6.0 }), MergePolicy::KnownKeysOnly);
        assert!(!merged);
        assert!(memo.get("volume_db").is_none(), "a Memo must not grow a volume");
    }

    #[test]
    fn known_keys_only_writes_shared_fields_on_every_type() {
        let patch = json!({ "color": "red", "volume_db": -6.0 });
        let mut memo = MemoCue::new().serialize();
        let mut audio = AudioCue::new().serialize();

        assert!(merge_properties(&mut memo, &patch, MergePolicy::KnownKeysOnly));
        assert!(merge_properties(&mut audio, &patch, MergePolicy::KnownKeysOnly));

        assert_eq!(memo["color"], "red");
        assert_eq!(audio["color"], "red");
        assert_eq!(audio["volume_db"], -6.0);
    }

    #[test]
    fn all_keys_merges_everything() {
        let mut memo = MemoCue::new().serialize();
        assert!(merge_properties(&mut memo, &json!({ "anything": 1 }), MergePolicy::AllKeys));
        assert_eq!(memo["anything"], 1);
    }

    #[test]
    fn merged_patch_survives_a_registry_rebuild() {
        let registry = registry();
        let mut json = AudioCue::new().serialize();
        merge_properties(
            &mut json,
            &json!({ "continue_mode": "auto_follow", "is_disabled": true, "pre_wait_ms": 1500 }),
            MergePolicy::KnownKeysOnly,
        );
        let rebuilt = registry.from_json(json).unwrap().serialize();
        assert_eq!(rebuilt["continue_mode"], "auto_follow");
        assert_eq!(rebuilt["is_disabled"], true);
        assert_eq!(rebuilt["pre_wait_ms"], 1500);
    }

    #[test]
    fn targeting_cue_aims_at_the_targets_and_lands_after_the_last_one() {
        let registry = registry();
        let mut list = CueList::new("Main");
        let a = numbered(Box::new(AudioCue::new()), "1");
        let b = numbered(Box::new(AudioCue::new()), "2");
        let c = numbered(Box::new(MemoCue::new()), "3");
        let (a_id, b_id) = (a.id(), b.id());
        list.push(a);
        list.push(b);
        list.push(c);

        let (fade, position) = build_targeting_cue(
            &registry, &list, &CueType::Fade, &[a_id, b_id], &json!({ "stop_at_end": true }),
        )
        .unwrap();

        let json = fade.serialize();
        assert_eq!(json["target_cue_ids"], json!([a_id.to_string(), b_id.to_string()]));
        assert_eq!(json["target_cue_numbers"], json!(["1", "2"]));
        assert_eq!(json["stop_at_end"], true);
        assert_eq!(position, 2, "right after cue 2, before the memo");
    }

    #[test]
    fn targeting_a_group_child_lands_after_its_group() {
        let registry = registry();
        let mut list = CueList::new("Main");
        let a = Box::new(AudioCue::new());
        let b = Box::new(AudioCue::new());
        let tail = Box::new(MemoCue::new());
        let (a_id, b_id) = (a.id(), b.id());
        list.push(a);
        list.push(b);
        list.push(tail);
        list.group_cues(&[a_id, b_id]).unwrap();

        let (_, position) =
            build_targeting_cue(&registry, &list, &CueType::Stop, &[b_id], &json!({})).unwrap();

        assert_eq!(position, 1, "after the group, not inside it");
    }

    #[test]
    fn a_cue_type_without_targets_is_refused() {
        let registry = registry();
        let mut list = CueList::new("Main");
        let a = Box::new(AudioCue::new());
        let a_id = a.id();
        list.push(a);

        let result = build_targeting_cue(&registry, &list, &CueType::Memo, &[a_id], &json!({}));
        assert!(result.is_err());
    }

    #[test]
    fn an_unknown_target_is_refused() {
        let registry = registry();
        let list = CueList::new("Main");
        let result = build_targeting_cue(&registry, &list, &CueType::Fade, &[Uuid::new_v4()], &json!({}));
        assert!(result.is_err());
    }

    #[test]
    fn new_cue_is_numbered_when_auto_renumber_is_off() {
        let registry = registry();
        let mut list = CueList::new("Main");
        list.auto_renumber = false;
        let a = numbered(Box::new(AudioCue::new()), "1");
        let a_id = a.id();
        list.push(a);

        let (fade, position) =
            build_targeting_cue(&registry, &list, &CueType::Fade, &[a_id], &json!({})).unwrap();
        insert_new_cue(&mut list, fade, position);

        assert_eq!(list.cues.len(), 2);
        assert_eq!(list.cues[1].number(), Some("2"));
    }
}
