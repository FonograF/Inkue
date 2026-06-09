//! [`OscPatch`] — a named UDP send target for OSC cues.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A named OSC send target stored in the workspace.
///
/// Each [`crate::cue::osc_cue::OscCue`] message references one patch by ID.
/// At GO time the patch is resolved from the workspace's `osc_patches` list
/// via [`crate::cue::context::CueContext::resolve_osc_patch`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OscPatch {
    /// Unique identifier.
    pub id: Uuid,
    /// Human-readable name shown in the inspector (e.g. `"QLab"`).
    pub name: String,
    /// Destination IP address (e.g. `"192.168.1.100"` or `"127.0.0.1"`).
    pub ip: String,
    /// UDP port to send to (e.g. `53000`).
    pub port: u16,
}

impl OscPatch {
    /// Create a new patch with a fresh UUID.
    pub fn new(name: impl Into<String>, ip: impl Into<String>, port: u16) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            ip: ip.into(),
            port,
        }
    }
}
