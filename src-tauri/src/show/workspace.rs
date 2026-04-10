//! [`Workspace`] — the top-level save unit for a WinCue show.
//!
//! Corresponds to a `.wincue` file on disk.

use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    cue::registry::CueRegistry,
    engine::device_manager::OutputPatch,
    preferences::AppPreferences,
};

use super::cue_list::CueList;

/// Serialisable workspace metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceMetadata {
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub modified_at: DateTime<Utc>,
}

impl WorkspaceMetadata {
    fn new(name: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            name: name.into(),
            created_at: now,
            modified_at: now,
        }
    }
}

/// The workspace — a complete show document.
pub struct Workspace {
    pub metadata: WorkspaceMetadata,
    /// All cue lists in this workspace.
    pub cue_lists: Vec<CueList>,
    /// Output patch table (shared across cue lists).
    pub output_patches: Vec<OutputPatch>,
    /// ID of the default output patch.
    pub default_output_patch_id: Option<Uuid>,
    /// Application-wide preferences (audio engine, defaults, …).
    pub preferences: AppPreferences,
    /// Path to the .wincue file on disk, if it has been saved.
    pub file_path: Option<PathBuf>,
    /// Whether the workspace has unsaved changes.
    pub is_modified: bool,
}

impl Workspace {
    /// Create a new, empty workspace with one default cue list.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            metadata: WorkspaceMetadata::new(name),
            cue_lists: vec![CueList::new("Cue List 1")],
            output_patches: Vec::new(),
            default_output_patch_id: None,
            preferences: AppPreferences::default(),
            file_path: None,
            is_modified: false,
        }
    }

    /// Mark the workspace as modified (unsaved changes exist).
    pub fn mark_modified(&mut self) {
        self.is_modified = true;
        self.metadata.modified_at = Utc::now();
    }

    /// The active (first) cue list, if any.
    pub fn active_cue_list(&self) -> Option<&CueList> {
        self.cue_lists.first()
    }

    /// Mutable access to the active cue list.
    pub fn active_cue_list_mut(&mut self) -> Option<&mut CueList> {
        self.cue_lists.first_mut()
    }

    // -----------------------------------------------------------------------
    // Persistence
    // -----------------------------------------------------------------------

    /// Serialise the workspace to a JSON string for writing to a `.wincue` file.
    pub fn to_json(&self) -> Result<String> {
        let cue_lists_json: Vec<serde_json::Value> = self
            .cue_lists
            .iter()
            .map(|cl| cl.to_json())
            .collect();

        let doc = serde_json::json!({
            "version": "1.0.0",
            "workspace": self.metadata,
            "output_patches": self.output_patches,
            "default_output_patch": self.default_output_patch_id,
            "preferences": self.preferences,
            "cue_lists": cue_lists_json,
        });

        serde_json::to_string_pretty(&doc).context("Failed to serialize workspace")
    }

    /// Save the workspace to the given path (or the previously saved path).
    pub fn save(&mut self, path: Option<PathBuf>) -> Result<()> {
        let target = path.or_else(|| self.file_path.clone())
            .ok_or_else(|| anyhow::anyhow!("No file path set for workspace"))?;

        let json = self.to_json()?;
        std::fs::write(&target, json)
            .with_context(|| format!("Failed to write workspace to {}", target.display()))?;

        // Derive the workspace name from the filename stem (e.g. "My Show" from
        // "My Show.wincue") so the title bar reflects the saved file immediately.
        if let Some(stem) = target.file_stem().and_then(|s| s.to_str()) {
            self.metadata.name = stem.to_string();
        }
        self.file_path = Some(target);
        self.is_modified = false;
        Ok(())
    }

    /// Load a workspace from a `.wincue` file.
    pub fn load(path: PathBuf, registry: &CueRegistry) -> Result<Self> {
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read workspace file: {}", path.display()))?;

        let doc: serde_json::Value =
            serde_json::from_str(&content).context("Invalid JSON in workspace file")?;

        let mut metadata: WorkspaceMetadata = serde_json::from_value(
            doc.get("workspace")
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("Missing 'workspace' key"))?,
        )?;

        let patches_val = doc.get("output_patches").cloned().unwrap_or_default();
        let output_patches: Vec<OutputPatch> =
            serde_json::from_value(patches_val).unwrap_or_default();

        let default_patch: Option<Uuid> = doc
            .get("default_output_patch")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok());

        let preferences: AppPreferences = doc
            .get("preferences")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let cue_lists_val = doc
            .get("cue_lists")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut cue_lists = Vec::new();
        for cl_val in cue_lists_val {
            cue_lists.push(CueList::from_json(cl_val, registry)?);
        }

        if cue_lists.is_empty() {
            cue_lists.push(CueList::new("Cue List 1"));
        }

        // Derive the name from the filename stem so it always matches the file,
        // even if the JSON still contains an older name (e.g. "Untitled").
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            metadata.name = stem.to_string();
        }

        Ok(Self {
            metadata,
            cue_lists,
            output_patches,
            default_output_patch_id: default_patch,
            preferences,
            file_path: Some(path),
            is_modified: false,
        })
    }
}
