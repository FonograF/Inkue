//! OSC receive server — listens on a UDP port and dispatches commands to the
//! frontend via Tauri events.
//!
//! Architecture:
//! - One background thread per server instance.
//! - `recv_from` with a 100 ms read timeout so config changes take effect quickly.
//! - On every acted-upon message: emits `osc-activity` (empty) + `osc-command`.
//! - The frontend listens for `osc-command` and calls the matching `invoke()`.

use std::net::UdpSocket;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use tauri::Emitter;

use crate::preferences::OscReceiveConfig;

/// Handle returned by [`OscServer::start`].  Drop or call [`OscServer::stop`]
/// to shut down the listener thread.
pub struct OscServer {
    config_tx: Sender<Option<OscReceiveConfig>>,
}

impl OscServer {
    /// Spawn the listener thread with the given initial config.
    pub fn start(config: OscReceiveConfig, app_handle: tauri::AppHandle) -> Self {
        let (tx, rx) = crossbeam_channel::bounded::<Option<OscReceiveConfig>>(4);

        std::thread::Builder::new()
            .name("wincue-osc-server".to_string())
            .spawn(move || server_loop(config, rx, app_handle))
            .expect("Failed to spawn OSC server thread");

        Self { config_tx: tx }
    }

    /// Apply a new configuration without restarting the app.  The listener
    /// thread picks up the change within 100 ms.
    pub fn reconfigure(&self, config: OscReceiveConfig) {
        let _ = self.config_tx.try_send(Some(config));
    }

    /// Gracefully shut down the listener thread.
    pub fn stop(&self) {
        let _ = self.config_tx.try_send(None);
    }
}

// ---------------------------------------------------------------------------
// Internal loop
// ---------------------------------------------------------------------------

fn server_loop(
    mut config: OscReceiveConfig,
    config_rx: Receiver<Option<OscReceiveConfig>>,
    app_handle: tauri::AppHandle,
) {
    loop {
        if !config.enabled {
            // Wait for a new config that re-enables the server.
            match config_rx.recv() {
                Ok(Some(new)) => { config = new; continue; }
                _ => return,
            }
        }

        let addr = format!("0.0.0.0:{}", config.port);
        let socket = match UdpSocket::bind(&addr) {
            Ok(s) => s,
            Err(e) => {
                log::error!("OSC server: failed to bind {addr}: {e}");
                // Wait a bit then retry or accept a new config.
                match config_rx.recv_timeout(Duration::from_secs(5)) {
                    Ok(Some(new)) => { config = new; continue; }
                    Ok(None) => return,
                    Err(_) => continue,
                }
            }
        };
        socket.set_read_timeout(Some(Duration::from_millis(100))).ok();

        log::info!("OSC server listening on {addr}");
        let mut buf = [0u8; 4096];

        loop {
            // Check for config changes before blocking.
            match config_rx.try_recv() {
                Ok(Some(new)) => { config = new; break; }
                Ok(None) => return,
                Err(_) => {}
            }

            match socket.recv_from(&mut buf) {
                Ok((n, src)) => {
                    if !is_allowed(&config.allowed_ips, &src.ip().to_string()) {
                        log::debug!("OSC: ignoring packet from non-allowlisted {src}");
                        continue;
                    }
                    match rosc::decoder::decode_udp(&buf[..n]) {
                        Ok((_, packet)) => handle_packet(&packet, &app_handle),
                        Err(e) => log::debug!("OSC: decode error: {e}"),
                    }
                }
                Err(e) if is_timeout(&e) => {}
                Err(e) => log::warn!("OSC recv error: {e}"),
            }
        }
    }
}

fn is_timeout(e: &std::io::Error) -> bool {
    matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock)
}

fn is_allowed(allowed_ips: &[String], src_ip: &str) -> bool {
    allowed_ips.is_empty() || allowed_ips.iter().any(|ip| ip == src_ip)
}

// ---------------------------------------------------------------------------
// Packet dispatch
// ---------------------------------------------------------------------------

fn handle_packet(packet: &rosc::OscPacket, app_handle: &tauri::AppHandle) {
    match packet {
        rosc::OscPacket::Message(msg) => handle_message(msg, app_handle),
        rosc::OscPacket::Bundle(bundle) => {
            for p in &bundle.content {
                handle_packet(p, app_handle);
            }
        }
    }
}

fn handle_message(msg: &rosc::OscMessage, app_handle: &tauri::AppHandle) {
    // Always emit a debug event regardless of whether the address matches WinCue.
    let args_display: Vec<String> = msg.args.iter().map(format_osc_arg).collect();
    let _ = app_handle.emit(
        "osc-debug",
        serde_json::json!({
            "addr": msg.addr,
            "args": args_display,
        }),
    );
    log::info!("OSC in: {} {:?}", msg.addr, args_display);

    let payload = match msg.addr.as_str() {
        "/wincue/go"        => serde_json::json!({ "command": "go" }),
        "/wincue/stop"      => serde_json::json!({ "command": "stop_all" }),
        "/wincue/hardstop"  => serde_json::json!({ "command": "hard_stop_all" }),
        "/wincue/pause"     => serde_json::json!({ "command": "pause_all" }),
        "/wincue/resume"    => serde_json::json!({ "command": "resume_all" }),
        addr if addr.starts_with("/wincue/cue/") => parse_cue_address(addr),
        _ => return,
    };

    let _ = app_handle.emit("osc-command", &payload);
    let _ = app_handle.emit("osc-activity", serde_json::json!({}));
}

fn format_osc_arg(arg: &rosc::OscType) -> String {
    match arg {
        rosc::OscType::Int(i)    => format!("i:{i}"),
        rosc::OscType::Float(f)  => format!("f:{f}"),
        rosc::OscType::Double(d) => format!("d:{d}"),
        rosc::OscType::String(s) => format!("s:{s:?}"),
        rosc::OscType::Bool(b)   => format!("b:{b}"),
        rosc::OscType::Long(l)   => format!("l:{l}"),
        rosc::OscType::Blob(b)   => format!("blob({} bytes)", b.len()),
        rosc::OscType::Nil       => "nil".to_string(),
        rosc::OscType::Inf       => "inf".to_string(),
        _                        => "?".to_string(),
    }
}

/// Parse `/wincue/cue/{number}/go|select|stop` and build the command payload.
fn parse_cue_address(addr: &str) -> serde_json::Value {
    let parts: Vec<&str> = addr.splitn(6, '/').collect();
    // parts: ["", "wincue", "cue", "{number}", "action"]
    if parts.len() == 5 {
        let number = parts[3];
        let action = parts[4];
        let command = match action {
            "go"     => "cue_go",
            "select" => "cue_select",
            "stop"   => "cue_stop",
            _ => return serde_json::json!({}),
        };
        serde_json::json!({ "command": command, "cue_number": number })
    } else {
        serde_json::json!({})
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ip_allowlist_empty_accepts_all() {
        assert!(is_allowed(&[], "192.168.1.1"));
        assert!(is_allowed(&[], "10.0.0.1"));
    }

    #[test]
    fn ip_allowlist_filters_correctly() {
        let allowed = vec!["192.168.1.100".to_string()];
        assert!(is_allowed(&allowed, "192.168.1.100"));
        assert!(!is_allowed(&allowed, "192.168.1.101"));
        assert!(!is_allowed(&allowed, "127.0.0.1"));
    }

    #[test]
    fn parse_cue_go_address() {
        let payload = parse_cue_address("/wincue/cue/1.5/go");
        assert_eq!(payload["command"], "cue_go");
        assert_eq!(payload["cue_number"], "1.5");
    }

    #[test]
    fn parse_cue_select_address() {
        let payload = parse_cue_address("/wincue/cue/Intro/select");
        assert_eq!(payload["command"], "cue_select");
        assert_eq!(payload["cue_number"], "Intro");
    }

    #[test]
    fn parse_cue_stop_address() {
        let payload = parse_cue_address("/wincue/cue/3/stop");
        assert_eq!(payload["command"], "cue_stop");
        assert_eq!(payload["cue_number"], "3");
    }
}
