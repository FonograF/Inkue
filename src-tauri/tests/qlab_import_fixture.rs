//! Zone (🟠 import) — a real QLab workspace, imported end to end.
//!
//! Opt-in: the fixture lives with the `qlab2inkue` reference converter, outside
//! this repository.  Run it with
//!
//! ```text
//! INKUE_QLAB_FIXTURE="C:/qlab2inkue/samples/All Cue Types.qlab5" \
//!     cargo test --test qlab_import_fixture -- --ignored
//! ```

mod common;

use std::path::Path;

use inkue_lib::show::Workspace;

fn fixture() -> Option<std::path::PathBuf> {
    std::env::var_os("INKUE_QLAB_FIXTURE").map(Into::into)
}

#[test]
#[ignore = "needs INKUE_QLAB_FIXTURE pointing at a .qlab4 / .qlab5 file"]
fn a_real_workspace_imports_and_loads_every_cue() {
    let path = fixture().expect("set INKUE_QLAB_FIXTURE");
    let (json, report) = inkue_lib::qlab_import::import_workspace(&path).expect("import");
    assert!(report.cue_count > 0, "the fixture holds cues");

    let registry = common::full_registry();
    let workspace = Workspace::from_json_str(&json, path.parent(), &registry).expect("load");
    let loaded: usize = workspace.cue_lists.iter().map(|l| count(&l.cues)).sum();
    assert_eq!(loaded, report.cue_count, "the loader must not skip an imported cue");
}

#[test]
#[ignore = "needs INKUE_QLAB_FIXTURE pointing at a .qlab4 / .qlab5 file"]
fn every_picture_plays_on_a_declared_output() {
    let path = fixture().expect("set INKUE_QLAB_FIXTURE");
    let (json, report) = inkue_lib::qlab_import::import_workspace(Path::new(&path)).expect("import");
    let document: serde_json::Value = serde_json::from_str(&json).unwrap();
    let declared: Vec<String> = document["video_outputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(declared.len(), report.video_outputs.len());

    let mut pictures = 0;
    walk(&document["cue_lists"], &mut |cue| {
        if matches!(cue["cue_type"].as_str(), Some("video" | "image" | "camera" | "text")) {
            pictures += 1;
            match cue["output_id"].as_str() {
                None => {}
                Some(id) => assert!(declared.contains(&id.to_string()), "cue on an undeclared output {id}"),
            }
        }
    });
    assert!(pictures > 0, "the fixture holds visual cues");
}

fn count(cues: &[Box<dyn inkue_lib::cue::traits::Cue>]) -> usize {
    cues.iter().map(|c| 1 + c.child_cues().map(count).unwrap_or(0)).sum()
}

fn walk(value: &serde_json::Value, visit: &mut dyn FnMut(&serde_json::Value)) {
    match value {
        serde_json::Value::Array(items) => items.iter().for_each(|v| walk(v, visit)),
        serde_json::Value::Object(map) => {
            if map.contains_key("cue_type") {
                visit(value);
            }
            for key in ["cues", "children"] {
                if let Some(nested) = map.get(key) {
                    walk(nested, visit);
                }
            }
        }
        _ => {}
    }
}
