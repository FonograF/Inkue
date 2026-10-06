//! Native output windows.
//!
//! - **Windows / Linux** — one `winit 0.30` event loop for the whole process
//!   serves every output window.  winit allows a single `EventLoop` per
//!   process, so it runs on a dedicated thread (`inkue-output-window`) and
//!   windows are created on request through an [`EventLoopProxy`].
//! - **macOS** — winit cannot be used (its `EventLoop` needs the AppKit main
//!   thread that Tauri's `NSApplication` already owns); see `macos_window.rs`,
//!   whose `NativeWindow` is re-exported here.
//!
//! Either way the backend hands the render thread a [`SendableHandles`] pair,
//! from which it builds its own `glutin` context.

#[cfg(target_os = "macos")]
pub(super) use super::macos_window::{create_window, NativeWindow};

use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

/// Raw window/display handles plus the initial size (physical pixels), handed
/// from the window backend to the render thread.
pub(super) struct SendableHandles {
    pub rwh: RawWindowHandle,
    pub rdh: RawDisplayHandle,
    pub width: u32,
    pub height: u32,
}
// SAFETY: RawWindowHandle / RawDisplayHandle are plain integer/pointer structs.
// The underlying OS objects outlive the render thread (windows live for the app).
unsafe impl Send for SendableHandles {}

#[cfg(not(target_os = "macos"))]
pub(super) use winit_backend::{create_window, NativeWindow};

/// The title of an output's window (taskbar, Alt-Tab, Mission Control).
pub(super) fn window_title(output_name: &str) -> String {
    format!("Inkue Output — {output_name}")
}

/// Whether a new output window can be built from the calling thread without
/// waiting on another one.  Always on Windows/Linux (the winit thread builds
/// it); on macOS only on the main thread — any other thread must not wait for
/// it while holding a lock the main thread might want.
pub(super) fn can_create_window_here() -> bool {
    #[cfg(target_os = "macos")]
    {
        objc2_foundation::MainThreadMarker::new().is_some()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

#[cfg(not(target_os = "macos"))]
mod winit_backend {
    use std::collections::HashMap;
    use std::sync::mpsc::{channel, Sender};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use anyhow::{anyhow, Result};
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
    use winit::application::ApplicationHandler;
    use winit::dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize};
    use winit::event::{ElementState, MouseButton, WindowEvent};
    use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
    use winit::keyboard::{Key, ModifiersState, NamedKey};
    use winit::window::{Fullscreen, Window, WindowAttributes, WindowId};

    use super::SendableHandles;
    use crate::engine::output_engine::output::Output;

    // ── The window of one output ─────────────────────────────────────────────

    /// The winit window of an [`Output`], set once its backend has created it
    /// and shared with every thread that shows, hides or places it.
    pub(in crate::engine::output_engine) struct NativeWindow {
        window: Mutex<Option<Arc<Window>>>,
    }

    impl NativeWindow {
        pub(in crate::engine::output_engine) fn new() -> Self {
            Self { window: Mutex::new(None) }
        }

        fn get(&self) -> Option<Arc<Window>> {
            self.window.lock().ok().and_then(|w| w.clone())
        }

        pub(in crate::engine::output_engine) fn show(&self) {
            if let Some(w) = self.get() {
                w.set_visible(true);
            }
        }

        pub(in crate::engine::output_engine) fn hide(&self) {
            if let Some(w) = self.get() {
                w.set_visible(false);
            }
        }

        pub(in crate::engine::output_engine) fn set_title(&self, title: &str) {
            if let Some(w) = self.get() {
                w.set_title(title);
            }
        }

        /// Close the window for good (its output is being destroyed).  The
        /// last reference is dropped on the event-loop thread, which owns it.
        pub(in crate::engine::output_engine) fn destroy(&self) {
            let Some(window) = self.window.lock().ok().and_then(|mut w| w.take()) else { return };
            window.set_visible(false);
            let proxy = LOOP_PROXY.lock().ok().and_then(|p| p.clone());
            let sent = proxy.is_some_and(|proxy| proxy.send_event(UserEvent::DestroyWindow { window }).is_ok());
            if !sent {
                log::warn!("[window] output event loop gone — window dropped off its thread");
            }
        }

        pub(in crate::engine::output_engine) fn toggle_fullscreen(&self) {
            let Some(w) = self.get() else { return };
            if w.fullscreen().is_some() {
                w.set_fullscreen(None);
            } else {
                w.set_fullscreen(Some(Fullscreen::Borderless(w.current_monitor())));
            }
        }

        /// Place the window fullscreen on the monitor whose top-left corner is
        /// `(x, y)` — **physical** virtual-screen coordinates from
        /// `list_screens()`.
        ///
        /// Uses `Fullscreen::Borderless` on the matched `MonitorHandle` rather
        /// than a manual move/resize: the old path passed the physical rect as
        /// a *logical* position, which winit multiplies by the current
        /// monitor's DPI scale — with any display above 100 % the window landed
        /// shifted and oversized (the "output drifts on GO" report).  Borderless
        /// fullscreen is DPI-proof, covers the taskbar, pins the window to the
        /// monitor, and works on Wayland where `set_outer_position` is a no-op.
        pub(in crate::engine::output_engine) fn place_on_rect(
            &self,
            x: i32,
            y: i32,
            width: u32,
            height: u32,
        ) {
            let Some(w) = self.get() else { return };
            let monitor = w.available_monitors().find(|m| {
                let p = m.position();
                p.x == x && p.y == y
            });
            match monitor {
                Some(m) => w.set_fullscreen(Some(Fullscreen::Borderless(Some(m)))),
                None => {
                    // The compositor reported different coordinates than
                    // list_screens() (possible on Wayland).  Land on the rect in
                    // physical pixels, then fullscreen whatever monitor the
                    // window ended up on.
                    w.set_fullscreen(None);
                    w.set_outer_position(PhysicalPosition::new(x, y));
                    let _ = w.request_inner_size(PhysicalSize::new(width, height));
                    w.set_fullscreen(Some(Fullscreen::Borderless(None)));
                }
            }
        }

        /// Leave fullscreen and return to a floating windowed rect.
        pub(in crate::engine::output_engine) fn set_windowed_floating(&self) {
            let Some(w) = self.get() else { return };
            w.set_fullscreen(None);
            w.set_outer_position(LogicalPosition::new(100, 100));
            let _ = w.request_inner_size(LogicalSize::new(1280u32, 720u32));
        }
    }

    // ── Event loop ───────────────────────────────────────────────────────────

    /// Requests the event-loop thread serves.
    enum UserEvent {
        CreateWindow {
            output: Arc<Output>,
            reply: Sender<Result<SendableHandles>>,
        },
        /// Forget a window and drop it here, on the thread that created it.
        DestroyWindow { window: Arc<Window> },
    }

    /// Per-window state of the event loop.
    struct WindowCtx {
        window: Arc<Window>,
        output: Arc<Output>,
        cursor_pos: PhysicalPosition<f64>,
        last_click: Option<Instant>,
    }

    struct OutputApp {
        /// Emits `output-keydown` so shortcuts keep working with an output focused.
        app_handle: tauri::AppHandle,
        modifiers: ModifiersState,
        windows: HashMap<WindowId, WindowCtx>,
        /// Requests that arrived before `resumed` (windows must not be created
        /// earlier).
        pending: Vec<(Arc<Output>, Sender<Result<SendableHandles>>)>,
        resumed: bool,
    }

    /// Resize direction from cursor position relative to window size.
    fn resize_direction(
        pos: PhysicalPosition<f64>,
        size: PhysicalSize<u32>,
        border: f64,
    ) -> Option<winit::window::ResizeDirection> {
        use winit::window::ResizeDirection::*;
        let (x, y) = (pos.x, pos.y);
        let (w, h) = (size.width as f64, size.height as f64);
        let left = x < border;
        let right = x > w - border;
        let top = y < border;
        let bottom = y > h - border;
        match (top, bottom, left, right) {
            (true, _, true, _) => Some(NorthWest),
            (true, _, _, true) => Some(NorthEast),
            (_, true, true, _) => Some(SouthWest),
            (_, true, _, true) => Some(SouthEast),
            (true, _, _, _) => Some(North),
            (_, true, _, _) => Some(South),
            (_, _, true, _) => Some(West),
            (_, _, _, true) => Some(East),
            _ => None,
        }
    }

    fn resize_cursor(dir: Option<winit::window::ResizeDirection>) -> winit::window::CursorIcon {
        use winit::window::{CursorIcon::*, ResizeDirection::*};
        match dir {
            Some(North) => NResize,
            Some(South) => SResize,
            Some(East) => EResize,
            Some(West) => WResize,
            Some(NorthEast) => NeResize,
            Some(NorthWest) => NwResize,
            Some(SouthEast) => SeResize,
            Some(SouthWest) => SwResize,
            None => Default,
        }
    }

    /// Translate a winit logical key to the DOM `KeyboardEvent.key` string the
    /// frontend shortcut handler expects. winit's `NamedKey` variants are named
    /// after the DOM UI Events key values, so `Debug` *is* the mapping — except
    /// `Space`, which the DOM spells `" "`.
    pub(super) fn dom_key(key: &Key) -> Option<String> {
        match key {
            Key::Character(c) => Some(c.to_string()),
            Key::Named(NamedKey::Space) => Some(" ".into()),
            Key::Named(n) => Some(format!("{n:?}")),
            _ => None,
        }
    }

    impl OutputApp {
        /// Create the window of `output` and answer the requester.
        fn create_window(
            &mut self,
            el: &ActiveEventLoop,
            output: Arc<Output>,
            reply: Sender<Result<SendableHandles>>,
        ) {
            let attrs = WindowAttributes::default()
                .with_title(super::window_title(&output.name()))
                .with_visible(false)
                .with_decorations(false)
                .with_resizable(true)
                .with_inner_size(LogicalSize::new(1920u32, 1080u32));

            let result = (|| -> Result<(Arc<Window>, SendableHandles)> {
                let window = Arc::new(
                    el.create_window(attrs).map_err(|e| anyhow!("create_window: {e}"))?,
                );
                let rwh = window.window_handle().map_err(|e| anyhow!("window_handle: {e}"))?.as_raw();
                let rdh = el.display_handle().map_err(|e| anyhow!("display_handle: {e}"))?.as_raw();
                Ok((window, SendableHandles { rwh, rdh, width: 1920, height: 1080 }))
            })();

            match result {
                Ok((window, handles)) => {
                    if let Ok(mut slot) = output.window.window.lock() {
                        *slot = Some(Arc::clone(&window));
                    }
                    self.windows.insert(
                        window.id(),
                        WindowCtx {
                            window,
                            output,
                            cursor_pos: PhysicalPosition::new(0.0, 0.0),
                            last_click: None,
                        },
                    );
                    let _ = reply.send(Ok(handles));
                }
                Err(e) => {
                    let _ = reply.send(Err(e));
                }
            }
        }
    }

    impl ApplicationHandler<UserEvent> for OutputApp {
        fn resumed(&mut self, el: &ActiveEventLoop) {
            self.resumed = true;
            for (output, reply) in std::mem::take(&mut self.pending) {
                self.create_window(el, output, reply);
            }
        }

        fn user_event(&mut self, el: &ActiveEventLoop, event: UserEvent) {
            match event {
                UserEvent::CreateWindow { output, reply } => {
                    if self.resumed {
                        self.create_window(el, output, reply);
                    } else {
                        self.pending.push((output, reply));
                    }
                }
                UserEvent::DestroyWindow { window } => {
                    self.windows.remove(&window.id());
                    drop(window);
                }
            }
        }

        fn window_event(&mut self, _el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
            if let WindowEvent::ModifiersChanged(m) = &event {
                self.modifiers = m.state();
                return;
            }
            let Some(ctx) = self.windows.get_mut(&id) else { return };
            let window = Arc::clone(&ctx.window);
            match event {
                WindowEvent::CloseRequested => {
                    window.set_visible(false);
                }

                WindowEvent::Resized(size) => {
                    ctx.output.set_surface_size(size.width, size.height);
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
                    ctx.cursor_pos = position;
                    let dir = resize_direction(position, window.inner_size(), 8.0);
                    window.set_cursor(resize_cursor(dir));
                }

                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                    ..
                } => {
                    let dir = resize_direction(ctx.cursor_pos, window.inner_size(), 8.0);
                    if let Some(d) = dir {
                        let _ = window.drag_resize_window(d);
                    } else {
                        let now = Instant::now();
                        let is_double = ctx
                            .last_click
                            .map(|t| now.duration_since(t) < Duration::from_millis(300))
                            .unwrap_or(false);
                        if is_double {
                            if window.fullscreen().is_some() {
                                window.set_fullscreen(None);
                            } else {
                                window.set_fullscreen(Some(Fullscreen::Borderless(
                                    window.current_monitor(),
                                )));
                            }
                            ctx.last_click = None;
                        } else {
                            ctx.last_click = Some(now);
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
    /// winit 0.30 guards EventLoop creation to the main thread by default on
    /// both Windows and Linux.  Platform-specific extension traits opt out of
    /// that guard.
    #[cfg(target_os = "windows")]
    fn build_event_loop() -> Result<EventLoop<UserEvent>> {
        use winit::platform::windows::EventLoopBuilderExtWindows;
        EventLoop::<UserEvent>::with_user_event()
            .with_any_thread(true)
            .build()
            .map_err(|e| anyhow!("EventLoop (Windows): {e}"))
    }

    /// Probe whether winit's X11 backend can actually run.
    ///
    /// winit's X11 backend hard-requires `libxkbcommon-x11` and **panics** (not
    /// a recoverable `build()` error) during window creation if it is absent —
    /// common on Wayland-only installs.  We `dlopen` it up-front so
    /// `build_event_loop` can choose Wayland cleanly instead of taking down the
    /// whole output engine.
    #[cfg(target_os = "linux")]
    fn x11_xkb_available() -> bool {
        use std::ffi::CString;
        for name in ["libxkbcommon-x11.so.0", "libxkbcommon-x11.so"] {
            let Ok(c) = CString::new(name) else { continue };
            // SAFETY: valid C string; the handle is closed again immediately.
            let h = unsafe { libc::dlopen(c.as_ptr(), libc::RTLD_LAZY) };
            if !h.is_null() {
                unsafe {
                    libc::dlclose(h);
                }
                return true;
            }
        }
        false
    }

    #[cfg(target_os = "linux")]
    fn build_event_loop() -> Result<EventLoop<UserEvent>> {
        // Prefer X11/XWayland over native Wayland for the output window.
        //
        // With a *native Wayland* EGL surface, Mesa's `eglSwapBuffers` blocks on
        // the compositor's frame callback regardless of the swap interval, which
        // serialises this output window's render-thread GL with WebKitGTK's UI
        // compositing on the same iGPU — the Inkue UI then crawls for the entire
        // duration of video playback (the failure the operator reported).
        // XWayland's X11/DRI EGL path honours `SwapInterval::DontWait` and keeps
        // the two GL clients decoupled, so the UI stays fluid while a video plays.
        //
        // X11 is selected only when `libxkbcommon-x11` is present (winit panics
        // otherwise); otherwise we fall back to native Wayland so the app still
        // runs.  Override with `INKUE_OUTPUT_BACKEND=wayland` for A/B testing.
        let force_wayland = std::env::var("INKUE_OUTPUT_BACKEND").as_deref() == Ok("wayland");
        let use_x11 = !force_wayland && x11_xkb_available();

        let mut b = EventLoop::<UserEvent>::with_user_event();
        if use_x11 {
            use winit::platform::x11::EventLoopBuilderExtX11;
            b.with_any_thread(true).with_x11();
            log::info!("[render] output window backend: X11/XWayland (default)");
            b.build().map_err(|e| anyhow!("EventLoop (Linux/XWayland): {e}"))
        } else {
            use winit::platform::wayland::EventLoopBuilderExtWayland;
            EventLoopBuilderExtWayland::with_any_thread(&mut b, true);
            if force_wayland {
                log::info!(
                    "[render] output window backend: native Wayland (forced via INKUE_OUTPUT_BACKEND)"
                );
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

    /// The proxy of the process-wide event loop, once started.
    static LOOP_PROXY: Mutex<Option<EventLoopProxy<UserEvent>>> = Mutex::new(None);

    /// Start the event-loop thread on first use and return its proxy.
    fn event_loop_proxy(app_handle: &tauri::AppHandle) -> Result<EventLoopProxy<UserEvent>> {
        let mut guard = LOOP_PROXY.lock().map_err(|_| anyhow!("window loop lock poisoned"))?;
        if let Some(proxy) = guard.as_ref() {
            return Ok(proxy.clone());
        }

        let (ready_tx, ready_rx) = channel::<Result<EventLoopProxy<UserEvent>>>();
        let app_handle = app_handle.clone();
        std::thread::Builder::new()
            .name("inkue-output-window".into())
            .spawn(move || {
                let event_loop = match build_event_loop() {
                    Ok(el) => el,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(event_loop.create_proxy()));
                let mut app = OutputApp {
                    app_handle,
                    modifiers: ModifiersState::empty(),
                    windows: HashMap::new(),
                    pending: Vec::new(),
                    resumed: false,
                };
                // A panic inside winit (no display server, missing X11 libs)
                // must not take the process down: the requester then sees its
                // reply channel close and falls back to a headless engine.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    event_loop.run_app(&mut app)
                }));
                if result.is_err() {
                    log::error!("[window] output event loop panicked (no display server?)");
                }
                if let Ok(mut proxy) = LOOP_PROXY.lock() {
                    *proxy = None;
                }
            })
            .map_err(|e| anyhow!("spawn output-window thread: {e}"))?;

        let proxy = ready_rx
            .recv()
            .map_err(|_| anyhow!("output window thread exited before starting"))??;
        *guard = Some(proxy.clone());
        Ok(proxy)
    }

    /// Create the native window of `output` and block until it exists.
    pub(in crate::engine::output_engine) fn create_window(
        output: &Arc<Output>,
        app_handle: &tauri::AppHandle,
    ) -> Result<SendableHandles> {
        let proxy = event_loop_proxy(app_handle)?;
        let (reply, answer) = channel();
        proxy
            .send_event(UserEvent::CreateWindow { output: Arc::clone(output), reply })
            .map_err(|_| anyhow!("output window loop is gone"))?;
        answer
            .recv()
            .map_err(|_| anyhow!("event loop exited before the window was created (no X11/Wayland display available?)"))?
    }

    #[cfg(test)]
    mod tests {
        use super::*;

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

        #[test]
        fn resize_direction_picks_corners_before_edges() {
            let size = PhysicalSize::new(800, 600);
            let dir = |x, y| resize_direction(PhysicalPosition::new(x, y), size, 8.0);
            assert!(matches!(dir(2.0, 2.0), Some(winit::window::ResizeDirection::NorthWest)));
            assert!(matches!(dir(798.0, 598.0), Some(winit::window::ResizeDirection::SouthEast)));
            assert!(matches!(dir(400.0, 2.0), Some(winit::window::ResizeDirection::North)));
            assert!(dir(400.0, 300.0).is_none());
        }
    }
}
