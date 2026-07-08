//! Zone (b) — workspace persistence & corrupt-file resilience.
//!
//! Two families:
//!  1. a real save → load round-trip through `Workspace::save`/`load`, and
//!  2. defensive loading of hand-crafted corrupt documents.
//!
//! Show-control angle: refusing to open a whole show over one bad cue is bad,
//! but silently dropping a cue the operator still sees in their paperwork is
//! *worse*. These tests pin down exactly which failures are fatal vs skipped,
//! and codify the silent-skip behaviour so any future change is deliberate.

mod common;

use common::*;
use inkue_lib::cue::types::CueType;
use inkue_lib::show::workspace::Workspace;

// ---------------------------------------------------------------------------
// Happy-path round-trip through the real on-disk save/load path
// ---------------------------------------------------------------------------

#[test]
fn save_load_roundtrip_preserves_cues() {
    let registry = full_registry();
    let dir = temp_dir("ws_roundtrip");
    let path = dir.join("show.inkue");

    let mut ws = Workspace::new("My Show");
    {
        let list = ws.active_cue_list_mut().expect("default cue list");
        for (t, name) in [
            (CueType::Memo, "Intro note"),
            (CueType::Audio, "Walk-in music"),
            (CueType::Stop, "Kill music"),
        ] {
            let mut cue = registry.create(&t).unwrap();
            cue.set_name(name.to_string());
            list.push(cue);
        }
    }
    ws.save(Some(path.clone())).expect("save should succeed");
    assert!(path.exists(), ".inkue file must be written");

    let loaded = Workspace::load(path, &registry).expect("load should succeed");
    let list = loaded.active_cue_list().expect("active cue list after load");
    let names: Vec<&str> = list.cues.iter().map(|c| c.name()).collect();
    assert_eq!(list.cues.len(), 3, "all three cues must survive save/load");
    assert!(names.contains(&"Intro note"));
    assert!(names.contains(&"Walk-in music"));
    assert!(names.contains(&"Kill music"));
}

// ---------------------------------------------------------------------------
// Helpers to craft workspace documents
// ---------------------------------------------------------------------------

/// Wrap a list of cue JSON values into a minimal valid workspace document.
fn workspace_doc(schema: u32, cues: Vec<serde_json::Value>) -> String {
    let list_id = "11111111-1111-4111-8111-111111111111";
    serde_json::json!({
        "schema_version": schema,
        "workspace": {
            "name": "T",
            "created_at": "2024-01-01T00:00:00Z",
            "modified_at": "2024-01-01T00:00:00Z"
        },
        "cue_lists": [{
            "id": list_id,
            "name": "List 1",
            "mode": "sequential",
            "cues": cues
        }],
        "active_cue_list_id": list_id
    })
    .to_string()
}

/// A valid cue JSON of the given type, produced via the real serializer.
fn valid_cue(t: CueType, name: &str) -> serde_json::Value {
    let r = full_registry();
    let mut cue = r.create(&t).unwrap();
    cue.set_name(name.to_string());
    cue.serialize()
}

// ---------------------------------------------------------------------------
// Corrupt-document resilience
// ---------------------------------------------------------------------------

#[test]
fn load_skips_unknown_cue_type_but_keeps_the_rest() {
    let registry = full_registry();
    let cues = vec![
        valid_cue(CueType::Memo, "Good 1"),
        serde_json::json!({ "type": "quantum", "id": "22222222-2222-4222-8222-222222222222", "name": "Bad" }),
        valid_cue(CueType::Memo, "Good 2"),
    ];
    let doc = workspace_doc(1, cues);

    let ws = Workspace::from_json_str(&doc, None, &registry)
        .expect("workspace must still load despite one unrecognised cue");
    let list = ws.active_cue_list().unwrap();
    assert_eq!(list.cues.len(), 2, "the two valid cues survive; the unknown one is skipped");
    let names: Vec<&str> = list.cues.iter().map(|c| c.name()).collect();
    assert_eq!(names, vec!["Good 1", "Good 2"]);
}

#[test]
fn skipped_unknown_cue_is_counted() {
    // FINDING B1 (fixed): an unrecognised/malformed cue is still dropped, but the
    // loss is no longer silent — `cues_skipped_on_load` records it so the command
    // layer can raise an operator banner instead of the cue vanishing unnoticed.
    let registry = full_registry();
    let cues = vec![
        valid_cue(CueType::Memo, "Kept"),
        serde_json::json!({ "type": "not_a_real_type", "id": "33333333-3333-4333-8333-333333333333" }),
    ];
    let doc = workspace_doc(1, cues);

    let ws = Workspace::from_json_str(&doc, None, &registry).expect("loads");
    assert_eq!(ws.active_cue_list().unwrap().cues.len(), 1, "only the valid cue loads");
    assert_eq!(ws.cues_skipped_on_load, 1, "the dropped cue must be counted");
}

#[test]
fn clean_load_reports_zero_skips() {
    let registry = full_registry();
    let doc = workspace_doc(1, vec![
        valid_cue(CueType::Memo, "A"),
        valid_cue(CueType::Audio, "B"),
    ]);
    let ws = Workspace::from_json_str(&doc, None, &registry).expect("loads");
    assert_eq!(ws.cues_skipped_on_load, 0, "a fully valid file skips nothing");
}

#[test]
fn load_skips_malformed_group_child_keeps_group() {
    let registry = full_registry();
    // Build a group that serialises with one valid + one bogus child.
    let mut group_json = valid_cue(CueType::Group, "Act 1");
    group_json["children"] = serde_json::json!([
        valid_cue(CueType::Memo, "Cue in group"),
        { "type": "quantum", "id": "44444444-4444-4444-8444-444444444444" }
    ]);
    let doc = workspace_doc(1, vec![group_json]);

    let ws = Workspace::from_json_str(&doc, None, &registry).expect("loads");
    let list = ws.active_cue_list().unwrap();
    assert_eq!(list.cues.len(), 1, "the group itself must load");
    let group = &list.cues[0];
    let children = group.child_cues().expect("group exposes its children");
    assert_eq!(children.len(), 1, "the bogus child is skipped, the valid one kept");
    assert_eq!(ws.cues_skipped_on_load, 1, "the dropped group child must be counted too");
}

#[test]
fn load_rejects_future_schema_version() {
    let registry = full_registry();
    let doc = workspace_doc(999, vec![valid_cue(CueType::Memo, "x")]);
    // Workspace has no Debug impl, so extract the error via match rather than expect_err.
    let msg = match Workspace::from_json_str(&doc, None, &registry) {
        Ok(_) => panic!("a newer-schema file must be rejected, not silently mis-parsed"),
        Err(e) => e.to_string(),
    };
    assert!(
        msg.contains("newer version") || msg.contains("schema"),
        "error should explain the schema mismatch, got: {msg}"
    );
}

#[test]
fn load_rejects_invalid_json() {
    let registry = full_registry();
    let msg = match Workspace::from_json_str("{ this is not valid json ", None, &registry) {
        Ok(_) => panic!("invalid JSON must be a clean error, not a silent Ok"),
        Err(e) => e.to_string(),
    };
    assert!(!msg.is_empty(), "error must carry a message");
}

#[test]
fn load_missing_workspace_key_errors() {
    let registry = full_registry();
    let doc = serde_json::json!({ "schema_version": 1, "cue_lists": [] }).to_string();
    assert!(
        Workspace::from_json_str(&doc, None, &registry).is_err(),
        "a document without the `workspace` metadata key must error"
    );
}

#[test]
fn load_tolerates_wrong_field_types_via_defaults() {
    // Lenient field parsing: an audio cue whose `volume_db` is the wrong JSON
    // type falls back to its default rather than failing the whole cue. This
    // is deliberate resilience — the cue still loads and plays.
    let registry = full_registry();
    let mut audio = valid_cue(CueType::Audio, "Music");
    audio["volume_db"] = serde_json::json!("this should be a number");
    let doc = workspace_doc(1, vec![audio]);

    let ws = Workspace::from_json_str(&doc, None, &registry).expect("loads with defaults");
    assert_eq!(ws.active_cue_list().unwrap().cues.len(), 1);
}

#[test]
fn relative_media_paths_are_absolutised_on_load() {
    // The .inkue stores paths relative to the workspace dir; load must resolve
    // them against `base_dir` so a broken/relative path never reaches playback.
    let registry = full_registry();
    let base = temp_dir("ws_paths");

    let mut audio = valid_cue(CueType::Audio, "Track");
    audio["file_path"] = serde_json::json!("audio/track.wav");
    let doc = workspace_doc(1, vec![audio]);

    let ws = Workspace::from_json_str(&doc, Some(base.as_path()), &registry).expect("loads");
    let cue = &ws.active_cue_list().unwrap().cues[0];
    let resolved = cue.media_file_path().expect("audio cue exposes its media path");
    assert!(
        resolved.is_absolute(),
        "relative path must be absolutised on load, got {}",
        resolved.display()
    );
    assert!(
        resolved.ends_with("track.wav"),
        "resolved path should still point at the media file, got {}",
        resolved.display()
    );
}
