//! Unified OpenGL Render API output path.
//!
//! Drives mpv with `vo=libmpv` and renders each frame into the default
//! framebuffer of an OS window via `glutin` (OpenGL Core) + `mpv_render_context`.
//! A fullscreen black quad handles fade-to-black.  The render loop and the GL
//! fade are identical on every OS — only native window creation differs.
//!
//! ## Window creation
//!
//! - **Windows / Linux** — `winit 0.30` creates the `winit::window::Window` from a
//!   background thread (stored as `Arc<Window>` in `GL_WINDOW`).
//! - **macOS** — winit cannot be used: its EventLoop demands the AppKit main thread,
//!   which Tauri's `NSApplication` already owns.  Instead `macos_window.rs` creates
//!   and drives an `NSWindow` directly via `objc2` (`super::macos_window`).
//!
//! In both cases creation yields a raw window/display handle pair, which the render
//! thread turns into a `glutin` GL context + `mpv_render_context`.
//!
//! ## Thread model
//!
//! | Thread                    | Role |
//! |---------------------------|------|
//! | `wincue-output-window`    | (Windows/Linux only) winit EventLoop + window events |
//! | `wincue-output-render`    | glutin context + mpv RenderContext + render loop |
//! | `wincue-output-mpv-events`| mpv_wait_event (PLAYBACK_RESTART, EOF, …) |

use std::ffi::{CStr, CString, c_void};
use std::num::NonZeroU32;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
// `Instant` is only used by the winit event-loop window backend (drag / double-click
// timing); macOS uses the AppKit backend instead and never touches it.
#[cfg(not(target_os = "macos"))]
use std::time::Instant;

use anyhow::{anyhow, Result};
use glow::HasContext;
use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextApi, ContextAttributesBuilder, NotCurrentGlContext, Version};
use glutin::display::{Display, DisplayApiPreference, GlDisplay};
use glutin::surface::{GlSurface, SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

// winit-based window backend (Windows + Linux only).
#[cfg(not(target_os = "macos"))]
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
#[cfg(not(target_os = "macos"))]
use winit::application::ApplicationHandler;
#[cfg(not(target_os = "macos"))]
use winit::dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize};
#[cfg(not(target_os = "macos"))]
use winit::event::{ElementState, MouseButton, WindowEvent};
#[cfg(not(target_os = "macos"))]
use winit::event_loop::{ActiveEventLoop, EventLoop};
#[cfg(not(target_os = "macos"))]
use winit::window::{Fullscreen, Window, WindowAttributes, WindowId};

use crate::engine::mpv_sys::{
    MpvLib, MpvOpenglFbo, MpvOpenglInitParams, MpvRenderParam,
    MPV_RENDER_PARAM_API_TYPE, MPV_RENDER_PARAM_FLIP_Y,
    MPV_RENDER_PARAM_OPENGL_FBO, MPV_RENDER_PARAM_OPENGL_INIT_PARAMS,
    MPV_RENDER_UPDATE_FRAME,
};
use super::types::MpvCtx;
use super::FADE_STATE;
use super::fade;

// ---------------------------------------------------------------------------
// Globals
// ---------------------------------------------------------------------------

/// Wakes the render thread when mpv signals a new frame is available.
pub(super) static RENDER_SIGNAL: OnceLock<Arc<(Mutex<bool>, Condvar)>> = OnceLock::new();

/// The winit output window, shared between the event-loop thread, the render
/// thread, and `OutputEngine` methods (show/hide/position/fullscreen).
/// macOS holds its `NSWindow` inside `macos_window` instead.
#[cfg(not(target_os = "macos"))]
pub(super) static GL_WINDOW: OnceLock<Arc<winit::window::Window>> = OnceLock::new();

/// Current window dimensions in physical pixels, written on resize / screen move
/// and read by the render thread to call `surface.resize()`.
static GL_WIDTH:  AtomicU32 = AtomicU32::new(1920);
static GL_HEIGHT: AtomicU32 = AtomicU32::new(1080);

// ---------------------------------------------------------------------------
// Public helpers called from OutputEngine
// ---------------------------------------------------------------------------

/// Wake the render thread immediately.
///
/// `tick_fade()` self-paces at 16 ms only while an animation is in progress
/// (`current_alpha != target_alpha`).  When a Fade Cue drives the overlay alpha
/// externally at 30 fps — setting `current == target` each step — the loop would
/// otherwise sleep up to 100 ms between redraws.  Calling this on each alpha
/// change keeps that fade smooth.
pub(super) fn wake() {
    if let Some(sig) = RENDER_SIGNAL.get() {
        if let Ok(mut r) = sig.0.lock() {
            *r = true;
            sig.1.notify_one();
        }
    }
}

/// Store new physical window dimensions and wake the render thread so it resizes
/// the GL surface.  Called by the macOS window backend after a screen move /
/// fullscreen toggle (the winit path drives this from its own `Resized` event).
#[cfg(target_os = "macos")]
pub(super) fn set_surface_size(width: u32, height: u32) {
    GL_WIDTH.store(width.max(1), Ordering::Relaxed);
    GL_HEIGHT.store(height.max(1), Ordering::Relaxed);
    wake();
}

pub(super) fn show() {
    #[cfg(not(target_os = "macos"))]
    if let Some(w) = GL_WINDOW.get() { w.set_visible(true); }
    #[cfg(target_os = "macos")]
    super::macos_window::show();
}

pub(super) fn hide() {
    #[cfg(not(target_os = "macos"))]
    if let Some(w) = GL_WINDOW.get() { w.set_visible(false); }
    #[cfg(target_os = "macos")]
    super::macos_window::hide();
}

pub(super) fn toggle_fullscreen() {
    #[cfg(not(target_os = "macos"))]
    if let Some(w) = GL_WINDOW.get() {
        if w.fullscreen().is_some() {
            w.set_fullscreen(None);
        } else {
            w.set_fullscreen(Some(Fullscreen::Borderless(w.current_monitor())));
        }
    }
    #[cfg(target_os = "macos")]
    super::macos_window::toggle_fullscreen();
}

/// Move/resize the winit window to `(x, y, width, height)` in logical pixels.
#[cfg(not(target_os = "macos"))]
pub(super) fn set_outer_rect(x: i32, y: i32, width: u32, height: u32) {
    if let Some(w) = GL_WINDOW.get() {
        w.set_outer_position(LogicalPosition::new(x, y));
        let _ = w.request_inner_size(LogicalSize::new(width, height));
    }
}

/// Place the macOS NSWindow fullscreen onto the given screen index.
#[cfg(target_os = "macos")]
pub(super) fn position_on_screen(screen_index: u32) {
    super::macos_window::position_on_screen(screen_index);
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Create the output window and spawn the render thread.
///
/// Blocks until `mpv_render_context_create()` succeeds so that no `loadfile`
/// can reach mpv before the render context is live.
pub(super) fn init(
    app_handle: &tauri::AppHandle,
    lib: Arc<MpvLib>,
    mpv_ctx: Arc<MpvCtx>,
) -> Result<()> {
    RENDER_SIGNAL.get_or_init(|| Arc::new((Mutex::new(false), Condvar::new())));
    let (rwh, rdh, width, height) = create_native_window(app_handle)?;
    GL_WIDTH.store(width.max(1), Ordering::Relaxed);
    GL_HEIGHT.store(height.max(1), Ordering::Relaxed);

    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();

    spawn_render_thread(
        SendableHandles { rwh, rdh, width, height },
        lib, mpv_ctx, ready_tx,
    )?;

    // On macOS, Tauri's NSApplication event loop hasn't started yet when setup()
    // runs. If glutin/CGL needs the run loop during context creation, blocking
    // here deadlocks: setup() waits for the render thread, the render thread
    // waits for the run loop, the run loop waits for setup() to return.
    // Solution: let the render thread initialise after the event loop starts and
    // watch for errors on a background watcher thread.
    #[cfg(target_os = "macos")]
    std::thread::Builder::new()
        .name("wincue-render-watcher".into())
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
// Sendable raw-handle pair
// ---------------------------------------------------------------------------

struct SendableHandles {
    rwh:    RawWindowHandle,
    rdh:    RawDisplayHandle,
    width:  u32,
    height: u32,
}
// SAFETY: RawWindowHandle / RawDisplayHandle are plain integer/pointer structs.
// The underlying OS objects outlive the render thread (window lives for the app).
unsafe impl Send for SendableHandles {}

// ---------------------------------------------------------------------------
// Window creation — macOS (AppKit NSWindow via objc2)
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
fn create_native_window(
    app_handle: &tauri::AppHandle,
) -> Result<(RawWindowHandle, RawDisplayHandle, u32, u32)> {
    super::macos_window::create(app_handle)
}

// ---------------------------------------------------------------------------
// Window creation — winit (Windows + Linux)
// ---------------------------------------------------------------------------

/// Resize direction from cursor position relative to window size.
#[cfg(not(target_os = "macos"))]
fn resize_direction(
    pos:    PhysicalPosition<f64>,
    size:   PhysicalSize<u32>,
    border: f64,
) -> Option<winit::window::ResizeDirection> {
    use winit::window::ResizeDirection::*;
    let (x, y)   = (pos.x, pos.y);
    let (w, h)   = (size.width as f64, size.height as f64);
    let left     = x < border;
    let right    = x > w - border;
    let top      = y < border;
    let bottom   = y > h - border;
    match (top, bottom, left, right) {
        (true,  _,     true,  _    ) => Some(NorthWest),
        (true,  _,     _,     true ) => Some(NorthEast),
        (_,     true,  true,  _    ) => Some(SouthWest),
        (_,     true,  _,     true ) => Some(SouthEast),
        (true,  _,     _,     _    ) => Some(North),
        (_,     true,  _,     _    ) => Some(South),
        (_,     _,     true,  _    ) => Some(West),
        (_,     _,     _,     true ) => Some(East),
        _                            => None,
    }
}

#[cfg(not(target_os = "macos"))]
fn resize_cursor(dir: Option<winit::window::ResizeDirection>) -> winit::window::CursorIcon {
    use winit::window::{CursorIcon::*, ResizeDirection::*};
    match dir {
        Some(North)     => NResize,
        Some(South)     => SResize,
        Some(East)      => EResize,
        Some(West)      => WResize,
        Some(NorthEast) => NeResize,
        Some(NorthWest) => NwResize,
        Some(SouthEast) => SeResize,
        Some(SouthWest) => SwResize,
        None            => Default,
    }
}

// ---------------------------------------------------------------------------
// winit ApplicationHandler — output window event loop
// ---------------------------------------------------------------------------

#[cfg(not(target_os = "macos"))]
struct OutputApp {
    /// One-shot sender: signals create_native_window() when the window is ready.
    tx:         Option<std::sync::mpsc::Sender<Result<SendableHandles>>>,
    window:     Option<Arc<Window>>,
    cursor_pos: PhysicalPosition<f64>,
    last_click: Option<Instant>,
}

#[cfg(not(target_os = "macos"))]
impl ApplicationHandler for OutputApp {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() { return; }

        let attrs = WindowAttributes::default()
            .with_title("WinCue Output")
            .with_visible(false)
            .with_decorations(false)
            .with_resizable(true)
            .with_inner_size(LogicalSize::new(1920u32, 1080u32));

        let window = match el.create_window(attrs) {
            Ok(w)  => Arc::new(w),
            Err(e) => {
                if let Some(tx) = self.tx.take() { let _ = tx.send(Err(anyhow!("create_window: {e}"))); }
                return;
            }
        };

        let rwh: RawWindowHandle = match window.window_handle() {
            Ok(h)  => h.as_raw(),
            Err(e) => {
                if let Some(tx) = self.tx.take() { let _ = tx.send(Err(anyhow!("window_handle: {e}"))); }
                return;
            }
        };
        let rdh: RawDisplayHandle = match el.display_handle() {
            Ok(h)  => h.as_raw(),
            Err(e) => {
                if let Some(tx) = self.tx.take() { let _ = tx.send(Err(anyhow!("display_handle: {e}"))); }
                return;
            }
        };

        GL_WINDOW.get_or_init(|| Arc::clone(&window));
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Ok(SendableHandles { rwh, rdh, width: 1920, height: 1080 }));
        }
        self.window = Some(window);
    }

    fn window_event(&mut self, _el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(window) = &self.window else { return; };
        match event {
            WindowEvent::CloseRequested => {
                window.set_visible(false);
            }

            WindowEvent::Resized(size) => {
                GL_WIDTH.store(size.width.max(1), Ordering::Relaxed);
                GL_HEIGHT.store(size.height.max(1), Ordering::Relaxed);
                if let Some(sig) = RENDER_SIGNAL.get() {
                    if let Ok(mut r) = sig.0.lock() { *r = true; sig.1.notify_one(); }
                }
            }

            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_pos = position;
                let dir = resize_direction(position, window.inner_size(), 8.0);
                window.set_cursor(resize_cursor(dir));
            }

            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left, ..
            } => {
                let dir = resize_direction(self.cursor_pos, window.inner_size(), 8.0);
                if let Some(d) = dir {
                    let _ = window.drag_resize_window(d);
                } else {
                    let now = Instant::now();
                    let is_double = self.last_click
                        .map(|t| now.duration_since(t) < Duration::from_millis(300))
                        .unwrap_or(false);
                    if is_double {
                        if window.fullscreen().is_some() {
                            window.set_fullscreen(None);
                        } else {
                            window.set_fullscreen(Some(Fullscreen::Borderless(window.current_monitor())));
                        }
                        self.last_click = None;
                    } else {
                        self.last_click = Some(now);
                        let _ = window.drag_window();
                    }
                }
            }

            _ => {}
        }
    }
}

/// Build a winit EventLoop that may be created from any thread.
///
/// winit 0.30 guards EventLoop creation to the main thread by default on both
/// Windows and Linux.  Platform-specific extension traits opt out of that guard.
#[cfg(target_os = "windows")]
fn build_event_loop() -> Result<EventLoop<()>> {
    use winit::platform::windows::EventLoopBuilderExtWindows;
    EventLoop::builder()
        .with_any_thread(true)
        .build()
        .map_err(|e| anyhow!("EventLoop (Windows): {e}"))
}

#[cfg(target_os = "linux")]
fn build_event_loop() -> Result<EventLoop<()>> {
    use winit::platform::x11::EventLoopBuilderExtX11;
    EventLoop::builder()
        .with_any_thread(true)
        .build()
        .map_err(|e| anyhow!("EventLoop (Linux/X11): {e}"))
}

/// Unified window creation for Windows and Linux via winit.
#[cfg(not(target_os = "macos"))]
fn create_native_window(
    _app_handle: &tauri::AppHandle,
) -> Result<(RawWindowHandle, RawDisplayHandle, u32, u32)> {
    let (tx, rx) = std::sync::mpsc::channel::<Result<SendableHandles>>();

    std::thread::Builder::new()
        .name("wincue-output-window".into())
        .spawn(move || {
            let event_loop = match build_event_loop() {
                Ok(el) => el,
                Err(e) => { let _ = tx.send(Err(anyhow!("{e}"))); return; }
            };
            // Clone before moving into OutputApp so we can report panics or
            // early exit (run_app returning without resumed() ever being called).
            let tx_err = tx.clone();
            let mut app = OutputApp {
                tx:         Some(tx),
                window:     None,
                cursor_pos: PhysicalPosition::new(0.0, 0.0),
                last_click: None,
            };
            let result = std::panic::catch_unwind(
                std::panic::AssertUnwindSafe(|| event_loop.run_app(&mut app))
            );
            match result {
                Err(_) => {
                    let _ = tx_err.send(Err(anyhow!(
                        "output window thread panicked (no display server?)"
                    )));
                }
                Ok(_) if app.tx.is_some() => {
                    // run_app returned normally but resumed() was never called.
                    let _ = tx_err.send(Err(anyhow!(
                        "event loop exited before window was created \
                         (no X11/Wayland display available?)"
                    )));
                }
                Ok(_) => {}
            }
        })
        .map_err(|e| anyhow!("spawn output-window thread: {e}"))?;

    let h = rx.recv()??;
    Ok((h.rwh, h.rdh, h.width, h.height))
}

// ---------------------------------------------------------------------------
// Spawn render thread
// ---------------------------------------------------------------------------

fn spawn_render_thread(
    handles:  SendableHandles,
    lib:      Arc<MpvLib>,
    mpv_ctx:  Arc<MpvCtx>,
    ready_tx: std::sync::mpsc::Sender<Result<()>>,
) -> Result<()> {
    std::thread::Builder::new()
        .name("wincue-output-render".into())
        .spawn(move || {
            if let Err(e) = render_thread_main(handles, lib, mpv_ctx, ready_tx) {
                log::error!("[render] fatal: {e}");
            }
        })
        .map_err(|e| anyhow!("spawn render thread: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Render thread
// ---------------------------------------------------------------------------

fn render_thread_main(
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
    let config_tpl = ConfigTemplateBuilder::new()
        .compatible_with_native_window(handles.rwh)
        .with_alpha_size(8)
        .build();
    let config = try_init!(unsafe {
        display.find_configs(config_tpl)
            .map_err(|e| anyhow!("find_configs: {e}"))?
            .next()
            .ok_or_else(|| anyhow!("no compatible GL config found"))
    });

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
    // mpv's own clock (video-sync=desync) paces playback, not our swap — blocking
    // on the driver's vblank here just adds a second, redundant sync point. Under
    // a compositor with a virtualized/emulated vblank (VMs), that block can stall
    // long enough to visibly stutter the whole desktop, not just this window.
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

    // ── 8. Fade-quad shader ───────────────────────────────────────────────────
    let (fade_program, fade_vao) = build_fade_shader(&gl)?;

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
    let signal_ptr = RENDER_SIGNAL.get().map(Arc::as_ptr).unwrap_or(std::ptr::null()) as *mut c_void;
    unsafe { (lib.mpv_render_context_set_update_callback)(render_ctx, Some(on_mpv_update), signal_ptr); }

    // ── 11. Render loop ───────────────────────────────────────────────────────
    let signal = RENDER_SIGNAL.get().expect("RENDER_SIGNAL not set");
    let (lock, cvar) = signal.as_ref();
    let mut w_px = handles.width;
    let mut h_px = handles.height;

    loop {
        let needs_animation = FADE_STATE.get()
            .and_then(|fs| fs.lock().ok())
            .map(|s| s.current_alpha != s.target_alpha)
            .unwrap_or(false);
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
        let new_w = GL_WIDTH.load(Ordering::Relaxed).max(1);
        let new_h = GL_HEIGHT.load(Ordering::Relaxed).max(1);
        if new_w != w_px || new_h != h_px {
            surface.resize(
                &ctx,
                NonZeroU32::new(new_w).unwrap(),
                NonZeroU32::new(new_h).unwrap(),
            );
            w_px = new_w;
            h_px = new_h;
        }

        let (alpha, done) = fade::tick_fade();
        if done { fade::execute_pending(); }

        let flags     = unsafe { (lib.mpv_render_context_update)(render_ctx) };
        let has_frame = flags & MPV_RENDER_UPDATE_FRAME != 0;
        if !has_frame && alpha == 0 { continue; }

        let mut fbo = MpvOpenglFbo { fbo: 0, w: w_px as i32, h: h_px as i32, internal_format: 0 };
        let mut flip = flip_y;
        let rp = [
            MpvRenderParam { type_: MPV_RENDER_PARAM_OPENGL_FBO, data: &mut fbo  as *mut _ as *mut c_void },
            MpvRenderParam { type_: MPV_RENDER_PARAM_FLIP_Y,     data: &mut flip as *mut _ as *mut c_void },
            MpvRenderParam { type_: 0, data: std::ptr::null_mut() },
        ];
        let ret = unsafe { (lib.mpv_render_context_render)(render_ctx, rp.as_ptr()) };
        if ret < 0 { log::warn!("[render] mpv_render_context_render: {ret}"); }

        if alpha > 0 { draw_fade_quad(&gl, fade_program, fade_vao, alpha as f32 / 255.0); }

        if let Err(e) = surface.swap_buffers(&ctx) { log::warn!("[render] swap: {e:?}"); }
        unsafe { (lib.mpv_render_context_report_swap)(render_ctx); }
    }
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
