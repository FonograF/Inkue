//! Which cpal host a stream opens on, and how Inkue's stored device ids map
//! onto that host's devices.
//!
//! Windows and macOS always use cpal's default host (WASAPI / CoreAudio; ASIO
//! is picked separately by the audio engine).  Linux prefers cpal's **native
//! PipeWire host** whenever a PipeWire daemon answers, and falls back to ALSA
//! otherwise (no daemon, the native host failed to start, or
//! `INKUE_AUDIO_HOST=alsa`).
//!
//! Inkue device ids are host-independent so saved Output Patches, Input
//! Patches and machine config survive a host change:
//!
//! | Stored id            | PipeWire host            | ALSA host           |
//! |----------------------|--------------------------|---------------------|
//! | `None` / `""`        | default sink / source    | ALSA `default`      |
//! | `default`/`pipewire` | default sink / source    | ALSA `default`      |
//! | `pw:<node.name>`     | that node                | ALSA `default` (*)  |
//! | anything else        | ALSA PCM of that name    | ALSA PCM            |
//!
//! (*) Without the native host a node cannot be targeted any more: the old
//! path set `PIPEWIRE_NODE` with `std::env::set_var` at runtime, which is
//! undefined behaviour while other threads read the environment (#11).

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait};

/// Environment variable that forces the Linux host: `alsa` skips the native
/// PipeWire host (escape hatch if a PipeWire setup misbehaves).
pub const HOST_ENV: &str = "INKUE_AUDIO_HOST";

/// Prefix of Inkue's synthetic device id for a PipeWire node.
pub const PW_PREFIX: &str = "pw:";

/// The cpal host family a stream opens on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKind {
    /// The OS default host: WASAPI on Windows, CoreAudio on macOS.
    System,
    /// cpal's native PipeWire client (Linux).
    PipeWire,
    /// ALSA (Linux): raw hardware, or PipeWire / PulseAudio through their
    /// ALSA plugins.
    Alsa,
}

/// Which device of a host a stored id selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceTarget<'a> {
    /// The host's default output (or input) device.
    Default,
    /// The device whose cpal `DeviceId::id()` equals this string.
    Id(&'a str),
}

/// Pick the Linux host: PipeWire when its daemon is reachable, unless
/// `forced` (the [`HOST_ENV`] value) asks for ALSA.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn choose_linux_host(pipewire_available: bool, forced: Option<&str>) -> HostKind {
    let force_alsa = forced.is_some_and(|v| v.trim().eq_ignore_ascii_case("alsa"));
    if pipewire_available && !force_alsa {
        HostKind::PipeWire
    } else {
        HostKind::Alsa
    }
}

/// Oldest libpipewire the native host runs on: cpal 0.18 is written against
/// 0.3.53, and its playback path reads `pw_buffer.requested` (0.3.49) — on an
/// older library that read lands past the end of the struct.  Ubuntu 22.04
/// ships 0.3.48 and therefore stays on ALSA.
pub const MIN_PIPEWIRE_LIB: (u32, u32, u32) = (0, 3, 53);

/// `true` when the libpipewire `version` string (`pw_get_library_version`,
/// e.g. `"1.0.5"`) is at least [`MIN_PIPEWIRE_LIB`].
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn pipewire_lib_supported(version: &str) -> bool {
    let mut parts = version.trim().split('.').map(|p| {
        let digits = p.find(|c: char| !c.is_ascii_digit()).map_or(p, |end| &p[..end]);
        digits.parse::<u32>().ok()
    });
    let mut next = || parts.next().flatten();
    match (next(), next(), next()) {
        (Some(major), Some(minor), micro) => (major, minor, micro.unwrap_or(0)) >= MIN_PIPEWIRE_LIB,
        _ => false,
    }
}

/// Map a stored Inkue device id onto `active` (the host the process would
/// open on): the host to open it on, and which of that host's devices.
///
/// Only the PipeWire host can hand an id over to another host — a raw ALSA
/// PCM (`hw:CARD=…`) saved in a patch keeps opening through ALSA, for a card
/// PipeWire does not hold.
pub fn route(id: Option<&str>, active: HostKind) -> (HostKind, DeviceTarget<'_>) {
    let Some(id) = id.filter(|s| !s.is_empty()) else {
        return (active, DeviceTarget::Default);
    };
    match active {
        HostKind::System => (active, DeviceTarget::Id(id)),
        HostKind::PipeWire => match id.strip_prefix(PW_PREFIX) {
            Some(node) if !node.is_empty() => (active, DeviceTarget::Id(node)),
            Some(_) => (active, DeviceTarget::Default),
            None if matches!(id, "default" | "pipewire") => (active, DeviceTarget::Default),
            None => (HostKind::Alsa, DeviceTarget::Id(id)),
        },
        HostKind::Alsa => {
            if id.starts_with(PW_PREFIX) || id == "default" {
                (active, DeviceTarget::Default)
            } else {
                (active, DeviceTarget::Id(id))
            }
        }
    }
}

/// `true` when a stream for stored id `id`, opened on host `kind`, is pinned
/// to one PipeWire node.  Losing that node raises no cpal error: WirePlumber
/// moves the stream to the default sink and it keeps streaming there, so the
/// device watchdog has to poll the node's presence instead
/// ([`pipewire_output_present`]).
pub fn pins_pipewire_node(kind: HostKind, id: Option<&str>) -> bool {
    matches!(route(id, kind), (HostKind::PipeWire, DeviceTarget::Id(_)))
}

/// Whether the PipeWire output node behind stored id `id` (`pw:<node.name>`)
/// still exists.  `None` when the PipeWire host could not be asked — callers
/// treat that as "unknown", never as "gone".  Always `Some(true)` off Linux.
///
/// Compares ids only (no per-device config query), so it is cheap enough for
/// the watchdog's 2 s tick.
pub fn pipewire_output_present(id: &str) -> Option<bool> {
    let (HostKind::PipeWire, DeviceTarget::Id(node)) = route(Some(id), HostKind::PipeWire) else {
        return Some(true);
    };
    #[cfg(target_os = "linux")]
    {
        linux::output_node_present(node)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = node;
        Some(true)
    }
}

/// cpal 0.18's synthetic PipeWire devices that follow the session default
/// (`src/host/pipewire/device.rs`): they map to Inkue's `default` id.
const PW_DEFAULT_NODES: [&str; 3] = ["output_default", "input_default", "sink_default"];

/// The id Inkue stores for a device whose cpal `DeviceId::id()` is `cpal_id`
/// on host `kind` — the inverse of [`route`].
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn inkue_device_id(kind: HostKind, cpal_id: &str) -> String {
    match kind {
        HostKind::PipeWire if PW_DEFAULT_NODES.contains(&cpal_id) => "default".to_string(),
        HostKind::PipeWire => format!("{PW_PREFIX}{cpal_id}"),
        HostKind::System | HostKind::Alsa => cpal_id.to_string(),
    }
}

/// `true` when a PipeWire host device belongs in an Inkue device list of one
/// direction: a real node, or the synthetic default of that direction (it
/// becomes Inkue's `default` entry) — never another application's stream.
///
/// cpal reports sinks (and duplex nodes) as `Duplex` and application
/// playback streams as `Output`, so outputs keep `Duplex` only; inputs drop
/// `Duplex` (a sink's monitor is not an input).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn lists_pipewire_device(cpal_id: &str, direction: cpal::DeviceDirection, input: bool) -> bool {
    match cpal_id {
        "output_default" => !input,
        "input_default" => input,
        "sink_default" => false,
        _ => match direction {
            cpal::DeviceDirection::Duplex => !input,
            cpal::DeviceDirection::Input => input,
            _ => false,
        },
    }
}

/// A device resolved on its host.  Keep `host` alive until the stream is
/// built: dropping the last PipeWire host tears the library down
/// (`pw_deinit`) only for the stream thread to bring it back up.
pub struct Resolved {
    /// The host `device` belongs to.
    pub host: cpal::Host,
    /// The device to open.
    pub device: cpal::Device,
    /// Which host family `host` is.
    pub kind: HostKind,
}

/// The host this process opens unpinned streams on, and its family.
pub fn system_host() -> (cpal::Host, HostKind) {
    #[cfg(target_os = "linux")]
    {
        linux::active_host()
    }
    #[cfg(not(target_os = "linux"))]
    {
        (cpal::default_host(), HostKind::System)
    }
}

/// Resolve a stored output device id (`None`/empty = default output).
pub fn output_device(id: Option<&str>) -> Result<Resolved> {
    resolve(id, false)
}

/// Resolve a stored input device id (`None`/empty = default input).
pub fn input_device(id: Option<&str>) -> Result<Resolved> {
    resolve(id, true)
}

fn resolve(id: Option<&str>, input: bool) -> Result<Resolved> {
    let (active_host, active) = system_host();
    let (kind, target) = route(id, active);
    let host = if kind == active { active_host } else { host_of(kind)? };
    let what = if input { "input" } else { "output" };
    let device = match target {
        DeviceTarget::Default => if input {
            host.default_input_device()
        } else {
            host.default_output_device()
        }
        .ok_or_else(|| anyhow!("No default audio {what} device found"))?,
        DeviceTarget::Id(name) => {
            let devices = if input {
                host.input_devices().map(|it| it.collect::<Vec<_>>())
            } else {
                host.output_devices().map(|it| it.collect::<Vec<_>>())
            }
            .map_err(|e| anyhow!("Failed to enumerate {what} devices: {e}"))?;
            devices
                .into_iter()
                .find(|d| d.id().ok().is_some_and(|i| i.id() == name))
                .ok_or_else(|| anyhow!("Audio {what} device '{name}' not found"))?
        }
    };
    if active == HostKind::Alsa && id.is_some_and(|i| i.starts_with(PW_PREFIX)) {
        log::warn!("No native PipeWire host: {id:?} opened as the ALSA default {what}");
    }
    Ok(Resolved { host, device, kind })
}

/// A fresh host of family `kind` other than the active one: only ever ALSA,
/// for a raw PCM id while the native PipeWire host is active.
fn host_of(kind: HostKind) -> Result<cpal::Host> {
    match kind {
        #[cfg(target_os = "linux")]
        HostKind::Alsa => Ok(linux::alsa_host()),
        _ => Err(anyhow!("No {kind:?} audio host to hand a device over to")),
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::ffi::{c_char, c_int, c_void, CStr};
    use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
    use std::sync::Once;

    use cpal::traits::{DeviceTrait, HostTrait};

    use super::{choose_linux_host, pipewire_lib_supported, HostKind, HOST_ENV};

    extern "C" {
        // libpipewire is linked in by cpal's `pipewire` feature.
        fn pw_get_library_version() -> *const c_char;
    }

    /// Version of the libpipewire actually loaded — it can be older than
    /// the headers Inkue was built against.
    fn library_version() -> Option<String> {
        // SAFETY: returns a pointer to a static NUL-terminated string (or
        // NULL); it needs no `pw_init` and touches no shared state.
        let ptr = unsafe { pw_get_library_version() };
        if ptr.is_null() {
            return None;
        }
        // SAFETY: non-null, NUL-terminated, 'static (see above).
        Some(unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned())
    }

    // ── `pw_stream_get_time_n` shim ──────────────────────────────────────
    //
    // pipewire-rs (feature `v0_3_50`, enabled through cpal) calls
    // `pw_stream_get_time_n`, which libpipewire only has since 0.3.50.  The
    // binary is linked with BIND_NOW, so importing it would stop Inkue from
    // *starting* on Ubuntu 22.04 / Mint 21 (libpipewire 0.3.48) — even with
    // PipeWire never used.  Inkue therefore defines the symbol itself (the
    // executable's definition satisfies pipewire-sys's reference at link
    // time) and forwards to libpipewire's own, looked up once with
    // `dlsym(RTLD_NEXT)` before any PipeWire stream exists.  Older libraries
    // never get here: the native host is gated to >= 0.3.53.

    type GetTimeN = unsafe extern "C" fn(*mut c_void, *mut c_void, usize) -> c_int;

    /// libpipewire's `pw_stream_get_time_n`, or null before / without it.
    static GET_TIME_N: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

    /// Look up libpipewire's `pw_stream_get_time_n` for the shim (once).
    fn resolve_get_time_n() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            // SAFETY: `RTLD_NEXT` from the executable searches the libraries
            // loaded after it — libpipewire, not this shim.  The name is a
            // valid NUL-terminated C string.
            let f = unsafe { libc::dlsym(libc::RTLD_NEXT, c"pw_stream_get_time_n".as_ptr()) };
            GET_TIME_N.store(f, Ordering::Release);
        });
    }

    /// See the shim notes above.  Called from cpal's PipeWire process
    /// callback (RT thread): one atomic load and an indirect call, no lock,
    /// no allocation.
    ///
    /// # Safety
    /// Same contract as libpipewire's `pw_stream_get_time_n`.
    #[no_mangle]
    pub unsafe extern "C" fn pw_stream_get_time_n(stream: *mut c_void, time: *mut c_void, size: usize) -> c_int {
        let f = GET_TIME_N.load(Ordering::Acquire);
        if f.is_null() {
            return -libc::ENOSYS;
        }
        // SAFETY: a non-null value is libpipewire's `pw_stream_get_time_n`
        // (stored by `resolve_get_time_n`), whose C signature is `GetTimeN`.
        let f: GetTimeN = unsafe { std::mem::transmute::<*mut c_void, GetTimeN>(f) };
        // SAFETY: arguments forwarded unchanged under the caller's contract.
        unsafe { f(stream, time, size) }
    }

    /// The daemon answers and the loaded library is new enough.
    fn pipewire_usable() -> bool {
        if !cpal::available_hosts().contains(&cpal::HostId::PipeWire) {
            return false;
        }
        let version = library_version().unwrap_or_default();
        let ok = pipewire_lib_supported(&version);
        if ok {
            resolve_get_time_n();
        }
        static WARNED: AtomicBool = AtomicBool::new(false);
        if !ok && !WARNED.swap(true, Ordering::Relaxed) {
            log::warn!("libpipewire {version:?} is too old for the native PipeWire host: using ALSA");
        }
        ok
    }

    /// PipeWire when its daemon answers (and is not overridden), else ALSA.
    pub(super) fn active_host() -> (cpal::Host, HostKind) {
        let forced = std::env::var(HOST_ENV).ok();
        if choose_linux_host(pipewire_usable(), forced.as_deref()) == HostKind::PipeWire {
            match cpal::host_from_id(cpal::HostId::PipeWire) {
                Ok(host) => return (host, HostKind::PipeWire),
                Err(e) => log::warn!("PipeWire host unavailable ({e}), falling back to ALSA"),
            }
        }
        (alsa_host(), HostKind::Alsa)
    }

    /// See [`super::pipewire_output_present`].
    pub(super) fn output_node_present(node: &str) -> Option<bool> {
        let host = cpal::host_from_id(cpal::HostId::PipeWire).ok()?;
        let mut devices = host.output_devices().ok()?;
        Some(devices.any(|d| d.id().ok().is_some_and(|i| i.id() == node)))
    }

    /// The ALSA host, built explicitly: `cpal::default_host()` would try
    /// PipeWire first.
    pub(super) fn alsa_host() -> cpal::Host {
        cpal::host_from_id(cpal::HostId::Alsa).unwrap_or_else(|e| {
            log::warn!("ALSA host unavailable ({e}), using cpal's default host");
            cpal::default_host()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_prefers_pipewire_when_its_daemon_answers() {
        assert_eq!(choose_linux_host(true, None), HostKind::PipeWire);
        assert_eq!(choose_linux_host(false, None), HostKind::Alsa);
    }

    #[test]
    fn linux_host_env_forces_alsa_only() {
        assert_eq!(choose_linux_host(true, Some("alsa")), HostKind::Alsa);
        assert_eq!(choose_linux_host(true, Some(" ALSA ")), HostKind::Alsa);
        // Anything else leaves the automatic choice alone.
        assert_eq!(choose_linux_host(true, Some("pipewire")), HostKind::PipeWire);
        assert_eq!(choose_linux_host(true, Some("")), HostKind::PipeWire);
        // Forcing PipeWire cannot conjure a daemon.
        assert_eq!(choose_linux_host(false, Some("pipewire")), HostKind::Alsa);
    }

    #[test]
    fn native_host_needs_libpipewire_0_3_53() {
        for ok in ["0.3.53", "0.3.65", "1.0.5", "1.4.2", "0.4", "1.2.7-1ubuntu1"] {
            assert!(pipewire_lib_supported(ok), "{ok} should be supported");
        }
        // Ubuntu 22.04 ships 0.3.48: no `pw_buffer.requested` yet.
        for old in ["0.3.48", "0.3.52", "0.2.7", "", "garbage", "1"] {
            assert!(!pipewire_lib_supported(old), "{old:?} should be rejected");
        }
    }

    #[test]
    fn empty_ids_select_the_default_on_every_host() {
        for kind in [HostKind::System, HostKind::PipeWire, HostKind::Alsa] {
            assert_eq!(route(None, kind), (kind, DeviceTarget::Default));
            assert_eq!(route(Some(""), kind), (kind, DeviceTarget::Default));
        }
    }

    #[test]
    fn pipewire_host_targets_a_node_by_its_name() {
        let id = "pw:alsa_output.usb-BEHRINGER_UMC404HD_192k-00.pro-output-0";
        assert_eq!(
            route(Some(id), HostKind::PipeWire),
            (HostKind::PipeWire, DeviceTarget::Id("alsa_output.usb-BEHRINGER_UMC404HD_192k-00.pro-output-0")),
        );
        assert_eq!(route(Some("pw:"), HostKind::PipeWire), (HostKind::PipeWire, DeviceTarget::Default));
    }

    #[test]
    fn pipewire_host_maps_default_pcms_to_the_session_default() {
        // "default" is the System Default entry; "pipewire" a pre-native ALSA id.
        for id in ["default", "pipewire"] {
            assert_eq!(route(Some(id), HostKind::PipeWire), (HostKind::PipeWire, DeviceTarget::Default));
        }
    }

    #[test]
    fn pipewire_host_hands_raw_alsa_ids_to_alsa() {
        let id = "hw:CARD=UMC404HD,DEV=0";
        assert_eq!(route(Some(id), HostKind::PipeWire), (HostKind::Alsa, DeviceTarget::Id(id)));
    }

    #[test]
    fn alsa_fallback_opens_nodes_on_the_default_pcm() {
        assert_eq!(route(Some("pw:alsa_output.pci-0000_00_1f.3"), HostKind::Alsa), (HostKind::Alsa, DeviceTarget::Default));
        assert_eq!(route(Some("default"), HostKind::Alsa), (HostKind::Alsa, DeviceTarget::Default));
        assert_eq!(route(Some("hw:0,0"), HostKind::Alsa), (HostKind::Alsa, DeviceTarget::Id("hw:0,0")));
    }

    #[test]
    fn system_host_ids_pass_through_untouched() {
        // WASAPI endpoint ids and CoreAudio UIDs are opaque; "default" is not special.
        for id in ["{0.0.0.00000000}.{guid}", "BuiltInSpeakerDevice", "default", "pw:x"] {
            assert_eq!(route(Some(id), HostKind::System), (HostKind::System, DeviceTarget::Id(id)));
        }
    }

    #[test]
    fn only_a_node_on_the_pipewire_host_is_pinned() {
        let node = Some("pw:alsa_output.usb-BEHRINGER_UMC404HD_192k-00.pro-output-0");
        assert!(pins_pipewire_node(HostKind::PipeWire, node));
        // The session default follows WirePlumber's choice: nothing to lose.
        for id in [None, Some(""), Some("default"), Some("pipewire"), Some("pw:")] {
            assert!(!pins_pipewire_node(HostKind::PipeWire, id), "{id:?}");
        }
        // A raw PCM opens through ALSA, which reports its loss itself.
        assert!(!pins_pipewire_node(HostKind::PipeWire, Some("hw:CARD=PCH,DEV=0")));
        // Without the native host a `pw:` id plays on the ALSA default.
        assert!(!pins_pipewire_node(HostKind::Alsa, node));
        assert!(!pins_pipewire_node(HostKind::System, node));
    }

    #[test]
    fn non_node_ids_are_always_present() {
        for id in ["default", "pw:", "hw:0,0"] {
            assert_eq!(pipewire_output_present(id), Some(true), "{id:?}");
        }
    }

    #[test]
    fn stored_ids_round_trip_through_the_pipewire_host() {
        // Saved patches hold `pw:<node.name>` — the id cpal's native host must give back.
        let node = "alsa_output.pci-0000_00_1f.3.analog-stereo";
        let stored = inkue_device_id(HostKind::PipeWire, node);
        assert_eq!(stored, format!("pw:{node}"));
        assert_eq!(route(Some(&stored), HostKind::PipeWire), (HostKind::PipeWire, DeviceTarget::Id(node)));
    }

    #[test]
    fn pipewire_synthetic_defaults_store_as_default() {
        for node in PW_DEFAULT_NODES {
            let stored = inkue_device_id(HostKind::PipeWire, node);
            assert_eq!(stored, "default");
            assert_eq!(route(Some(&stored), HostKind::PipeWire), (HostKind::PipeWire, DeviceTarget::Default));
        }
    }

    #[test]
    fn alsa_and_system_ids_store_verbatim() {
        assert_eq!(inkue_device_id(HostKind::Alsa, "hw:CARD=PCH,DEV=0"), "hw:CARD=PCH,DEV=0");
        assert_eq!(inkue_device_id(HostKind::System, "{0.0.0.00000000}"), "{0.0.0.00000000}");
    }

    #[test]
    fn pipewire_lists_sinks_as_outputs_and_sources_as_inputs() {
        use cpal::DeviceDirection::{Duplex, Input, Output};
        // A sink (cpal: Duplex) is an output, not an input.
        assert!(lists_pipewire_device("alsa_output.x", Duplex, false));
        assert!(!lists_pipewire_device("alsa_output.x", Duplex, true));
        // A source is an input only.
        assert!(lists_pipewire_device("alsa_input.x", Input, true));
        assert!(!lists_pipewire_device("alsa_input.x", Input, false));
        // Another application's playback stream is not an output target.
        assert!(!lists_pipewire_device("Firefox", Output, false));
        // One synthetic default per direction stands for Inkue's `default`.
        assert!(lists_pipewire_device("output_default", Output, false));
        assert!(!lists_pipewire_device("output_default", Output, true));
        assert!(lists_pipewire_device("input_default", Input, true));
        assert!(!lists_pipewire_device("input_default", Input, false));
        assert!(!lists_pipewire_device("sink_default", Duplex, false));
        assert!(!lists_pipewire_device("sink_default", Duplex, true));
    }
}
