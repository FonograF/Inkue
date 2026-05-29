//! Pre-arm helper for [`VideoCue`](crate::cue::video_cue::VideoCue).
//!
//! Called from command handlers and the event loop whenever the Playhead
//! moves.  No transport logic or cue-list internals are modified — this
//! module only reads cue metadata and delegates to [`OutputEngine`].

use std::sync::Arc;

use crate::{
    cue::types::{CueId, CueType},
    engine::output_engine::OutputEngine,
    show::cue_list::CueList,
};

/// Respond to a Playhead change by (re-)arming the output engine.
///
/// 1. Always cancels any existing pre-arm.
/// 2. If the new playhead cue is a `VideoCue` with a file assigned and mpv is
///    idle, calls [`OutputEngine::pre_arm_voice`] so the pipe is pre-connected
///    and the ring buffer consumer is installed before GO.
pub fn update_video_pre_arm(
    new_cue_id: Option<CueId>,
    cue_list: &CueList,
    output_engine: &Arc<OutputEngine>,
    output_screen: Option<u32>,
) {
    output_engine.cancel_pre_arm();

    let Some(cue_id) = new_cue_id else { return };
    let Some(cue)    = cue_list.get(&cue_id) else { return };

    if cue.cue_type() != CueType::Video { return }

    let data = cue.serialize();

    let file_path = match data.get("file_path").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => std::path::PathBuf::from(s),
        _ => return,
    };

    let volume_db  = data.get("volume_db") .and_then(|v| v.as_f64()).unwrap_or(0.0);
    let loop_count = data.get("loop_count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let start_ms   = data.get("start_time_ms").and_then(|v| v.as_u64());
    let end_ms     = data.get("end_time_ms")  .and_then(|v| v.as_u64());

    if let Err(e) = output_engine.pre_arm_voice(
        cue_id,
        &file_path,
        None,
        volume_db,
        loop_count,
        start_ms,
        end_ms,
        None,
        output_screen,
    ) {
        log::warn!("[pre-arm] failed to pre-arm cue {cue_id}: {e}");
    }
}
