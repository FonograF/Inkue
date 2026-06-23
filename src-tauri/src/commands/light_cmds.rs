//! Tauri commands for the DMX lighting engine: output configuration, the live
//! monitor snapshot, blackout, and a debug single-channel poke.
//!
//! The live monitor is event-driven (`dmx-monitor`, emitted from `lib.rs`); the
//! `dmx_get_snapshot` command is only a one-shot fallback for initial paint.

use serde::Serialize;
use tauri::{Emitter, State};
use uuid::Uuid;

use crate::cue::light_cue::ParamTarget;
use crate::engine::dmx_engine::{ChannelWidth, DmxSnapshot};
use crate::engine::dmx_sink::UniverseOutput;
use crate::engine::fixture::{
    builtin_fixture_types, find_conflicts, FixtureConflict, FixtureGroup, FixtureType, PatchedFixture,
};
use crate::engine::DmxEngine;
use crate::state::AppState;

/// One universe's live output bytes, for the DMX monitor view.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DmxUniverseSnapshot {
    pub universe: u16,
    pub channels: Vec<u8>,
}

/// Build a sorted, serialisable snapshot of every active universe.
/// Shared by the `dmx_get_snapshot` command and the `dmx-monitor` event thread.
pub fn snapshot_dto(engine: &DmxEngine) -> Vec<DmxUniverseSnapshot> {
    let mut out: Vec<DmxUniverseSnapshot> = engine
        .snapshot()
        .into_iter()
        .map(|(universe, data)| DmxUniverseSnapshot { universe, channels: data.to_vec() })
        .collect();
    out.sort_by_key(|s| s.universe);
    out
}

/// Configure which universes are transmitted and how (sACN / Art-Net + destination).
///
/// Persists the mapping into the workspace (so it travels with the show) and
/// pushes it to the engine.  Called when the operator edits the outputs — not
/// on load, where the backend pushes the stored mapping itself.
#[tauri::command]
pub fn dmx_set_outputs(
    outputs: Vec<UniverseOutput>,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    {
        let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
        ws.universe_outputs = outputs.clone();
        ws.mark_modified();
    }
    state.dmx_engine.set_outputs(outputs);
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(())
}

/// Return the workspace's stored universe output mapping.
#[tauri::command]
pub fn dmx_get_outputs(state: State<'_, AppState>) -> Result<Vec<UniverseOutput>, String> {
    let ws = state.workspace.lock().map_err(|e| e.to_string())?;
    Ok(ws.universe_outputs.clone())
}

/// Debug: set a single DMX channel (1-based `address`) to `value` (0–255), no fade.
#[tauri::command]
pub fn dmx_set_channel(
    universe: u16,
    address: u16,
    value: u8,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let channel = address.saturating_sub(1);
    state
        .dmx_engine
        .set_channel(universe, channel, ChannelWidth::Bit8, value as f64 / 255.0);
    Ok(())
}

/// Toggle the global blackout override.
#[tauri::command]
pub fn dmx_set_blackout(on: bool, state: State<'_, AppState>) -> Result<(), String> {
    state.dmx_engine.set_blackout(on);
    Ok(())
}

/// Whether blackout is currently active.
#[tauri::command]
pub fn dmx_get_blackout(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.dmx_engine.is_blackout())
}

/// One-shot snapshot of every active universe (initial paint; live feed is the
/// `dmx-monitor` event).
#[tauri::command]
pub fn dmx_get_snapshot(state: State<'_, AppState>) -> Result<Vec<DmxUniverseSnapshot>, String> {
    Ok(snapshot_dto(&state.dmx_engine))
}

// ---------------------------------------------------------------------------
// Fixture patch
// ---------------------------------------------------------------------------

/// The built-in fixture templates offered when patching.
#[tauri::command]
pub fn list_builtin_fixture_types() -> Vec<FixtureType> {
    builtin_fixture_types()
}

/// Every patched fixture in the workspace.
#[tauri::command]
pub fn list_fixtures(state: State<'_, AppState>) -> Result<Vec<PatchedFixture>, String> {
    let ws = state.workspace.lock().map_err(|e| e.to_string())?;
    Ok(ws.fixtures.clone())
}

/// Patch a new fixture from a type at the given address.  Returns the created fixture.
#[tauri::command]
pub fn add_fixture(
    label: String,
    universe: u16,
    base_address: u16,
    fixture_type: FixtureType,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<PatchedFixture, String> {
    let fixture = PatchedFixture::new(label, universe, base_address, fixture_type);
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    ws.fixtures.push(fixture.clone());
    ws.mark_modified();
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(fixture)
}

/// Replace an existing fixture (matched by ID).
#[tauri::command]
pub fn update_fixture(
    fixture: PatchedFixture,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    if let Some(existing) = ws.fixtures.iter_mut().find(|f| f.id == fixture.id) {
        *existing = fixture;
        ws.mark_modified();
        let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
        Ok(())
    } else {
        Err(format!("Fixture {} not found", fixture.id))
    }
}

/// Remove a fixture by ID.
#[tauri::command]
pub fn remove_fixture(
    fixture_id: String,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let id: Uuid = fixture_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let before = ws.fixtures.len();
    ws.fixtures.retain(|f| f.id != id);
    if ws.fixtures.len() == before {
        return Err(format!("Fixture {id} not found"));
    }
    ws.mark_modified();
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(())
}

/// Address clashes between patched fixtures, for the patch UI to warn about.
#[tauri::command]
pub fn get_fixture_conflicts(state: State<'_, AppState>) -> Result<Vec<FixtureConflict>, String> {
    let ws = state.workspace.lock().map_err(|e| e.to_string())?;
    Ok(find_conflicts(&ws.fixtures))
}

/// Identify a fixture by driving its parameters to a visible value (`on = true`)
/// or back to zero (`on = false`), immediately (no fade).  Lets the operator
/// match a patched fixture to a physical instrument.
#[tauri::command]
pub fn dmx_test_fixture(
    fixture_id: String,
    on: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let id: Uuid = fixture_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let fixture = ws.fixtures.iter().find(|f| f.id == id).ok_or("Fixture not found")?;
    for (i, param) in fixture.fixture_type.parameters.iter().enumerate() {
        if let Some((universe, channel, width)) = fixture.resolve_channel(i) {
            let value = if on { param.kind.identify_value() } else { 0.0 };
            state.dmx_engine.set_channel(universe, channel, width, value);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Live Dashboard — sculpt fixtures live, then capture the look into a cue
// ---------------------------------------------------------------------------

/// Read one channel's normalised value `[0, 1]` from a universe snapshot.
fn read_norm_from_snapshot(snap: &DmxSnapshot, universe: u16, channel: u16, width: ChannelWidth) -> f64 {
    let Some(buf) = snap.get(&universe) else { return 0.0 };
    let i = channel as usize;
    match width {
        ChannelWidth::Bit8 => *buf.get(i).unwrap_or(&0) as f64 / 255.0,
        ChannelWidth::Bit16 => {
            let hi = *buf.get(i).unwrap_or(&0) as u16;
            let lo = *buf.get(i + 1).unwrap_or(&0) as u16;
            ((hi << 8) | lo) as f64 / 65535.0
        }
    }
}

/// Set one fixture parameter to `value` (0–1) immediately (no fade) — the live
/// editing path for the Dashboard sliders / colour picker.
#[tauri::command]
pub fn dmx_set_fixture_param(
    fixture_id: String,
    param_index: usize,
    value: f64,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let id: Uuid = fixture_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let fixture = ws.fixtures.iter().find(|f| f.id == id).ok_or("Fixture not found")?;
    if let Some((universe, channel, width)) = fixture.resolve_channel(param_index) {
        state.dmx_engine.set_channel(universe, channel, width, value);
    }
    Ok(())
}

/// Drive every patched fixture's parameters to zero (no fade) — Dashboard "Clear".
#[tauri::command]
pub fn dmx_clear_fixtures(state: State<'_, AppState>) -> Result<(), String> {
    let ws = state.workspace.lock().map_err(|e| e.to_string())?;
    for fixture in &ws.fixtures {
        for i in 0..fixture.fixture_type.parameters.len() {
            if let Some((universe, channel, width)) = fixture.resolve_channel(i) {
                state.dmx_engine.set_channel(universe, channel, width, 0.0);
            }
        }
    }
    Ok(())
}

/// Capture the current live state of every patched fixture as a full set of
/// Light Cue targets ("record look").  Pure read — the caller applies these to
/// the cue through the normal `update_cue` path (single write + undo step).
#[tauri::command]
pub fn capture_live_targets(state: State<'_, AppState>) -> Result<Vec<ParamTarget>, String> {
    let snapshot = state.dmx_engine.snapshot();
    let ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let mut targets = Vec::new();
    for fixture in &ws.fixtures {
        for i in 0..fixture.fixture_type.parameters.len() {
            if let Some((universe, channel, width)) = fixture.resolve_channel(i) {
                targets.push(ParamTarget::Fixture {
                    fixture_id: fixture.id.to_string(),
                    param_index: i,
                    value: read_norm_from_snapshot(&snapshot, universe, channel, width),
                });
            }
        }
    }
    Ok(targets)
}

// ---------------------------------------------------------------------------
// Fixture groups
// ---------------------------------------------------------------------------

/// Every fixture group in the workspace.
#[tauri::command]
pub fn list_fixture_groups(state: State<'_, AppState>) -> Result<Vec<FixtureGroup>, String> {
    let ws = state.workspace.lock().map_err(|e| e.to_string())?;
    Ok(ws.fixture_groups.clone())
}

/// Create a fixture group from a label and member fixture IDs.
#[tauri::command]
pub fn add_fixture_group(
    label: String,
    fixture_ids: Vec<String>,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<FixtureGroup, String> {
    let ids: Vec<Uuid> = fixture_ids
        .iter()
        .filter_map(|s| s.parse().ok())
        .collect();
    let group = FixtureGroup::new(label, ids);
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    ws.fixture_groups.push(group.clone());
    ws.mark_modified();
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(group)
}

/// Replace an existing fixture group (matched by ID).
#[tauri::command]
pub fn update_fixture_group(
    group: FixtureGroup,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    if let Some(existing) = ws.fixture_groups.iter_mut().find(|g| g.id == group.id) {
        *existing = group;
        ws.mark_modified();
        let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
        Ok(())
    } else {
        Err(format!("Group {} not found", group.id))
    }
}

/// Remove a fixture group by ID.
#[tauri::command]
pub fn remove_fixture_group(
    group_id: String,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let id: Uuid = group_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let before = ws.fixture_groups.len();
    ws.fixture_groups.retain(|g| g.id != id);
    if ws.fixture_groups.len() == before {
        return Err(format!("Group {id} not found"));
    }
    ws.mark_modified();
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(())
}
