//! Pre-arm helper for [`VideoCue`](crate::cue::video_cue::VideoCue).
//!
//! Called from command handlers and the event loop whenever the Playhead
//! moves.  No transport logic or cue-list internals are modified — this
//! module only reads cue metadata and delegates to [`VideoEngine`].

use std::sync::Arc;

use crate::{
    cue::types::{CueId, CueType},
    engine::video_engine::VideoEngine,
    show::cue_list::CueList,
};

/// Respond to a Playhead change by (re-)arming the video engine.
///
/// 1. Always cancels any existing pre-arm (the old playhead cue is no longer
///    next).
/// 2. If the new playhead cue is a `VideoCue` with a file assigned and mpv is
///    idle, calls [`VideoEngine::pre_arm_voice`] so the pipe is pre-connected
///    and the ring buffer consumer is installed before the operator presses GO.
///
/// Errors from `pre_arm_voice` are logged but not propagated — a failed
/// pre-arm degrades gracefully to the normal `play_voice` path on GO.
pub fn update_video_pre_arm(
    new_cue_id: Option<CueId>,
    cue_list: &CueList,
    video_engine: &Arc<VideoEngine>,
) {
    // Always cancel the previous pre-arm when the Playhead moves.
    video_engine.cancel_pre_arm();

    let Some(cue_id) = new_cue_id else { return };
    let Some(cue)    = cue_list.get(&cue_id) else { return };

    if cue.cue_type() != CueType::Video { return }

    let data = cue.serialize();

    // Only pre-arm if the cue has a file assigned.
    let file_path = match data.get("file_path").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => std::path::PathBuf::from(s),
        _ => return,
    };

    let volume_db  = data.get("volume_db") .and_then(|v| v.as_f64()).unwrap_or(0.0);
    let loop_count = data.get("loop_count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let start_ms   = data.get("start_time_ms").and_then(|v| v.as_u64());
    let end_ms     = data.get("end_time_ms")  .and_then(|v| v.as_u64());
    let screen_idx = data.get("screen_index") .and_then(|v| v.as_u64()).map(|v| v as u32);

    if let Err(e) = video_engine.pre_arm_voice(
        cue_id,
        &file_path,
        None,        // surface_id — not yet used
        volume_db,
        loop_count,
        start_ms,
        end_ms,
        None,        // fade_in — applied at play time
        screen_idx,
    ) {
        log::warn!("[pre-arm] failed to pre-arm cue {cue_id}: {e}");
    }
}
