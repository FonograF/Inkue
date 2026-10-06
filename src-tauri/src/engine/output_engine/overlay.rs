//! The overlay context of an output: the on-output timer (mpv OSD), the Text
//! Cue (`osd-overlay`) and projector test patterns.
//!
//! Every output owns one overlay mpv context next to its video slots.  It is
//! composited on top of the layer stack **only while it shows something** —
//! see [`Output::overlay_active`].

use std::ffi::{c_char, c_void, CString};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use anyhow::{anyhow, Result};

use super::output::{Output, OverlayMpv};
use super::types::{MpvCtx, TestPattern, VideoGeometry};
use super::{apply_scalar_geometry, cs, opt_str, prop_str};
use crate::engine::mpv_sys::{
    MpvLib, MpvNode, MpvNodeList, MpvNodeUnion, MPV_FORMAT_INT64, MPV_FORMAT_NODE_MAP,
    MPV_FORMAT_NONE, MPV_FORMAT_STRING,
};

/// mpv `osd-overlay` ID reserved for the Text Cue surface.  Distinct from the
/// timer (which uses `osd-msg1`, a separate OSD channel).
const TEXT_OSD_OVERLAY_ID: i64 = 47;

/// Create and initialise an overlay mpv context.
///
/// The caller owns the returned context until it hands it to an [`Output`].
pub(super) fn create_overlay_context(lib: &Arc<MpvLib>) -> Result<Arc<MpvCtx>> {
    let ctx = unsafe { (lib.mpv_create)() };
    if ctx.is_null() {
        return Err(anyhow!("mpv_create() returned null"));
    }

    unsafe {
        // mpv renders into our own native window via mpv_render_context_render()
        // instead of creating its own window.
        opt_str(lib, ctx, "vo", "libmpv");
        // This context is the layer compositor's OVERLAY (timer OSD, Text Cue,
        // test patterns): idle frames must be transparent so it never masks the
        // video slots below.  `background=none` is mpv ≥ 0.38; `alpha=yes`
        // covers older libmpv.
        opt_str(lib, ctx, "background", "none");
        opt_str(lib, ctx, "alpha", "yes");

        // The overlay context never decodes real media — it carries the timer
        // OSD, Text Cues and the lavfi-generated test patterns, all of which are
        // software sources.  Asking for hardware decoding here only creates a
        // d3d11/vaapi device that can fail to initialise and pollute the log
        // (issue #5); it buys nothing.
        opt_str(lib, ctx, "hwdec", "no");

        opt_str(lib, ctx, "osc", "no");
        opt_str(lib, ctx, "osd-level", "1");
        opt_str(lib, ctx, "input-default-bindings", "no");
        opt_str(lib, ctx, "input-vo-keyboard", "no");

        // Under vo=libmpv mpv has no window of its own on any OS — our host
        // window owns all mouse input (dragging, double-click fullscreen).
        opt_str(lib, ctx, "input-cursor", "no");

        opt_str(lib, ctx, "keep-open", "no");
        opt_str(lib, ctx, "idle", "yes");

        // mpv plays VIDEO ONLY.  Each video's audio track is decoded separately
        // as a normal AudioEngine voice (Output Patch routing, VU, fades).
        opt_str(lib, ctx, "ao", "null");
        opt_str(lib, ctx, "audio", "no");
        opt_str(lib, ctx, "video-sync", "desync");

        let v = cs("v");
        (lib.mpv_request_log_messages)(ctx, v.as_ptr());

        let ret = (lib.mpv_initialize)(ctx);
        if ret < 0 {
            (lib.mpv_terminate_destroy)(ctx);
            return Err(anyhow!("mpv_initialize() failed with code {ret}"));
        }

        // OSD style for the cue timer overlay (applied after init as properties).
        prop_str(lib, ctx, "osd-font-size", "120");
        prop_str(lib, ctx, "osd-color", "#FFFFFF");
        prop_str(lib, ctx, "osd-border-color", "#000000");
        prop_str(lib, ctx, "osd-border-size", "3");
        prop_str(lib, ctx, "osd-align-x", "center");
        prop_str(lib, ctx, "osd-align-y", "center");
        prop_str(lib, ctx, "osd-margin-x", "0");
        prop_str(lib, ctx, "osd-margin-y", "0");
    }
    Ok(Arc::new(MpvCtx(ctx)))
}

impl Output {
    /// `(lib, ctx)` of the overlay context, once the output has one.
    fn overlay_mpv(&self) -> Option<(&MpvLib, *mut c_void)> {
        self.overlay.get().map(|o: &OverlayMpv| (&*o.lib, o.ctx.0))
    }

    /// Run `mpv stop` on the overlay context.
    fn stop_overlay_playback(&self) {
        if let Some((lib, ctx)) = self.overlay_mpv() {
            unsafe {
                let stop = cs("stop");
                let args: [*const c_char; 2] = [stop.as_ptr(), std::ptr::null()];
                (lib.mpv_command)(ctx, args.as_ptr());
            }
        }
    }

    /// Load the transparent lavfi dummy into the overlay context (idempotent).
    ///
    /// mpv needs a decoded video surface to composite OSD/text at all — **idle**
    /// renders ignore the OSD *and* clear the target to opaque black on some
    /// libmpv builds (0.41-dev on Windows honours neither `background=none` nor
    /// OSD in idle; measured 2026-07-11).  With a fully transparent RGBA source
    /// loaded, mpv honours the source alpha and composites the OSD with correct
    /// per-pixel alpha, so timer/text float over the video layers below.
    ///
    /// Called whenever timer/text OSD content appears.  No-op while a test
    /// pattern is showing — the pattern is the overlay surface then.
    fn ensure_overlay_surface(&self) {
        if self.test_pattern_active.load(Ordering::Relaxed)
            || self.overlay_has_dummy.swap(true, Ordering::Relaxed)
        {
            return;
        }
        if let Some((lib, ctx)) = self.overlay_mpv() {
            unsafe {
                let cmd = cs("loadfile");
                // Tiny + fully transparent: `format=rgba` keeps the alpha plane,
                // 10 fps keeps the OSD recomposited without measurable cost.
                let url = cs("av://lavfi:color=c=black@0.0:s=64x64:r=10,format=rgba");
                let flags = cs("replace");
                let idx = cs("0");
                let opts = cs("audio=no,loop-file=inf");
                let args: [*const c_char; 6] = [
                    cmd.as_ptr(), url.as_ptr(), flags.as_ptr(),
                    idx.as_ptr(), opts.as_ptr(), std::ptr::null(),
                ];
                let ret = (lib.mpv_command)(ctx, args.as_ptr());
                if ret < 0 {
                    log::warn!("[output '{}'] overlay dummy loadfile failed: {ret}", self.name());
                }
            }
        }
        self.wake();
    }

    /// Unload the overlay dummy once neither the timer nor a Text Cue needs it.
    fn release_overlay_surface_if_idle(&self) {
        if self.overlay_active() || !self.overlay_has_dummy.swap(false, Ordering::Relaxed) {
            return;
        }
        self.stop_overlay_playback();
        self.wake();
    }

    // ── On-output timer (OSD) ────────────────────────────────────────────────

    /// Show `text` as the on-output timer; an empty string hides it.
    pub(super) fn set_timer_text(&self, text: &str) {
        // The OSD only renders over a decoded surface (see `ensure_overlay_surface`),
        // and the overlay is only composited while flagged active.
        if text.is_empty() {
            self.timer_osd_active.store(false, Ordering::Relaxed);
        } else {
            self.timer_osd_active.store(true, Ordering::Relaxed);
            self.ensure_overlay_surface();
        }
        if let Some((lib, ctx)) = self.overlay_mpv() {
            unsafe { prop_str(lib, ctx, "osd-msg1", text) };
        }
        if text.is_empty() {
            self.release_overlay_surface_if_idle();
            self.mark_overlay_dirty();
        }
    }

    /// Apply font, size, alignment and margin of the timer OSD.
    pub(super) fn set_timer_osd_style(
        &self,
        font: &str,
        font_size: u32,
        align: (&str, &str),
        margin: &str,
    ) {
        if let Some((lib, ctx)) = self.overlay_mpv() {
            unsafe {
                prop_str(lib, ctx, "osd-font", font);
                prop_str(lib, ctx, "osd-font-size", &font_size.to_string());
                prop_str(lib, ctx, "osd-align-x", align.0);
                prop_str(lib, ctx, "osd-align-y", align.1);
                prop_str(lib, ctx, "osd-margin-x", margin);
                prop_str(lib, ctx, "osd-margin-y", margin);
            }
        }
    }

    // ── Text Cue ─────────────────────────────────────────────────────────────

    /// Display an ASS-tagged text string on this output.
    ///
    /// Uses mpv's `osd-overlay` command (`format=ass-events`), the API-supported
    /// way to draw client-supplied ASS: it honours full override tags (`\an`,
    /// `\fn`, `\fs`, `\c` …), is independent of `osd-level`, persists across file
    /// loads, and composites over whatever the VO shows.
    pub(super) fn show_text(&self, ass_text: &str) {
        // Hint the render loop to keep compositing OSD-only changes.
        self.text_overlay_active.store(true, Ordering::Relaxed);
        // The ASS overlay only renders over a decoded surface.
        self.ensure_overlay_surface();
        if let Some((lib, ctx)) = self.overlay_mpv() {
            // osd-overlay persists across file loads, so it can be set right
            // away — the render loop composites it as soon as it renders.
            unsafe { osd_overlay_set(lib, ctx, ass_text) };
        }
        self.set_overlay_alpha(0);
    }

    /// Clear the text set via [`Output::show_text`].  Restores the opaque-black
    /// idle state when no video or image content is on this output.
    pub(super) fn clear_text(&self) {
        self.text_overlay_active.store(false, Ordering::Relaxed);
        if let Some((lib, ctx)) = self.overlay_mpv() {
            unsafe { osd_overlay_remove(lib, ctx) };
        }
        self.release_overlay_surface_if_idle();
        self.mark_overlay_dirty();
        // Blackout only when the whole stage is empty (no slot occupied).
        if !self.has_content() {
            self.set_overlay_alpha(255);
        }
    }

    // ── Test patterns ────────────────────────────────────────────────────────

    /// Show a calibration pattern, replacing whatever the overlay held (incl.
    /// the transparent OSD dummy).  Shown with **neutral cue geometry** — only
    /// the output's own transform applies, which is exactly what alignment and
    /// colorimetry need.
    pub(super) fn show_pattern(&self, pattern: &TestPattern, width: u32, height: u32) {
        let Some((lib, ctx)) = self.overlay_mpv() else { return };

        self.test_pattern_active.store(true, Ordering::Relaxed);
        self.overlay_has_dummy.store(false, Ordering::Relaxed);

        let url = pattern.mpv_url(width, height);
        apply_scalar_geometry(lib, ctx, &VideoGeometry::default());
        unsafe {
            prop_str(lib, ctx, "video-crop", "");
            // Patterns behave like images: play immediately, no paused-load
            // handshake, and no keep-open (a previous held video may have set it).
            prop_str(lib, ctx, "pause", "no");
            prop_str(lib, ctx, "keep-open", "no");

            let opts = if pattern.is_file() {
                // A custom image needs image-display-duration to hold on screen.
                cs("audio=no,image-display-duration=inf")
            } else {
                cs("audio=no")
            };
            let Ok(path_cstr) = CString::new(url.as_str()) else {
                log::warn!("[output] test pattern path contains NUL byte");
                return;
            };
            let cmd = cs("loadfile");
            let flags = cs("replace");
            let idx = cs("0");
            // loadfile signature: <url> <flags> <index> <options>.
            let args: [*const c_char; 6] = [
                cmd.as_ptr(), path_cstr.as_ptr(), flags.as_ptr(),
                idx.as_ptr(), opts.as_ptr(), std::ptr::null(),
            ];
            let ret = (lib.mpv_command)(ctx, args.as_ptr());
            if ret < 0 {
                log::warn!("[output '{}'] test pattern loadfile failed: {ret} ({url})", self.name());
            }
        }
        self.set_overlay_alpha(0);
    }

    /// Clear the test pattern: stop playback and return to opaque black.
    pub(super) fn clear_pattern(&self) {
        self.stop_overlay_playback();
        self.test_pattern_active.store(false, Ordering::Relaxed);
        // Timer/Text OSD may still be live — give them their surface back.
        if self.overlay_active() {
            self.ensure_overlay_surface();
        }
        self.mark_overlay_dirty();
        self.set_overlay_alpha(255);
    }

    /// Panic: cut the overlay and close the blackout curtain.
    pub(super) fn panic_overlay(&self) {
        self.stop_overlay_playback();
        self.set_overlay_alpha(255);
    }
}

// ---------------------------------------------------------------------------
// mpv `osd-overlay` helpers
// ---------------------------------------------------------------------------

/// Show the Text Cue ASS string via mpv's `osd-overlay` command.
///
/// `res_y=720` is the ASS script reference height, so `\fs` sizes stay
/// proportional to the output regardless of its actual resolution.
unsafe fn osd_overlay_set(lib: &MpvLib, ctx: *mut c_void, ass_text: &str) {
    let Ok(data_v) = CString::new(ass_text) else {
        log::warn!("[output] osd-overlay text contains an interior NUL — ignored");
        return;
    };
    let (name, id, format, data_k, res_y) =
        (cs("name"), cs("id"), cs("format"), cs("data"), cs("res_y"));
    let (name_v, format_v) = (cs("osd-overlay"), cs("ass-events"));

    let mut keys: [*const c_char; 5] =
        [name.as_ptr(), id.as_ptr(), format.as_ptr(), data_k.as_ptr(), res_y.as_ptr()];
    let mut values: [MpvNode; 5] = [
        MpvNode { u: MpvNodeUnion { string: name_v.as_ptr() },    format: MPV_FORMAT_STRING },
        MpvNode { u: MpvNodeUnion { int64: TEXT_OSD_OVERLAY_ID }, format: MPV_FORMAT_INT64  },
        MpvNode { u: MpvNodeUnion { string: format_v.as_ptr() },  format: MPV_FORMAT_STRING },
        MpvNode { u: MpvNodeUnion { string: data_v.as_ptr() },    format: MPV_FORMAT_STRING },
        MpvNode { u: MpvNodeUnion { int64: 720 },                 format: MPV_FORMAT_INT64  },
    ];
    command_node_map(lib, ctx, &mut keys, &mut values);
}

/// Remove the Text Cue `osd-overlay` (`format=none`).
unsafe fn osd_overlay_remove(lib: &MpvLib, ctx: *mut c_void) {
    let (name, id, format) = (cs("name"), cs("id"), cs("format"));
    let (name_v, format_v) = (cs("osd-overlay"), cs("none"));

    let mut keys: [*const c_char; 3] = [name.as_ptr(), id.as_ptr(), format.as_ptr()];
    let mut values: [MpvNode; 3] = [
        MpvNode { u: MpvNodeUnion { string: name_v.as_ptr() },    format: MPV_FORMAT_STRING },
        MpvNode { u: MpvNodeUnion { int64: TEXT_OSD_OVERLAY_ID }, format: MPV_FORMAT_INT64  },
        MpvNode { u: MpvNodeUnion { string: format_v.as_ptr() },  format: MPV_FORMAT_STRING },
    ];
    command_node_map(lib, ctx, &mut keys, &mut values);
}

/// Run `mpv_command_node` with a `MPV_FORMAT_NODE_MAP` built from parallel
/// `keys`/`values` slices, freeing any memory mpv allocates for the result.
unsafe fn command_node_map(
    lib: &MpvLib,
    ctx: *mut c_void,
    keys: &mut [*const c_char],
    values: &mut [MpvNode],
) {
    debug_assert_eq!(keys.len(), values.len());
    let mut list = MpvNodeList {
        num: keys.len() as i32,
        values: values.as_mut_ptr(),
        keys: keys.as_mut_ptr(),
    };
    let arg = MpvNode { u: MpvNodeUnion { list: &mut list }, format: MPV_FORMAT_NODE_MAP };
    let mut result = MpvNode { u: MpvNodeUnion { int64: 0 }, format: MPV_FORMAT_NONE };
    let ret = (lib.mpv_command_node)(ctx, &arg, &mut result);
    (lib.mpv_free_node_contents)(&mut result);
    if ret < 0 {
        log::warn!("[output] mpv_command_node(osd-overlay) failed: {ret}");
    }
}
