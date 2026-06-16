//! Fade overlay helpers: alpha animation, load execution, stop execution.
//!
//! The fade overlay is implemented differently per platform:
//!   Windows  — a child `WS_EX_LAYERED` Win32 window with `SetLayeredWindowAttributes`
//!   Mac/Linux — an mpv `osd-overlay` ASS drawing that covers the entire video surface

use std::ffi::{c_void, CString};
use std::time::Instant;

use crate::engine::mpv_sys::MpvLib;

use super::types::{FadePending, FadePendingParams, PendingVideoStart};
use super::{
    cs, FADE_STATE, OUTPUT_MPV_CTX, OUTPUT_MPV_LIB,
    OUTPUT_PENDING_VIDEO_START,
};
#[cfg(target_os = "windows")]
use super::{FADE_OVERLAY_HWND, FADE_TIMER_ID};

// ---------------------------------------------------------------------------
// Visual overlay — platform implementations
// ---------------------------------------------------------------------------

/// Apply `alpha` to the visual overlay (Win32 layered window or mpv OSD).
/// Does NOT update `FADE_STATE.current_alpha` — caller owns that.
pub(super) fn apply_overlay_alpha(alpha: u8) {
    #[cfg(target_os = "windows")]
    if let Some(&overlay) = FADE_OVERLAY_HWND.get() {
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::SetLayeredWindowAttributes;
            const LWA_ALPHA: u32 = 0x2;
            SetLayeredWindowAttributes(overlay, 0, alpha, LWA_ALPHA);
        }
    }

    #[cfg(not(target_os = "windows"))]
    if let (Some(lib), Some(ctx)) = (OUTPUT_MPV_LIB.get(), OUTPUT_MPV_CTX.get()) {
        if alpha == 0 {
            // Remove the OSD overlay entirely when fully transparent.
            unsafe {
                let cmd  = cs("osd-overlay");
                let id   = cs("1");
                let none = cs("none");
                let empty = cs("");
                let args: [*const std::ffi::c_char; 5] = [
                    cmd.as_ptr(), id.as_ptr(), none.as_ptr(), empty.as_ptr(), std::ptr::null(),
                ];
                (lib.mpv_command)(ctx.0, args.as_ptr());
            }
        } else {
            // ASS drawing: full-screen black rectangle with variable primary alpha.
            // \1a&H<AA>& — primary alpha (00=opaque, FF=transparent in ASS).
            // res_x / res_y define the coordinate space; mpv scales to the window.
            let ass_alpha = format!("{:02X}", 255u8.saturating_sub(alpha));
            let ass_text = cs(&format!(
                "{{\\an7\\pos(0,0)\\c&H000000&\\1a&H{}&\\bord0\\shad0\\p1}}m 0 0 l 1920 0 l 1920 1080 l 0 1080{{\\p0}}",
                ass_alpha
            ));
            unsafe {
                let cmd     = cs("osd-overlay");
                let id      = cs("1");
                let fmt     = cs("ass-events");
                let res_x   = cs("1920");
                let res_y   = cs("1080");
                let args: [*const std::ffi::c_char; 7] = [
                    cmd.as_ptr(), id.as_ptr(), fmt.as_ptr(), ass_text.as_ptr(),
                    res_x.as_ptr(), res_y.as_ptr(), std::ptr::null(),
                ];
                (lib.mpv_command)(ctx.0, args.as_ptr());
            }
        }
    }
}

/// Set the fade overlay alpha (0 = transparent, 255 = opaque black).
///
/// Applies the visual change AND updates `FADE_STATE.current_alpha` so that
/// stop/start transitions can read the correct starting alpha.
pub(super) fn set_overlay_alpha(alpha: u8) {
    apply_overlay_alpha(alpha);
    if let Some(fs) = FADE_STATE.get() {
        if let Ok(mut state) = fs.lock() {
            state.current_alpha = alpha;
        }
    }
}

// ---------------------------------------------------------------------------
// Shared pending-action executor
// ---------------------------------------------------------------------------

/// Execute whatever action was pending after the fade completed.
///
/// `on_image_fade_in` is called when an image needs a fade-in timer armed:
///   Windows  — sets a Win32 timer (16 ms)
///   non-Windows — no-op (the cross-platform fade thread picks up FADE_STATE)
fn do_execute_fade_pending(on_image_fade_in: impl FnOnce()) {
    let pending = FADE_STATE
        .get()
        .and_then(|fs| fs.lock().ok().and_then(|mut s| s.pending.take()));

    match pending {
        Some(FadePending::Load(params)) => {
            if let (Some(lib), Some(ctx)) = (OUTPUT_MPV_LIB.get(), OUTPUT_MPV_CTX.get()) {
                execute_load_params(&params, lib, ctx.0);
            }
            if params.is_image {
                // Images are not gated on PLAYBACK_RESTART — reveal them now.
                if params.fade_in_ms > 0 {
                    if let Some(fs) = FADE_STATE.get() {
                        let mut state = fs.lock().unwrap();
                        state.start_alpha = 255;
                        state.current_alpha = 255;
                        state.target_alpha = 0;
                        state.duration_ms = params.fade_in_ms;
                        state.start_time = Instant::now();
                        state.timer_active = true;
                        state.pending = None;
                    }
                    on_image_fade_in();
                } else {
                    set_overlay_alpha(0);
                }
            }
            // Video: leave the overlay black.  The first PLAYBACK_RESTART reveals,
            // unpauses and runs the fade-in (params.fade_in_ms) once frame 0 is up.
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

// ---------------------------------------------------------------------------
// Platform-specific wrappers
// ---------------------------------------------------------------------------

/// Execute the pending fade action — called from the Win32 `WM_TIMER` handler.
#[cfg(target_os = "windows")]
pub(super) fn execute_fade_pending(hwnd: isize) {
    do_execute_fade_pending(|| {
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::SetTimer;
            SetTimer(hwnd, FADE_TIMER_ID, 16, None);
        }
    });
}

/// Execute the pending fade action — called from the cross-platform fade thread.
/// The fade thread is already running and will pick up the new FADE_STATE automatically.
#[cfg(not(target_os = "windows"))]
pub(super) fn execute_fade_pending_nw() {
    do_execute_fade_pending(|| {
        // No timer setup needed — cross_platform_fade_loop() detects FADE_STATE changes.
    });
}

// ---------------------------------------------------------------------------
// Cross-platform fade animation thread (non-Windows)
// ---------------------------------------------------------------------------

/// Background thread that drives fade animations on Mac and Linux.
///
/// On Windows the Win32 `WM_TIMER` mechanism handles this instead.
/// Polls `FADE_STATE` every ~16 ms and interpolates alpha toward the target.
#[cfg(not(target_os = "windows"))]
pub(super) fn run_cross_platform_fade_loop() {
    loop {
        std::thread::sleep(std::time::Duration::from_millis(16));

        let step = FADE_STATE.get().and_then(|fs| {
            fs.lock().ok().and_then(|mut state| {
                if state.target_alpha == state.current_alpha {
                    return None;
                }

                let elapsed_ms = state.start_time.elapsed().as_millis() as u32;
                let done = state.duration_ms == 0 || elapsed_ms >= state.duration_ms;

                let alpha = if done {
                    state.target_alpha
                } else {
                    let t = elapsed_ms as f32 / state.duration_ms as f32;
                    let start = state.start_alpha as f32;
                    let end   = state.target_alpha as f32;
                    (start + (end - start) * t).round() as u8
                };

                state.current_alpha = alpha;
                Some((alpha, done))
            })
        });

        if let Some((alpha, done)) = step {
            apply_overlay_alpha(alpha);
            if done {
                execute_fade_pending_nw();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// mpv loadfile executor (cross-platform)
// ---------------------------------------------------------------------------

/// Send an mpv `loadfile` command for the given content parameters.
/// Called either immediately (no fade) or from `execute_*_fade_pending` after fade-out.
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
            // An image must display immediately — make sure mpv is not left
            // paused from a prior video load, and clear any armed video reveal.
            (lib.mpv_set_property_string)(ctx, cs("pause").as_ptr(), cs("no").as_ptr());
            if let Some(m) = OUTPUT_PENDING_VIDEO_START.get() {
                if let Ok(mut p) = m.lock() {
                    *p = None;
                }
            }

            let duration_val = params
                .display_duration_ms
                .map(|ms| format!("{:.3}", ms as f64 / 1000.0))
                .unwrap_or_else(|| "inf".to_string());
            let opts_str = format!("audio=no,image-display-duration={duration_val}");
            let file_opts = cs(&opts_str);
            let cmd   = cs("loadfile");
            let flags = cs("replace");
            let args: [*const std::ffi::c_char; 5] = [
                cmd.as_ptr(), path_cstr.as_ptr(), flags.as_ptr(),
                file_opts.as_ptr(), std::ptr::null(),
            ];
            let ret = (lib.mpv_command)(ctx, args.as_ptr());
            if ret < 0 {
                log::warn!("[output] mpv loadfile (image) failed: {ret}");
            }
        } else {
            // Video is muted — `audio=no` keeps mpv's display clock free of any
            // audio-sync logic.  The audio track is decoded separately and played
            // as an AudioEngine voice, resumed in lockstep at the first frame.
            let mut opts: Vec<String> = vec!["audio=no".to_string()];
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
            let args: [*const std::ffi::c_char; 5] = [
                cmd_cstr.as_ptr(), path_cstr.as_ptr(), replace_cstr.as_ptr(),
                opts_cstr.as_ptr(), std::ptr::null(),
            ];
            (lib.mpv_set_property_string)(ctx, cs("hwdec").as_ptr(), cs("auto").as_ptr());
            (lib.mpv_set_property_string)(ctx, cs("profile").as_ptr(), cs("fast").as_ptr());
            (lib.mpv_set_property_string)(
                ctx, cs("video-sync").as_ptr(), cs("desync").as_ptr(),
            );

            // Load the video *paused*.  While paused mpv finishes opening the
            // file, initialises the decoder and buffers the first frames — all
            // behind the black overlay.  The first PLAYBACK_RESTART then reveals,
            // unpauses and resumes the paired audio voice, so playback starts from
            // frame 0 with a warm decoder and zero A/V offset.
            (lib.mpv_set_property_string)(ctx, cs("pause").as_ptr(), cs("yes").as_ptr());
            if let Some(m) = OUTPUT_PENDING_VIDEO_START.get() {
                if let Ok(mut p) = m.lock() {
                    *p = Some(PendingVideoStart { fade_in_ms: params.fade_in_ms });
                }
            }

            let ret = (lib.mpv_command)(ctx, args.as_ptr());
            if ret < 0 {
                log::warn!("[output] mpv loadfile (video) failed: {ret}");
            }
            log::info!("[output] loadfile (paused) sent: {} opts=[{opts_str}]", params.path);
        }
    }
}
