//! mpv event loop thread — runs for the lifetime of the application.
//!
//! Receives mpv events (file loaded, EOF, errors, log messages) and forwards
//! status to the Tauri event loop via the `status_tx` channel.

use std::ffi::c_void;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;

use crate::engine::mpv_sys::{
    MpvEventEndFile, MpvEventLogMessage, MpvLib,
    MPV_END_FILE_REASON_EOF, MPV_END_FILE_REASON_ERROR,
    MPV_EVENT_END_FILE, MPV_EVENT_FILE_LOADED, MPV_EVENT_LOG_MESSAGE,
    MPV_EVENT_PLAYBACK_RESTART, MPV_EVENT_SEEK, MPV_EVENT_SHUTDOWN,
    MPV_EVENT_START_FILE, MPV_EVENT_VIDEO_RECONFIG, MPV_FORMAT_DOUBLE,
};
use crate::engine::AudioEngine;

use super::types::{MpvCtx, OutputStatus, VoiceId};
use super::{cs, OUTPUT_CURRENT_AUDIO_VOICE};

pub(super) fn mpv_event_loop(
    lib: Arc<MpvLib>,
    ctx: Arc<MpvCtx>,
    current_voice: Arc<Mutex<Option<VoiceId>>>,
    status_tx: Sender<OutputStatus>,
    go_sent_at: Arc<Mutex<Option<Instant>>>,
    audio_engine: Arc<AudioEngine>,
) {
    // Failsafe: a paused video load is revealed + unpaused by PLAYBACK_RESTART.
    // If that event is ever delayed or missing, this deadline forces the reveal
    // so the output can never get stuck on a permanent black screen.
    let mut reveal_deadline: Option<Instant> = None;

    loop {
        let event = unsafe { (lib.mpv_wait_event)(ctx.0, 1.0) };

        if let Some(deadline) = reveal_deadline {
            if Instant::now() >= deadline {
                reveal_deadline = None;
                let pending = super::OUTPUT_PENDING_VIDEO_START
                    .get()
                    .and_then(|m| m.lock().ok().and_then(|mut p| p.take()));
                if let Some(start) = pending {
                    log::warn!(
                        "[output-mpv] PLAYBACK_RESTART watchdog fired — forcing \
                         reveal/unpause (mpv did not signal first frame)"
                    );
                    start_video_playback(&lib, &ctx, &audio_engine, start.fade_in_ms);
                }
            }
        }

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
                // A fractional per-cue crop parks here until the source
                // dimensions (video-params/w|h) exist — which they do once the
                // video output reconfigures.  One-shot: applying the pixel
                // crop triggers another VIDEO_RECONFIG, by which time the
                // pending slot is already empty.
                let pending = super::PENDING_CROP
                    .get()
                    .and_then(|m| m.lock().ok())
                    .and_then(|g| *g);
                if let Some(geometry) = pending {
                    if super::try_apply_crop(&lib, ctx.0, &geometry) {
                        if let Some(m) = super::PENDING_CROP.get() {
                            if let Ok(mut p) = m.lock() {
                                *p = None;
                            }
                        }
                    }
                }
            }

            MPV_EVENT_PLAYBACK_RESTART => {
                let go_time = *go_sent_at.lock().unwrap();
                let Some(t) = go_time else {
                    // Image / idle (incl. the Text Cue lavfi dummy).  The text is
                    // an osd-overlay, which persists across the file load, so
                    // nothing needs re-applying here.
                    log::debug!("[output-mpv] PLAYBACK_RESTART (image/idle)");
                    continue;
                };

                // First frame after a *paused* load: reveal + unpause exactly
                // once.  Frame 0 is decoded, the d3d11 decoder is warm and the
                // frame queue is primed, so unpausing starts audio and video
                // together from frame 0 — no offset, no decoder-warmup freeze.
                let pending = super::OUTPUT_PENDING_VIDEO_START
                    .get()
                    .and_then(|m| m.lock().ok().and_then(|mut p| p.take()));

                match pending {
                    Some(start) => {
                        reveal_deadline = None;
                        start_video_playback(&lib, &ctx, &audio_engine, start.fade_in_ms);
                        log::info!(
                            "[output-mpv] PLAYBACK_RESTART — first frame {}ms after GO \
                             (revealed, unpaused, audio voice resumed)",
                            t.elapsed().as_millis(),
                        );
                    }
                    None => {
                        // Loop restart / seek: content already shown, audio voice
                        // already playing — nothing to do.
                        log::debug!("[output-mpv] PLAYBACK_RESTART (loop/seek)");
                    }
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
                        // libavcodec logging is process-global: mpv routes it to
                        // the **first** core created, which is this overlay
                        // context — so a video slot's `h264: Failed setup for
                        // format d3d11: hwaccel initialisation returned error.`
                        // lands here, never on the slot that loaded the file
                        // (verified: a second core loading the file receives
                        // none of them).  This is therefore where the
                        // software-decoding fallback for issue #5 is armed.
                        if super::slot::reports_hwdec_failure(trimmed) {
                            super::slot::fall_back_to_software("output");
                        }
                    }
                }
            }

            MPV_EVENT_FILE_LOADED => {
                log::info!("[output-mpv] MPV_EVENT_FILE_LOADED");
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

                // Arm the reveal watchdog for a paused video load: PLAYBACK_RESTART
                // normally fires within milliseconds, but if it does not we still
                // reveal + unpause rather than hang on black.
                let has_pending = super::OUTPUT_PENDING_VIDEO_START
                    .get()
                    .and_then(|m| m.lock().ok())
                    .map(|p| p.is_some())
                    .unwrap_or(false);
                if has_pending {
                    reveal_deadline = Some(Instant::now() + Duration::from_millis(2500));
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
                                stop_paired_audio(&audio_engine);
                                *go_sent_at.lock().unwrap() = None;
                                // Natural end: mpv goes idle (keep-open=no) but does
                                // not reliably emit a final render update, so the GL
                                // loop would keep the last decoded frame on screen
                                // (same class of bug as the hard-cut stop, 0.9.2).
                                // Paint the opaque black quad over it — identical end
                                // state to an operator stop.
                                super::fade::set_overlay_alpha(255);
                                let _ = status_tx
                                    .send(OutputStatus::Completed { voice_id: vid });
                            }
                            MPV_END_FILE_REASON_ERROR => {
                                stop_paired_audio(&audio_engine);
                                *go_sent_at.lock().unwrap() = None;
                                super::fade::set_overlay_alpha(255);
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

/// Reveal the output and begin playback of a video that was loaded paused.
///
/// Resumes the paired audio voice (decoded from the video's audio track),
/// unpauses mpv, and reveals the overlay — either immediately (hard cut) or via
/// a fade-from-black for `fade_in_ms > 0`.  Audio and video both start from
/// frame 0, so there is no A/V offset.
fn start_video_playback(
    lib: &MpvLib,
    ctx: &MpvCtx,
    audio_engine: &Arc<AudioEngine>,
    fade_in_ms: u32,
) {
    // Resume the paired audio voice (submitted paused at GO) so it starts in
    // lockstep with the first video frame.
    if let Some(av) = OUTPUT_CURRENT_AUDIO_VOICE.get() {
        if let Some(aid) = *av.lock().unwrap() {
            let _ = audio_engine.resume_voice(aid);
        }
    }

    // Unpause: frame 0 is decoded and the decoder is warm, so playback starts
    // smoothly with audio and video aligned.
    unsafe {
        (lib.mpv_set_property_string)(ctx.0, cs("pause").as_ptr(), cs("no").as_ptr());
    }

    if fade_in_ms > 0 {
        // Set FADE_STATE for fade-from-black; the render thread picks up
        // target_alpha=0 and animates automatically.
        if let Some(fs) = super::FADE_STATE.get() {
            if let Ok(mut s) = fs.lock() {
                s.start_alpha   = 255;
                s.current_alpha = 255;
                s.target_alpha  = 0;
                s.duration_ms   = fade_in_ms;
                s.start_time    = Instant::now();
                s.timer_active  = false;
                s.pending       = None;
            }
        }
        // Wake the render loop so it animates the fade-from-black.
        super::render::wake();
    } else {
        super::fade::set_overlay_alpha(0);
    }
}

/// Stop the current video's paired audio voice (on natural EOF or mpv error),
/// clearing it so it is not stopped twice.
fn stop_paired_audio(audio_engine: &Arc<AudioEngine>) {
    if let Some(av) = OUTPUT_CURRENT_AUDIO_VOICE.get() {
        let aid = av.lock().unwrap().take();
        if let Some(aid) = aid {
            use crate::engine::ring_command::FadeCurve;
            let _ = audio_engine.stop_voice(aid, 0, FadeCurve::Linear);
        }
    }
}
