//! mpv event loop thread — runs for the lifetime of the application.
//!
//! Receives mpv events (file loaded, EOF, errors, log messages) and forwards
//! status to the Tauri event loop via the `status_tx` channel.

use std::ffi::c_void;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crossbeam_channel::Sender;

use crate::engine::mpv_sys::{
    MpvEventEndFile, MpvEventLogMessage, MpvLib,
    MPV_END_FILE_REASON_EOF, MPV_END_FILE_REASON_ERROR,
    MPV_EVENT_END_FILE, MPV_EVENT_FILE_LOADED, MPV_EVENT_LOG_MESSAGE,
    MPV_EVENT_PLAYBACK_RESTART, MPV_EVENT_SEEK, MPV_EVENT_SHUTDOWN,
    MPV_EVENT_START_FILE, MPV_EVENT_VIDEO_RECONFIG, MPV_FORMAT_DOUBLE,
};

use super::types::{MpvCtx, OutputStatus, VoiceId};
use super::{cs, WM_SETUP_MPV_CHILD};

pub(super) fn mpv_event_loop(
    lib: Arc<MpvLib>,
    ctx: Arc<MpvCtx>,
    current_voice: Arc<Mutex<Option<VoiceId>>>,
    status_tx: Sender<OutputStatus>,
    parent_hwnd: isize,
    go_sent_at: Arc<Mutex<Option<Instant>>>,
    video_pcm_active: Arc<std::sync::atomic::AtomicBool>,
) {
    loop {
        let event = unsafe { (lib.mpv_wait_event)(ctx.0, 2.0) };
        if event.is_null() {
            continue;
        }
        let event_id = unsafe { (*event).event_id };

        match event_id {
            MPV_EVENT_SHUTDOWN => break,

            MPV_EVENT_START_FILE => {
                log::info!("[output-mpv] MPV_EVENT_START_FILE");
            }

            MPV_EVENT_SEEK => {
                log::info!("[output-mpv] MPV_EVENT_SEEK");
            }

            MPV_EVENT_VIDEO_RECONFIG => {
                log::info!("[output-mpv] MPV_EVENT_VIDEO_RECONFIG");
            }

            MPV_EVENT_PLAYBACK_RESTART => {
                let go_time = *go_sent_at.lock().unwrap();
                if let Some(t) = go_time {
                    let ms = t.elapsed().as_millis();
                    video_pcm_active.store(true, Ordering::Release);
                    // Reveal content now that the first live frame is rendering.
                    // Checking timer_active avoids interrupting a fade-in animation.
                    let fade_active = super::FADE_STATE
                        .get()
                        .and_then(|fs| fs.lock().ok())
                        .map(|s| s.timer_active)
                        .unwrap_or(false);
                    if !fade_active {
                        super::fade::set_overlay_alpha(0);
                    }
                    log::info!(
                        "[output-mpv] MPV_EVENT_PLAYBACK_RESTART — {ms}ms after GO \
                         (video PCM mixing activated)"
                    );
                } else {
                    video_pcm_active.store(false, Ordering::Release);
                    log::info!(
                        "[output-mpv] MPV_EVENT_PLAYBACK_RESTART during pre-arm \
                         (gate closed)"
                    );
                }
            }

            MPV_EVENT_LOG_MESSAGE => {
                let data = unsafe { (*event).data as *const MpvEventLogMessage };
                if !data.is_null() {
                    let level_cstr = unsafe { std::ffi::CStr::from_ptr((*data).level) };
                    let text_cstr  = unsafe { std::ffi::CStr::from_ptr((*data).text) };
                    let level   = level_cstr.to_string_lossy();
                    let text    = text_cstr.to_string_lossy();
                    let trimmed = text.trim_end_matches('\n');
                    if !trimmed.is_empty() {
                        match level.as_ref() {
                            "fatal" | "error" => log::error!("[mpv] {trimmed}"),
                            "warn"            => log::warn! ("[mpv] {trimmed}"),
                            "info"            => log::info! ("[mpv] {trimmed}"),
                            _                 => log::debug!("[mpv] {trimmed}"),
                        }
                    }
                }
            }

            MPV_EVENT_FILE_LOADED => {
                log::info!("[output-mpv] MPV_EVENT_FILE_LOADED");
                unsafe {
                    use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;
                    PostMessageW(parent_hwnd, WM_SETUP_MPV_CHILD, 0, 0);
                }

                let mut duration_secs: f64 = 0.0;
                let ret = unsafe {
                    let name = cs("duration");
                    (lib.mpv_get_property)(
                        ctx.0, name.as_ptr(), MPV_FORMAT_DOUBLE,
                        &mut duration_secs as *mut f64 as *mut c_void,
                    )
                };
                if ret == 0 {
                    if let Some(vid) = *current_voice.lock().unwrap() {
                        let _ = status_tx.send(OutputStatus::Duration {
                            voice_id: vid,
                            duration_ms: (duration_secs * 1000.0) as u64,
                        });
                    }
                }
            }

            MPV_EVENT_END_FILE => {
                let data_ptr = unsafe { (*event).data };
                if let Some(end_data) =
                    unsafe { (data_ptr as *mut MpvEventEndFile).as_ref() }
                {
                    use crate::engine::mpv_sys::{
                        MPV_END_FILE_REASON_STOP, MPV_END_FILE_REASON_QUIT,
                    };
                    let reason_name = match end_data.reason {
                        MPV_END_FILE_REASON_EOF   => "EOF",
                        MPV_END_FILE_REASON_STOP  => "STOP",
                        MPV_END_FILE_REASON_QUIT  => "QUIT",
                        MPV_END_FILE_REASON_ERROR => "ERROR",
                        _                          => "UNKNOWN",
                    };
                    log::info!(
                        "[output-mpv] MPV_EVENT_END_FILE reason={reason_name} ({})",
                        end_data.reason
                    );

                    let voice_id = match end_data.reason {
                        MPV_END_FILE_REASON_EOF | MPV_END_FILE_REASON_ERROR => {
                            current_voice.lock().unwrap().take()
                        }
                        _ => *current_voice.lock().unwrap(),
                    };

                    if let Some(vid) = voice_id {
                        match end_data.reason {
                            MPV_END_FILE_REASON_EOF => {
                                video_pcm_active.store(false, Ordering::Release);
                                *go_sent_at.lock().unwrap() = None;
                                let _ = status_tx
                                    .send(OutputStatus::Completed { voice_id: vid });
                            }
                            MPV_END_FILE_REASON_ERROR => {
                                video_pcm_active.store(false, Ordering::Release);
                                *go_sent_at.lock().unwrap() = None;
                                let msg = format!("mpv error (code {})", end_data.error);
                                let _ = status_tx.send(OutputStatus::Error {
                                    voice_id: vid, message: msg,
                                });
                            }
                            _ => {}
                        }
                    }
                }
            }

            _ => {}
        }
    }
}
