//! Tauri command handlers, grouped by domain.

use std::path::Path;

use uuid::Uuid;

use crate::health::{self, HealthAlert, HealthLevel};

/// Health-alert key for a cue whose audio file failed to decode.
fn decode_alert_key(cue_id: Uuid) -> String {
    format!("cue-decode-{cue_id}")
}

/// Surface a background audio-decode failure to the operator.
///
/// A present-but-undecodable file (corrupt, or a codec even the libmpv fallback
/// can't read) otherwise leaves the cue permanently silent — `file_duration()`
/// stays `None`, so GO is a silent no-op and nothing tells the operator why.
/// This raises a per-cue banner instead; [`clear_decode_failure`] retires it
/// when a later decode of the same cue succeeds (e.g. after a relink).
pub(crate) fn surface_decode_failure(cue_id: Uuid, path: &Path) {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("its audio file");
    health::set(HealthAlert::new(
        decode_alert_key(cue_id),
        HealthLevel::Warning,
        format!("An audio cue could not decode '{name}' — it will not play. Check or relink the file."),
    ));
}

/// Clear a cue's decode-failure banner (its audio decoded successfully).
pub(crate) fn clear_decode_failure(cue_id: Uuid) {
    health::clear(&decode_alert_key(cue_id));
}

pub mod cue_cmds;
pub mod cue_list_cmds;
pub mod device_cmds;
pub mod health_cmds;
pub mod input_cmds;
pub mod light_cmds;
pub mod log_cmds;
pub mod timecode_cmds;
pub mod midi_cmds;
pub mod network_cmds;
pub mod osc_cmds;
pub mod preferences_cmds;
pub mod preflight_cmds;
pub mod recovery_cmds;
pub mod transport_cmds;
pub mod undo_cmds;
pub mod workspace_cmds;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_failure_alert_is_set_and_cleared() {
        // Unique id per run → no clash with the process-wide health registry.
        let id = Uuid::new_v4();
        let key = decode_alert_key(id);

        surface_decode_failure(id, Path::new("C:/media/broken.wav"));
        let alert = health::snapshot().into_iter().find(|a| a.key == key);
        let alert = alert.expect("a decode failure must raise a health alert");
        assert!(alert.message.contains("broken.wav"), "alert should name the file");

        clear_decode_failure(id);
        assert!(
            health::snapshot().iter().all(|a| a.key != key),
            "a later successful decode must clear the alert"
        );
    }
}
