//! Unified OpenGL Render API output path — one render thread per output.
//!
//! Drives mpv with `vo=libmpv` and renders each frame into the default
//! framebuffer of an OS window via `glutin` (OpenGL Core) + `mpv_render_context`.
//! A fullscreen black quad handles fade-to-black.  The render loop and the GL
//! fade are identical on every OS and for every output — only native window
//! creation differs (`window.rs`: winit on Windows/Linux, AppKit/objc2 on macOS).
//!
//! ## Thread model (per output)
//!
//! | Thread                    | Role |
//! |---------------------------|------|
//! | `inkue-output-window`    | (Windows/Linux only, **one for all outputs**) winit EventLoop + window events |
//! | `inkue-output-render`    | glutin context + `mpv_render_context` + render loop of one output |
//! | `inkue-output-mpv-events`| overlay context log / diagnostics |

use std::ffi::{CStr, CString, c_void};
use std::num::NonZeroU32;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Result};
use glow::HasContext;
use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextApi, ContextAttributesBuilder, NotCurrentGlContext, Version};
use glutin::display::{Display, DisplayApiPreference, GlDisplay};
use glutin::surface::{GlSurface, SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

use crate::engine::mpv_sys::{
    MpvLib, MpvOpenglFbo, MpvOpenglInitParams, MpvRenderParam,
    MPV_RENDER_PARAM_API_TYPE, MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME,
    MPV_RENDER_PARAM_FLIP_Y, MPV_RENDER_PARAM_OPENGL_FBO,
    MPV_RENDER_PARAM_OPENGL_INIT_PARAMS, MPV_RENDER_UPDATE_FRAME,
};
use super::crossfade::{self, MixLink};
use super::output::Output;
use super::slot;
use super::types::{MpvCtx, VoiceId};
use super::window::{create_window, SendableHandles};

/// One visual layer of an output, as the compositor draws it this frame.
struct LayerDraw {
    slot_index: usize,
    voice: VoiceId,
    layer_key: u64,
    opacity: f32,
    blend_mode: i32,
    has_new_frame: bool,
    render_ctx: *mut c_void,
}

/// The GL programs of the layer compositor.
struct Compositor {
    composite: (glow::Program, glow::VertexArray),
    mix: (glow::Program, glow::VertexArray),
    blit: (glow::Program, glow::VertexArray),
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Create the native window of `output` and spawn its render thread.
///
/// Blocks until `mpv_render_context_create()` succeeds so that no `loadfile`
/// can reach mpv before the render context is live.
pub(super) fn init(
    output: &Arc<Output>,
    app_handle: &tauri::AppHandle,
    lib: Arc<MpvLib>,
    mpv_ctx: Arc<MpvCtx>,
) -> Result<()> {
    let handles = create_window(output, app_handle)?;
    output.set_surface_size(handles.width, handles.height);

    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();

    let thread = spawn_render_thread(Arc::clone(output), handles, lib, mpv_ctx, ready_tx)?;
    output.adopt_thread(thread);

    // On macOS, Tauri's NSApplication event loop hasn't started yet when setup()
    // runs. If glutin/CGL needs the run loop during context creation, blocking
    // here deadlocks: setup() waits for the render thread, the render thread
    // waits for the run loop, the run loop waits for setup() to return.
    // Solution: let the render thread initialise after the event loop starts and
    // watch for errors on a background watcher thread.
    #[cfg(target_os = "macos")]
    std::thread::Builder::new()
        .name("inkue-render-watcher".into())
        .spawn(move || match ready_rx.recv() {
            Ok(Ok(())) => log::info!("[render] macOS GL context ready"),
            Ok(Err(e)) => log::error!("[render] macOS GL init failed: {e}"),
            Err(_) => log::error!("[render] macOS render thread closed before ready"),
        })
        .ok();

    #[cfg(not(target_os = "macos"))]
    ready_rx
        .recv()
        .map_err(|_| anyhow!("render thread exited before signalling ready"))??;

    Ok(())
}

// ---------------------------------------------------------------------------
// Spawn render thread
// ---------------------------------------------------------------------------

fn spawn_render_thread(
    output:   Arc<Output>,
    handles:  SendableHandles,
    lib:      Arc<MpvLib>,
    mpv_ctx:  Arc<MpvCtx>,
    ready_tx: std::sync::mpsc::Sender<Result<()>>,
) -> Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("inkue-output-render".into())
        .spawn(move || {
            let label = output.name();
            if let Err(e) = render_thread_main(output, handles, lib, mpv_ctx, ready_tx) {
                log::error!("[render] output '{label}' fatal: {e}");
            }
        })
        .map_err(|e| anyhow!("spawn render thread: {e}"))
}

// ---------------------------------------------------------------------------
// Render thread
// ---------------------------------------------------------------------------

/// Picks the GL framebuffer config for the output window.
///
/// An alpha channel is only a nicety (the output window is opaque), so it is
/// requested first and dropped when the driver exposes no such config — the
/// NVIDIA proprietary stack on X11 only offers alpha-less configs for some
/// visuals, and giving up there left the whole video engine headless.
fn pick_gl_config(display: &Display, window: RawWindowHandle) -> Result<glutin::config::Config> {
    for alpha_size in [Some(8u8), None] {
        let mut template = ConfigTemplateBuilder::new().compatible_with_native_window(window);
        if let Some(bits) = alpha_size {
            template = template.with_alpha_size(bits);
        }
        let found = unsafe { display.find_configs(template.build()) }
            .map_err(|e| anyhow!("find_configs: {e}"))?
            .next();
        if let Some(config) = found {
            if alpha_size.is_none() {
                log::warn!("[render] no GL config with alpha — using an alpha-less one");
            }
            return Ok(config);
        }
    }
    Err(anyhow!("no compatible GL config found"))
}

fn render_thread_main(
    output:   Arc<Output>,
    handles:  SendableHandles,
    lib:      Arc<MpvLib>,
    mpv_ctx:  Arc<MpvCtx>,
    ready_tx: std::sync::mpsc::Sender<Result<()>>,
) -> Result<()> {
    macro_rules! try_init {
        ($expr:expr) => {
            match $expr {
                Ok(v) => v,
                Err(e) => {
                    let msg = format!("{e}");
                    let _ = ready_tx.send(Err(anyhow!("{msg}")));
                    return Err(anyhow!("{msg}"));
                }
            }
        };
    }

    // ── 1. glutin Display ────────────────────────────────────────────────────
    let display = try_init!(create_display(handles.rdh, handles.rwh));

    // ── 2. GL config ─────────────────────────────────────────────────────────
    let config = try_init!(pick_gl_config(&display, handles.rwh));

    // ── 3. Context (OpenGL Core, not yet current) ────────────────────────────
    // macOS exposes only 3.2 and 4.1 core profiles (no 3.3); request 3.2 there.
    // Our shaders are `#version 150 core`, which both 3.2 and 3.3 contexts accept.
    #[cfg(target_os = "macos")]
    let gl_version = Version::new(3, 2);
    #[cfg(not(target_os = "macos"))]
    let gl_version = Version::new(3, 3);
    let ctx_attrs = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::OpenGl(Some(gl_version)))
        .build(Some(handles.rwh));
    let not_current = try_init!(unsafe {
        display.create_context(&config, &ctx_attrs)
            .map_err(|e| anyhow!("create_context: {e}"))
    });

    // ── 4. Window surface ─────────────────────────────────────────────────────
    let w0 = NonZeroU32::new(handles.width).unwrap_or(NonZeroU32::new(1).unwrap());
    let h0 = NonZeroU32::new(handles.height).unwrap_or(NonZeroU32::new(1).unwrap());
    let surf_attrs = SurfaceAttributesBuilder::<WindowSurface>::new()
        .with_srgb(Some(false))
        .build(handles.rwh, w0, h0);
    let surface = try_init!(unsafe {
        display.create_window_surface(&config, &surf_attrs)
            .map_err(|e| anyhow!("create_window_surface: {e}"))
    });

    // ── 5. Make context current on THIS thread ────────────────────────────────
    let ctx = try_init!(not_current.make_current(&surface)
        .map_err(|e| anyhow!("make_current: {e}")));

    // ── 6. vsync ──────────────────────────────────────────────────────────────
    // DontWait on every OS. mpv's own clock (video-sync=desync) paces playback, so
    // our swap is not the timing source — blocking on the driver's vblank only adds
    // a redundant sync point.
    //
    // Do NOT switch Linux to SwapInterval::Wait(1): on Mesa/Wayland with a weak
    // shared-memory iGPU, blocking inside eglSwapBuffers holds a driver lock for the
    // whole vblank wait, serialising this render thread's GL with WebKitGTK's
    // compositing on the main thread — which starved the Inkue UI to ~1 fps for the
    // entire duration of video playback (regression seen 2026-06; reverted). Under a
    // VM with an emulated vblank the same block can stall the whole desktop.
    if let Err(e) = surface.set_swap_interval(&ctx, SwapInterval::DontWait) {
        log::warn!("[render] swap_interval: {e:?}");
    }

    // ── 7. glow GL loader ─────────────────────────────────────────────────────
    // Used only on this render thread — no Arc/sharing needed.
    let display_box = Box::new(display);
    let gl = unsafe {
        glow::Context::from_loader_function_cstr(|name| {
            display_box.get_proc_address(name) as *const _
        })
    };

    // ── 8. Fade-quad + warp shaders ───────────────────────────────────────────
    let (fade_program, fade_vao) = build_fade_shader(&gl)?;
    let (warp_program, warp_vao) = build_warp_shader(&gl)?;

    // ── 9. mpv render context with OpenGL backend ─────────────────────────────
    let display_ptr = &*display_box as *const Display as *mut c_void;
    let mut gl_init = MpvOpenglInitParams {
        get_proc_address:     gl_get_proc_address,
        get_proc_address_ctx: display_ptr,
    };
    let api_str = CString::new("opengl").unwrap();
    let flip_y: i32 = 1;
    let params = [
        MpvRenderParam { type_: MPV_RENDER_PARAM_API_TYPE,           data: api_str.as_ptr() as *mut c_void },
        MpvRenderParam { type_: MPV_RENDER_PARAM_OPENGL_INIT_PARAMS, data: &mut gl_init as *mut _ as *mut c_void },
        MpvRenderParam { type_: 0, data: std::ptr::null_mut() },
    ];
    let mut render_ctx: *mut c_void = std::ptr::null_mut();
    let ret = unsafe { (lib.mpv_render_context_create)(&mut render_ctx, mpv_ctx.0, params.as_ptr()) };
    if ret < 0 {
        let _ = ready_tx.send(Err(anyhow!("mpv_render_context_create: {ret}")));
        return Err(anyhow!("mpv_render_context_create: {ret}"));
    }
    log::info!("[render] mpv render context created (OpenGL {}.{} Core)", gl_version.major, gl_version.minor);
    let _ = ready_tx.send(Ok(()));

    // ── 10. Update callback ───────────────────────────────────────────────────
    let signal_ptr = Arc::as_ptr(&output.signal) as *mut c_void;
    unsafe { (lib.mpv_render_context_set_update_callback)(render_ctx, Some(on_mpv_update), signal_ptr); }

    // ── 11. Render loop ───────────────────────────────────────────────────────
    let (lock, cvar) = output.signal.as_ref();
    let mut w_px = handles.width;
    let mut h_px = handles.height;

    // Layer compositor state: the overlay context (timer OSD / Text Cue /
    // test patterns) renders into its own target like every video slot; the
    // ping-pong pair accumulates the blend stack, and the mix pair averages
    // the composites of a dissolve (allocated on the first crossfade).
    let compositor = Compositor {
        composite: build_composite_shader(&gl)?,
        mix: build_mix_shader(&gl)?,
        blit: build_blit_shader(&gl)?,
    };
    let mut overlay_target: Option<WarpTarget> = None;
    let mut slot_targets: Vec<Option<WarpTarget>> = Vec::new();
    let mut slot_valid: Vec<bool> = Vec::new();
    let mut pingpong: [Option<WarpTarget>; 2] = [None, None];
    let mut mix_targets: [Option<WarpTarget>; 2] = [None, None];

    // Opt-in output frame-rate cap (Linux).  `INKUE_OUTPUT_FPS=30` makes the render
    // loop present at most ~30 fps, halving the output window's GPU compositing load so
    // a weak shared-memory iGPU keeps headroom for the WebKitGTK UI during playback.
    // Off by default (0/unset = uncapped) — it trades some video smoothness, so it is a
    // knob the operator turns on only if the UI still lags after hwdec/XWayland.
    #[cfg(target_os = "linux")]
    let min_present_interval: Option<Duration> = std::env::var("INKUE_OUTPUT_FPS").ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|&fps| fps > 0)
        .map(|fps| Duration::from_micros(1_000_000 / fps as u64));
    #[cfg(target_os = "linux")]
    if let Some(iv) = min_present_interval {
        log::info!("[render] output FPS cap enabled: ~{} fps", 1_000_000 / iv.as_micros().max(1) as u64);
    }
    #[cfg(target_os = "linux")]
    let mut last_present = std::time::Instant::now();

    loop {
        // The output is being destroyed: hand every render context back while
        // this thread's GL context is still current, then let it go.
        if output.shutdown.load(Ordering::Acquire) {
            release_render_contexts(&lib, &output, render_ctx);
            log::info!("[render] output '{}' render thread ended", output.name());
            return Ok(());
        }

        // Create render contexts for slots the engine spawned since last pass.
        let slots = output.slots_snapshot();
        for s in &slots {
            if s.needs_render_init.swap(false, Ordering::AcqRel) {
                let mut gl_init2 = MpvOpenglInitParams {
                    get_proc_address:     gl_get_proc_address,
                    get_proc_address_ctx: display_ptr,
                };
                let api_str2 = CString::new("opengl").unwrap();
                let params2 = [
                    MpvRenderParam { type_: MPV_RENDER_PARAM_API_TYPE,           data: api_str2.as_ptr() as *mut c_void },
                    MpvRenderParam { type_: MPV_RENDER_PARAM_OPENGL_INIT_PARAMS, data: &mut gl_init2 as *mut _ as *mut c_void },
                    MpvRenderParam { type_: 0, data: std::ptr::null_mut() },
                ];
                let mut rc: *mut c_void = std::ptr::null_mut();
                let ret = unsafe { (lib.mpv_render_context_create)(&mut rc, s.mpv_ctx.0, params2.as_ptr()) };
                if ret < 0 {
                    log::error!("[render] slot {} render context failed: {ret}", s.index);
                } else {
                    unsafe { (lib.mpv_render_context_set_update_callback)(rc, Some(on_mpv_update), signal_ptr); }
                    s.render_ctx.store(rc, Ordering::Release);
                    log::info!("[render] slot {} render context created", s.index);
                }
            }
        }

        // Per-slot opacity animations and dissolves pace the loop at 16 ms
        // just like the master fade — a crossfade between two still images
        // has no mpv frame to wake it.
        let master_animating = output.fade.lock()
            .map(|s| s.current_alpha != s.target_alpha)
            .unwrap_or(false);
        let slots_animating = slots.iter().any(slot::is_animating);
        let needs_animation = master_animating || slots_animating;
        let timeout = if needs_animation { Duration::from_millis(16) } else { Duration::from_millis(100) };

        {
            let mut ready = lock.lock().unwrap();
            if !*ready {
                let (g, _) = cvar.wait_timeout(ready, timeout).unwrap();
                ready = g;
            }
            *ready = false;
        }

        // Apply pending resize from the event loop / window backend.
        let new_w = output.width.load(Ordering::Relaxed).max(1);
        let new_h = output.height.load(Ordering::Relaxed).max(1);
        if new_w != w_px || new_h != h_px {
            surface.resize(
                &ctx,
                NonZeroU32::new(new_w).unwrap(),
                NonZeroU32::new(new_h).unwrap(),
            );
            w_px = new_w;
            h_px = new_h;
        }

        let (alpha, _) = output.tick_fade();

        // Overlay context (timer OSD / Text Cue / test patterns / win32 path).
        let flags     = unsafe { (lib.mpv_render_context_update)(render_ctx) };
        let has_frame = flags & MPV_RENDER_UPDATE_FRAME != 0;
        let text_active = output.text_overlay_active.load(Ordering::Relaxed);
        // Warp params changed since the last pass — must redraw even without a
        // new mpv frame (paused video / held image), or alignment edits would
        // only show on the next frame.
        let warp_dirty = output.warp_dirty.swap(false, Ordering::Relaxed)
            || output.overlay_dirty.swap(false, Ordering::Relaxed);

        // Tick each slot: advance opacity anims, finish pending unloads, and
        // check for fresh frames.  Ticks must run even while hidden so stop
        // fades can finish, but rendering below is gated on visibility.
        let slots = output.slots_snapshot();
        let mut layers: Vec<LayerDraw> = Vec::with_capacity(slots.len());
        let mut any_slot_frame = false;
        for s in &slots {
            let rc = s.render_ctx.load(Ordering::Acquire);
            if rc.is_null() {
                continue;
            }
            let sflags = unsafe { (lib.mpv_render_context_update)(rc) };
            let s_new_frame = sflags & MPV_RENDER_UPDATE_FRAME != 0;
            let (opacity, _still_animating) = slot::tick_slot(s);
            let Some((voice, layer_key, blend_mode)) = s
                .state
                .lock()
                .ok()
                .map(|st| (st.voice_id, st.layer_key, st.blend_mode.shader_id()))
            else { continue };
            let Some(voice) = voice else {
                if let Some(v) = slot_valid.get_mut(s.index) { *v = false; }
                continue;
            };
            any_slot_frame |= s_new_frame;
            layers.push(LayerDraw {
                slot_index: s.index,
                voice,
                layer_key,
                opacity,
                blend_mode,
                has_new_frame: s_new_frame,
                render_ctx: rc,
            });
        }
        layers.sort_by_key(|l| l.layer_key);

        // Dissolves on this output (crossfades), expanded into the weighted
        // composites that render them exactly.
        let plan = crossfade::plan_dissolves(&mix_links(&slots, &layers));
        for &(slot_index, factor) in &plan.approximated {
            if let Some(layer) = layers.iter_mut().find(|l| l.slot_index == slot_index) {
                layer.opacity *= factor;
            }
        }

        // Do not commit frames while the output window is hidden.  On Wayland
        // a wl_surface.commit() with a buffer permanently maps the surface, so
        // a single frame emitted before show_output() would make the window
        // appear at startup instead of staying invisible until the operator
        // opens it.  show() sets this flag and wakes the loop so the first
        // committed frame arrives immediately when the window is revealed.
        if !output.visible.load(Ordering::Relaxed) { continue; }
        // Skip rendering when nothing changed anywhere: no new frame from any
        // mpv, no animation, no active layers or overlay work.  Text/timer
        // overlays render unconditionally (mpv does not signal OSD-only
        // changes in idle mode).
        if !has_frame && !any_slot_frame && alpha == 0 && !text_active && !warp_dirty
            && !needs_animation && layers.is_empty() { continue; }

        // Opt-in FPS cap: drop video frames arriving faster than the target interval.
        // Never throttle a fade animation or a Text overlay redraw (must stay smooth);
        // mpv wakes us again on the next frame, so the latest one still presents.
        #[cfg(target_os = "linux")]
        if let Some(iv) = min_present_interval {
            if !needs_animation && !text_active && last_present.elapsed() < iv {
                continue;
            }
        }

        // ── Size all offscreen targets ────────────────────────────────────────
        let mut targets_ok = ensure_warp_target(&gl, &mut overlay_target, w_px, h_px).is_ok();
        targets_ok &= ensure_warp_target(&gl, &mut pingpong[0], w_px, h_px).is_ok();
        targets_ok &= ensure_warp_target(&gl, &mut pingpong[1], w_px, h_px).is_ok();
        if slot_targets.len() < slots.len() {
            slot_targets.resize_with(slots.len(), || None);
            slot_valid.resize(slots.len(), false);
        }
        for l in &layers {
            if let Some(t) = slot_targets.get_mut(l.slot_index) {
                targets_ok &= ensure_warp_target(&gl, t, w_px, h_px).is_ok();
            }
        }
        if !targets_ok {
            log::warn!("[render] compositor targets unavailable — skipping frame");
            continue;
        }

        // ── Render mpv contexts into their targets ────────────────────────────
        // Overlay: render every pass **while it shows something** (timer OSD /
        // Text Cue / test pattern) — OSD-only changes never signal a new
        // frame.  While inactive it is neither rendered nor composited: mpv's
        // *idle* render clears the target to opaque black on some libmpv
        // builds (`background=none` ignored in idle — measured on 0.41-dev,
        // Windows), which would mask every video layer below.
        // All render calls pass block_for_target_time=0: the default (1) makes
        // each call sleep until *that* context's frame display time, and with
        // several contexts sharing this one thread the waits serialise — two
        // simultaneous videos stuttered even when one was fully transparent.
        // Our loop is paced by the update callbacks instead; each context just
        // hands over its current frame (video-sync=desync owns the clock).
        let mut no_block: i32 = 0;
        let overlay_on = output.overlay_active();
        if let (true, Some(t)) = (overlay_on, &overlay_target) {
            let mut fbo = MpvOpenglFbo { fbo: t.fbo.0.get() as i32, w: w_px as i32, h: h_px as i32, internal_format: 0 };
            let mut flip = flip_y;
            let rp = [
                MpvRenderParam { type_: MPV_RENDER_PARAM_OPENGL_FBO, data: &mut fbo  as *mut _ as *mut c_void },
                MpvRenderParam { type_: MPV_RENDER_PARAM_FLIP_Y,     data: &mut flip as *mut _ as *mut c_void },
                MpvRenderParam { type_: MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME, data: &mut no_block as *mut _ as *mut c_void },
                MpvRenderParam { type_: 0, data: std::ptr::null_mut() },
            ];
            let ret = unsafe { (lib.mpv_render_context_render)(render_ctx, rp.as_ptr()) };
            if ret < 0 { log::warn!("[render] overlay render: {ret}"); }
        }
        let overlay_valid = overlay_on;

        for l in &layers {
            let needs = l.has_new_frame || !slot_valid.get(l.slot_index).copied().unwrap_or(false);
            if !needs {
                continue;
            }
            if let Some(Some(t)) = slot_targets.get(l.slot_index) {
                let mut fbo = MpvOpenglFbo { fbo: t.fbo.0.get() as i32, w: w_px as i32, h: h_px as i32, internal_format: 0 };
                let mut flip = flip_y;
                let rp = [
                    MpvRenderParam { type_: MPV_RENDER_PARAM_OPENGL_FBO, data: &mut fbo  as *mut _ as *mut c_void },
                    MpvRenderParam { type_: MPV_RENDER_PARAM_FLIP_Y,     data: &mut flip as *mut _ as *mut c_void },
                    MpvRenderParam { type_: MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME, data: &mut no_block as *mut _ as *mut c_void },
                    MpvRenderParam { type_: 0, data: std::ptr::null_mut() },
                ];
                let ret = unsafe { (lib.mpv_render_context_render)(l.render_ctx, rp.as_ptr()) };
                if ret < 0 { log::warn!("[render] slot {} render: {ret}", l.slot_index); }
                if let Some(v) = slot_valid.get_mut(l.slot_index) { *v = true; }
            }
        }

        // ── Composite the layer stack ─────────────────────────────────────────
        // Base = opaque black; each layer blends over the accumulated result.
        // A dissolve in flight composites the stack once per side and mixes
        // the pictures (crossfade.rs) — never a dip, never a cut in the bars.
        let targets = StackTargets { slots: &slot_targets, pingpong: &pingpong, w: w_px, h: h_px };
        let stage = if plan.is_plain() {
            composite_stack(&gl, &compositor, &layers, &[], &targets)
        } else {
            let mixable = ensure_warp_target(&gl, &mut mix_targets[0], w_px, h_px).is_ok()
                && ensure_warp_target(&gl, &mut mix_targets[1], w_px, h_px).is_ok();
            if mixable {
                mix_dissolves(&gl, &compositor, &plan, &layers, &targets, &mix_targets)
            } else {
                log::warn!("[render] mix targets unavailable — dissolve shown unmixed");
                composite_stack(&gl, &compositor, &layers, &[], &targets)
            }
        };
        let Some((staged, free)) = stage else { continue };
        let mut final_tex = staged;

        // Overlay (timer / text / patterns) on top — only while active.
        if overlay_valid {
            if let (Some(t), Some(dst)) = (&overlay_target, &pingpong[free]) {
                draw_composite_pass(
                    &gl, compositor.composite.0, compositor.composite.1,
                    final_tex, t.tex, 0, 1.0, w_px, h_px, dst.fbo,
                );
                final_tex = dst.tex;
            }
        }

        // ── Present: warp (or plain blit) + master fade quad ──────────────────
        let warp = output.warp.lock().ok().and_then(|g| *g);
        match warp {
            Some(hinv) => draw_warp_pass(&gl, warp_program, warp_vao, final_tex, &hinv, w_px, h_px),
            None => draw_blit_pass(&gl, compositor.blit.0, compositor.blit.1, final_tex, w_px, h_px),
        }

        if alpha > 0 { draw_fade_quad(&gl, fade_program, fade_vao, alpha as f32 / 255.0); }

        if let Err(e) = surface.swap_buffers(&ctx) { log::warn!("[render] swap: {e:?}"); }
        unsafe { (lib.mpv_render_context_report_swap)(render_ctx); }
        for l in &layers {
            if l.has_new_frame {
                unsafe { (lib.mpv_render_context_report_swap)(l.render_ctx); }
            }
        }
        #[cfg(target_os = "linux")]
        { last_present = std::time::Instant::now(); }
    }
}

/// Free every mpv render context of `output` — its slots' and its overlay's.
/// libmpv requires it on the GL thread, with the context current, before the
/// cores are destroyed.
fn release_render_contexts(lib: &MpvLib, output: &Output, overlay_ctx: *mut c_void) {
    for slot in output.slots_snapshot() {
        let rc = slot.render_ctx.swap(std::ptr::null_mut(), Ordering::AcqRel);
        if !rc.is_null() {
            // SAFETY: created on this thread by `mpv_render_context_create`;
            // swapped out above so nothing renders with it again.
            unsafe { (lib.mpv_render_context_free)(rc) };
        }
    }
    // SAFETY: the overlay's render context, created at the top of
    // `render_thread_main` and used by no one else.
    unsafe { (lib.mpv_render_context_free)(overlay_ctx) };
}

// ---------------------------------------------------------------------------
// GL proc-address bridge for mpv
// ---------------------------------------------------------------------------

unsafe extern "C" fn gl_get_proc_address(user_ctx: *mut c_void, name: *const std::ffi::c_char) -> *mut c_void {
    let display = unsafe { &*(user_ctx as *const Display) };
    let cname   = unsafe { CStr::from_ptr(name) };
    display.get_proc_address(cname) as *mut c_void
}

// ---------------------------------------------------------------------------
// mpv update callback
// ---------------------------------------------------------------------------

unsafe extern "C" fn on_mpv_update(ctx: *mut c_void) {
    if ctx.is_null() { return; }
    let signal = unsafe { &*(ctx as *const (Mutex<bool>, Condvar)) };
    if let Ok(mut ready) = signal.0.lock() {
        *ready = true;
        signal.1.notify_one();
    }
}

// ---------------------------------------------------------------------------
// Platform-specific glutin Display creation
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
fn create_display(rdh: RawDisplayHandle, _rwh: RawWindowHandle) -> Result<Display> {
    // Pass None so glutin uses its own temporary invisible window for WGL
    // extension loading — avoids double SetPixelFormat on our actual HWND.
    let display = unsafe {
        Display::new(rdh, DisplayApiPreference::WglThenEgl(None))
            .map_err(|e| anyhow!("WGL display: {e}"))?
    };
    Ok(display)
}

#[cfg(target_os = "macos")]
fn create_display(rdh: RawDisplayHandle, _rwh: RawWindowHandle) -> Result<Display> {
    let display = unsafe {
        Display::new(rdh, DisplayApiPreference::Cgl)
            .map_err(|e| anyhow!("CGL display: {e}"))?
    };
    Ok(display)
}

#[cfg(target_os = "linux")]
fn create_display(rdh: RawDisplayHandle, _rwh: RawWindowHandle) -> Result<Display> {
    // Try EGL first (works on both X11 and Wayland), fall back to GLX (X11 only).
    let display = unsafe {
        Display::new(rdh, DisplayApiPreference::EglThenGlx(Box::new(|_| {})))
            .map_err(|e| anyhow!("EGL/GLX display: {e}"))?
    };
    Ok(display)
}

// ---------------------------------------------------------------------------
// Fade-quad shader (fullscreen black triangle)
// ---------------------------------------------------------------------------

fn build_fade_shader(gl: &glow::Context) -> Result<(glow::Program, glow::VertexArray)> {
    // `#version 150 core` is the highest GLSL accepted by macOS's 3.2 core profile,
    // and is a strict subset of what the Windows/Linux 3.3 contexts accept — one
    // shader for all three. `gl_VertexID` + const array constructors are valid in 150.
    const VERT: &str = r#"
#version 150 core
const vec2 POS[3] = vec2[3](vec2(-1,-1), vec2(3,-1), vec2(-1,3));
void main() { gl_Position = vec4(POS[gl_VertexID], 0.0, 1.0); }
"#;
    const FRAG: &str = r#"
#version 150 core
uniform float u_alpha;
out vec4 color;
void main() { color = vec4(0.0, 0.0, 0.0, u_alpha); }
"#;
    unsafe {
        let vs = gl.create_shader(glow::VERTEX_SHADER).map_err(|e| anyhow!("{e}"))?;
        gl.shader_source(vs, VERT);
        gl.compile_shader(vs);
        if !gl.get_shader_compile_status(vs) { return Err(anyhow!("vert: {}", gl.get_shader_info_log(vs))); }

        let fs = gl.create_shader(glow::FRAGMENT_SHADER).map_err(|e| anyhow!("{e}"))?;
        gl.shader_source(fs, FRAG);
        gl.compile_shader(fs);
        if !gl.get_shader_compile_status(fs) { return Err(anyhow!("frag: {}", gl.get_shader_info_log(fs))); }

        let prog = gl.create_program().map_err(|e| anyhow!("{e}"))?;
        gl.attach_shader(prog, vs); gl.attach_shader(prog, fs);
        gl.link_program(prog);
        if !gl.get_program_link_status(prog) { return Err(anyhow!("link: {}", gl.get_program_info_log(prog))); }
        gl.detach_shader(prog, vs); gl.delete_shader(vs);
        gl.detach_shader(prog, fs); gl.delete_shader(fs);

        let vao = gl.create_vertex_array().map_err(|e| anyhow!("{e}"))?;
        log::info!("[render] fade shader compiled");
        Ok((prog, vao))
    }
}

fn draw_fade_quad(gl: &glow::Context, program: glow::Program, vao: glow::VertexArray, alpha: f32) {
    unsafe {
        gl.enable(glow::BLEND);
        gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
        gl.use_program(Some(program));
        if let Some(loc) = gl.get_uniform_location(program, "u_alpha") {
            gl.uniform_1_f32(Some(&loc), alpha);
        }
        gl.bind_vertex_array(Some(vao));
        gl.draw_arrays(glow::TRIANGLES, 0, 3);
        gl.bind_vertex_array(None);
        gl.use_program(None);
        gl.disable(glow::BLEND);
    }
}

// ---------------------------------------------------------------------------
// Output warp pass (corner pin / fine rotation)
// ---------------------------------------------------------------------------

/// Offscreen target mpv renders into when the warp is active; the warp pass
/// then samples it with the inverse homography.
struct WarpTarget {
    fbo: glow::Framebuffer,
    tex: glow::Texture,
    w:   u32,
    h:   u32,
}

/// Create (or resize) the warp FBO to the current window size.
fn ensure_warp_target(
    gl: &glow::Context,
    slot: &mut Option<WarpTarget>,
    w: u32,
    h: u32,
) -> Result<()> {
    if let Some(t) = slot {
        if t.w == w && t.h == h {
            return Ok(());
        }
    }
    unsafe {
        if let Some(old) = slot.take() {
            gl.delete_framebuffer(old.fbo);
            gl.delete_texture(old.tex);
        }
        let tex = gl.create_texture().map_err(|e| anyhow!("warp tex: {e}"))?;
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_image_2d(
            glow::TEXTURE_2D, 0, glow::RGBA8 as i32,
            w as i32, h as i32, 0,
            glow::RGBA, glow::UNSIGNED_BYTE, None,
        );
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
        gl.bind_texture(glow::TEXTURE_2D, None);

        let fbo = gl.create_framebuffer().map_err(|e| anyhow!("warp fbo: {e}"))?;
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0,
        );
        let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        if status != glow::FRAMEBUFFER_COMPLETE {
            gl.delete_framebuffer(fbo);
            gl.delete_texture(tex);
            return Err(anyhow!("warp FBO incomplete: 0x{status:x}"));
        }
        *slot = Some(WarpTarget { fbo, tex, w, h });
        log::info!("[render] warp target (re)created: {w}x{h}");
    }
    Ok(())
}

/// Fullscreen inverse-homography pass: for every window pixel, sample where in
/// the mpv frame it comes from; pixels outside the destination quad are black.
fn build_warp_shader(gl: &glow::Context) -> Result<(glow::Program, glow::VertexArray)> {
    const VERT: &str = r#"
#version 150 core
const vec2 POS[3] = vec2[3](vec2(-1,-1), vec2(3,-1), vec2(-1,3));
void main() { gl_Position = vec4(POS[gl_VertexID], 0.0, 1.0); }
"#;
    // All warp math is in y-down normalized window space ([0,1]², origin at the
    // top-left — matching the editor UI).  gl_FragCoord is y-up, so flip once
    // on input; the mpv texture is rendered with FLIP_Y (y-up), so flip once
    // more on sampling.
    const FRAG: &str = r#"
#version 150 core
uniform sampler2D u_tex;
uniform mat3  u_hinv;
uniform vec2  u_size;
out vec4 color;
void main() {
    vec2 win = vec2(gl_FragCoord.x / u_size.x, 1.0 - gl_FragCoord.y / u_size.y);
    vec3 t = u_hinv * vec3(win, 1.0);
    if (t.z == 0.0) { color = vec4(0.0, 0.0, 0.0, 1.0); return; }
    vec2 uv = t.xy / t.z;
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
        color = vec4(0.0, 0.0, 0.0, 1.0);
    } else {
        color = texture(u_tex, vec2(uv.x, 1.0 - uv.y));
    }
}
"#;
    unsafe {
        let vs = gl.create_shader(glow::VERTEX_SHADER).map_err(|e| anyhow!("{e}"))?;
        gl.shader_source(vs, VERT);
        gl.compile_shader(vs);
        if !gl.get_shader_compile_status(vs) { return Err(anyhow!("warp vert: {}", gl.get_shader_info_log(vs))); }

        let fs = gl.create_shader(glow::FRAGMENT_SHADER).map_err(|e| anyhow!("{e}"))?;
        gl.shader_source(fs, FRAG);
        gl.compile_shader(fs);
        if !gl.get_shader_compile_status(fs) { return Err(anyhow!("warp frag: {}", gl.get_shader_info_log(fs))); }

        let prog = gl.create_program().map_err(|e| anyhow!("{e}"))?;
        gl.attach_shader(prog, vs); gl.attach_shader(prog, fs);
        gl.link_program(prog);
        if !gl.get_program_link_status(prog) { return Err(anyhow!("warp link: {}", gl.get_program_info_log(prog))); }
        gl.detach_shader(prog, vs); gl.delete_shader(vs);
        gl.detach_shader(prog, fs); gl.delete_shader(fs);

        let vao = gl.create_vertex_array().map_err(|e| anyhow!("{e}"))?;
        log::info!("[render] warp shader compiled");
        Ok((prog, vao))
    }
}

// ---------------------------------------------------------------------------
// Layer compositor (blend stack) + plain blit
// ---------------------------------------------------------------------------

/// One blend step: `result = blend(backdrop, layer, mode, opacity)`.
///
/// The per-channel math is [`super::blend::GLSL_BLEND_FN`], whose executable
/// spec is the Rust `blend_channel` in `blend.rs` — keep them identical.
fn build_composite_shader(gl: &glow::Context) -> Result<(glow::Program, glow::VertexArray)> {
    const VERT: &str = r#"
#version 150 core
const vec2 POS[3] = vec2[3](vec2(-1,-1), vec2(3,-1), vec2(-1,3));
out vec2 v_uv;
void main() {
    gl_Position = vec4(POS[gl_VertexID], 0.0, 1.0);
    v_uv = POS[gl_VertexID] * 0.5 + 0.5;
}
"#;
    let frag = format!(
        r#"
#version 150 core
uniform sampler2D u_backdrop;
uniform sampler2D u_layer;
uniform int   u_blend_mode;
uniform float u_opacity;
in vec2 v_uv;
out vec4 color;
{}
void main() {{
    vec4 b = texture(u_backdrop, v_uv);
    vec4 s = texture(u_layer, v_uv);
    float sa = clamp(s.a * u_opacity, 0.0, 1.0);
    float ao = sa + b.a * (1.0 - sa);
    vec3 rgb = vec3(0.0);
    for (int c = 0; c < 3; c++) {{
        float blended = (1.0 - b.a) * s[c] + b.a * blend_channel(u_blend_mode, b[c], s[c]);
        rgb[c] = sa * blended + (1.0 - sa) * b.a * b[c];
    }}
    if (ao > 0.0) rgb /= ao;
    color = vec4(rgb, ao);
}}
"#,
        super::blend::GLSL_BLEND_FN,
    );
    build_program(gl, VERT, &frag, "composite")
}

/// Plain textured fullscreen blit (composite → window when no warp).
fn build_blit_shader(gl: &glow::Context) -> Result<(glow::Program, glow::VertexArray)> {
    const VERT: &str = r#"
#version 150 core
const vec2 POS[3] = vec2[3](vec2(-1,-1), vec2(3,-1), vec2(-1,3));
out vec2 v_uv;
void main() {
    gl_Position = vec4(POS[gl_VertexID], 0.0, 1.0);
    v_uv = POS[gl_VertexID] * 0.5 + 0.5;
}
"#;
    const FRAG: &str = r#"
#version 150 core
uniform sampler2D u_tex;
in vec2 v_uv;
out vec4 color;
void main() { color = vec4(texture(u_tex, v_uv).rgb, 1.0); }
"#;
    build_program(gl, VERT, FRAG, "blit")
}

/// Compile + link a program and create its (empty) VAO.
fn build_program(
    gl: &glow::Context,
    vert: &str,
    frag: &str,
    name: &str,
) -> Result<(glow::Program, glow::VertexArray)> {
    unsafe {
        let vs = gl.create_shader(glow::VERTEX_SHADER).map_err(|e| anyhow!("{e}"))?;
        gl.shader_source(vs, vert);
        gl.compile_shader(vs);
        if !gl.get_shader_compile_status(vs) {
            return Err(anyhow!("{name} vert: {}", gl.get_shader_info_log(vs)));
        }

        let fs = gl.create_shader(glow::FRAGMENT_SHADER).map_err(|e| anyhow!("{e}"))?;
        gl.shader_source(fs, frag);
        gl.compile_shader(fs);
        if !gl.get_shader_compile_status(fs) {
            return Err(anyhow!("{name} frag: {}", gl.get_shader_info_log(fs)));
        }

        let prog = gl.create_program().map_err(|e| anyhow!("{e}"))?;
        gl.attach_shader(prog, vs);
        gl.attach_shader(prog, fs);
        gl.link_program(prog);
        if !gl.get_program_link_status(prog) {
            return Err(anyhow!("{name} link: {}", gl.get_program_info_log(prog)));
        }
        gl.detach_shader(prog, vs);
        gl.delete_shader(vs);
        gl.detach_shader(prog, fs);
        gl.delete_shader(fs);

        let vao = gl.create_vertex_array().map_err(|e| anyhow!("{e}"))?;
        log::info!("[render] {name} shader compiled");
        Ok((prog, vao))
    }
}

/// One blend step of the layer stack into `dst_fbo`.
#[allow(clippy::too_many_arguments)]
fn draw_composite_pass(
    gl: &glow::Context,
    program: glow::Program,
    vao: glow::VertexArray,
    backdrop_tex: glow::Texture,
    layer_tex: glow::Texture,
    blend_mode: i32,
    opacity: f32,
    w: u32,
    h: u32,
    dst_fbo: glow::Framebuffer,
) {
    unsafe {
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(dst_fbo));
        gl.viewport(0, 0, w as i32, h as i32);
        gl.disable(glow::BLEND);
        gl.use_program(Some(program));
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(backdrop_tex));
        gl.active_texture(glow::TEXTURE1);
        gl.bind_texture(glow::TEXTURE_2D, Some(layer_tex));
        if let Some(loc) = gl.get_uniform_location(program, "u_backdrop") {
            gl.uniform_1_i32(Some(&loc), 0);
        }
        if let Some(loc) = gl.get_uniform_location(program, "u_layer") {
            gl.uniform_1_i32(Some(&loc), 1);
        }
        if let Some(loc) = gl.get_uniform_location(program, "u_blend_mode") {
            gl.uniform_1_i32(Some(&loc), blend_mode);
        }
        if let Some(loc) = gl.get_uniform_location(program, "u_opacity") {
            gl.uniform_1_f32(Some(&loc), opacity);
        }
        gl.bind_vertex_array(Some(vao));
        gl.draw_arrays(glow::TRIANGLES, 0, 3);
        gl.bind_vertex_array(None);
        gl.active_texture(glow::TEXTURE1);
        gl.bind_texture(glow::TEXTURE_2D, None);
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, None);
        gl.use_program(None);
    }
}

/// The textures a stack composite reads and writes.
struct StackTargets<'a> {
    slots: &'a [Option<WarpTarget>],
    pingpong: &'a [Option<WarpTarget>; 2],
    w: u32,
    h: u32,
}

/// Composite every layer not in `excluded` over opaque black, ping-ponging
/// between the two buffers.  Returns the texture holding the result and the
/// index of the ping-pong buffer left free.
fn composite_stack(
    gl: &glow::Context,
    compositor: &Compositor,
    layers: &[LayerDraw],
    excluded: &[usize],
    targets: &StackTargets<'_>,
) -> Option<(glow::Texture, usize)> {
    let mut src = 0usize;
    let base = targets.pingpong[src].as_ref()?;
    unsafe {
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(base.fbo));
        gl.viewport(0, 0, targets.w as i32, targets.h as i32);
        gl.clear_color(0.0, 0.0, 0.0, 1.0);
        gl.clear(glow::COLOR_BUFFER_BIT);
    }
    for l in layers {
        if l.opacity <= 0.0 || excluded.contains(&l.slot_index) {
            continue;
        }
        let Some(Some(layer_t)) = targets.slots.get(l.slot_index) else { continue };
        let dst = 1 - src;
        let (Some(backdrop), Some(out)) = (&targets.pingpong[src], &targets.pingpong[dst]) else {
            continue;
        };
        draw_composite_pass(
            gl, compositor.composite.0, compositor.composite.1,
            backdrop.tex, layer_t.tex, l.blend_mode, l.opacity, targets.w, targets.h, out.fbo,
        );
        src = dst;
    }
    targets.pingpong[src].as_ref().map(|t| (t.tex, 1 - src))
}

/// A frame with dissolves in it: composite the stack once per pass of the
/// plan and fold each composite into a running weighted average held by the
/// mix pair.  Returns the averaged picture and a free ping-pong buffer.
fn mix_dissolves(
    gl: &glow::Context,
    compositor: &Compositor,
    plan: &crossfade::DissolvePlan,
    layers: &[LayerDraw],
    targets: &StackTargets<'_>,
    mix_targets: &[Option<WarpTarget>; 2],
) -> Option<(glow::Texture, usize)> {
    let mut accumulated = 0.0_f32;
    let mut average: Option<usize> = None; // mix_targets[i] holds the average so far
    for pass in &plan.passes {
        let (composite, _) = composite_stack(gl, compositor, layers, &pass.excluded, targets)?;
        let dst = average.map_or(0, |current| 1 - current);
        let out = mix_targets[dst].as_ref()?;
        let (previous, factor) = match average {
            None => (composite, 1.0),
            Some(current) => (
                mix_targets[current].as_ref()?.tex,
                crossfade::fold_factor(accumulated, pass.weight),
            ),
        };
        draw_mix_pass(
            gl, compositor.mix.0, compositor.mix.1,
            previous, composite, factor, targets.w, targets.h, out.fbo,
        );
        average = Some(dst);
        accumulated += pass.weight;
    }
    let picture = mix_targets[average?].as_ref()?.tex;
    Some((picture, 0))
}

/// The dissolves on this output, in slot indices: each incoming layer and the
/// outgoing layers it mixes away here.
fn mix_links(slots: &[Arc<slot::VideoSlot>], layers: &[LayerDraw]) -> Vec<MixLink> {
    let now = std::time::Instant::now();
    layers
        .iter()
        .filter_map(|l| {
            let incoming = slots.iter().find(|s| s.index == l.slot_index)?;
            let (mixed, weight) = slot::dissolve_of(incoming, now)?;
            let outgoing = mixed
                .iter()
                .filter_map(|voice| layers.iter().find(|o| o.voice == *voice))
                .map(|o| (o.slot_index, o.layer_key))
                .collect();
            Some(MixLink { incoming: l.slot_index, incoming_key: l.layer_key, outgoing, weight })
        })
        .collect()
}

/// `color = mix(a, b, t)` — folds one composite of a dissolve into the
/// running average.  Both inputs are opaque (composited over black).
fn build_mix_shader(gl: &glow::Context) -> Result<(glow::Program, glow::VertexArray)> {
    const VERT: &str = r#"
#version 150 core
const vec2 POS[3] = vec2[3](vec2(-1,-1), vec2(3,-1), vec2(-1,3));
out vec2 v_uv;
void main() {
    gl_Position = vec4(POS[gl_VertexID], 0.0, 1.0);
    v_uv = POS[gl_VertexID] * 0.5 + 0.5;
}
"#;
    const FRAG: &str = r#"
#version 150 core
uniform sampler2D u_a;
uniform sampler2D u_b;
uniform float u_t;
in vec2 v_uv;
out vec4 color;
void main() { color = mix(texture(u_a, v_uv), texture(u_b, v_uv), u_t); }
"#;
    build_program(gl, VERT, FRAG, "mix")
}

/// One fold of the dissolve average into `dst_fbo`.
#[allow(clippy::too_many_arguments)]
fn draw_mix_pass(
    gl: &glow::Context,
    program: glow::Program,
    vao: glow::VertexArray,
    a: glow::Texture,
    b: glow::Texture,
    t: f32,
    w: u32,
    h: u32,
    dst_fbo: glow::Framebuffer,
) {
    unsafe {
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(dst_fbo));
        gl.viewport(0, 0, w as i32, h as i32);
        gl.disable(glow::BLEND);
        gl.use_program(Some(program));
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(a));
        gl.active_texture(glow::TEXTURE1);
        gl.bind_texture(glow::TEXTURE_2D, Some(b));
        if let Some(loc) = gl.get_uniform_location(program, "u_a") {
            gl.uniform_1_i32(Some(&loc), 0);
        }
        if let Some(loc) = gl.get_uniform_location(program, "u_b") {
            gl.uniform_1_i32(Some(&loc), 1);
        }
        if let Some(loc) = gl.get_uniform_location(program, "u_t") {
            gl.uniform_1_f32(Some(&loc), t);
        }
        gl.bind_vertex_array(Some(vao));
        gl.draw_arrays(glow::TRIANGLES, 0, 3);
        gl.bind_vertex_array(None);
        gl.active_texture(glow::TEXTURE1);
        gl.bind_texture(glow::TEXTURE_2D, None);
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, None);
        gl.use_program(None);
    }
}

/// Blit the final composite to the window's default framebuffer.
fn draw_blit_pass(
    gl: &glow::Context,
    program: glow::Program,
    vao: glow::VertexArray,
    tex: glow::Texture,
    w: u32,
    h: u32,
) {
    unsafe {
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        gl.viewport(0, 0, w as i32, h as i32);
        gl.disable(glow::BLEND);
        gl.use_program(Some(program));
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        if let Some(loc) = gl.get_uniform_location(program, "u_tex") {
            gl.uniform_1_i32(Some(&loc), 0);
        }
        gl.bind_vertex_array(Some(vao));
        gl.draw_arrays(glow::TRIANGLES, 0, 3);
        gl.bind_vertex_array(None);
        gl.bind_texture(glow::TEXTURE_2D, None);
        gl.use_program(None);
    }
}

/// Draw the warp pass into the window's default framebuffer.
fn draw_warp_pass(
    gl: &glow::Context,
    program: glow::Program,
    vao: glow::VertexArray,
    tex: glow::Texture,
    hinv: &[f32; 9],
    w: u32,
    h: u32,
) {
    unsafe {
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        gl.viewport(0, 0, w as i32, h as i32);
        gl.disable(glow::BLEND);
        gl.use_program(Some(program));
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        if let Some(loc) = gl.get_uniform_location(program, "u_tex") {
            gl.uniform_1_i32(Some(&loc), 0);
        }
        if let Some(loc) = gl.get_uniform_location(program, "u_hinv") {
            // Our matrix is row-major; transpose=true converts for GLSL.
            gl.uniform_matrix_3_f32_slice(Some(&loc), true, hinv);
        }
        if let Some(loc) = gl.get_uniform_location(program, "u_size") {
            gl.uniform_2_f32(Some(&loc), w as f32, h as f32);
        }
        gl.bind_vertex_array(Some(vao));
        gl.draw_arrays(glow::TRIANGLES, 0, 3);
        gl.bind_vertex_array(None);
        gl.bind_texture(glow::TEXTURE_2D, None);
        gl.use_program(None);
    }
}
