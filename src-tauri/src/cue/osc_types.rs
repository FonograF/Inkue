//! OSC argument and message types used by [`super::osc_cue::OscCue`].

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A single OSC argument value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum OscArg {
    Int(i32),
    Float(f32),
    Str(String),
    Bool(bool),
}

/// One OSC message to send on GO — targets a named patch with an address and args.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OscMessage {
    /// ID of the [`crate::engine::osc_patch::OscPatch`] to send to.
    pub patch_id: Uuid,
    /// OSC address string, e.g. `"/my/device/volume"`.
    pub address: String,
    /// Arguments to include in the message.
    pub args: Vec<OscArg>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc_arg_int_roundtrip() {
        let arg = OscArg::Int(42);
        let json = serde_json::to_string(&arg).unwrap();
        let back: OscArg = serde_json::from_str(&json).unwrap();
        assert_eq!(back, OscArg::Int(42));
    }

    #[test]
    fn osc_arg_float_roundtrip() {
        let arg = OscArg::Float(2.5);
        let json = serde_json::to_string(&arg).unwrap();
        let back: OscArg = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, OscArg::Float(f) if (f - 2.5).abs() < 1e-5));
    }

    #[test]
    fn osc_arg_str_roundtrip() {
        let arg = OscArg::Str("hello".to_string());
        let json = serde_json::to_string(&arg).unwrap();
        let back: OscArg = serde_json::from_str(&json).unwrap();
        assert_eq!(back, OscArg::Str("hello".to_string()));
    }

    #[test]
    fn osc_arg_bool_roundtrip() {
        let arg = OscArg::Bool(true);
        let json = serde_json::to_string(&arg).unwrap();
        let back: OscArg = serde_json::from_str(&json).unwrap();
        assert_eq!(back, OscArg::Bool(true));
    }

    #[test]
    fn osc_message_roundtrip() {
        let msg = OscMessage {
            patch_id: Uuid::nil(),
            address: "/test/addr".to_string(),
            args: vec![OscArg::Int(1), OscArg::Float(2.0), OscArg::Str("x".into()), OscArg::Bool(false)],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: OscMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.address, "/test/addr");
        assert_eq!(back.args.len(), 4);
    }
}
