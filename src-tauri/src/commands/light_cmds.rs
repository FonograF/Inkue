//! Tauri commands for the DMX lighting engine: output configuration, the live
//! monitor snapshot, blackout, and a debug single-channel poke.
//!
//! The live monitor is event-driven (`dmx-monitor`, emitted from `lib.rs`); the
//! `dmx_get_snapshot` command is only a one-shot fallback for initial paint.

use serde::Serialize;
use tauri::State;

use crate::engine::dmx_engine::ChannelWidth;
use crate::engine::dmx_sink::UniverseOutput;
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
#[tauri::command]
pub fn dmx_set_outputs(
    outputs: Vec<UniverseOutput>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.dmx_engine.set_outputs(outputs);
    Ok(())
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
