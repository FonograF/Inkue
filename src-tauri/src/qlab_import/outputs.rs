//! QLab video stages (QLab 5) and surfaces (QLab 4) → Inkue video outputs.
//!
//! A QLab video cue names where it plays:
//!
//! - **QLab 5** — `viewportID`, the `uniqueID` of the `viewport` of a
//!   `VideoStage` listed in `settings.Video.stages`;
//! - **QLab 4** — `surfaceID` (an integer), an `F53VideoSurface` listed in
//!   `settings.Video.surfaces`, with `defaultSurfaceID` for the rest.
//!
//! Inkue's equivalent is an output.  The first stage (QLab 5) or the default
//! surface (QLab 4) becomes the **main** output; every other one becomes an
//! extra output named after it.  They are imported floating: a QLab
//! workspace's display routing names Mac screens the importing machine does
//! not have, so the operator assigns the screens (Preferences → Outputs).
//!
//! Output ids are stable across re-imports: a QLab 5 stage keeps its own
//! `uniqueID`; a QLab 4 surface id is embedded in a fixed UUID pattern —
//! `qlab2inkue` derives the very same one.

use std::collections::HashMap;

use serde_json::{json, Value};

/// The outputs a workspace declares, and which one each QLab key maps to.
#[derive(Debug, Default)]
pub struct VideoOutputs {
    /// Inkue `video_outputs` entries (extra outputs only).
    extras: Vec<Value>,
    /// QLab viewport / surface key → Inkue output id (`None` = main output).
    by_qlab_key: HashMap<String, Option<String>>,
}

impl VideoOutputs {
    /// Read the stages (QLab 5) or surfaces (QLab 4) of a resolved `settings`.
    pub fn from_settings(settings: &Value) -> Self {
        let mut outputs = Self::default();
        let video = settings.get("Video").unwrap_or(&Value::Null);
        if let Some(stages) = video.get("stages").and_then(Value::as_array) {
            outputs.read_stages(stages);
        } else if let Some(surfaces) = video.get("surfaces").and_then(Value::as_array) {
            let default = video.get("defaultSurfaceID").and_then(Value::as_i64);
            outputs.read_surfaces(surfaces, default);
        }
        outputs
    }

    fn read_stages(&mut self, stages: &[Value]) {
        for (index, stage) in stages.iter().enumerate() {
            let Some(viewport) = stage.pointer("/viewport/uniqueID").and_then(Value::as_str) else {
                continue;
            };
            let id = (index > 0).then(|| {
                stage
                    .get("uniqueID")
                    .and_then(Value::as_str)
                    .and_then(|id| uuid::Uuid::parse_str(id).ok())
                    .unwrap_or_else(uuid::Uuid::new_v4)
                    .to_string()
            });
            let name = stage.get("name").and_then(Value::as_str).unwrap_or("QLab Stage");
            self.add(viewport.to_string(), id, name);
        }
    }

    fn read_surfaces(&mut self, surfaces: &[Value], default: Option<i64>) {
        let main = default
            .filter(|d| surfaces.iter().any(|s| surface_id(s) == Some(*d)))
            .or_else(|| surfaces.first().and_then(surface_id));
        for surface in surfaces {
            let Some(qlab_id) = surface_id(surface) else { continue };
            let id = (Some(qlab_id) != main).then(|| surface_output_id(qlab_id));
            let name = surface.get("name").and_then(Value::as_str).unwrap_or("QLab Surface");
            self.add(qlab_id.to_string(), id, name);
        }
    }

    fn add(&mut self, key: String, id: Option<String>, name: &str) {
        if let Some(id) = &id {
            self.extras.push(json!({
                "id": id,
                "name": unique_name(name, &self.extras),
                "screen": Value::Null,
            }));
        }
        self.by_qlab_key.insert(key, id);
    }

    /// The Inkue output a QLab cue plays on — `None` for the main output,
    /// including a cue that names a stage the workspace no longer has.
    pub fn output_of(&self, cue: &Value) -> Option<String> {
        let key = cue
            .get("viewportID")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| cue.get("surfaceID").and_then(Value::as_i64).map(|id| id.to_string()))?;
        self.by_qlab_key.get(&key).cloned().flatten()
    }

    /// The extra outputs to write into the workspace (`video_outputs`).
    pub fn extras(&self) -> &[Value] {
        &self.extras
    }

    /// Names of the extra outputs, for the import report.
    pub fn names(&self) -> Vec<String> {
        self.extras
            .iter()
            .filter_map(|o| o.get("name").and_then(Value::as_str).map(str::to_string))
            .collect()
    }
}

fn surface_id(surface: &Value) -> Option<i64> {
    surface.get("surfaceID").and_then(Value::as_i64)
}

/// The output id a QLab 4 surface becomes: its id in a fixed UUID pattern, so
/// a re-import of the same show yields the same outputs.
pub fn surface_output_id(surface_id: i64) -> String {
    format!("00000000-0000-4000-8000-{:012x}", (surface_id as u64) & 0xFFFF_FFFF_FFFF)
}

/// Inkue refuses two outputs with the same name; QLab does not.
fn unique_name(name: &str, taken: &[Value]) -> String {
    let used = |candidate: &str| {
        taken.iter().any(|o| {
            o.get("name").and_then(Value::as_str).is_some_and(|n| n.eq_ignore_ascii_case(candidate))
        })
    };
    if !used(name) {
        return name.to_string();
    }
    (2..).map(|n| format!("{name} {n}")).find(|c| !used(c)).unwrap_or_else(|| name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage(name: &str, stage_id: &str, viewport_id: &str) -> Value {
        json!({ "__class__": "VideoStage", "name": name, "uniqueID": stage_id,
                "viewport": { "uniqueID": viewport_id } })
    }

    fn qlab5(stages: Vec<Value>) -> VideoOutputs {
        VideoOutputs::from_settings(&json!({ "Video": { "stages": stages } }))
    }

    const STAGE_2: &str = "5F0B2C1E-0D8E-4C59-9A3B-7E2A1C4D5E6F";

    #[test]
    fn a_single_stage_is_the_main_output() {
        // The real fixture: one stage, the cue names its viewport.
        let outputs = qlab5(vec![stage("Stage 1", "880528AD-A7B0-4C74-B9EA-DF03A9233C24",
                                       "9A890563-17D8-4827-B7EF-D8398DA33C72")]);
        assert!(outputs.extras().is_empty());
        let cue = json!({ "viewportID": "9A890563-17D8-4827-B7EF-D8398DA33C72" });
        assert_eq!(outputs.output_of(&cue), None);
    }

    #[test]
    fn every_other_stage_becomes_an_extra_output_with_its_name_and_id() {
        let outputs = qlab5(vec![
            stage("Stage 1", "880528AD-A7B0-4C74-B9EA-DF03A9233C24", "VP-1"),
            stage("Façade", STAGE_2, "VP-2"),
        ]);
        assert_eq!(outputs.names(), vec!["Façade"]);
        let id = STAGE_2.to_lowercase();
        assert_eq!(outputs.extras()[0]["id"], id);
        assert!(outputs.extras()[0]["screen"].is_null(), "imported floating");
        assert_eq!(outputs.output_of(&json!({ "viewportID": "VP-2" })), Some(id));
        assert_eq!(outputs.output_of(&json!({ "viewportID": "VP-1" })), None);
    }

    #[test]
    fn a_cue_on_an_unknown_stage_plays_on_the_main_output() {
        let outputs = qlab5(vec![stage("Stage 1", "880528AD-A7B0-4C74-B9EA-DF03A9233C24", "VP-1")]);
        assert_eq!(outputs.output_of(&json!({ "viewportID": "gone" })), None);
        assert_eq!(outputs.output_of(&json!({})), None);
    }

    #[test]
    fn qlab4_surfaces_map_around_the_default_one() {
        let settings = json!({ "Video": {
            "defaultSurfaceID": 56931260,
            "surfaces": [
                { "name": "Surface 1", "surfaceID": 56931260 },
                { "name": "Retour", "surfaceID": 77 },
            ],
        }});
        let outputs = VideoOutputs::from_settings(&settings);
        assert_eq!(outputs.names(), vec!["Retour"]);
        assert_eq!(outputs.output_of(&json!({ "surfaceID": 56931260 })), None);
        assert_eq!(
            outputs.output_of(&json!({ "surfaceID": 77 })),
            Some("00000000-0000-4000-8000-00000000004d".to_string()),
        );
    }

    #[test]
    fn a_surface_id_always_gives_the_same_output_id() {
        assert_eq!(surface_output_id(56931260), surface_output_id(56931260));
        assert!(uuid::Uuid::parse_str(&surface_output_id(56931260)).is_ok());
        assert_ne!(surface_output_id(1), surface_output_id(2));
    }

    #[test]
    fn duplicate_stage_names_are_told_apart() {
        let outputs = qlab5(vec![
            stage("Main", "880528AD-A7B0-4C74-B9EA-DF03A9233C24", "VP-1"),
            stage("Projector", STAGE_2, "VP-2"),
            stage("Projector", "6A1B2C3D-4E5F-4A7B-8C9D-0E1F2A3B4C5D", "VP-3"),
        ]);
        assert_eq!(outputs.names(), vec!["Projector", "Projector 2"]);
    }

    #[test]
    fn a_workspace_without_video_settings_has_no_extra_output() {
        assert!(VideoOutputs::from_settings(&json!({})).extras().is_empty());
    }
}
