//! [`Workspace`] — the top-level save unit for a WinCue show.
//!
//! Corresponds to a `.wincue` file on disk.

/// Bumped whenever the `.wincue` JSON format gains a breaking change.
/// Files written by newer WinCue versions (schema > this) are rejected
/// at load time to prevent silent data corruption.
pub const SCHEMA_VERSION: u32 = 1;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    cue::{registry::CueRegistry, traits::Cue, types::CueType},
    engine::{audio_input::InputPatch, device_manager::OutputPatch, dmx_sink::UniverseOutput, fixture::{FixtureGroup, PatchedFixture}, osc_patch::OscPatch},
    preferences::AppPreferences,
};

use super::cue_list::CueList;

// ---------------------------------------------------------------------------
// Path helpers — keep file paths relative in the .wincue JSON so workspaces
// are portable across machines and drive letters.
// ---------------------------------------------------------------------------

/// Recursively walk a cues JSON array and convert absolute `file_path` values
/// to paths relative to `base` (the directory containing the .wincue file).
fn relativize_paths(value: &mut serde_json::Value, base: &std::path::Path) {
    match value {
        serde_json::Value::Array(arr) => {
            for item in arr.iter_mut() {
                relativize_paths(item, base);
            }
        }
        serde_json::Value::Object(obj) => {
            if let Some(serde_json::Value::String(p)) = obj.get("file_path") {
                let path = std::path::Path::new(p.as_str());
                if path.is_absolute() {
                    if let Ok(rel) = path.strip_prefix(base) {
                        // Use forward slashes so the file is readable on any OS.
                        let rel_str = rel.to_string_lossy().replace('\\', "/");
                        obj.insert("file_path".into(), serde_json::Value::String(rel_str));
                    }
                    // If strip_prefix fails (file is on a different drive), keep absolute.
                }
            }
            // Recurse into group children.
            if let Some(children) = obj.get_mut("children") {
                relativize_paths(children, base);
            }
        }
        _ => {}
    }
}

/// Recursively walk a cues JSON array and resolve relative `file_path` values
/// to absolute paths using `base` (the directory containing the .wincue file).
fn absolutize_paths(value: &mut serde_json::Value, base: &std::path::Path) {
    match value {
        serde_json::Value::Array(arr) => {
            for item in arr.iter_mut() {
                absolutize_paths(item, base);
            }
        }
        serde_json::Value::Object(obj) => {
            if let Some(serde_json::Value::String(p)) = obj.get("file_path") {
                let path = std::path::Path::new(p.as_str());
                if path.is_relative() && !p.is_empty() {
                    let abs = base.join(path);
                    obj.insert("file_path".into(),
                        serde_json::Value::String(abs.to_string_lossy().into_owned()));
                }
            }
            if let Some(children) = obj.get_mut("children") {
                absolutize_paths(children, base);
            }
        }
        _ => {}
    }
}

/// Recursively walk cue JSON and replace `file_path` values using `path_map`
/// (absolute path → new relative path).  Paths not present in the map are
/// left unchanged so the subsequent `relativize_paths` pass can handle them.
fn remap_paths(value: &mut serde_json::Value, path_map: &HashMap<PathBuf, String>) {
    match value {
        serde_json::Value::Array(arr) => {
            for item in arr.iter_mut() {
                remap_paths(item, path_map);
            }
        }
        serde_json::Value::Object(obj) => {
            if let Some(serde_json::Value::String(p)) = obj.get("file_path") {
                let path = PathBuf::from(p.as_str());
                if let Some(new_rel) = path_map.get(&path) {
                    obj.insert("file_path".into(), serde_json::Value::String(new_rel.clone()));
                }
            }
            if let Some(children) = obj.get_mut("children") {
                remap_paths(children, path_map);
            }
        }
        _ => {}
    }
}

fn sanitize_for_filename(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 32 => '_',
            c => c,
        })
        .collect();
    s.trim().to_string()
}

fn unique_filename(orig: &str, used: &mut HashSet<String>) -> String {
    if used.insert(orig.to_string()) {
        return orig.to_string();
    }
    let (stem, ext) = orig
        .rfind('.')
        .map_or((orig, ""), |i| (&orig[..i], &orig[i..]));
    let mut n = 1u32;
    loop {
        let candidate = format!("{stem}_{n}{ext}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        n += 1;
    }
}

/// Recursively collect `(cue_type, abs_path)` for every Audio / Video / Image
/// cue in the tree, including those nested inside groups.
fn collect_all_media_paths(
    cues: &[Box<dyn Cue>],
    out: &mut Vec<(CueType, PathBuf)>,
) {
    for cue in cues {
        if let ct @ (CueType::Audio | CueType::Video | CueType::Image) = cue.cue_type() {
            if let Some(path) = cue.media_file_path() {
                if !path.as_os_str().is_empty() {
                    out.push((ct, path.to_path_buf()));
                }
            }
        }
        if let Some(children) = cue.child_cues() {
            collect_all_media_paths(children, out);
        }
    }
}

// ---------------------------------------------------------------------------
// Collect & Save report
// ---------------------------------------------------------------------------

/// Result returned by [`Workspace::collect_and_save`].
#[derive(Debug, Serialize)]
pub struct CollectReport {
    /// Absolute path to the newly created `.wincue` file.
    pub workspace_path: String,
    /// Number of media files successfully copied to the new location.
    pub files_copied: u32,
    /// Number of files already at their destination (source == destination).
    pub files_skipped: u32,
    /// Paths of files referenced by cues but missing from disk.
    pub files_missing: Vec<String>,
}

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
    /// ID of the currently active cue list.
    pub active_cue_list_id: Uuid,
    /// Output patch table (shared across cue lists).
    pub output_patches: Vec<OutputPatch>,
    /// ID of the default output patch.
    pub default_output_patch_id: Option<Uuid>,
    /// OSC send patch table.
    pub osc_patches: Vec<OscPatch>,
    /// Live audio input patch table (Mic Cues).
    pub input_patches: Vec<InputPatch>,
    /// DMX universe → destination mapping (sACN / Art-Net outputs).
    pub universe_outputs: Vec<UniverseOutput>,
    /// Lighting fixture patch (named instruments at DMX addresses).
    pub fixtures: Vec<PatchedFixture>,
    /// Fixture groups (drive several fixtures from one Light Cue control).
    pub fixture_groups: Vec<FixtureGroup>,
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
        let default_list = CueList::new("Cue List 1");
        let active_id = default_list.id;
        Self {
            metadata: WorkspaceMetadata::new(name),
            cue_lists: vec![default_list],
            active_cue_list_id: active_id,
            output_patches: Vec::new(),
            default_output_patch_id: None,
            osc_patches: Vec::new(),
            input_patches: Vec::new(),
            universe_outputs: Vec::new(),
            fixtures: Vec::new(),
            fixture_groups: Vec::new(),
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

    /// The active cue list, identified by `active_cue_list_id`.
    pub fn active_cue_list(&self) -> Option<&CueList> {
        self.cue_lists.iter().find(|cl| cl.id == self.active_cue_list_id)
    }

    /// Mutable access to the active cue list.
    pub fn active_cue_list_mut(&mut self) -> Option<&mut CueList> {
        let id = self.active_cue_list_id;
        self.cue_lists.iter_mut().find(|cl| cl.id == id)
    }

    /// Look up any cue list by its ID.
    pub fn cue_list_by_id(&self, id: Uuid) -> Option<&CueList> {
        self.cue_lists.iter().find(|cl| cl.id == id)
    }

    /// Mutable access to any cue list by its ID.
    pub fn cue_list_by_id_mut(&mut self, id: Uuid) -> Option<&mut CueList> {
        self.cue_lists.iter_mut().find(|cl| cl.id == id)
    }

    // -----------------------------------------------------------------------
    // Persistence
    // -----------------------------------------------------------------------

    /// Serialise the workspace to a JSON string, with file paths made relative
    /// to `save_path` so the `.wincue` file is portable.
    fn to_json(&self, save_path: &Path) -> Result<String> {
        let base = save_path.parent();

        let mut cue_lists_json: Vec<serde_json::Value> = self
            .cue_lists
            .iter()
            .map(|cl| cl.to_json())
            .collect();

        if let Some(base_dir) = base {
            for cl in &mut cue_lists_json {
                if let Some(cues) = cl.get_mut("cues") {
                    relativize_paths(cues, base_dir);
                }
            }
        }

        let doc = serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "workspace": self.metadata,
            "output_patches": self.output_patches,
            "default_output_patch": self.default_output_patch_id,
            "osc_patches": self.osc_patches,
            "input_patches": self.input_patches,
            "universe_outputs": self.universe_outputs,
            "fixtures": self.fixtures,
            "fixture_groups": self.fixture_groups,
            "preferences": self.preferences,
            "cue_lists": cue_lists_json,
            "active_cue_list_id": self.active_cue_list_id,
        });

        serde_json::to_string_pretty(&doc).context("Failed to serialize workspace")
    }

    /// Save the workspace to the given path (or the previously saved path).
    pub fn save(&mut self, path: Option<PathBuf>) -> Result<()> {
        let target = path.or_else(|| self.file_path.clone())
            .ok_or_else(|| anyhow::anyhow!("No file path set for workspace"))?;

        let json = self.to_json(&target)?;
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

    /// Copy all media files referenced by this workspace into
    /// `{target_dir}/{workspace_name}/audio|video|images/` and write a
    /// self-contained `.wincue` file with updated relative paths.
    ///
    /// The workspace in memory is **not modified** — this is a pure export.
    pub fn collect_and_save(&self, target_dir: &Path) -> Result<CollectReport> {
        let raw_name = if self.metadata.name.is_empty() { "Untitled" } else { &self.metadata.name };
        let safe_name = {
            let s = sanitize_for_filename(raw_name);
            if s.is_empty() { "Untitled".into() } else { s }
        };

        let project_dir = target_dir.join(&safe_name);
        for sub in &["audio", "video", "images"] {
            std::fs::create_dir_all(project_dir.join(sub))
                .with_context(|| format!("Cannot create {}/{sub}", project_dir.display()))?;
        }

        // Collect (cue_type, abs_path) pairs, deduplicating by path.
        let mut raw: Vec<(CueType, PathBuf)> = Vec::new();
        for cl in &self.cue_lists {
            collect_all_media_paths(&cl.cues, &mut raw);
        }
        let mut seen: HashSet<PathBuf> = HashSet::new();
        let deduped: Vec<(CueType, PathBuf)> =
            raw.into_iter().filter(|(_, p)| seen.insert(p.clone())).collect();

        // Per-subfolder used-filename sets for conflict resolution.
        let mut used: HashMap<&str, HashSet<String>> = [("audio", HashSet::new()), ("video", HashSet::new()), ("images", HashSet::new())]
            .into_iter()
            .collect();

        let mut path_map: HashMap<PathBuf, String> = HashMap::new();
        let mut files_copied = 0u32;
        let mut files_skipped = 0u32;
        let mut files_missing: Vec<String> = Vec::new();

        for (cue_type, abs_path) in &deduped {
            if !abs_path.exists() {
                files_missing.push(abs_path.to_string_lossy().into_owned());
                continue;
            }

            let subfolder = match cue_type {
                CueType::Audio => "audio",
                CueType::Video => "video",
                CueType::Image => "images",
                _ => continue,
            };

            let orig_name = abs_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "file".into());

            let dest_name = unique_filename(&orig_name, used.get_mut(subfolder).unwrap());
            let dest_path = project_dir.join(subfolder).join(&dest_name);
            let new_rel = format!("{subfolder}/{dest_name}");

            if abs_path == &dest_path {
                // Source == destination: file is already at the right place.
                files_skipped += 1;
            } else {
                std::fs::copy(abs_path, &dest_path).with_context(|| {
                    format!("Failed to copy {} → {}", abs_path.display(), dest_path.display())
                })?;
                files_copied += 1;
            }
            path_map.insert(abs_path.clone(), new_rel);
        }

        let new_wincue_path = project_dir.join(format!("{safe_name}.wincue"));
        let json = self.to_json_collected(&new_wincue_path, &path_map)?;
        std::fs::write(&new_wincue_path, json)
            .with_context(|| format!("Failed to write {}", new_wincue_path.display()))?;

        Ok(CollectReport {
            workspace_path: new_wincue_path.to_string_lossy().into_owned(),
            files_copied,
            files_skipped,
            files_missing,
        })
    }

    /// Like [`Self::to_json`] but applies `path_map` before the standard
    /// `relativize_paths` step, so collected files get their new relative paths.
    fn to_json_collected(
        &self,
        save_path: &Path,
        path_map: &HashMap<PathBuf, String>,
    ) -> Result<String> {
        let base = save_path.parent();

        let mut cue_lists_json: Vec<serde_json::Value> =
            self.cue_lists.iter().map(|cl| cl.to_json()).collect();

        for cl in &mut cue_lists_json {
            if let Some(cues) = cl.get_mut("cues") {
                remap_paths(cues, path_map);
                if let Some(base_dir) = base {
                    relativize_paths(cues, base_dir);
                }
            }
        }

        let doc = serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "workspace": self.metadata,
            "output_patches": self.output_patches,
            "default_output_patch": self.default_output_patch_id,
            "osc_patches": self.osc_patches,
            "input_patches": self.input_patches,
            "universe_outputs": self.universe_outputs,
            "fixtures": self.fixtures,
            "fixture_groups": self.fixture_groups,
            "preferences": self.preferences,
            "cue_lists": cue_lists_json,
            "active_cue_list_id": self.active_cue_list_id,
        });

        serde_json::to_string_pretty(&doc).context("Failed to serialize collected workspace")
    }

    /// Load a workspace from a `.wincue` file.
    pub fn load(path: PathBuf, registry: &CueRegistry) -> Result<Self> {
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read workspace file: {}", path.display()))?;

        let doc: serde_json::Value =
            serde_json::from_str(&content).context("Invalid JSON in workspace file")?;

        let file_schema: u32 = doc
            .get("schema_version")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        if file_schema > SCHEMA_VERSION {
            anyhow::bail!(
                "This workspace was created with a newer version of WinCue \
                 (schema v{file_schema}, this app supports up to v{SCHEMA_VERSION}). \
                 Please update WinCue to open it."
            );
        }

        let mut metadata: WorkspaceMetadata = serde_json::from_value(
            doc.get("workspace")
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("Missing 'workspace' key"))?,
        )?;

        let patches_val = doc.get("output_patches").cloned().unwrap_or_default();
        let output_patches: Vec<OutputPatch> =
            serde_json::from_value(patches_val).unwrap_or_default();

        let osc_patches: Vec<OscPatch> = doc
            .get("osc_patches")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let input_patches: Vec<InputPatch> = doc
            .get("input_patches")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let universe_outputs: Vec<UniverseOutput> = doc
            .get("universe_outputs")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let fixtures: Vec<PatchedFixture> = doc
            .get("fixtures")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let fixture_groups: Vec<FixtureGroup> = doc
            .get("fixture_groups")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

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

        let base_dir = path.parent().map(|p| p.to_path_buf());
        let mut cue_lists = Vec::new();
        for mut cl_val in cue_lists_val {
            if let Some(ref base) = base_dir {
                if let Some(cues) = cl_val.get_mut("cues") {
                    absolutize_paths(cues, base);
                }
            }
            cue_lists.push(CueList::from_json(cl_val, registry)?);
        }

        if cue_lists.is_empty() {
            cue_lists.push(CueList::new("Cue List 1"));
        }

        // Resolve the active cue list: try the saved ID, fall back to first list.
        let saved_active_id: Option<Uuid> = doc
            .get("active_cue_list_id")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok());
        let active_cue_list_id = saved_active_id
            .filter(|id| cue_lists.iter().any(|cl| cl.id == *id))
            .unwrap_or_else(|| cue_lists[0].id);

        // Derive the name from the filename stem so it always matches the file,
        // even if the JSON still contains an older name (e.g. "Untitled").
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            metadata.name = stem.to_string();
        }

        Ok(Self {
            metadata,
            cue_lists,
            active_cue_list_id,
            output_patches,
            default_output_patch_id: default_patch,
            osc_patches,
            input_patches,
            universe_outputs,
            fixtures,
            fixture_groups,
            preferences,
            file_path: Some(path),
            is_modified: false,
        })
    }
}
