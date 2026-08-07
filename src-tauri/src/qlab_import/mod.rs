//! Import a QLab workspace (`.qlab4` / `.qlab5`) as an Inkue workspace.
//!
//! Reading someone else's file format to import work the **user already owns**
//! is interoperability, not appropriation: no QLab code is involved, and the
//! container is Apple's `NSKeyedArchiver`, not a Figure 53 format. QLab is a
//! trademark of Figure 53, LLC; Inkue is not affiliated with or endorsed by
//! them.
//!
//! Three layers:
//!
//! - [`archive`] resolves the NSKeyedArchiver object graph (the only genuinely
//!   novel code — everything else is mapping).
//! - [`patches`] reads the workspace's destination tables.
//! - [`cues`] maps one QLab cue to one Inkue cue.
//!
//! The reference implementation is the standalone Python converter in
//! `qlab2inkue/`, which can decode an unknown `.qlab5` offline and whose
//! `test_mapping.py` runs against a fixture holding one cue of every QLab 5
//! type. Decode a real file there before assuming a property name.
//!
//! **Media paths.** The document is emitted with bundle-relative paths, and
//! the caller loads it with the bundle folder as the base — so an imported
//! show plays immediately, without writing anything into the user's QLab
//! project. `Collect and Save` makes it self-contained afterwards.

pub mod archive;
pub mod cues;
pub mod patches;

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};

use patches::Patches;

/// What one QLab cue became, for the post-import report.
#[derive(Debug, Clone, Serialize)]
pub struct ImportedCue {
    /// QLab class name, e.g. `"AudioCue"`.
    pub qlab_class: String,
    pub cue_number: Option<String>,
    pub cue_name: String,
    /// Inkue cue type it became.
    pub inkue_type: String,
    /// Set when the cue could not be represented faithfully.
    pub note: Option<String>,
}

/// The outcome of an import, for the UI to summarise.
#[derive(Debug, Clone, Serialize)]
pub struct ImportReport {
    pub workspace_name: String,
    pub cue_count: usize,
    pub cue_list_count: usize,
    /// Cues the operator has to look at: Memo placeholders for types with no
    /// Inkue counterpart, plus ones imported deliberately incomplete (a Script
    /// cue arrives disarmed). Not the same as "failed" — nothing is lost.
    pub needs_attention: usize,
    pub media_found: usize,
    pub media_missing: Vec<String>,
    pub cues: Vec<ImportedCue>,
}

/// QLab class → Inkue control cue type. Inkue's `resume` has no QLab
/// counterpart, and QLab's `TargetCue` has no Inkue one.
const CONTROL_CLASSES: [(&str, &str); 7] = [
    ("StartCue", "start"),
    ("PauseCue", "pause"),
    ("LoadCue", "load"),
    ("ResetCue", "reset"),
    ("GotoCue", "goto"),
    ("ArmCue", "arm"),
    ("DisarmCue", "disarm"),
];

/// Decode `path` and build an Inkue workspace document plus a report.
///
/// Returns `(workspace_json, report)`. The JSON carries bundle-relative media
/// paths; load it with the file's parent directory as the base.
pub fn import_workspace(path: &Path) -> Result<(String, ImportReport)> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    let outer = archive::resolve(&bytes)?;

    let workspace_name = outer
        .get("workspaceName")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("Imported QLab Show")
        .to_string();
    let settings = outer.get("settings").cloned().unwrap_or(json!({}));
    let patches = Patches::from_settings(&settings);

    // The cue tree is a second archive nested inside the first.
    let nested = outer
        .get("cueLists")
        .and_then(archive::nested_data)
        .ok_or_else(|| anyhow!("no cue lists in this workspace"))?;
    let root = archive::resolve(&nested)?;

    // Pre-pass: learn each cue's starting pan from a following pan fade.
    let mut pan_starts = std::collections::HashMap::new();
    cues::collect_pan_starts(&root, &mut pan_starts);

    let mut ctx = MapContext { patches: &patches, pan_starts: &pan_starts };
    let mut report_cues = Vec::new();
    let mut cue_lists = Vec::new();
    for list in root.get("cues").and_then(Value::as_array).unwrap_or(&Vec::new()) {
        cue_lists.push(map_cue_list(list, &mut ctx, &mut report_cues));
    }
    if cue_lists.is_empty() {
        cue_lists.push(json!({
            "id": uuid::Uuid::new_v4().to_string(),
            "name": "Cue List 1",
            "mode": "sequential",
            "playhead_cue_id": Value::Null,
            "cues": [],
        }));
    }

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let document = json!({
        "schema_version": 1,
        "workspace": { "name": workspace_name, "created_at": now, "modified_at": now },
        "output_patches": [], "default_output_patch": Value::Null,
        "osc_patches": patches.osc,
        "input_patches": [], "universe_outputs": [],
        "fixtures": [], "fixture_groups": [],
        "cue_lists": cue_lists,
        "active_cue_list_id": cue_lists[0]["id"],
    });

    let base_dir = path.parent().unwrap_or(Path::new("."));
    let (media_found, media_missing) = count_media(&document, base_dir);
    let needs_attention = report_cues.iter().filter(|c| c.note.is_some()).count();

    let report = ImportReport {
        workspace_name: document["workspace"]["name"].as_str().unwrap_or("").to_string(),
        cue_count: report_cues.len(),
        cue_list_count: cue_lists.len(),
        needs_attention,
        media_found,
        media_missing,
        cues: report_cues,
    };
    Ok((serde_json::to_string(&document)?, report))
}

/// Everything the per-cue mapping needs beyond the cue itself.
struct MapContext<'a> {
    patches: &'a Patches,
    /// Target cue id → the pan a following pan fade starts from.
    pan_starts: &'a std::collections::HashMap<String, f64>,
}

fn map_cue_list(list: &Value, ctx: &mut MapContext, report: &mut Vec<ImportedCue>) -> Value {
    let cues: Vec<Value> = list
        .get("cues")
        .and_then(Value::as_array)
        .map(|items| items.iter().map(|c| map_cue(c, ctx, report)).collect())
        .unwrap_or_default();

    // QLab cue *carts* are a distinct kind of list and Inkue has the same
    // concept — flattening them to sequential would change how the show runs.
    let is_cart = list.get("cart").and_then(Value::as_bool).unwrap_or(false);
    let playhead = if is_cart {
        Value::Null
    } else {
        cues.first().map(|c| c["id"].clone()).unwrap_or(Value::Null)
    };

    json!({
        "id": list.get("uniqueID").and_then(Value::as_str)
            .and_then(|s| uuid::Uuid::parse_str(s).ok())
            .unwrap_or_else(uuid::Uuid::new_v4).to_string(),
        "name": list.get("name").and_then(Value::as_str).unwrap_or("Cue List"),
        "mode": if is_cart { "cart" } else { "sequential" },
        "playhead_cue_id": playhead,
        "cues": cues,
    })
}

/// QLab group modes → Inkue. Codes read from a decoded QLab 5 tutorial holding
/// one group of each: 0/1/2 all mean "start first, children chain by their own
/// continue modes"; 3 is timeline (children start together, offset by their
/// pre-waits); 4 start-random; 6 playlist.
fn group_mode(code: i64) -> (&'static str, bool) {
    match code {
        3 => ("simultaneous", false),
        4 => ("start_random", false),
        6 => ("playlist", true),
        _ => ("sequential", false),
    }
}

fn map_cue(cue: &Value, ctx: &mut MapContext, report: &mut Vec<ImportedCue>) -> Value {
    let class = cue.get("__class__").and_then(Value::as_str).unwrap_or("?").to_string();
    let name = cue.get("name").and_then(Value::as_str).unwrap_or("").to_string();
    let number = cue.get("number").and_then(Value::as_str).filter(|s| !s.is_empty())
        .map(str::to_string);

    let mut note: Option<String> = None;
    let mapped = match class.as_str() {
        "AudioCue" => cues::audio(cue, ctx.pan_starts),
        "VideoCue" => cues::video(cue),
        // A camera cue has no file. One that carries a media target anyway
        // (seen in older workspaces) is really a video cue.
        "CameraCue" => {
            if cues::media_path(cue).is_some() {
                cues::video(cue)
            } else {
                cues::camera(cue, ctx.patches)
            }
        }
        "GroupCue" => {
            let children: Vec<Value> = cue
                .get("cues")
                .and_then(Value::as_array)
                .map(|items| items.iter().map(|c| map_cue(c, ctx, report)).collect())
                .unwrap_or_default();
            let (mode, loops) = group_mode(cue.get("groupMode").and_then(Value::as_i64).unwrap_or(0));
            let mut group = match cues::memo(cue, "") {
                Value::Object(map) => map,
                _ => unreachable!(),
            };
            group.insert("type".into(), json!("group"));
            group.insert("cue_type".into(), json!("group"));
            group.remove("memo_text");
            group.insert("group_mode".into(), json!(mode));
            group.insert("playlist_loop".into(), json!(loops));
            group.insert("children".into(), json!(children));
            Value::Object(group)
        }
        "MemoCue" => cues::memo(cue, &name),
        "TitlesCue" => cues::titles(cue),
        "MicCue" => cues::mic(cue),
        "OSCCue" => cues::osc(cue, ctx.patches),
        "MIDICue" => cues::midi(cue, ctx.patches),
        "MIDIFileCue" => cues::midi_file(cue, ctx.patches),
        "TimecodeCue" => cues::timecode(cue, ctx.patches),
        "FadeCue" => cues::fade(cue),
        "StopCue" => cues::stop(cue),
        "DevampCue" => cues::devamp(cue),
        "WaitCue" => cues::wait(cue),
        "ScriptCue" => {
            note = Some("AppleScript kept in the notes; the cue is disarmed".into());
            cues::script(cue)
        }
        "LightCue" => {
            let command = cue.get("commandText").and_then(Value::as_str).unwrap_or("");
            note = Some("QLab's light command language has no Inkue mapping".into());
            cues::unconvertible(
                cue,
                "QLab Light cue",
                &format!("Light command, to rebuild as an Inkue Light Cue:\n{command}"),
            )
        }
        "TargetCue" => {
            let target = cue.get("targetCueNumber").and_then(Value::as_str).unwrap_or("");
            note = Some("retargets a cue at runtime; Inkue targets are fixed".into());
            cues::unconvertible(
                cue,
                "QLab Target cue",
                &format!("Retargets cue \"{target}\" at runtime; this has to be redesigned."),
            )
        }
        other => {
            if let Some((_, cue_type)) = CONTROL_CLASSES.iter().find(|(c, _)| *c == other) {
                cues::control(cue, cue_type)
            } else {
                note = Some("no Inkue equivalent".into());
                cues::unconvertible(cue, &format!("Unconverted QLab {other}"), "")
            }
        }
    };

    report.push(ImportedCue {
        qlab_class: class,
        cue_number: number,
        cue_name: name,
        inkue_type: mapped["cue_type"].as_str().unwrap_or("?").to_string(),
        note,
    });
    mapped
}

/// How many referenced media files resolve against the bundle folder.
fn count_media(document: &Value, base_dir: &Path) -> (usize, Vec<String>) {
    fn walk(cues: &Value, base_dir: &Path, found: &mut usize, missing: &mut Vec<String>) {
        for cue in cues.as_array().unwrap_or(&Vec::new()) {
            if let Some(path) = cue.get("file_path").and_then(Value::as_str) {
                if !path.is_empty() {
                    if base_dir.join(path).exists() {
                        *found += 1;
                    } else {
                        missing.push(path.to_string());
                    }
                }
            }
            if let Some(children) = cue.get("children") {
                walk(children, base_dir, found, missing);
            }
        }
    }
    let mut found = 0;
    let mut missing = Vec::new();
    for list in document["cue_lists"].as_array().unwrap_or(&Vec::new()) {
        walk(&list["cues"], base_dir, &mut found, &mut missing);
    }
    (found, missing)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_an_error_not_a_panic() {
        let err = import_workspace(Path::new("no/such/show.qlab5")).unwrap_err();
        assert!(err.to_string().contains("cannot read"));
    }

    #[test]
    fn a_file_that_is_not_a_workspace_is_rejected() {
        let path = std::env::temp_dir().join(format!("{}.qlab5", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"definitely not a plist").unwrap();
        assert!(import_workspace(&path).is_err());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn group_modes_map_from_qlabs_integers() {
        assert_eq!(group_mode(0), ("sequential", false));
        assert_eq!(group_mode(3), ("simultaneous", false));
        assert_eq!(group_mode(4), ("start_random", false));
        assert_eq!(group_mode(6), ("playlist", true));
    }

    #[test]
    fn media_counting_separates_found_from_missing() {
        let dir = std::env::temp_dir();
        let name = format!("{}.txt", uuid::Uuid::new_v4());
        std::fs::write(dir.join(&name), b"x").unwrap();
        let document = json!({ "cue_lists": [{ "cues": [
            { "file_path": name },
            { "file_path": "audio/gone.wav" },
            { "children": [{ "file_path": "video/also-gone.mov" }] },
        ]}]});
        let (found, missing) = count_media(&document, &dir);
        assert_eq!(found, 1);
        assert_eq!(missing.len(), 2);
        let _ = std::fs::remove_file(dir.join(&name));
    }
}
