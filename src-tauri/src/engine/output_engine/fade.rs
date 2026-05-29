//! Fade overlay helpers: alpha animation, load execution, stop execution.

use std::ffi::{c_void, CString};
use std::time::Instant;

use crate::cue::types::db_to_linear;
use crate::engine::mpv_sys::MpvLib;

use super::types::{FadePending, FadePendingParams};
use super::{cs, FADE_OVERLAY_HWND, FADE_STATE, FADE_TIMER_ID, OUTPUT_MPV_CTX, OUTPUT_MPV_LIB};

/// Set the fade overlay alpha (0 = transparent, 255 = opaque black).
///
/// Also updates `FADE_STATE.current_alpha` so stop/start transitions can
/// read the correct starting alpha for their animations.
pub(super) fn set_overlay_alpha(alpha: u8) {
    if let Some(&overlay) = FADE_OVERLAY_HWND.get() {
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::SetLayeredWindowAttributes;
            const LWA_ALPHA: u32 = 0x2;
            SetLayeredWindowAttributes(overlay, 0, alpha, LWA_ALPHA);
        }
    }
    if let Some(fs) = FADE_STATE.get() {
        if let Ok(mut state) = fs.lock() {
            state.current_alpha = alpha;
        }
    }
}

/// Execute whatever action was pending when the fade timer reached its target.
/// Called from the Win32 timer handler after the fade completes.
pub(super) fn execute_fade_pending(hwnd: isize) {
    let pending = FADE_STATE
        .get()
        .and_then(|fs| fs.lock().ok().and_then(|mut s| s.pending.take()));

    match pending {
        Some(FadePending::Load(params)) => {
            if let (Some(lib), Some(ctx)) = (OUTPUT_MPV_LIB.get(), OUTPUT_MPV_CTX.get()) {
                execute_load_params(&params, lib, ctx.0);
            }
            let fade_in_ms = params.fade_in_ms;
            if fade_in_ms > 0 {
                if let Some(fs) = FADE_STATE.get() {
                    let mut state = fs.lock().unwrap();
                    state.start_alpha = 255;
                    state.current_alpha = 255;
                    state.target_alpha = 0;
                    state.duration_ms = fade_in_ms;
                    state.start_time = Instant::now();
                    state.timer_active = true;
                    state.pending = None;
                }
                unsafe {
                    use windows_sys::Win32::UI::WindowsAndMessaging::SetTimer;
                    SetTimer(hwnd, FADE_TIMER_ID, 16, None);
                }
            } else {
                set_overlay_alpha(0);
            }
        }
        Some(FadePending::Stop) => {
            // Guard: if new content was loaded while the stop fade was running, don't
            // send mpv stop — that would kill the new content.  Just clear the overlay.
            let has_new_content = super::OUTPUT_CURRENT_VOICE
                .get()
                .and_then(|cv| cv.lock().ok())
                .map(|cv| cv.is_some())
                .unwrap_or(false);
            if has_new_content {
                set_overlay_alpha(0);
                return;
            }
            if let (Some(lib), Some(ctx)) = (OUTPUT_MPV_LIB.get(), OUTPUT_MPV_CTX.get()) {
                unsafe {
                    let stop = cs("stop");
                    let args: [*const std::ffi::c_char; 2] =
                        [stop.as_ptr(), std::ptr::null()];
                    (lib.mpv_command)(ctx.0, args.as_ptr());
                }
            }
            // Overlay stays at alpha=255 (black) — window visible, content stopped.
        }
        None => {
            // Fade-in completed — nothing more to do.
        }
    }
}

/// Send an mpv `loadfile` command for the given content parameters.
/// Called either immediately (no fade) or from `execute_fade_pending` after fade-out.
pub(super) fn execute_load_params(params: &FadePendingParams, lib: &MpvLib, ctx: *mut c_void) {
    unsafe {
        let path_cstr = match CString::new(params.path.as_str()) {
            Ok(c) => c,
            Err(_) => {
                log::warn!("[output] execute_load_params: path contains NUL byte");
                return;
            }
        };

        if params.is_image {
            let file_opts = cs("audio=no,image-display-duration=inf");
            let cmd   = cs("loadfile");
            let flags = cs("replace");
            let idx   = cs("0");
            let args: [*const std::ffi::c_char; 6] = [
                cmd.as_ptr(), path_cstr.as_ptr(), flags.as_ptr(),
                idx.as_ptr(), file_opts.as_ptr(), std::ptr::null(),
            ];
            let ret = (lib.mpv_command)(ctx, args.as_ptr());
            if ret < 0 {
                log::warn!("[output] mpv loadfile (image) failed: {ret}");
            }
        } else {
            let vol_pct = (100.0 * db_to_linear(params.volume_db)).clamp(0.0, 1000.0);
            let vol_str = cs(&format!("{vol_pct:.2}"));
            (lib.mpv_set_property_string)(ctx, cs("volume").as_ptr(), vol_str.as_ptr());

            let mut opts: Vec<String> = Vec::new();
            if let Some(start) = params.start_ms {
                opts.push(format!("start={:.3}", start as f64 / 1000.0));
            }
            if let Some(end) = params.end_ms {
                opts.push(format!("end={:.3}", end as f64 / 1000.0));
            }
            let loop_val = if params.loop_count == u32::MAX {
                "inf".to_string()
            } else if params.loop_count == 0 {
                "no".to_string()
            } else {
                params.loop_count.to_string()
            };
            opts.push(format!("loop-file={loop_val}"));

            let opts_str     = opts.join(",");
            let opts_cstr    = cs(&opts_str);
            let cmd_cstr     = cs("loadfile");
            let replace_cstr = cs("replace");
            let index_cstr   = cs("0");
            let args: [*const std::ffi::c_char; 6] = [
                cmd_cstr.as_ptr(), path_cstr.as_ptr(), replace_cstr.as_ptr(),
                index_cstr.as_ptr(), opts_cstr.as_ptr(), std::ptr::null(),
            ];
            (lib.mpv_set_property_string)(ctx, cs("hwdec").as_ptr(), cs("auto").as_ptr());
            (lib.mpv_set_property_string)(ctx, cs("profile").as_ptr(), cs("fast").as_ptr());
            (lib.mpv_set_property_string)(
                ctx, cs("video-sync").as_ptr(), cs("desync").as_ptr(),
            );
            (lib.mpv_set_property_string)(
                ctx, cs("audio-buffer").as_ptr(), cs("0.06").as_ptr(),
            );
            (lib.mpv_set_property_string)(
                ctx, cs("initial-audio-sync").as_ptr(), cs("no").as_ptr(),
            );

            let ret = (lib.mpv_command)(ctx, args.as_ptr());
            if ret < 0 {
                log::warn!("[output] mpv loadfile (video) failed: {ret}");
            }
            log::info!("[output] loadfile sent: {} opts=[{opts_str}]", params.path);
        }
    }
}
