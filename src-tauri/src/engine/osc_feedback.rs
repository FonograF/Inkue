//! OSC feedback — broadcasts the currently-running cue's number and name to a
//! configurable UDP destination whenever the active cue changes.
//!
//! Useful for driving external displays (Open Stage Control, QLab, …) without
//! needing to author an OscSendCue for every cue in the show.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

struct Cfg {
    enabled: bool,
    host: String,
    port: u16,
    /// Media-progress send rate in Hz (0 = progress feedback off).
    progress_hz: u8,
}

static CFG: OnceLock<Mutex<Cfg>> = OnceLock::new();

/// Lock-free mirror of `Cfg::enabled` for hot-path readers (the 30 fps event
/// loop checks this every tick to decide whether to compute feedback payloads at
/// all — see [`is_enabled`]).
static ENABLED: AtomicBool = AtomicBool::new(false);

fn cfg() -> &'static Mutex<Cfg> {
    CFG.get_or_init(|| {
        Mutex::new(Cfg { enabled: false, host: String::new(), port: 0, progress_hz: 10 })
    })
}

/// Apply (or hot-update) the feedback destination.  Safe to call from any thread.
pub fn apply(enabled: bool, host: String, port: u16, progress_hz: u8) {
    if let Ok(mut g) = cfg().lock() {
        g.enabled     = enabled;
        g.host        = host;
        g.port        = port;
        g.progress_hz = progress_hz;
    }
    ENABLED.store(enabled, Ordering::Relaxed);
}

/// Whether OSC feedback is currently enabled.  A relaxed atomic read — cheap
/// enough to call every event-loop tick to skip building feedback payloads.
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
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
// Media progress feedback
// ---------------------------------------------------------------------------

/// Per-slot progress values: `(progress 0..1, elapsed s, remaining s, duration s)`.
/// `remaining`/`duration` are `-1.0` when unknown (vamp, live feed, ∞ loop).
pub type ProgressSlot = (f32, f32, f32, f32);

/// Last progress send, as ms since an arbitrary process-start epoch.
static LAST_PROGRESS_SEND: Mutex<Option<std::time::Instant>> = Mutex::new(None);
/// Slots filled by the previous send — freed slots get one zeroing pulse so
/// client gauges do not freeze at the last value.
static PREV_PROGRESS_COUNT: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Rate gate for progress feedback: `true` when a pulse is due (consumes the
/// interval).  The event loop calls this before building the payload so the
/// off-pulse ticks cost nothing.
pub fn progress_due() -> bool {
    let hz = {
        let Ok(g) = cfg().lock() else { return false };
        if !g.enabled || g.progress_hz == 0 {
            return false;
        }
        g.progress_hz
    };
    let interval = std::time::Duration::from_millis(1000 / hz.max(1) as u64);
    let Ok(mut last) = LAST_PROGRESS_SEND.lock() else { return false };
    let now = std::time::Instant::now();
    match *last {
        Some(t) if now.duration_since(t) < interval => false,
        _ => {
            *last = Some(now);
            true
        }
    }
}

/// Send media progress for the running cues (same slot order as
/// [`send_running`]: slot 0 = topmost running cue).
///
/// Addresses per slot `i` (0..[`MAX_RUNNING`]):
///   `/inkue/cue/{i}/progress   <float 0..1>`
///   `/inkue/cue/{i}/elapsed    <float seconds>`
///   `/inkue/cue/{i}/remaining  <float seconds, -1 = unknown>`
///   `/inkue/cue/{i}/duration   <float seconds, -1 = unknown>`
pub fn send_progress(slots: &[ProgressSlot]) {
    let count = slots.len().min(MAX_RUNNING);
    let prev = PREV_PROGRESS_COUNT.swap(count, Ordering::Relaxed);

    let mut msgs: Vec<(String, rosc::OscType)> = Vec::with_capacity((count.max(prev)) * 4);
    for (i, &(progress, elapsed, remaining, duration)) in slots.iter().take(MAX_RUNNING).enumerate() {
        msgs.push((format!("/inkue/cue/{i}/progress"),  rosc::OscType::Float(progress)));
        msgs.push((format!("/inkue/cue/{i}/elapsed"),   rosc::OscType::Float(elapsed)));
        msgs.push((format!("/inkue/cue/{i}/remaining"), rosc::OscType::Float(remaining)));
        msgs.push((format!("/inkue/cue/{i}/duration"),  rosc::OscType::Float(duration)));
    }
    // One zeroing pulse for slots freed since the last send.
    for i in count..prev.min(MAX_RUNNING) {
        msgs.push((format!("/inkue/cue/{i}/progress"),  rosc::OscType::Float(0.0)));
        msgs.push((format!("/inkue/cue/{i}/elapsed"),   rosc::OscType::Float(0.0)));
        msgs.push((format!("/inkue/cue/{i}/remaining"), rosc::OscType::Float(-1.0)));
        msgs.push((format!("/inkue/cue/{i}/duration"),  rosc::OscType::Float(-1.0)));
    }
    if msgs.is_empty() {
        return;
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

/// Non-consuming peek: is either an on-demand cue-list or playhead send pending?
/// Used by the event loop's fast-path guard so it does not clear the flags (the
/// real sends do that) — unlike [`is_playhead_requested`], which consumes.
pub fn any_request_pending() -> bool {
    PENDING_LIST_REQUEST.load(Ordering::Relaxed)
        || PENDING_PLAYHEAD_REQUEST.load(Ordering::Relaxed)
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

    let Ok(socket) = super::net_interface::udp_send_socket() else { return };
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
