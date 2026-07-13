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
//! | `inkue-output-window`    | (Windows/Linux only) winit EventLoop + window events |
//! | `inkue-output-render`    | glutin context + mpv RenderContext + render loop |
//! | `inkue-output-mpv-events`| mpv_wait_event (PLAYBACK_RESTART, EOF, …) |

use std::ffi::{CStr, CString, c_void};
use std::num::NonZeroU32;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
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
use winit::keyboard::{Key, ModifiersState, NamedKey};
#[cfg(not(target_os = "macos"))]
use winit::window::{Fullscreen, Window, WindowAttributes, WindowId};

use crate::engine::mpv_sys::{
    MpvLib, MpvOpenglFbo, MpvOpenglInitParams, MpvRenderParam,
    MPV_RENDER_PARAM_API_TYPE, MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME,
    MPV_RENDER_PARAM_FLIP_Y, MPV_RENDER_PARAM_OPENGL_FBO,
    MPV_RENDER_PARAM_OPENGL_INIT_PARAMS, MPV_RENDER_UPDATE_FRAME,
};
use super::types::MpvCtx;
use super::FADE_STATE;
use super::fade;
use super::slot;

// ---------------------------------------------------------------------------
// Globals
// ---------------------------------------------------------------------------

/// Wakes the render thread when mpv signals a new frame is available.
pub(super) static RENDER_SIGNAL: OnceLock<Arc<(Mutex<bool>, Condvar)>> = OnceLock::new();

/// Set to `true` while a Text Cue overlay is active.
///
/// When set, the render loop does **not** skip on `!has_frame && alpha==0` so
/// that the Text Cue's `osd-overlay` ASS is composited and displayed even in
/// idle mode (mpv does not signal `MPV_RENDER_UPDATE_FRAME` for OSD-only changes).
pub(super) static TEXT_OVERLAY_ACTIVE: AtomicBool = AtomicBool::new(false);

/// `true` while the output window is user-visible.
///
/// Set by `show()` and cleared by `hide()`.  The render loop must NOT commit
/// frames before this flag is set: on Wayland a `wl_surface.commit()` with a
/// buffer permanently maps the surface (the window appears), so emitting even
/// one frame while the window is "hidden" makes it visible at startup before
/// the operator opens it.
pub(super) static OUTPUT_VISIBLE: AtomicBool = AtomicBool::new(false);

/// The winit output window, shared between the event-loop thread, the render
/// thread, and `OutputEngine` methods (show/hide/position/fullscreen).
/// macOS holds its `NSWindow` inside `macos_window` instead.
#[cfg(not(target_os = "macos"))]
pub(super) static GL_WINDOW: OnceLock<Arc<winit::window::Window>> = OnceLock::new();

/// Current window dimensions in physical pixels, written on resize / screen move
/// and read by the render thread to call `surface.resize()`.
static GL_WIDTH:  AtomicU32 = AtomicU32::new(1920);
static GL_HEIGHT: AtomicU32 = AtomicU32::new(1080);

/// Inverse homography for the global output warp (corner pin / fine rotation),
/// row-major.  `None` = identity: mpv renders straight into the window's
/// default framebuffer with zero extra cost.  Set via [`set_output_warp`].
static OUTPUT_WARP: Mutex<Option<[f32; 9]>> = Mutex::new(None);
/// One-shot "warp params changed" flag: forces a redraw even when mpv has no
/// new frame (paused video, held image), so edits in the alignment editor are
/// visible immediately.
static WARP_DIRTY: AtomicBool = AtomicBool::new(false);
/// One-shot "overlay went inactive" flag: forces one redraw so the last
/// composited overlay image (timer text, cleared pattern) leaves the screen
/// even when no layer is animating and mpv signals no new frame.
static OVERLAY_DIRTY: AtomicBool = AtomicBool::new(false);

/// Force one redraw after an overlay deactivation (timer cleared, Text Cue
/// ended, test pattern cleared).
pub(super) fn mark_overlay_dirty() {
    OVERLAY_DIRTY.store(true, Ordering::Relaxed);
    wake();
}

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
    OUTPUT_VISIBLE.store(true, Ordering::Relaxed);
    #[cfg(not(target_os = "macos"))]
    if let Some(w) = GL_WINDOW.get() { w.set_visible(true); }
    #[cfg(target_os = "macos")]
    super::macos_window::show();
    // Wake the render loop so it commits the first frame immediately.  On
    // Wayland the surface is only mapped once a buffer arrives; without this
    // wake the window would not appear until the next mpv signal (up to 100 ms).
    wake();
}

pub(super) fn hide() {
    OUTPUT_VISIBLE.store(false, Ordering::Relaxed);
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

/// Place the output window fullscreen on the monitor whose top-left corner is
/// `(x, y)` — **physical** virtual-screen coordinates from `list_screens()`.
///
/// Uses `Fullscreen::Borderless` on the matched `MonitorHandle` rather than a
/// manual move/resize: the old path passed the physical rect as a *logical*
/// position, which winit multiplies by the current monitor's DPI scale — with
/// any display above 100 % the window landed shifted and oversized (the
/// "output drifts on GO" report). Borderless fullscreen is DPI-proof, covers
/// the taskbar, pins the window to the monitor, and works on Wayland where
/// `set_outer_position` is a no-op.
#[cfg(not(target_os = "macos"))]
pub(super) fn set_fullscreen_on_rect(x: i32, y: i32, width: u32, height: u32) {
    let Some(w) = GL_WINDOW.get() else { return };
    let monitor = w.available_monitors().find(|m| {
        let p = m.position();
        p.x == x && p.y == y
    });
    match monitor {
        Some(m) => w.set_fullscreen(Some(Fullscreen::Borderless(Some(m)))),
        None => {
            // The compositor reported different coordinates than list_screens()
            // (possible on Wayland). Land on the rect in physical pixels, then
            // fullscreen whatever monitor the window ended up on.
            w.set_fullscreen(None);
            w.set_outer_position(PhysicalPosition::new(x, y));
            let _ = w.request_inner_size(PhysicalSize::new(width, height));
            w.set_fullscreen(Some(Fullscreen::Borderless(None)));
        }
    }
}

/// Place the macOS NSWindow fullscreen onto the given screen index.
#[cfg(target_os = "macos")]
pub(super) fn position_on_screen(screen_index: u32) {
    super::macos_window::position_on_screen(screen_index);
}

/// Install (or clear) the global output warp and wake the render thread so
/// the change shows immediately — even on a paused frame or a test pattern.
pub(super) fn set_output_warp(matrix: Option<[f32; 9]>) {
    if let Ok(mut w) = OUTPUT_WARP.lock() {
        *w = matrix;
    }
    WARP_DIRTY.store(true, Ordering::Relaxed);
    wake();
}

/// Restore the output window to a floating windowed rect — exits the
/// fullscreen-on-screen placement applied by `set_outer_rect` /
/// `position_on_screen` when the operator switches back to "Floating window".
pub(super) fn set_windowed_floating() {
    #[cfg(not(target_os = "macos"))]
    if let Some(w) = GL_WINDOW.get() {
        w.set_fullscreen(None);
        w.set_outer_position(LogicalPosition::new(100, 100));
        let _ = w.request_inner_size(LogicalSize::new(1280u32, 720u32));
    }
    #[cfg(target_os = "macos")]
    super::macos_window::set_windowed();
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
    /// Emits `output-keydown` so shortcuts keep working with the output focused.
    app_handle: tauri::AppHandle,
    modifiers:  ModifiersState,
}

/// Translate a winit logical key to the DOM `KeyboardEvent.key` string the
/// frontend shortcut handler expects. winit's `NamedKey` variants are named
/// after the DOM UI Events key values, so `Debug` *is* the mapping — except
/// `Space`, which the DOM spells `" "`.
#[cfg(not(target_os = "macos"))]
fn dom_key(key: &Key) -> Option<String> {
    match key {
        Key::Character(c) => Some(c.to_string()),
        Key::Named(NamedKey::Space) => Some(" ".into()),
        Key::Named(n) => Some(format!("{n:?}")),
        _ => None,
    }
}

#[cfg(not(target_os = "macos"))]
impl ApplicationHandler for OutputApp {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() { return; }

        let attrs = WindowAttributes::default()
            .with_title("Inkue Output")
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

            WindowEvent::ModifiersChanged(m) => {
                self.modifiers = m.state();
            }

            WindowEvent::KeyboardInput { event, is_synthetic, .. } => {
                // The output window would swallow these otherwise — GO / panic
                // must keep working while the operator has it focused. Forward
                // to the main webview, which replays them into the regular
                // window-level shortcut handler (repeats included, matching
                // native DOM keydown behaviour).
                //
                // `is_synthetic` must be skipped: on Windows, winit fabricates
                // Pressed events for every key physically held when the window
                // gains focus — F9 (show output) activates this window while
                // F9 is still down, and forwarding that ghost press would
                // instantly toggle the window hidden again.
                if event.state == ElementState::Pressed && !is_synthetic {
                    if let Some(key) = dom_key(&event.logical_key) {
                        use tauri::Emitter;
                        let _ = self.app_handle.emit(
                            "output-keydown",
                            serde_json::json!({
                                "key":   key,
                                "ctrl":  self.modifiers.control_key(),
                                "alt":   self.modifiers.alt_key(),
                                "shift": self.modifiers.shift_key(),
                                "meta":  self.modifiers.super_key(),
                            }),
                        );
                    }
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

/// Probe whether winit's X11 backend can actually run.
///
/// winit's X11 backend hard-requires `libxkbcommon-x11` and **panics** (not a
/// recoverable `build()` error) during window creation if it is absent — common on
/// Wayland-only installs.  We `dlopen` it up-front so `build_event_loop` can choose
/// Wayland cleanly instead of taking down the whole output engine.
#[cfg(target_os = "linux")]
fn x11_xkb_available() -> bool {
    use std::ffi::CString;
    for name in ["libxkbcommon-x11.so.0", "libxkbcommon-x11.so"] {
        let Ok(c) = CString::new(name) else { continue };
        // SAFETY: valid C string; the handle is closed again immediately.
        let h = unsafe { libc::dlopen(c.as_ptr(), libc::RTLD_LAZY) };
        if !h.is_null() {
            unsafe { libc::dlclose(h); }
            return true;
        }
    }
    false
}

#[cfg(target_os = "linux")]
fn build_event_loop() -> Result<EventLoop<()>> {
    // Prefer X11/XWayland over native Wayland for the output window.
    //
    // With a *native Wayland* EGL surface, Mesa's `eglSwapBuffers` blocks on the
    // compositor's frame callback regardless of the swap interval, which serialises
    // this output window's render-thread GL with WebKitGTK's UI compositing on the
    // same iGPU — the Inkue UI then crawls for the entire duration of video playback
    // (the failure the operator reported).  XWayland's X11/DRI EGL path honours
    // `SwapInterval::DontWait` and keeps the two GL clients decoupled, so the UI stays
    // fluid while a video plays.
    //
    // X11 is selected only when `libxkbcommon-x11` is present (winit panics otherwise);
    // otherwise we fall back to native Wayland so the app still runs.  Override with
    // `INKUE_OUTPUT_BACKEND=wayland` for A/B testing.
    let force_wayland = std::env::var("INKUE_OUTPUT_BACKEND").as_deref() == Ok("wayland");
    let use_x11 = !force_wayland && x11_xkb_available();

    let mut b = EventLoop::builder();
    if use_x11 {
        use winit::platform::x11::EventLoopBuilderExtX11;
        b.with_any_thread(true).with_x11();
        log::info!("[render] output window backend: X11/XWayland (default)");
        b.build().map_err(|e| anyhow!("EventLoop (Linux/XWayland): {e}"))
    } else {
        use winit::platform::wayland::EventLoopBuilderExtWayland;
        EventLoopBuilderExtWayland::with_any_thread(&mut b, true);
        if force_wayland {
            log::info!("[render] output window backend: native Wayland (forced via INKUE_OUTPUT_BACKEND)");
        } else {
            log::warn!(
                "[render] output window backend: native Wayland — XWayland unavailable \
                 (libxkbcommon-x11 not found); the UI may lag during video playback. \
                 Install the 'libxkbcommon-x11-0' package to enable the smoother XWayland path."
            );
        }
        b.build().map_err(|e| anyhow!("EventLoop (Linux/Wayland): {e}"))
    }
}

/// Unified window creation for Windows and Linux via winit.
#[cfg(not(target_os = "macos"))]
fn create_native_window(
    app_handle: &tauri::AppHandle,
) -> Result<(RawWindowHandle, RawDisplayHandle, u32, u32)> {
    let (tx, rx) = std::sync::mpsc::channel::<Result<SendableHandles>>();
    let app_handle = app_handle.clone();

    std::thread::Builder::new()
        .name("inkue-output-window".into())
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
                app_handle,
                modifiers:  ModifiersState::empty(),
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
        .name("inkue-output-render".into())
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
    let signal_ptr = RENDER_SIGNAL.get().map(Arc::as_ptr).unwrap_or(std::ptr::null()) as *mut c_void;
    unsafe { (lib.mpv_render_context_set_update_callback)(render_ctx, Some(on_mpv_update), signal_ptr); }

    // ── 11. Render loop ───────────────────────────────────────────────────────
    let signal = RENDER_SIGNAL.get().expect("RENDER_SIGNAL not set");
    let (lock, cvar) = signal.as_ref();
    let mut w_px = handles.width;
    let mut h_px = handles.height;

    // Layer compositor state: the overlay context (timer OSD / Text Cue /
    // test patterns) renders into its own target like every video slot; the
    // ping-pong pair accumulates the blend stack.
    let (composite_program, composite_vao) = build_composite_shader(&gl)?;
    let (blit_program, blit_vao) = build_blit_shader(&gl)?;
    let mut overlay_target: Option<WarpTarget> = None;
    let mut slot_targets: Vec<Option<WarpTarget>> = Vec::new();
    let mut slot_valid: Vec<bool> = Vec::new();
    let mut pingpong: [Option<WarpTarget>; 2] = [None, None];

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
        // Create render contexts for slots the engine spawned since last pass.
        let slots = slot_snapshot();
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

        // Per-slot opacity animations pace the loop at 16 ms just like the
        // master fade.
        let master_animating = FADE_STATE.get()
            .and_then(|fs| fs.lock().ok())
            .map(|s| s.current_alpha != s.target_alpha)
            .unwrap_or(false);
        let slots_animating = slots.iter().any(|s| {
            s.state.lock().map(|st| st.anim.is_animating()).unwrap_or(false)
        });
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

        // Overlay context (timer OSD / Text Cue / test patterns / win32 path).
        let flags     = unsafe { (lib.mpv_render_context_update)(render_ctx) };
        let has_frame = flags & MPV_RENDER_UPDATE_FRAME != 0;
        let text_active = TEXT_OVERLAY_ACTIVE.load(Ordering::Relaxed);
        // Warp params changed since the last pass — must redraw even without a
        // new mpv frame (paused video / held image), or alignment edits would
        // only show on the next frame.
        let warp_dirty = WARP_DIRTY.swap(false, Ordering::Relaxed)
            || OVERLAY_DIRTY.swap(false, Ordering::Relaxed);

        // Tick each slot: advance opacity anims, finish pending unloads, and
        // check for fresh frames.  Ticks must run even while hidden so stop
        // fades can finish, but rendering below is gated on visibility.
        let slots = slot_snapshot();
        struct LayerDraw {
            slot_index: usize,
            layer_key: u64,
            opacity: f32,
            blend_mode: i32,
            has_new_frame: bool,
            render_ctx: *mut c_void,
        }
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
            if voice.is_none() {
                if let Some(v) = slot_valid.get_mut(s.index) { *v = false; }
                continue;
            }
            any_slot_frame |= s_new_frame;
            layers.push(LayerDraw {
                slot_index: s.index,
                layer_key,
                opacity,
                blend_mode,
                has_new_frame: s_new_frame,
                render_ctx: rc,
            });
        }
        layers.sort_by_key(|l| l.layer_key);

        // Do not commit frames while the output window is hidden.  On Wayland
        // a wl_surface.commit() with a buffer permanently maps the surface, so
        // a single frame emitted before show_output() would make the window
        // appear at startup instead of staying invisible until the operator
        // opens it.  show() sets this flag and wakes the loop so the first
        // committed frame arrives immediately when the window is revealed.
        if !OUTPUT_VISIBLE.load(Ordering::Relaxed) { continue; }
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
        let overlay_on = super::overlay_active();
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

        // ── Composite the layer stack (ping-pong) ─────────────────────────────
        // Base = opaque black; each layer blends over the accumulated result.
        let mut src = 0usize; // pingpong[src] holds the accumulated composite
        unsafe {
            let base = pingpong[src].as_ref().map(|t| t.fbo);
            gl.bind_framebuffer(glow::FRAMEBUFFER, base);
            gl.viewport(0, 0, w_px as i32, h_px as i32);
            gl.clear_color(0.0, 0.0, 0.0, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
        }
        for l in &layers {
            if l.opacity <= 0.0 {
                continue;
            }
            let Some(Some(layer_t)) = slot_targets.get(l.slot_index) else { continue };
            let dst = 1 - src;
            let (backdrop_tex, dst_fbo) = match (&pingpong[src], &pingpong[dst]) {
                (Some(a), Some(b)) => (a.tex, b.fbo),
                _ => continue,
            };
            draw_composite_pass(
                &gl, composite_program, composite_vao,
                backdrop_tex, layer_t.tex, l.blend_mode, l.opacity, w_px, h_px, dst_fbo,
            );
            src = dst;
        }
        // Overlay (timer / text / patterns) on top — only while active.
        if overlay_valid {
            if let Some(t) = &overlay_target {
                let dst = 1 - src;
                if let (Some(a), Some(b)) = (&pingpong[src], &pingpong[dst]) {
                    draw_composite_pass(
                        &gl, composite_program, composite_vao,
                        a.tex, t.tex, 0, 1.0, w_px, h_px, b.fbo,
                    );
                    src = dst;
                }
            }
        }

        // ── Present: warp (or plain blit) + master fade quad ──────────────────
        let warp = OUTPUT_WARP.lock().ok().and_then(|g| *g);
        let final_tex = pingpong[src].as_ref().map(|t| t.tex);
        if let Some(tex) = final_tex {
            match warp {
                Some(hinv) => draw_warp_pass(&gl, warp_program, warp_vao, tex, &hinv, w_px, h_px),
                None => draw_blit_pass(&gl, blit_program, blit_vao, tex, w_px, h_px),
            }
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

/// Snapshot of the slot registry for one render pass.
fn slot_snapshot() -> Vec<Arc<super::slot::VideoSlot>> {
    super::slot::all_slots()
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(all(test, not(target_os = "macos")))]
mod tests {
    use super::dom_key;
    use winit::keyboard::{Key, NamedKey};

    #[test]
    fn dom_key_space_uses_dom_spelling() {
        assert_eq!(dom_key(&Key::Named(NamedKey::Space)).as_deref(), Some(" "));
    }

    #[test]
    fn dom_key_named_keys_match_dom_values() {
        for (key, dom) in [
            (NamedKey::Escape, "Escape"),
            (NamedKey::ArrowUp, "ArrowUp"),
            (NamedKey::ArrowDown, "ArrowDown"),
            (NamedKey::Delete, "Delete"),
            (NamedKey::Backspace, "Backspace"),
            (NamedKey::F5, "F5"),
            (NamedKey::F9, "F9"),
        ] {
            assert_eq!(dom_key(&Key::Named(key)).as_deref(), Some(dom));
        }
    }

    #[test]
    fn dom_key_characters_pass_through() {
        assert_eq!(dom_key(&Key::Character("s".into())).as_deref(), Some("s"));
        assert_eq!(dom_key(&Key::Character("S".into())).as_deref(), Some("S"));
        assert_eq!(dom_key(&Key::Character("[".into())).as_deref(), Some("["));
        assert_eq!(dom_key(&Key::Character(",".into())).as_deref(), Some(","));
    }

    #[test]
    fn dom_key_dead_keys_are_dropped() {
        assert_eq!(dom_key(&Key::Dead(None)), None);
    }
}
