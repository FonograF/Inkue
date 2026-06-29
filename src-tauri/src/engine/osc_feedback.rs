//! OSC feedback — broadcasts the currently-running cue's number and name to a
//! configurable UDP destination whenever the active cue changes.
//!
//! Useful for driving external displays (Open Stage Control, QLab, …) without
//! needing to author an OscSendCue for every cue in the show.

use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

struct Cfg {
    enabled: bool,
    host: String,
    port: u16,
}

static CFG: OnceLock<Mutex<Cfg>> = OnceLock::new();

fn cfg() -> &'static Mutex<Cfg> {
    CFG.get_or_init(|| {
        Mutex::new(Cfg { enabled: false, host: String::new(), port: 0 })
    })
}

/// Apply (or hot-update) the feedback destination.  Safe to call from any thread.
pub fn apply(enabled: bool, host: String, port: u16) {
    if let Ok(mut g) = cfg().lock() {
        g.enabled = enabled;
        g.host    = host;
        g.port    = port;
    }
}

/// Maximum number of simultaneously-running cues tracked via OSC feedback.
const MAX_RUNNING: usize = 8;

/// Send all currently-running cues to the configured destination.
///
/// `cues` is ordered (first = topmost running cue in the list).
/// Addresses sent:
///   `/inkue/cue/count          <int>`       — number of running cues
///   `/inkue/cue/number         <string>`    — first cue number (compat)
///   `/inkue/cue/name           <string>`    — first cue name   (compat)
///   `/inkue/cue/active         <1 | 0>`     — 1 if any running
///   `/inkue/cue/N/number       <string>`    — Nth cue number (N = 0..MAX)
///   `/inkue/cue/N/name         <string>`    — Nth cue name
pub fn send_running(cues: &[(String, String)]) {
    let count = cues.len().min(MAX_RUNNING);
    let first_num  = cues.first().map(|(n, _)| n.as_str()).unwrap_or("");
    let first_name = cues.first().map(|(_, n)| n.as_str()).unwrap_or("");

    let mut msgs: Vec<(String, rosc::OscType)> = Vec::new();

    // Multi-line list: "1 — Intro\n2 — Main Theme" (one cue per line).
    let list = cues.iter().take(MAX_RUNNING)
        .map(|(n, name)| {
            if n.is_empty() { name.clone() }
            else if name.is_empty() { n.clone() }
            else { format!("{n}  —  {name}") }
        })
        .collect::<Vec<_>>()
        .join("\n");

    msgs.push(("/inkue/cue/count".into(),  rosc::OscType::Int(count as i32)));
    msgs.push(("/inkue/cue/list".into(),   rosc::OscType::String(list)));
    msgs.push(("/inkue/cue/number".into(), rosc::OscType::String(first_num.to_owned())));
    msgs.push(("/inkue/cue/name".into(),   rosc::OscType::String(first_name.to_owned())));
    msgs.push(("/inkue/cue/active".into(), rosc::OscType::Int(if count > 0 { 1 } else { 0 })));

    // Indexed slots — fill active, clear unused.
    for i in 0..MAX_RUNNING {
        let (num, name) = cues.get(i)
            .map(|(n, m)| (n.as_str(), m.as_str()))
            .unwrap_or(("", ""));
        msgs.push((format!("/inkue/cue/{i}/number"), rosc::OscType::String(num.to_owned())));
        msgs.push((format!("/inkue/cue/{i}/name"),   rosc::OscType::String(name.to_owned())));
    }

    let refs: Vec<(&str, rosc::OscType)> = msgs.iter()
        .map(|(a, v)| (a.as_str(), v.clone()))
        .collect();
    send_messages(&refs);
}

// ---------------------------------------------------------------------------
// On-demand cue list request flag
// ---------------------------------------------------------------------------

static PENDING_LIST_REQUEST: AtomicBool = AtomicBool::new(false);
static PENDING_PLAYHEAD_REQUEST: AtomicBool = AtomicBool::new(false);

/// Request an immediate send of the full cue list on the next event-loop tick.
/// Called by the OSC server when `/inkue/cues/request` is received.
pub fn request_cue_list() {
    PENDING_LIST_REQUEST.store(true, Ordering::Relaxed);
}

/// Returns `true` if an OSC client requested the cue list since the last send.
pub fn is_cue_list_requested() -> bool {
    PENDING_LIST_REQUEST.load(Ordering::Relaxed)
}

/// Request an immediate send of the current playhead state on the next event-loop tick.
/// Called by the OSC server when `/inkue/playhead/request` is received.
pub fn request_playhead() {
    PENDING_PLAYHEAD_REQUEST.store(true, Ordering::Relaxed);
}

/// Returns `true` if an OSC client requested the playhead state since the last send.
pub fn is_playhead_requested() -> bool {
    PENDING_PLAYHEAD_REQUEST.swap(false, Ordering::Relaxed)
}

/// Send the full ordered cue list to the configured destination.
///
/// `cues` is the complete flat list `(number, name)` in display order.
/// Addresses sent:
///   `/inkue/cues/count    <int>`    — total number of cues
///   `/inkue/cues/options  <string>` — JSON `[["num","num — name"],...]`
///                                      ready for use as a `dropdown` values
///                                      property in Open Stage Control.
pub fn send_cue_list(cues: &[(String, String)]) {
    PENDING_LIST_REQUEST.store(false, Ordering::Relaxed);

    // Simple array of "num|name" entries.  The pipe separator lets onValue
    // split out the cue number cheaply without ambiguity.
    let entries: Vec<String> = cues
        .iter()
        .map(|(num, name)| {
            let entry = match (num.is_empty(), name.is_empty()) {
                (true, _) => name.clone(),
                (_, true) => num.clone(),
                _         => format!("{num} | {name}"),
            };
            format!("\"{}\"", entry.replace('"', "\\\""))
        })
        .collect();
    let json = format!("[{}]", entries.join(","));

    send_messages(&[
        ("/inkue/cues/count",   rosc::OscType::Int(cues.len() as i32)),
        ("/inkue/cues/options", rosc::OscType::String(json)),
    ]);
}

/// Send the playhead (next cue to GO) info to the configured destination.
///
/// Addresses:
///   `/inkue/playhead/number  <string>`
///   `/inkue/playhead/name    <string>`
pub fn send_playhead(number: &str, name: &str) {
    send_messages(&[
        ("/inkue/playhead/number", rosc::OscType::String(number.to_owned())),
        ("/inkue/playhead/name",   rosc::OscType::String(name.to_owned())),
    ]);
}

fn send_messages(messages: &[(&str, rosc::OscType)]) {
    let (host, port) = {
        let Ok(g) = cfg().lock() else { return };
        if !g.enabled || g.host.is_empty() { return; }
        (g.host.clone(), g.port)
    };

    let Ok(socket) = UdpSocket::bind("0.0.0.0:0") else { return };
    let target = format!("{host}:{port}");

    for (addr, arg) in messages {
        let packet = rosc::OscPacket::Message(rosc::OscMessage {
            addr: addr.to_string(),
            args: vec![arg.clone()],
        });
        if let Ok(bytes) = rosc::encoder::encode(&packet) {
            let _ = socket.send_to(&bytes, &target);
        }
    }
}
