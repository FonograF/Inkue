//! Fade overlay helpers — shared between the GL unified path and the legacy Win32 path.
//!
//! **GL unified path (default)**
//!   `FADE_STATE` is the single source of truth for the current overlay alpha.
//!   `tick_fade()` is called by the render thread each frame to advance the
//!   animation.  `execute_pending()` fires when a fade completes.  No separate
//!   fade thread is needed; the render loop drives animation timing.
//!
//! **Legacy Win32 path (`legacy-win32-output` feature)**
//!   `apply_overlay_alpha` drives `SetLayeredWindowAttributes`; a Win32 timer
//!   in `output_wnd_proc` advances the animation via `execute_fade_pending`.

use super::{cs, FADE_STATE, OUTPUT_CURRENT_VOICE, OUTPUT_MPV_CTX, OUTPUT_MPV_LIB};
use super::types::{FadePending, FadePendingParams, PendingVideoStart};
use std::ffi::c_void;
use crate::engine::mpv_sys::MpvLib;

// Win32 fade overlay imports.
#[cfg(output_win32)]
use super::FADE_OVERLAY_HWND;

// ---------------------------------------------------------------------------
// Unified: alpha state
// ---------------------------------------------------------------------------

/// Hard-cut the overlay to `alpha` with no animation.
///
/// Sets `current_alpha`, `target_alpha`, and resets `duration_ms` so that
/// `tick_fade()` holds at this value without transitioning.  Calling only
/// `s.current_alpha = alpha` while leaving a stale `target_alpha` would cause
/// `tick_fade()` to immediately snap back to the old target.
pub(super) fn set_overlay_alpha(alpha: u8) {
    if let Some(fs) = FADE_STATE.get() {
        if let Ok(mut s) = fs.lock() {
            s.current_alpha = alpha;
            s.target_alpha  = alpha;
            s.start_alpha   = alpha;
            s.duration_ms   = 0;
            s.start_time    = std::time::Instant::now();
        }
    }

    // Win32: also push to the layered overlay window immediately.
    #[cfg(output_win32)]
    apply_overlay_alpha(alpha);

    // GL path: the render loop self-paces at 16 ms only while animating; wake it so
    // externally-driven alpha changes (Fade Cue at 30 fps) redraw the quad at once.
    #[cfg(output_winit)]
    super::render::wake();
}

// ---------------------------------------------------------------------------
// GL unified path: per-frame tick + pending action executor
// ---------------------------------------------------------------------------

/// Advance the fade animation by one render-thread frame.
///
/// Returns `(current_alpha, did_complete)`.  `did_complete` is `true` exactly
/// once — on the frame where `current_alpha` first reaches `target_alpha`.
/// The caller should invoke `execute_pending()` when `did_complete` is `true`.
#[cfg(output_winit)]
pub(super) fn tick_fade() -> (u8, bool) {
    let Some(fs) = FADE_STATE.get() else {
        return (0, false);
    };
    let mut state = match fs.lock() {
        Ok(s) => s,
        Err(_) => return (0, false),
    };

    if state.current_alpha == state.target_alpha {
        return (state.current_alpha, false);
    }

    let elapsed = state.start_time.elapsed().as_millis() as u32;
    let t = if state.duration_ms == 0 {
        1.0_f32
    } else {
        (elapsed as f32 / state.duration_ms as f32).min(1.0)
    };
    let start = state.start_alpha as f32;
    let end   = state.target_alpha as f32;
    let alpha = (start + (end - start) * t).round().clamp(0.0, 255.0) as u8;
    state.current_alpha = alpha;

    let done = t >= 1.0;
    if done {
        state.current_alpha = state.target_alpha;
    }
    (alpha, done)
}

/// Execute the action that was pending behind a completed fade.
///
/// Called by the render thread immediately after `tick_fade()` returns
/// `did_complete = true`.
#[cfg(output_winit)]
pub(super) fn execute_pending() {
    let pending = FADE_STATE
        .get()
        .and_then(|fs| fs.lock().ok().and_then(|mut s| s.pending.take()));

    match pending {
        Some(FadePending::Stop) => {
            // Guard: new content may have been loaded while the stop fade ran.
            // In that case, don't issue a `stop` command — just clear the overlay.
            let has_new_content = OUTPUT_CURRENT_VOICE
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
            // Overlay stays at alpha=255 (black); mpv has no content to show.
        }
        None => {
            // Fade-in completed — nothing more to do.
        }
    }
}

// ---------------------------------------------------------------------------
// Legacy Win32 path — Win32 layered-window implementation
// ---------------------------------------------------------------------------

/// Apply `alpha` directly to the Win32 layered fade overlay window.
///
/// Does NOT update `FADE_STATE.current_alpha`; use `set_overlay_alpha` for that.
#[cfg(output_win32)]
pub(super) fn apply_overlay_alpha(alpha: u8) {
    if let Some(&overlay) = FADE_OVERLAY_HWND.get() {
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::SetLayeredWindowAttributes;
            const LWA_ALPHA: u32 = 0x2;
            SetLayeredWindowAttributes(overlay, 0, alpha, LWA_ALPHA);
        }
    }
}

/// Execute the pending fade action — called from the Win32 `WM_TIMER` handler.
#[cfg(output_win32)]
pub(super) fn execute_fade_pending(_hwnd: isize) {
    let pending = FADE_STATE
        .get()
        .and_then(|fs| fs.lock().ok().and_then(|mut s| s.pending.take()));

    match pending {
        Some(FadePending::Stop) => {
            let has_new_content = OUTPUT_CURRENT_VOICE
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
        }
        None => {}
    }
}

// ---------------------------------------------------------------------------
// mpv loadfile executor (shared by both paths)
// ---------------------------------------------------------------------------

/// Send an mpv `loadfile` command for the given content parameters.
pub(super) fn execute_load_params(params: &FadePendingParams, lib: &MpvLib, ctx: *mut c_void) {
    use std::ffi::CString;

    unsafe {
        let path_cstr = match CString::new(params.path.as_str()) {
            Ok(c)  => c,
            Err(_) => {
                log::warn!("[output] execute_load_params: path contains NUL byte");
                return;
            }
        };

        if params.is_image {
            (lib.mpv_set_property_string)(ctx, cs("pause").as_ptr(), cs("no").as_ptr());
            if let Some(m) = super::OUTPUT_PENDING_VIDEO_START.get() {
                if let Ok(mut p) = m.lock() { *p = None; }
            }

            let duration_val = params
                .display_duration_ms
                .map(|ms| format!("{:.3}", ms as f64 / 1000.0))
                .unwrap_or_else(|| "inf".to_string());
            let opts_str  = format!("audio=no,image-display-duration={duration_val}");
            let file_opts = cs(&opts_str);
            let cmd       = cs("loadfile");
            let flags     = cs("replace");
            let idx       = cs("0");
            // mpv loadfile signature is `loadfile <url> <flags> <index> <options>` —
            // <index> must be present (ignored for "replace") or mpv tries to parse
            // the options string itself as the index integer and fails. Required on
            // both Windows (mpv ≥ 0.38) and Linux (tested against mpv 0.41.0).
            let args: [*const std::ffi::c_char; 6] = [
                cmd.as_ptr(), path_cstr.as_ptr(), flags.as_ptr(),
                idx.as_ptr(), file_opts.as_ptr(), std::ptr::null(),
            ];
            let ret = (lib.mpv_command)(ctx, args.as_ptr());
            if ret < 0 { log::warn!("[output] mpv loadfile (image) failed: {ret}"); }
        } else {
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
            (lib.mpv_set_property_string)(ctx, cs("hwdec").as_ptr(), cs("auto-copy").as_ptr());
            (lib.mpv_set_property_string)(ctx, cs("video-sync").as_ptr(), cs("desync").as_ptr());

            // Load paused: frame 0 decoded → PLAYBACK_RESTART → reveal + unpause.
            (lib.mpv_set_property_string)(ctx, cs("pause").as_ptr(), cs("yes").as_ptr());
            if let Some(m) = super::OUTPUT_PENDING_VIDEO_START.get() {
                if let Ok(mut p) = m.lock() {
                    *p = Some(PendingVideoStart { fade_in_ms: params.fade_in_ms });
                }
            }

            let index_cstr = cs("0");
            // See loadfile signature note in the image branch above.
            let args: [*const std::ffi::c_char; 6] = [
                cmd_cstr.as_ptr(), path_cstr.as_ptr(), replace_cstr.as_ptr(),
                index_cstr.as_ptr(), opts_cstr.as_ptr(), std::ptr::null(),
            ];
            let ret = (lib.mpv_command)(ctx, args.as_ptr());
            if ret < 0 { log::warn!("[output] mpv loadfile (video) failed: {ret}"); }
            log::info!("[output] loadfile (paused) sent: {} opts=[{opts_str}]", params.path);
        }
    }
}
