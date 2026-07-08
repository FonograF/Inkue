//! [`DeviceManager`] enumerates audio output devices; [`OutputPatch`] is the
//! named device+channels mapping (QLab's Output Patch concept) stored in the
//! workspace and resolved by cues at GO time.

use std::sync::mpsc;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Serialisable summary of an audio output device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// Stable identifier derived from the device name.
    pub id: String,
    /// Human-readable device name returned by the OS.
    pub name: String,
    /// Number of output channels the device supports.
    pub channels: u16,
    /// Supported sample rate (first offered by the device).
    pub sample_rate: u32,
}

/// Unique identifier for an Output Patch.
pub type OutputPatchId = Uuid;

/// A named mapping from a label to a specific audio device + channel range.
///
/// Every [`AudioCue`](crate::cue::audio_cue::AudioCue) references an
/// `OutputPatch` rather than a device directly, so re-patching a show
/// requires changing only one place.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputPatch {
    pub id: OutputPatchId,
    /// Display label shown in the UI (e.g. "Main PA", "Monitors").
    pub name: String,
    /// The OS device identifier this patch routes to.
    pub device_id: String,
    /// Zero-based channel indices on the target device (e.g. [0, 1] for stereo L/R).
    pub channels: Vec<u16>,
    /// Mixer fader for this patch, in dB (0 = unity).  Applied as a gain
    /// multiplier to every voice routed through the patch.
    #[serde(default)]
    pub gain_db: f32,
}

impl OutputPatch {
    /// Create a new patch with a fresh UUID.
    pub fn new(name: impl Into<String>, device_id: impl Into<String>, channels: Vec<u16>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            device_id: device_id.into(),
            channels,
            gain_db: 0.0,
        }
    }
}

/// Manages device enumeration.
///
/// The Output Patch table itself lives in the workspace
/// (`Workspace::output_patches`) — persisted with the show and read by
/// `CueContext::resolve_patch` at GO time.
pub struct DeviceManager {
    /// Cached list of available devices; refreshed on demand.
    cached_devices: Vec<DeviceInfo>,
}

impl DeviceManager {
    /// Create a new manager with an **empty** cache.
    ///
    /// Enumeration is deliberately not run here.  On Windows the WASAPI device
    /// query costs ~100 ms per device and can hang indefinitely on a misbehaving
    /// driver (cpal #867); since `new()` is called on the main thread during app
    /// setup, enumerating here froze startup (audio dead, hotkeys unresponsive).
    /// The cache is warmed by a bounded background refresh spawned from
    /// [`AudioEngine::new`](crate::engine::audio_engine::AudioEngine::new) and
    /// re-filled on demand through [`replace_cache`](Self::replace_cache).
    pub fn new() -> Self {
        Self {
            cached_devices: Vec::new(),
        }
    }

    /// Overwrite the cached device list with a freshly enumerated one.
    ///
    /// Callers enumerate **off** this struct's lock — via
    /// [`enumerate_output_devices`] wrapped in [`run_bounded`] — and only lock to
    /// store the result, so the `DeviceManager` mutex is never held across a slow
    /// (or hung) WASAPI call.
    pub fn replace_cache(&mut self, devices: Vec<DeviceInfo>) {
        self.cached_devices = devices;
    }

    /// Return the cached device list.
    pub fn devices(&self) -> &[DeviceInfo] {
        &self.cached_devices
    }

    /// Return the default output device info, if one exists.
    pub fn default_device(&self) -> Option<&DeviceInfo> {
        let host = cpal::default_host();
        let default_id = host
            .default_output_device()
            .and_then(|d| d.id().ok().map(|i| i.id().to_string()))?;
        self.cached_devices.iter().find(|d| d.id == default_id)
    }

}

impl Default for DeviceManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Upper bound on one output-device enumeration before callers fall back to
/// whatever they already have.  Generous for a healthy multi-device Windows box
/// (~100 ms/device) yet short enough that a hung driver never strands the UI.
pub const ENUM_TIMEOUT: Duration = Duration::from_secs(4);

/// Build a [`DeviceInfo`] from a cpal output device (queries its default config).
fn output_device_info(device: &cpal::Device) -> DeviceInfo {
    let id = device
        .id()
        .ok()
        .map(|i| i.id().to_string())
        .unwrap_or_else(|| device.to_string());
    let name = device.to_string();
    let (channels, sample_rate) = device
        .default_output_config()
        .map(|c| (c.channels(), c.sample_rate()))
        .unwrap_or((2, 44100));
    DeviceInfo {
        id,
        name,
        channels,
        sample_rate,
    }
}

/// Enumerate output devices directly from cpal.
///
/// **Slow on Windows** — WASAPI queries every device's mix format and can hang
/// on a bad driver (cpal #867).  Never call this on the main thread: wrap it in
/// [`run_bounded`], and inside a Tauri command run it via `spawn_blocking`.
pub fn enumerate_output_devices() -> Vec<DeviceInfo> {
    let host = cpal::default_host();
    let mut devices = Vec::new();
    if let Ok(iter) = host.output_devices() {
        for device in iter {
            devices.push(output_device_info(&device));
        }
    }
    // Fall back to just the default device if full enumeration yielded nothing.
    if devices.is_empty() {
        if let Some(device) = host.default_output_device() {
            devices.push(output_device_info(&device));
        }
    }
    devices
}

/// Run `job` on a scratch thread, waiting at most `timeout` for its result.
///
/// Returns `None` on timeout — the scratch thread is then detached and its
/// eventual value dropped.  This is the guard that stops a hung device
/// enumeration from blocking the caller (and, for a main-thread caller, the
/// whole UI).
pub fn run_bounded<T, F>(timeout: Duration, job: F) -> Option<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("inkue-device-enum".to_string())
        .spawn(move || {
            let _ = tx.send(job());
        })
        .ok()?;
    rx.recv_timeout(timeout).ok()
}

// ---------------------------------------------------------------------------
// Linux: PipeWire device enumeration + PIPEWIRE_NODE helper
// ---------------------------------------------------------------------------

/// On non-Linux, noop passthrough kept for call-site compatibility.
#[cfg(not(target_os = "linux"))]
pub fn humanize_linux_devices(devices: Vec<DeviceInfo>) -> Vec<DeviceInfo> {
    devices
}

/// Enumerate audio devices via PipeWire on Linux.
/// Returns (input_devices, output_devices).  Falls back to an empty vec if
/// `pw-dump` is unavailable — callers should then use cpal enumeration.
#[cfg(target_os = "linux")]
pub fn query_pipewire_devices() -> (Vec<DeviceInfo>, Vec<DeviceInfo>) {
    let Ok(out) = std::process::Command::new("pw-dump").output() else {
        return (Vec::new(), Vec::new());
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let Ok(data) = serde_json::from_str::<serde_json::Value>(&text) else {
        return (Vec::new(), Vec::new());
    };
    let Some(arr) = data.as_array() else {
        return (Vec::new(), Vec::new());
    };

    let mut inputs  = Vec::new();
    let mut outputs = Vec::new();

    for node in arr {
        let node_type = node.get("type").and_then(|v| v.as_str()).unwrap_or_default();
        if !node_type.contains("Node") { continue; }
        let props = match node.get("info").and_then(|i| i.get("props")) {
            Some(p) => p,
            None    => continue,
        };
        let cls = props.get("media.class").and_then(|v| v.as_str()).unwrap_or_default();
        if cls != "Audio/Source" && cls != "Audio/Sink" { continue; }

        let node_name = props.get("node.name").and_then(|v| v.as_str()).unwrap_or_default();
        if node_name.is_empty() { continue; }
        let nick = props.get("node.nick").and_then(|v| v.as_str()).unwrap_or_default();
        let desc = props.get("node.description").and_then(|v| v.as_str()).unwrap_or_default();
        // Prefer nick when the description is just the nick repeated with
        // PipeWire profile noise (e.g. "UMC404HD 192k Direct UMC404HD 192k").
        let description = if !nick.is_empty() && (desc.is_empty() || desc.starts_with(nick)) {
            nick
        } else if !desc.is_empty() {
            desc
        } else {
            node_name
        };
        let channels: u16 = props
            .get("audio.channels").and_then(|v| v.as_u64())
            .unwrap_or(2) as u16;

        let info = DeviceInfo {
            id: format!("pw:{node_name}"),
            name: description.to_string(),
            channels,
            sample_rate: 48_000,
        };
        if cls == "Audio/Source" {
            inputs.push(info);
        } else {
            outputs.push(info);
        }
    }
    (inputs, outputs)
}

/// Return a device list for Linux using PipeWire enumeration.
/// `is_input = true` → Audio/Source nodes; `false` → Audio/Sink nodes.
/// Falls back to `fallback` if PipeWire is unavailable.
#[cfg(target_os = "linux")]
pub fn linux_devices(is_input: bool, fallback: Vec<DeviceInfo>) -> Vec<DeviceInfo> {
    let (inputs, outputs) = query_pipewire_devices();
    let mut nodes = if is_input { inputs } else { outputs };
    if nodes.is_empty() {
        return fallback;
    }
    // Prepend "System Default" so the user can always choose the OS default.
    nodes.insert(0, DeviceInfo {
        id:          "default".to_string(),
        name:        "System Default".to_string(),
        channels:    2,
        sample_rate: 48_000,
    });
    nodes
}

/// If `device_id` is a `pw:…` synthetic ID, return the bare PipeWire node name.
pub fn pipewire_node_of(device_id: &str) -> Option<&str> {
    device_id.strip_prefix("pw:")
}

/// RAII guard that sets `PIPEWIRE_NODE` for the current process while held and
/// removes it on drop.  A static mutex serialises concurrent stream opens so
/// the env-var is never overwritten by a racing thread.
#[cfg(target_os = "linux")]
pub struct PwNodeGuard(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);

#[cfg(target_os = "linux")]
impl Drop for PwNodeGuard {
    fn drop(&mut self) {
        // SAFETY: We still hold the mutex — no other thread touches this var.
        unsafe { std::env::remove_var("PIPEWIRE_NODE"); }
        // MutexGuard drops here, releasing the lock.
    }
}

#[cfg(target_os = "linux")]
static PW_OPEN_MTX: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();

/// Lock the stream-open mutex, set `PIPEWIRE_NODE=node_name`, and return a
/// guard whose `Drop` removes the var and releases the lock.
#[cfg(target_os = "linux")]
pub fn acquire_pw_node(node_name: &str) -> PwNodeGuard {
    let mtx = PW_OPEN_MTX.get_or_init(|| std::sync::Mutex::new(()));
    let guard = mtx.lock().unwrap_or_else(|p| p.into_inner());
    // SAFETY: mutex is held; no other thread calls set_var concurrently.
    unsafe { std::env::set_var("PIPEWIRE_NODE", node_name); }
    PwNodeGuard(guard)
}

/// Legacy shim — still called from `preferences_cmds` for the ALSA fallback path
/// (non-Linux is a noop, Linux now uses `linux_devices` instead).
#[cfg(target_os = "linux")]
pub fn humanize_linux_devices(devices: Vec<DeviceInfo>) -> Vec<DeviceInfo> {
    devices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_bounded_returns_value_when_job_finishes_in_time() {
        let out = run_bounded(Duration::from_secs(2), || 21 * 2);
        assert_eq!(out, Some(42));
    }

    #[test]
    fn run_bounded_gives_up_on_a_slow_job() {
        // A job slower than the timeout must not block the caller past `timeout`.
        let start = std::time::Instant::now();
        let out = run_bounded(Duration::from_millis(50), || {
            std::thread::sleep(Duration::from_millis(750));
            42
        });
        assert_eq!(out, None);
        assert!(
            start.elapsed() < Duration::from_millis(500),
            "run_bounded returned only after the slow job finished — it did not time out"
        );
    }

    #[test]
    fn new_device_manager_starts_with_an_empty_cache() {
        // Startup must not enumerate (that is what froze the app on Windows).
        assert!(DeviceManager::new().devices().is_empty());
    }

    #[test]
    fn replace_cache_overwrites_the_device_list() {
        let mut mgr = DeviceManager::new();
        mgr.replace_cache(vec![DeviceInfo {
            id: "dev-1".into(),
            name: "Test".into(),
            channels: 2,
            sample_rate: 48_000,
        }]);
        assert_eq!(mgr.devices().len(), 1);
        assert_eq!(mgr.devices()[0].id, "dev-1");
    }
}
