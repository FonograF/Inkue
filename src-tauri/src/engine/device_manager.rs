//! [`DeviceManager`] enumerates audio output devices and manages Output Patches.
//!
//! An Output Patch is a named mapping from a human-readable label to a
//! specific WASAPI/ASIO device and a set of channels — identical to QLab's
//! Output Patch concept.

use std::collections::HashMap;

use anyhow::{anyhow, Result};
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
}

impl OutputPatch {
    /// Create a new patch with a fresh UUID.
    pub fn new(name: impl Into<String>, device_id: impl Into<String>, channels: Vec<u16>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            device_id: device_id.into(),
            channels,
        }
    }
}

/// Manages device enumeration and the Output Patch table.
pub struct DeviceManager {
    patches: HashMap<OutputPatchId, OutputPatch>,
    /// Cached list of available devices; refreshed on demand.
    cached_devices: Vec<DeviceInfo>,
}

impl DeviceManager {
    /// Create a new manager and immediately enumerate available devices.
    pub fn new() -> Self {
        let mut mgr = Self {
            patches: HashMap::new(),
            cached_devices: Vec::new(),
        };
        // Best-effort: ignore errors during initial enumeration.
        let _ = mgr.refresh_devices();
        mgr
    }

    /// Re-enumerate output devices from the OS.  Call this when a device
    /// hotplug event is received.
    pub fn refresh_devices(&mut self) -> Result<()> {
        let host = cpal::default_host();
        let mut devices = Vec::new();

        for device in host.output_devices()? {
            let id = device.id().ok().map(|i| i.id().to_string()).unwrap_or_else(|| device.to_string());
            let name = device.to_string();
            let config = device.default_output_config();
            let (channels, sample_rate) = config
                .map(|c| (c.channels(), c.sample_rate()))
                .unwrap_or((2, 44100));

            devices.push(DeviceInfo {
                id,
                name,
                channels,
                sample_rate,
            });
        }

        self.cached_devices = devices;
        Ok(())
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

    // -----------------------------------------------------------------------
    // Output Patches
    // -----------------------------------------------------------------------

    /// Add or replace a patch in the table.
    pub fn upsert_patch(&mut self, patch: OutputPatch) {
        self.patches.insert(patch.id, patch);
    }

    /// Remove a patch by ID.
    pub fn remove_patch(&mut self, id: &OutputPatchId) {
        self.patches.remove(id);
    }

    /// Look up a patch by ID.
    pub fn patch(&self, id: &OutputPatchId) -> Option<&OutputPatch> {
        self.patches.get(id)
    }

    /// Return all patches.
    pub fn patches(&self) -> Vec<&OutputPatch> {
        self.patches.values().collect()
    }

    /// Resolve an Output Patch to the underlying cpal [`cpal::Device`].
    /// Returns `Err` if the patch does not exist or the device is not found.
    pub fn resolve_device(&self, patch_id: &OutputPatchId) -> Result<cpal::Device> {
        let patch = self
            .patches
            .get(patch_id)
            .ok_or_else(|| anyhow!("Output patch {:?} not found", patch_id))?;

        let host = cpal::default_host();
        for device in host.output_devices()? {
            if device.id().ok().map(|id| id.id() == patch.device_id).unwrap_or(false) {
                return Ok(device);
            }
        }
        Err(anyhow!(
            "Audio device '{}' not found (patch '{}')",
            patch.device_id,
            patch.name
        ))
    }

    /// Create a sensible default patch pointing at the system default device.
    pub fn create_default_patch(&mut self) -> Option<OutputPatchId> {
        let device_id = cpal::default_host()
            .default_output_device()
            .and_then(|d| d.id().ok().map(|i| i.id().to_string()))?;

        let patch = OutputPatch::new("Default Output", device_id, vec![0, 1]);
        let id = patch.id;
        self.upsert_patch(patch);
        Some(id)
    }
}

impl Default for DeviceManager {
    fn default() -> Self {
        Self::new()
    }
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
        let description = props
            .get("node.description").and_then(|v| v.as_str())
            .or_else(|| props.get("node.nick").and_then(|v| v.as_str()))
            .unwrap_or(node_name);
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
