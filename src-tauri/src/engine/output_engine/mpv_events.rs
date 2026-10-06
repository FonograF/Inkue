//! Overlay-context event loop — one thread per output, running as long as the
//! output does.
//!
//! The overlay context only ever shows the timer OSD, a Text Cue and test
//! patterns (video slots own their events, see `slot.rs`), so there is nothing
//! to forward to the show: this thread surfaces mpv's diagnostics in the log and
//! arms the software-decoding fallback, because libavcodec's messages land
//! here (see below).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::engine::mpv_sys::{MpvEventLogMessage, MpvLib, MPV_EVENT_LOG_MESSAGE, MPV_EVENT_SHUTDOWN};

use super::types::MpvCtx;

/// Drain the overlay context's events until `stop` is raised (the output is
/// being destroyed — its teardown wakes this wait) or mpv shuts down.
pub(super) fn overlay_event_loop(lib: Arc<MpvLib>, ctx: Arc<MpvCtx>, label: String, stop: Arc<AtomicBool>) {
    loop {
        let event = unsafe { (lib.mpv_wait_event)(ctx.0, 1.0) };
        if stop.load(Ordering::Acquire) {
            break;
        }
        if event.is_null() {
            continue;
        }
        match unsafe { (*event).event_id } {
            MPV_EVENT_SHUTDOWN => break,

            MPV_EVENT_LOG_MESSAGE => {
                let data = unsafe { (*event).data as *const MpvEventLogMessage };
                if data.is_null() {
                    continue;
                }
                let level = unsafe { std::ffi::CStr::from_ptr((*data).level) }.to_string_lossy();
                let text = unsafe { std::ffi::CStr::from_ptr((*data).text) }.to_string_lossy();
                let trimmed = text.trim_end_matches('\n');
                if trimmed.is_empty() {
                    continue;
                }
                match level.as_ref() {
                    "fatal" | "error" => log::error!("[mpv:{label}] {trimmed}"),
                    "warn" => log::warn!("[mpv:{label}] {trimmed}"),
                    "info" => log::info!("[mpv:{label}] {trimmed}"),
                    _ => log::debug!("[mpv:{label}] {trimmed}"),
                }
                // libavcodec logging is process-global: mpv routes it to the
                // **first** core created, which is an overlay context — so a
                // video slot's `h264: Failed setup for format d3d11: hwaccel
                // initialisation returned error.` lands here, never on the slot
                // that loaded the file (verified: a second core loading the
                // file receives none of them).  This is therefore where the
                // software-decoding fallback for issue #5 is armed.
                if super::slot::reports_hwdec_failure(trimmed) {
                    super::slot::fall_back_to_software("output");
                }
            }

            _ => {}
        }
    }
}
