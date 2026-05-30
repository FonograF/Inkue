//! [`CueRegistry`] maps [`CueType`] to a [`CueFactory`].
//!
//! To add a new cue type, implement [`Cue`](super::traits::Cue) and
//! [`CueFactory`](super::traits::CueFactory), then call
//! [`CueRegistry::register`] at startup.  No other code needs to change.

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use serde_json::Value;

use super::{
    traits::{Cue, CueFactory},
    types::CueType,
};

/// Global factory registry.  All cue types must be registered before any
/// workspace can be loaded from JSON.
pub struct CueRegistry {
    factories: HashMap<CueType, Box<dyn CueFactory>>,
}

impl CueRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
        }
    }

    /// Register a factory for the given cue type.
    /// Overwrites any previously registered factory for that type.
    pub fn register(&mut self, cue_type: CueType, factory: Box<dyn CueFactory>) {
        self.factories.insert(cue_type, factory);
    }

    /// Create a fresh, default-initialised cue of the given type.
    pub fn create(&self, cue_type: &CueType) -> Result<Box<dyn Cue>> {
        self.factories
            .get(cue_type)
            .map(|f| f.create())
            .ok_or_else(|| anyhow!("No factory registered for cue type: {:?}", cue_type))
    }

    /// Deserialise a cue from its persisted JSON representation.
    /// The JSON must contain a `"type"` field that matches a registered [`CueType`].
    ///
    /// [`CueType::Group`] is handled specially: children are deserialised
    /// recursively using this same registry, bypassing the normal factory path.
    pub fn from_json(&self, value: Value) -> Result<Box<dyn Cue>> {
        let cue_type: CueType = serde_json::from_value(
            value
                .get("type")
                .cloned()
                .ok_or_else(|| anyhow!("Cue JSON missing 'type' field"))?,
        )?;

        if cue_type == CueType::Group {
            return super::group_cue::GroupCue::from_json_with_registry(&value, self);
        }

        self.factories
            .get(&cue_type)
            .ok_or_else(|| anyhow!("No factory registered for cue type: {:?}", cue_type))?
            .from_json(value)
    }

    /// Returns `true` if a factory is registered for the given type.
    pub fn has(&self, cue_type: &CueType) -> bool {
        self.factories.contains_key(cue_type)
    }
}

impl Default for CueRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cue::{memo_cue::MemoCue, audio_cue::AudioCueFactory, memo_cue::MemoCueFactory};

    #[test]
    fn register_and_create_memo() {
        let mut registry = CueRegistry::new();
        registry.register(CueType::Memo, Box::new(MemoCueFactory));

        let cue = registry.create(&CueType::Memo).expect("should create memo");
        assert_eq!(cue.cue_type(), CueType::Memo);
    }

    #[test]
    fn create_unknown_type_returns_error() {
        let registry = CueRegistry::new();
        let result = registry.create(&CueType::Audio);
        assert!(result.is_err(), "Expected error for unregistered type");
    }

    #[test]
    fn from_json_roundtrip_memo() {
        let mut registry = CueRegistry::new();
        registry.register(CueType::Memo, Box::new(MemoCueFactory));

        let mut cue = MemoCue::new();
        cue.set_name("Test Memo".to_string());
        cue.set_number(Some("1".to_string()));
        let json = cue.serialize();

        let deserialized = registry.from_json(json).expect("should deserialize");
        assert_eq!(deserialized.name(), "Test Memo");
        assert_eq!(deserialized.number(), Some("1"));
    }

    #[test]
    fn from_json_missing_type_returns_error() {
        let registry = CueRegistry::new();
        let json = serde_json::json!({ "name": "orphan" });
        assert!(registry.from_json(json).is_err());
    }

    #[test]
    fn register_and_create_audio() {
        let mut registry = CueRegistry::new();
        registry.register(CueType::Audio, Box::new(AudioCueFactory));
        let cue = registry.create(&CueType::Audio).expect("should create audio cue");
        assert_eq!(cue.cue_type(), CueType::Audio);
    }
}
