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
            let name = device.name().unwrap_or_else(|_| "Unknown".to_string());
            let config = device.default_output_config();
            let (channels, sample_rate) = config
                .map(|c| (c.channels(), c.sample_rate().0))
                .unwrap_or((2, 44100));

            devices.push(DeviceInfo {
                id: name.clone(),
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
        let default_name = host
            .default_output_device()
            .and_then(|d| d.name().ok())?;
        self.cached_devices.iter().find(|d| d.name == default_name)
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
            if device.name().ok().as_deref() == Some(&patch.device_id) {
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
            .and_then(|d| d.name().ok())?;

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
