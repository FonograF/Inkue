//! Fade overlay helpers (master blackout quad).
//!
//! `FADE_STATE` is the single source of truth for the current overlay alpha.
//! `tick_fade()` is called by the render thread each frame to advance the
//! animation.  `execute_pending()` fires when a fade completes.  No separate
//! fade thread is needed; the render loop drives animation timing.

use super::{cs, FADE_STATE, OUTPUT_CURRENT_VOICE, OUTPUT_MPV_CTX, OUTPUT_MPV_LIB};
use super::types::FadePending;

// ---------------------------------------------------------------------------
// Alpha state
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

    // The render loop self-paces at 16 ms only while animating; wake it so
    // externally-driven alpha changes (Fade Cue at 30 fps) redraw the quad at once.
    super::render::wake();
}

// ---------------------------------------------------------------------------
// Per-frame tick + pending action executor
// ---------------------------------------------------------------------------

/// Advance the fade animation by one render-thread frame.
///
/// Returns `(current_alpha, did_complete)`.  `did_complete` is `true` exactly
/// once — on the frame where `current_alpha` first reaches `target_alpha`.
/// The caller should invoke `execute_pending()` when `did_complete` is `true`.
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
