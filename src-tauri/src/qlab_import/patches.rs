//! Workspace-level patch tables from a QLab workspace's `settings`.
//!
//! QLab cues reference their destinations by patch id. Inkue needs an OSC
//! patch object for an OSC cue, and a plain **device name** for a MIDI cue —
//! so the tables are read once up front and consulted while cues are mapped.

use std::collections::HashMap;

use serde_json::{json, Value};

/// A UUID of all zeroes — an OSC message with nowhere to go. Preflight reports
/// it as a missing patch rather than the import failing outright.
const NIL_UUID: &str = "00000000-0000-0000-0000-000000000000";

/// Resolved destination tables for one workspace.
#[derive(Debug, Default)]
pub struct Patches {
    /// Inkue `osc_patches` entries, emitted into the workspace document.
    pub osc: Vec<Value>,
    /// QLab `networkPatchID` → Inkue OSC patch uuid.
    osc_by_qlab_id: HashMap<String, String>,
    /// QLab `midiPatchID` → MIDI output port name.
    midi_by_qlab_id: HashMap<String, String>,
    /// QLab `cameraPatchID` → capture device name.
    camera_by_qlab_id: HashMap<String, String>,
}

impl Patches {
    /// Read every patch table out of a resolved `settings` dictionary.
    pub fn from_settings(settings: &Value) -> Self {
        let mut patches = Self::default();
        patches.read_network(settings);
        patches.read_midi(settings);
        patches.read_camera(settings);
        patches
    }

    fn read_network(&mut self, settings: &Value) {
        let Some(list) = settings.pointer("/Network/networkPatches").and_then(Value::as_array)
        else {
            return;
        };
        for patch in list {
            let Some(data) = patch.get("data") else { continue };
            let Some(qlab_id) = data.get("uniqueID").and_then(Value::as_str) else { continue };
            let client = data
                .get("clientStates")
                .and_then(Value::as_array)
                .and_then(|c| c.first());
            let host = client
                .and_then(|c| c.get("host"))
                .and_then(Value::as_str)
                .unwrap_or("127.0.0.1");
            let port = client
                .and_then(|c| c.get("port"))
                .and_then(Value::as_i64)
                .unwrap_or(53000);

            let id = normalise_uuid(qlab_id);
            self.osc.push(json!({
                "id": id,
                "name": data.get("name").and_then(Value::as_str).unwrap_or("QLab Patch"),
                // Inkue sends to "{ip}:{port}", which resolves hostnames — but
                // spelling out loopback keeps a DNS lookup off the cue path.
                "ip": if host.eq_ignore_ascii_case("localhost") { "127.0.0.1" } else { host },
                "port": port.clamp(1, 65535),
            }));
            self.osc_by_qlab_id.insert(qlab_id.to_string(), id);
        }
    }

    fn read_midi(&mut self, settings: &Value) {
        let Some(list) = settings.pointer("/MIDI/midiPatches").and_then(Value::as_array) else {
            return;
        };
        for patch in list {
            let data = patch.get("data").unwrap_or(patch);
            let Some(qlab_id) = data.get("uniqueID").and_then(Value::as_str) else { continue };
            let name = data
                .get("destinationName")
                .or_else(|| data.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("");
            self.midi_by_qlab_id.insert(qlab_id.to_string(), name.to_string());
        }
    }

    fn read_camera(&mut self, settings: &Value) {
        let Some(list) = settings.pointer("/Camera/cameraPatches").and_then(Value::as_array) else {
            return;
        };
        for patch in list {
            let Some(qlab_id) = patch.get("uniqueID").and_then(Value::as_str) else { continue };
            let name = patch
                .pointer("/serializedDescription/name")
                .or_else(|| patch.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("");
            self.camera_by_qlab_id.insert(qlab_id.to_string(), name.to_string());
        }
    }

    /// The Inkue OSC patch id for a QLab network patch, falling back to the
    /// only patch in the show (the common single-destination case).
    pub fn osc_patch(&self, qlab_id: Option<&str>) -> String {
        if let Some(id) = qlab_id.and_then(|id| self.osc_by_qlab_id.get(id)) {
            return id.clone();
        }
        match self.osc.first().and_then(|p| p.get("id")).and_then(Value::as_str) {
            Some(only) => only.to_string(),
            None => NIL_UUID.to_string(),
        }
    }

    pub fn midi_port(&self, qlab_id: Option<&str>) -> String {
        qlab_id
            .and_then(|id| self.midi_by_qlab_id.get(id))
            .cloned()
            .unwrap_or_default()
    }

    pub fn camera_device(&self, qlab_id: Option<&str>) -> String {
        qlab_id
            .and_then(|id| self.camera_by_qlab_id.get(id))
            .cloned()
            .unwrap_or_default()
    }
}

/// QLab ids are UUIDs already; normalise the case so they match Inkue's form.
fn normalise_uuid(id: &str) -> String {
    uuid::Uuid::parse_str(id)
        .unwrap_or_else(|_| uuid::Uuid::new_v4())
        .to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Value {
        json!({
            "Network": { "networkPatches": [{
                "data": {
                    "uniqueID": "EEFA94D4-A2D7-4E60-B4B1-AF785383F70A",
                    "name": "Patch 1",
                    "clientStates": [{ "host": "localhost", "port": 53000 }],
                },
            }]},
            "Camera": { "cameraPatches": [{
                "uniqueID": "66E48FF8-D203-428C-A9A3-C7551CB684C0",
                "name": "Patch 1",
                "serializedDescription": { "name": "FaceTime HD Camera" },
            }]},
            "MIDI": { "midiPatches": [] },
        })
    }

    #[test]
    fn a_network_patch_becomes_an_osc_patch() {
        let patches = Patches::from_settings(&settings());
        assert_eq!(patches.osc.len(), 1);
        assert_eq!(patches.osc[0]["port"], json!(53000));
        assert_eq!(patches.osc[0]["ip"], json!("127.0.0.1"), "loopback spelled out");
        assert_eq!(
            patches.osc_patch(Some("EEFA94D4-A2D7-4E60-B4B1-AF785383F70A")),
            "eefa94d4-a2d7-4e60-b4b1-af785383f70a"
        );
    }

    #[test]
    fn an_unknown_patch_falls_back_to_the_only_one() {
        let patches = Patches::from_settings(&settings());
        assert_eq!(patches.osc_patch(None), patches.osc[0]["id"].as_str().unwrap());
    }

    #[test]
    fn with_no_patches_at_all_osc_points_nowhere_rather_than_failing() {
        let patches = Patches::default();
        assert_eq!(patches.osc_patch(Some("whatever")), NIL_UUID);
    }

    #[test]
    fn a_camera_patch_yields_its_device_name() {
        let patches = Patches::from_settings(&settings());
        assert_eq!(
            patches.camera_device(Some("66E48FF8-D203-428C-A9A3-C7551CB684C0")),
            "FaceTime HD Camera"
        );
        assert_eq!(patches.camera_device(Some("nope")), "");
    }

    #[test]
    fn absent_settings_sections_are_not_an_error() {
        let patches = Patches::from_settings(&json!({}));
        assert!(patches.osc.is_empty());
        assert_eq!(patches.midi_port(Some("x")), "");
    }
}
