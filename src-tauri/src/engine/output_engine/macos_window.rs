//! macOS native output windows for the unified GL path (`render.rs`) — one
//! `NSWindow` per output (see [`NativeWindow`]).
//!
//! winit cannot be used here: its `EventLoop` must own the AppKit main thread,
//! which Tauri's `NSApplication` already runs.  So we create and drive a plain
//! borderless `NSWindow` directly through the Objective-C runtime (`objc2`),
//! hand its `contentView` (an `NSView`) to `glutin` as the CGL drawable, and let
//! the shared render thread in `render.rs` do everything else exactly as it does
//! on Windows/Linux.
//!
//! ## Threading
//!
//! Every AppKit call must run on the main thread.  `create()` is invoked from
//! `OutputEngine::new()` inside Tauri's `.setup()`, which *is* the main thread,
//! so the window is built inline there.  The runtime control helpers
//! (`show`/`hide`/`position_on_screen`/`toggle_fullscreen`) are called later from
//! Tauri command / event-loop worker threads, so they marshal onto the main
//! thread via `AppHandle::run_on_main_thread`.
//!
//! Cocoa selectors are rock-stable, so we drive AppKit via raw `msg_send!` rather
//! than `objc2-app-kit`'s version-churny typed bindings.  AppKit is linked by
//! `build.rs` (`cargo::rustc-link-lib=framework=AppKit`).

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use anyhow::{anyhow, Result};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::AnyObject;
use objc2::{class, msg_send, msg_send_id};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize};
use raw_window_handle::{
    AppKitDisplayHandle, AppKitWindowHandle, RawDisplayHandle, RawWindowHandle,
};

use super::output::Output;
use super::window::SendableHandles;

// AppKit constants (stable ABI values from <AppKit/AppKit.h>).
/// `NSWindowStyleMaskResizable` (1 << 3) — resizable window without a title bar.
/// Using this alone keeps the window borderless (no auto-show on app activation)
/// while giving the OS-managed resize grips at the edges.
const NS_WINDOW_STYLE_MASK_RESIZABLE: usize = 1 << 3;
const NS_BACKING_STORE_BUFFERED: usize = 2;
/// Normal window level (0) — output sits alongside other windows and can go behind them.
const NS_NORMAL_WINDOW_LEVEL: isize = 0;
/// Level used when the output window is fullscreen.  Must be above the menu-bar level
/// (24) so the window truly covers the whole screen including the status bar.
const NS_FULLSCREEN_WINDOW_LEVEL: isize = 25;
/// `NSWindowCollectionBehaviorCanJoinAllSpaces` (1 << 0).
const NS_COLLECTION_CAN_JOIN_ALL_SPACES: usize = 1 << 0;
/// `NSWindowCollectionBehaviorFullScreenAuxiliary` (1 << 8) — lets the borderless
/// output coexist over another app's native-fullscreen space.
const NS_COLLECTION_FULLSCREEN_AUXILIARY: usize = 1 << 8;
/// `NSEventMaskLeftMouseDown` — used for the double-click fullscreen monitor.
const NS_EVENT_MASK_LEFT_MOUSE_DOWN: usize = 1 << 1;

const INITIAL_WIDTH: f64 = 960.0;
const INITIAL_HEIGHT: f64 = 540.0;

/// Cascade offset between successive output windows, so a second floating
/// output does not open exactly on top of the first.
const CASCADE_STEP: f64 = 40.0;

/// App handle used to marshal control calls onto the main thread.
static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();
/// Every output window by `*mut NSWindow` address, so the shared mouse monitor
/// can find the output an event belongs to.
static WINDOWS: Mutex<Vec<(usize, Arc<MacState>)>> = Mutex::new(Vec::new());
/// The single local `NSEvent` monitor (double-click → fullscreen) is installed
/// once and serves every output window.
static MOUSE_MONITOR: AtomicUsize = AtomicUsize::new(0);

/// State of one output's `NSWindow`.  Shared with the closures that run on the
/// main thread.
struct MacState {
    /// Raw `*mut NSWindow` (as `usize`), retained until [`NativeWindow::destroy`].
    /// 0 = none.
    window: AtomicUsize,
    /// The `NSWindowDidResizeNotification` observer token (the notification
    /// center holds it until it is removed).  0 = none.
    observer: AtomicUsize,
    /// Whether the window currently fills a whole screen (set by screen
    /// placement / fullscreen toggle).
    fullscreen: AtomicBool,
    /// Windowed frame saved before a fullscreen toggle, restored on toggle-back.
    saved_frame: Mutex<Option<(f64, f64, f64, f64)>>,
    /// The output this window belongs to — told the physical size after every
    /// resize or screen move.
    output: OnceLock<Weak<Output>>,
}

impl MacState {
    /// Report the physical pixel size to the render thread.
    fn report_size(&self, width: u32, height: u32) {
        if let Some(output) = self.output.get().and_then(Weak::upgrade) {
            output.set_surface_size(width, height);
        }
    }
}

/// The `NSWindow` of one output.
pub(in crate::engine::output_engine) struct NativeWindow {
    state: Arc<MacState>,
}

// ---------------------------------------------------------------------------
// Public API (called from OutputEngine / Output)
// ---------------------------------------------------------------------------

impl NativeWindow {
    pub(in crate::engine::output_engine) fn new() -> Self {
        Self {
            state: Arc::new(MacState {
                window: AtomicUsize::new(0),
                observer: AtomicUsize::new(0),
                fullscreen: AtomicBool::new(false),
                saved_frame: Mutex::new(None),
                output: OnceLock::new(),
            }),
        }
    }

    /// Order the output window to the front (show).
    pub(in crate::engine::output_engine) fn show(&self) {
        on_main(&self.state, |_, window| unsafe {
            let _: () = msg_send![window, orderFrontRegardless];
        });
    }

    /// Order the output window out (hide).
    pub(in crate::engine::output_engine) fn hide(&self) {
        on_main(&self.state, |_, window| unsafe {
            let nil: *mut AnyObject = std::ptr::null_mut();
            let _: () = msg_send![window, orderOut: nil];
        });
    }

    /// Name the window after its output.
    pub(in crate::engine::output_engine) fn set_title(&self, title: &str) {
        let Ok(title) = std::ffi::CString::new(title) else { return };
        on_main(&self.state, move |_, window| unsafe {
            let ns_title: *mut AnyObject =
                msg_send![class!(NSString), stringWithUTF8String: title.as_ptr()];
            let _: () = msg_send![window, setTitle: ns_title];
        });
    }

    /// Close the window for good (its output is being destroyed; its render
    /// thread — and with it the CGL context on its view — is already gone).
    pub(in crate::engine::output_engine) fn destroy(&self) {
        on_main(&self.state, |state, window| unsafe {
            if let Ok(mut all) = WINDOWS.lock() {
                all.retain(|(ptr, _)| *ptr != window as usize);
            }
            let observer = state.observer.swap(0, Ordering::SeqCst);
            if observer != 0 {
                let nc: *mut AnyObject = msg_send![class!(NSNotificationCenter), defaultCenter];
                let _: () = msg_send![nc, removeObserver: observer as *mut AnyObject];
            }
            let nil: *mut AnyObject = std::ptr::null_mut();
            let _: () = msg_send![window, orderOut: nil];
            let _: () = msg_send![window, close];
            state.window.store(0, Ordering::SeqCst);
            // Balance the retain `build_window` kept to hold the window alive
            // (`setReleasedWhenClosed: false`, so `close` did not release it).
            let _: () = msg_send![window, release];
        });
    }

    /// Place the window fullscreen onto `NSScreen[screen_index]` (clamped).
    pub(in crate::engine::output_engine) fn place_on_screen(&self, screen_index: u32) {
        on_main(&self.state, move |state, window| unsafe {
            let screens: *mut AnyObject = msg_send![class!(NSScreen), screens];
            if screens.is_null() {
                return;
            }
            let count: usize = msg_send![screens, count];
            if count == 0 {
                return;
            }
            let idx = (screen_index as usize).min(count - 1);
            let screen: *mut AnyObject = msg_send![screens, objectAtIndex: idx];
            if screen.is_null() {
                return;
            }
            let frame: NSRect = msg_send![screen, frame];
            // Raise above the menu bar so the window truly covers the full screen.
            let _: () = msg_send![window, setLevel: NS_FULLSCREEN_WINDOW_LEVEL];
            let _: () = msg_send![window, setFrame: frame, display: true];
            state.fullscreen.store(true, Ordering::SeqCst);
            // Use physical pixels so the GL surface covers the full screen on Retina.
            let view: *mut AnyObject = msg_send![window, contentView];
            let phys: NSSize = msg_send![view, convertSizeToBacking: frame.size];
            state.report_size(phys.width as u32, phys.height as u32);
        });
    }

    /// Restore the saved windowed frame if the window is currently fullscreen
    /// (no-op otherwise).  Used when the operator selects "Floating window".
    pub(in crate::engine::output_engine) fn set_windowed_floating(&self) {
        if self.state.fullscreen.load(Ordering::SeqCst) {
            self.toggle_fullscreen();
        }
    }

    /// Toggle the window between its saved windowed frame and fullscreen on its
    /// current screen — the macOS counterpart of winit's `Fullscreen::Borderless`.
    pub(in crate::engine::output_engine) fn toggle_fullscreen(&self) {
        toggle_fullscreen(&self.state);
    }

    /// Create the borderless output `NSWindow` for `output` and return the raw
    /// handles + initial size (physical pixels) for the render thread's
    /// `glutin` surface.
    pub(in crate::engine::output_engine) fn create(
        &self,
        output: &Arc<Output>,
        app_handle: &tauri::AppHandle,
    ) -> Result<SendableHandles> {
        APP_HANDLE.get_or_init(|| app_handle.clone());
        let _ = self.state.output.set(Arc::downgrade(output));
        let cascade = WINDOWS.lock().map(|w| w.len()).unwrap_or(0) as f64 * CASCADE_STEP;

        // Build on the main thread.  The first output is created during
        // Tauri's `.setup()`, which *is* the main thread, so it builds inline;
        // later ones are dispatched and awaited.
        let (view_ptr, width, height) = if MainThreadMarker::new().is_some() {
            build_window(&self.state, cascade)
        } else {
            let (tx, rx) = std::sync::mpsc::channel::<(usize, u32, u32)>();
            let state = Arc::clone(&self.state);
            app_handle
                .run_on_main_thread(move || {
                    let _ = tx.send(build_window(&state, cascade));
                })
                .map_err(|e| anyhow!("run_on_main_thread (window create): {e}"))?;
            rx.recv()
                .map_err(|_| anyhow!("main-thread NSWindow creation did not complete"))?
        };

        let ns_view = NonNull::new(view_ptr as *mut c_void)
            .ok_or_else(|| anyhow!("NSWindow contentView was nil"))?;
        let rwh = RawWindowHandle::AppKit(AppKitWindowHandle::new(ns_view));
        let rdh = RawDisplayHandle::AppKit(AppKitDisplayHandle::new());
        Ok(SendableHandles { rwh, rdh, width, height })
    }
}

/// Create the `NSWindow` of `output` — see [`NativeWindow::create`].
pub(super) fn create_window(
    output: &Arc<Output>,
    app_handle: &tauri::AppHandle,
) -> Result<SendableHandles> {
    output.window.create(output, app_handle)
}

fn toggle_fullscreen(state: &Arc<MacState>) {
    on_main(state, |state, window| unsafe {
        if state.fullscreen.load(Ordering::SeqCst) {
            // Fallback if no saved frame (e.g. window was shown via place_on_screen
            // without ever being in windowed mode first).
            let (x, y, w, h) = state
                .saved_frame
                .lock()
                .unwrap()
                .unwrap_or((100.0, 100.0, 960.0, 540.0));
            let rect = NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
            // Restore normal window level before resizing so the window re-enters
            // the normal stacking order.
            let _: () = msg_send![window, setLevel: NS_NORMAL_WINDOW_LEVEL];
            let _: () = msg_send![window, setFrame: rect, display: true];
            // Physical pixels for the GL surface.
            let view: *mut AnyObject = msg_send![window, contentView];
            let phys: NSSize = msg_send![view, convertSizeToBacking: NSSize::new(w, h)];
            state.report_size(phys.width as u32, phys.height as u32);
            state.fullscreen.store(false, Ordering::SeqCst);
        } else {
            let cur: NSRect = msg_send![window, frame];
            *state.saved_frame.lock().unwrap() =
                Some((cur.origin.x, cur.origin.y, cur.size.width, cur.size.height));
            let mut screen: *mut AnyObject = msg_send![window, screen];
            if screen.is_null() {
                screen = msg_send![class!(NSScreen), mainScreen];
            }
            if !screen.is_null() {
                let frame: NSRect = msg_send![screen, frame];
                // Raise above the menu bar for true fullscreen coverage.
                let _: () = msg_send![window, setLevel: NS_FULLSCREEN_WINDOW_LEVEL];
                let _: () = msg_send![window, setFrame: frame, display: true];
                // Physical pixels for the GL surface.
                let view: *mut AnyObject = msg_send![window, contentView];
                let phys: NSSize = msg_send![view, convertSizeToBacking: frame.size];
                state.report_size(phys.width as u32, phys.height as u32);
            }
            state.fullscreen.store(true, Ordering::SeqCst);
        }
    });
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

/// Build the NSWindow on the current (main) thread; store it in `state` and
/// return the `contentView` pointer + initial size in **physical pixels**.
/// `cascade` offsets the window so several outputs do not stack exactly.
fn build_window(state: &Arc<MacState>, cascade: f64) -> (usize, u32, u32) {
    unsafe {
        // Center on the main screen (the one with the menu bar).
        let (win_x, win_y) = {
            let ms: *mut AnyObject = msg_send![class!(NSScreen), mainScreen];
            if ms.is_null() {
                (100.0_f64 + cascade, 100.0_f64 + cascade)
            } else {
                let sf: NSRect = msg_send![ms, frame];
                (
                    sf.origin.x + (sf.size.width - INITIAL_WIDTH) / 2.0 + cascade,
                    sf.origin.y + (sf.size.height - INITIAL_HEIGHT) / 2.0 - cascade,
                )
            }
        };
        let rect = NSRect::new(
            NSPoint::new(win_x, win_y),
            NSSize::new(INITIAL_WIDTH, INITIAL_HEIGHT),
        );
        // alloc/init are memory-management-family selectors: objc2 requires
        // `msg_send_id!` (not `msg_send!`) so the +1 retain is tracked.
        let alloc: Allocated<AnyObject> = msg_send_id![class!(NSWindow), alloc];
        // Borderless-resizable: NSWindowStyleMaskResizable alone (= 8) keeps the window
        // frameless so AppKit never auto-shows it on app activation (NSWindowStyleMaskTitled
        // triggers that), while still providing OS-managed resize grips at the edges.
        let window: Retained<AnyObject> = msg_send_id![
            alloc,
            initWithContentRect: rect,
            styleMask: NS_WINDOW_STYLE_MASK_RESIZABLE,
            backing: NS_BACKING_STORE_BUFFERED,
            defer: false
        ];
        // Raw pointer to the (heap-stable) NSWindow; the `forget` below keeps the
        // retain so the window outlives this Retained — `destroy` releases it.
        let window_ptr: *mut AnyObject = (&*window as *const AnyObject) as *mut AnyObject;

        // Closing must not deallocate it: `destroy` releases it explicitly.
        let _: () = msg_send![window_ptr, setReleasedWhenClosed: false];
        // Drag the borderless window by its background.
        let _: () = msg_send![window_ptr, setMovableByWindowBackground: true];
        // Normal level in windowed mode; raised above the menu bar when fullscreen.
        let _: () = msg_send![window_ptr, setLevel: NS_NORMAL_WINDOW_LEVEL];
        let behavior: usize =
            NS_COLLECTION_CAN_JOIN_ALL_SPACES | NS_COLLECTION_FULLSCREEN_AUXILIARY;
        let _: () = msg_send![window_ptr, setCollectionBehavior: behavior];
        let _: () = msg_send![window_ptr, setOpaque: true];

        // Paint the window black behind the GL surface so there is never a white
        // flash between show and the first committed frame.
        let black: *mut AnyObject = msg_send![class!(NSColor), blackColor];
        let _: () = msg_send![window_ptr, setBackgroundColor: black];

        let view: *mut AnyObject = msg_send![window_ptr, contentView];

        // Physical pixel size — critical for Retina displays.  CGL/glutin work in
        // physical pixels, so passing logical size would render content in only the
        // bottom-left fraction of the framebuffer.
        let phys: NSSize = msg_send![view, convertSizeToBacking: NSSize::new(INITIAL_WIDTH, INITIAL_HEIGHT)];
        let phys_w = (phys.width as u32).max(1);
        let phys_h = (phys.height as u32).max(1);

        state.window.store(window_ptr as usize, Ordering::SeqCst);
        if let Ok(mut all) = WINDOWS.lock() {
            all.push((window_ptr as usize, Arc::clone(state)));
        }
        std::mem::forget(window);

        // Output window starts hidden; shown on first GO or by F9 / View menu.
        let nil: *mut AnyObject = std::ptr::null_mut();
        let _: () = msg_send![window_ptr, orderOut: nil];

        // Keep GL surface size in sync when the user drags the window border.
        let observer = register_resize_observer(window_ptr, Arc::clone(state));
        state.observer.store(observer as usize, Ordering::SeqCst);

        // Double-click anywhere in an output window → toggle fullscreen.
        register_dblclick_monitor();

        log::info!(
            "[macos-window] NSWindow created (resizable, \
             {INITIAL_WIDTH}x{INITIAL_HEIGHT} logical at ({win_x},{win_y}), \
             {phys_w}x{phys_h} physical)"
        );

        (view as usize, phys_w, phys_h)
    }
}

/// Update the output's surface size from the window's current physical pixel
/// size.  Called from `windowDidResize:` (main thread).
fn update_physical_size(state: &MacState) {
    let ptr = state.window.load(Ordering::SeqCst);
    if ptr == 0 {
        return;
    }
    unsafe {
        let window = ptr as *mut AnyObject;
        let view: *mut AnyObject = msg_send![window, contentView];
        let bounds: NSRect = msg_send![view, bounds];
        let phys: NSSize = msg_send![view, convertSizeToBacking: bounds.size];
        state.report_size((phys.width as u32).max(1), (phys.height as u32).max(1));
    }
}

/// Register an `NSNotificationCenter` observer so that when the user resizes the
/// window by dragging its edge, the GL surface is immediately updated.  Returns
/// the observer token, needed to remove it when the window is destroyed.
fn register_resize_observer(window_ptr: *mut AnyObject, state: Arc<MacState>) -> *mut AnyObject {
    use block2::RcBlock;
    unsafe {
        let name: *mut AnyObject = msg_send![
            class!(NSString),
            stringWithUTF8String: c"NSWindowDidResizeNotification".as_ptr()
        ];
        // queue: nil → block runs on the thread that posts the notification (main).
        let block = RcBlock::new(move |_notif: *mut AnyObject| {
            update_physical_size(&state);
        });
        let nc: *mut AnyObject = msg_send![class!(NSNotificationCenter), defaultCenter];
        let nil: *mut AnyObject = std::ptr::null_mut();
        let observer: *mut AnyObject = msg_send![
            nc,
            addObserverForName: name,
            object: window_ptr,
            queue: nil,
            usingBlock: &*block
        ];
        // NSNotificationCenter copies the block; we abandon our Rc without
        // dropping so the block stays alive until the observer is removed.
        std::mem::forget(block);
        observer
    }
}

/// Register the local `NSEvent` monitor (once for all outputs): double-click
/// inside an output window toggles its fullscreen, matching the winit
/// double-click behaviour on Windows/Linux.
fn register_dblclick_monitor() {
    use block2::RcBlock;
    if MOUSE_MONITOR.load(Ordering::SeqCst) != 0 {
        return;
    }
    unsafe {
        let block = RcBlock::new(|event: *mut AnyObject| -> *mut AnyObject {
            let click_count: isize = msg_send![event, clickCount];
            if click_count == 2 {
                let event_window: *mut AnyObject = msg_send![event, window];
                let event_window = event_window as usize;
                let hit = WINDOWS
                    .lock()
                    .ok()
                    .and_then(|all| all.iter().find(|(ptr, _)| *ptr == event_window).map(|(_, s)| Arc::clone(s)));
                if let Some(state) = hit {
                    toggle_fullscreen(&state);
                }
            }
            event
        });
        let monitor: *mut AnyObject = msg_send![
            class!(NSEvent),
            addLocalMonitorForEventsMatchingMask: NS_EVENT_MASK_LEFT_MOUSE_DOWN,
            handler: &*block
        ];
        if !monitor.is_null() {
            // Retain the monitor so it is never deallocated (app-lifetime singleton).
            let _: *mut AnyObject = msg_send![monitor, retain];
            MOUSE_MONITOR.store(monitor as usize, Ordering::SeqCst);
        }
        std::mem::forget(block);
    }
}

/// Run `f` with the window's state and live `*mut NSWindow` on the main thread
/// (inline if already there, otherwise marshalled via the Tauri app handle).
fn on_main<F>(state: &Arc<MacState>, f: F)
where
    F: FnOnce(&MacState, *mut AnyObject) + Send + 'static,
{
    let state = Arc::clone(state);
    let run = move || {
        let ptr = state.window.load(Ordering::SeqCst);
        if ptr != 0 {
            f(&state, ptr as *mut AnyObject);
        }
    };

    if MainThreadMarker::new().is_some() {
        run();
    } else if let Some(app) = APP_HANDLE.get() {
        let _ = app.run_on_main_thread(run);
    } else {
        log::error!("[macos-window] no app handle to reach the main thread");
    }
}
