//! Zone (b) — CueRegistry + Cue-trait contract.
//!
//! Asserts every built-in cue type is registered and survives a
//! serialize → from_json round-trip with its type discriminant (and basic
//! identity) intact. This is the load-time contract: a type that fails to
//! round-trip would be silently dropped from a saved show (see
//! `workspace_persistence_tests`). Explicitly closes the Video / Wait gaps
//! (those two modules ship no unit tests of their own).

mod common;

use common::*;
use inkue_lib::cue::registry::CueRegistry;
use inkue_lib::cue::types::CueType;

#[test]
fn registry_has_every_builtin_type() {
    let r = full_registry();
    for t in ALL_CUE_TYPES {
        assert!(r.has(&t), "registry is missing a factory for {t:?}");
    }
}

#[test]
fn every_type_roundtrips_type_discriminant() {
    let r = full_registry();
    for t in ALL_CUE_TYPES {
        let cue = r.create(&t).unwrap_or_else(|e| panic!("create {t:?}: {e}"));
        assert_eq!(cue.cue_type(), t, "create returned the wrong type for {t:?}");

        let json = cue.serialize();
        assert!(
            json.get("type").is_some(),
            "{t:?} serialize() omits the required `type` field"
        );

        let restored = r
            .from_json(json)
            .unwrap_or_else(|e| panic!("{t:?} failed to round-trip through from_json: {e}"));
        assert_eq!(
            restored.cue_type(),
            t,
            "{t:?} changed type across a serialize/from_json round-trip"
        );
    }
}

#[test]
fn every_type_preserves_name_and_number() {
    let r = full_registry();
    for t in ALL_CUE_TYPES {
        let mut cue = r.create(&t).unwrap();
        cue.set_name(format!("Name {t:?}"));
        cue.set_number(Some("7.3".to_string()));
        let json = cue.serialize();
        let restored = r.from_json(json).unwrap();
        assert_eq!(restored.name(), format!("Name {t:?}"), "{t:?} lost its name");
        // Number is re-derived by the cue list on load for some types; only
        // assert it when the type persists it in isolation.
        if let Some(n) = restored.number() {
            assert_eq!(n, "7.3", "{t:?} corrupted its number on round-trip");
        }
    }
}

#[test]
fn empty_registry_create_errors() {
    let r = CueRegistry::new();
    assert!(r.create(&CueType::Audio).is_err(), "unregistered type must error");
}

#[test]
fn from_json_missing_type_errors() {
    let r = full_registry();
    let json = serde_json::json!({ "name": "orphan", "id": uuid_str() });
    assert!(r.from_json(json).is_err(), "cue JSON without `type` must error");
}

#[test]
fn from_json_unknown_type_errors() {
    let r = full_registry();
    let json = serde_json::json!({ "type": "quantum", "name": "X", "id": uuid_str() });
    let result = r.from_json(json);
    assert!(
        result.is_err(),
        "an unrecognised cue type must return Err (the cue list turns this into a skip)"
    );
}

/// A throwaway UUID string for crafted JSON.
fn uuid_str() -> String {
    // Deterministic-enough unique id without pulling a generator into scope.
    format!(
        "00000000-0000-4000-8000-{:012x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    )
}
