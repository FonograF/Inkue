//! Selected network interface — one machine-level choice applied to every
//! UDP socket Inkue opens (OSC receive/send, OSC feedback, sACN/Art-Net DMX).
//!
//! `None` (Automatic) preserves the historical behaviour: bind to all
//! interfaces (`0.0.0.0`) and let the OS routing table pick the egress path.
//! When an interface is selected, receive sockets bind to its IPv4 address,
//! send sockets bind their local endpoint to it, and sACN multicast egress is
//! pinned to it via `IP_MULTICAST_IF`.

use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::sync::RwLock;

use serde::Serialize;

use crate::health::{self, HealthAlert, HealthLevel};
use crate::preferences::NetworkInterfaceConfig;

/// Health-registry key for "configured interface not found" warnings.
const HEALTH_KEY: &str = "network-interface";

/// One IPv4-capable network interface, as shown in Preferences → Network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NetworkInterfaceInfo {
    /// OS interface name (`"Ethernet"`, `"en0"`, `"eth0"`, …).
    pub name: String,
    /// IPv4 address, dotted-quad.
    pub ip: String,
    /// `true` for loopback interfaces (still selectable — useful for testing).
    pub is_loopback: bool,
}

/// Enumerate the machine's IPv4 interfaces (loopback included, flagged).
pub fn list() -> Vec<NetworkInterfaceInfo> {
    let mut interfaces: Vec<NetworkInterfaceInfo> = match if_addrs::get_if_addrs() {
        Ok(addrs) => addrs
            .iter()
            .filter_map(|iface| match iface.ip() {
                IpAddr::V4(ip) => Some(NetworkInterfaceInfo {
                    name: iface.name.clone(),
                    ip: ip.to_string(),
                    is_loopback: iface.is_loopback(),
                }),
                IpAddr::V6(_) => None,
            })
            .collect(),
        Err(e) => {
            log::warn!("[net] interface enumeration failed: {e}");
            Vec::new()
        }
    };
    // Physical interfaces first, then loopback; stable name order within each.
    interfaces.sort_by(|a, b| {
        a.is_loopback
            .cmp(&b.is_loopback)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.ip.cmp(&b.ip))
    });
    interfaces
}

/// The currently-selected bind address.  `None` = Automatic.
static SELECTED: RwLock<Option<Ipv4Addr>> = RwLock::new(None);

/// Resolve `config` against the interfaces present right now.
///
/// Match order: interface name first (survives DHCP address changes), then the
/// stored IP (survives interface renames).  Returns `None` when the config is
/// Automatic or nothing matches.
fn resolve(
    config: &NetworkInterfaceConfig,
    interfaces: &[NetworkInterfaceInfo],
) -> Option<Ipv4Addr> {
    let name = config.interface_name.as_deref()?;
    if let Some(found) = interfaces.iter().find(|i| i.name == name) {
        return found.ip.parse().ok();
    }
    let stored_ip = config.interface_ip.as_deref()?;
    if interfaces.iter().any(|i| i.ip == stored_ip) {
        return stored_ip.parse().ok();
    }
    None
}

/// Apply (or hot-update) the interface selection.  Safe to call from any thread.
///
/// When the configured interface cannot be found, falls back to Automatic and
/// raises a health banner so the operator notices before the show starts.
pub fn apply(config: &NetworkInterfaceConfig) {
    let resolved = resolve(config, &list());

    match (&config.interface_name, resolved) {
        (None, _) => health::clear(HEALTH_KEY),
        (Some(_), Some(ip)) => {
            log::info!("[net] network traffic pinned to interface {ip}");
            health::clear(HEALTH_KEY);
        }
        (Some(name), None) => {
            log::warn!("[net] configured interface \"{name}\" not found — using all interfaces");
            health::set(HealthAlert::new(
                HEALTH_KEY,
                HealthLevel::Warning,
                format!("Network interface \"{name}\" not found — using all interfaces"),
            ));
        }
    }

    if let Ok(mut g) = SELECTED.write() {
        *g = resolved;
    }
}

/// The selected interface's IPv4 address, if one is pinned.
pub fn selected_ipv4() -> Option<Ipv4Addr> {
    SELECTED.read().ok().and_then(|g| *g)
}

/// Bind address for receive sockets: the selected interface, or `0.0.0.0`.
pub fn bind_ip() -> IpAddr {
    IpAddr::V4(selected_ipv4().unwrap_or(Ipv4Addr::UNSPECIFIED))
}

/// Bind an ephemeral UDP socket for sending, pinned to the selected interface.
///
/// If the pinned bind fails (interface vanished between checks), falls back to
/// all interfaces so a show never goes silent because of a stale selection.
pub fn udp_send_socket() -> std::io::Result<UdpSocket> {
    if let Some(ip) = selected_ipv4() {
        match UdpSocket::bind((ip, 0)) {
            Ok(socket) => return Ok(socket),
            Err(e) => {
                log::warn!("[net] bind to selected interface {ip} failed ({e}) — using any");
            }
        }
    }
    UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iface(name: &str, ip: &str, lo: bool) -> NetworkInterfaceInfo {
        NetworkInterfaceInfo { name: name.into(), ip: ip.into(), is_loopback: lo }
    }

    fn config(name: Option<&str>, ip: Option<&str>) -> NetworkInterfaceConfig {
        NetworkInterfaceConfig {
            interface_name: name.map(String::from),
            interface_ip: ip.map(String::from),
        }
    }

    #[test]
    fn automatic_resolves_to_none() {
        let interfaces = [iface("Ethernet", "192.168.1.10", false)];
        assert_eq!(resolve(&config(None, None), &interfaces), None);
        assert_eq!(resolve(&config(None, Some("192.168.1.10")), &interfaces), None);
    }

    #[test]
    fn name_match_wins_and_follows_current_ip() {
        // The stored IP is stale (DHCP renewed); the name match returns the
        // interface's *current* address.
        let interfaces = [iface("Ethernet", "192.168.1.42", false)];
        let resolved = resolve(&config(Some("Ethernet"), Some("192.168.1.10")), &interfaces);
        assert_eq!(resolved, Some(Ipv4Addr::new(192, 168, 1, 42)));
    }

    #[test]
    fn ip_fallback_when_interface_renamed() {
        let interfaces = [iface("Ethernet 2", "192.168.1.10", false)];
        let resolved = resolve(&config(Some("Ethernet"), Some("192.168.1.10")), &interfaces);
        assert_eq!(resolved, Some(Ipv4Addr::new(192, 168, 1, 10)));
    }

    #[test]
    fn missing_interface_resolves_to_none() {
        let interfaces = [iface("Wi-Fi", "10.0.0.5", false)];
        assert_eq!(resolve(&config(Some("Ethernet"), Some("192.168.1.10")), &interfaces), None);
        assert_eq!(resolve(&config(Some("Ethernet"), None), &interfaces), None);
    }

    #[test]
    fn list_orders_physical_before_loopback() {
        // `list()` hits the real OS — only check the invariant that holds on
        // any machine: loopback entries sort after physical ones.
        let interfaces = list();
        let first_loopback = interfaces.iter().position(|i| i.is_loopback);
        let last_physical = interfaces.iter().rposition(|i| !i.is_loopback);
        if let (Some(lo), Some(phys)) = (first_loopback, last_physical) {
            assert!(phys < lo, "physical interfaces must sort before loopback");
        }
    }

    #[test]
    fn udp_send_socket_binds_automatic() {
        // No selection applied in tests — must bind an ephemeral any-interface socket.
        let socket = udp_send_socket().expect("bind failed");
        assert_eq!(socket.local_addr().unwrap().ip(), IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    }
}
