//! Tauri commands for the machine-level network interface selection.

use tauri::State;

use crate::{
    engine::net_interface::{self, NetworkInterfaceInfo},
    preferences::NetworkInterfaceConfig,
    state::AppState,
};

/// Enumerate the machine's IPv4 network interfaces.
#[tauri::command]
pub fn list_network_interfaces() -> Vec<NetworkInterfaceInfo> {
    net_interface::list()
}

/// Return the persisted network interface selection.
#[tauri::command]
pub fn get_network_config() -> NetworkInterfaceConfig {
    crate::machine_config::load_network()
}

/// Persist a new interface selection and hot-apply it: rebinds the OSC
/// receive socket and every DMX sink so the change takes effect immediately.
#[tauri::command]
pub fn set_network_config(
    config: NetworkInterfaceConfig,
    state: State<'_, AppState>,
) -> Result<(), String> {
    crate::machine_config::save_network(&config).map_err(|e| e.to_string())?;
    net_interface::apply(&config);
    state.osc_server.reconfigure(crate::machine_config::load_osc());
    state.dmx_engine.rebind_sinks();
    Ok(())
}
