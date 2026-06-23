//! macOS native output window for the unified GL path (`render.rs`).
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
use std::sync::{Mutex, OnceLock};

use anyhow::{anyhow, Result};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::AnyObject;
use objc2::{class, msg_send, msg_send_id};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize};
use raw_window_handle::{
    AppKitDisplayHandle, AppKitWindowHandle, RawDisplayHandle, RawWindowHandle,
};

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

/// Raw `*mut NSWindow` (as `usize`), retained for the app's lifetime.  0 = none.
static MAC_WINDOW: AtomicUsize = AtomicUsize::new(0);
/// Whether the window currently fills a whole screen (set by screen placement /
/// fullscreen toggle).
static MAC_FULLSCREEN: AtomicBool = AtomicBool::new(false);
/// Windowed frame saved before a fullscreen toggle, restored on toggle-back.
static MAC_SAVED_FRAME: Mutex<Option<(f64, f64, f64, f64)>> = Mutex::new(None);
/// App handle used to marshal control calls onto the main thread.
static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();
/// Retained NSEvent local monitor for double-click → fullscreen.
static MOUSE_MONITOR: AtomicUsize = AtomicUsize::new(0);

// ---------------------------------------------------------------------------
// Public API (called from render.rs / OutputEngine)
// ---------------------------------------------------------------------------

/// Create the borderless output `NSWindow` and return the raw handles + initial
/// size (physical pixels) for the render thread's `glutin` surface.
pub(super) fn create(
    app_handle: &tauri::AppHandle,
) -> Result<(RawWindowHandle, RawDisplayHandle, u32, u32)> {
    APP_HANDLE.get_or_init(|| app_handle.clone());

    // Build on the main thread.  In normal startup we already are it (`.setup()`),
    // so build inline; otherwise dispatch and wait.
    let (view_ptr, width, height) = if MainThreadMarker::new().is_some() {
        build_window()
    } else {
        let (tx, rx) = std::sync::mpsc::channel::<(usize, u32, u32)>();
        app_handle
            .run_on_main_thread(move || {
                let _ = tx.send(build_window());
            })
            .map_err(|e| anyhow!("run_on_main_thread (window create): {e}"))?;
        rx.recv()
            .map_err(|_| anyhow!("main-thread NSWindow creation did not complete"))?
    };

    let ns_view = NonNull::new(view_ptr as *mut c_void)
        .ok_or_else(|| anyhow!("NSWindow contentView was nil"))?;
    let window_handle = AppKitWindowHandle::new(ns_view);
    let rwh = RawWindowHandle::AppKit(window_handle);
    let rdh = RawDisplayHandle::AppKit(AppKitDisplayHandle::new());
    Ok((rwh, rdh, width, height))
}

/// Order the output window to the front (show).
pub(super) fn show() {
    on_main(|window| unsafe {
        let _: () = msg_send![window, orderFrontRegardless];
    });
}

/// Order the output window out (hide).
pub(super) fn hide() {
    on_main(|window| unsafe {
        let nil: *mut AnyObject = std::ptr::null_mut();
        let _: () = msg_send![window, orderOut: nil];
    });
}

/// Place the window fullscreen onto `NSScreen[screen_index]` (clamped).
pub(super) fn position_on_screen(screen_index: u32) {
    on_main(move |window| unsafe {
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
        MAC_FULLSCREEN.store(true, Ordering::SeqCst);
        // Use physical pixels so the GL surface covers the full screen on Retina.
        let view: *mut AnyObject = msg_send![window, contentView];
        let phys: NSSize = msg_send![view, convertSizeToBacking: frame.size];
        super::render::set_surface_size(phys.width as u32, phys.height as u32);
    });
}

/// Toggle the window between its saved windowed frame and fullscreen on its
/// current screen — the macOS counterpart of winit's `Fullscreen::Borderless`.
pub(super) fn toggle_fullscreen() {
    on_main(|window| unsafe {
        if MAC_FULLSCREEN.load(Ordering::SeqCst) {
            // Fallback if no saved frame (e.g. window was shown via position_on_screen
            // without ever being in windowed mode first).
            let (x, y, w, h) = MAC_SAVED_FRAME
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
            super::render::set_surface_size(phys.width as u32, phys.height as u32);
            MAC_FULLSCREEN.store(false, Ordering::SeqCst);
        } else {
            let cur: NSRect = msg_send![window, frame];
            *MAC_SAVED_FRAME.lock().unwrap() =
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
                super::render::set_surface_size(phys.width as u32, phys.height as u32);
            }
            MAC_FULLSCREEN.store(true, Ordering::SeqCst);
        }
    });
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

/// Build the NSWindow on the current (main) thread; store it and return the
/// `contentView` pointer + initial size in **physical pixels**.
fn build_window() -> (usize, u32, u32) {
    unsafe {
        // Center on the main screen (the one with the menu bar).
        let (win_x, win_y) = {
            let ms: *mut AnyObject = msg_send![class!(NSScreen), mainScreen];
            if ms.is_null() {
                (100.0_f64, 100.0_f64)
            } else {
                let sf: NSRect = msg_send![ms, frame];
                (
                    sf.origin.x + (sf.size.width  - INITIAL_WIDTH)  / 2.0,
                    sf.origin.y + (sf.size.height - INITIAL_HEIGHT) / 2.0,
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
        // Raw pointer to the (heap-stable) NSWindow; the `forget` below leaks the
        // retain so the window outlives this Retained and lives for the whole app.
        let window_ptr: *mut AnyObject = (&*window as *const AnyObject) as *mut AnyObject;

        // Keep alive forever; closing must not deallocate it.
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

        MAC_WINDOW.store(window_ptr as usize, Ordering::SeqCst);
        std::mem::forget(window);

        // Output window starts hidden; shown on first GO or by F9 / View menu.
        let nil: *mut AnyObject = std::ptr::null_mut();
        let _: () = msg_send![window_ptr, orderOut: nil];

        // Keep GL surface size in sync when the user drags the window border.
        register_resize_observer(window_ptr);

        // Double-click anywhere in the output window → toggle fullscreen.
        register_dblclick_monitor(window_ptr);

        log::info!(
            "[macos-window] NSWindow created (resizable, \
             {INITIAL_WIDTH}x{INITIAL_HEIGHT} logical at ({win_x},{win_y}), \
             {phys_w}x{phys_h} physical)"
        );

        (view as usize, phys_w, phys_h)
    }
}

/// Update `GL_WIDTH`/`GL_HEIGHT` from the current window's physical pixel size.
/// Called from `windowDidResize:` (main thread).
fn update_physical_size() {
    let ptr = MAC_WINDOW.load(Ordering::SeqCst);
    if ptr == 0 {
        return;
    }
    unsafe {
        let window = ptr as *mut AnyObject;
        let view: *mut AnyObject = msg_send![window, contentView];
        let bounds: NSRect = msg_send![view, bounds];
        let phys: NSSize = msg_send![view, convertSizeToBacking: bounds.size];
        let w = (phys.width as u32).max(1);
        let h = (phys.height as u32).max(1);
        super::render::set_surface_size(w, h);
    }
}

/// Register an `NSNotificationCenter` observer so that when the user resizes the
/// window by dragging its edge, the GL surface is immediately updated.
fn register_resize_observer(window_ptr: *mut AnyObject) {
    use block2::RcBlock;
    unsafe {
        let name: *mut AnyObject = msg_send![
            class!(NSString),
            stringWithUTF8String: c"NSWindowDidResizeNotification".as_ptr()
        ];
        // queue: nil → block runs on the thread that posts the notification (main).
        let block = RcBlock::new(|_notif: *mut AnyObject| {
            update_physical_size();
        });
        let nc: *mut AnyObject = msg_send![class!(NSNotificationCenter), defaultCenter];
        let nil: *mut AnyObject = std::ptr::null_mut();
        let _obs: *mut AnyObject = msg_send![
            nc,
            addObserverForName: name,
            object: window_ptr,
            queue: nil,
            usingBlock: &*block
        ];
        // NSNotificationCenter copies the block; we abandon our Rc without
        // dropping so the block stays alive for the app's lifetime.
        std::mem::forget(block);
    }
}

/// Register a local `NSEvent` monitor: double-click inside the output window
/// toggles fullscreen, matching the winit double-click behaviour on Windows/Linux.
fn register_dblclick_monitor(_window_ptr: *mut AnyObject) {
    use block2::RcBlock;
    unsafe {
        // Use MAC_WINDOW instead of capturing window_ptr (raw pointer is !Send).
        let block = RcBlock::new(|event: *mut AnyObject| -> *mut AnyObject {
            let click_count: isize = msg_send![event, clickCount];
            if click_count == 2 {
                let event_window: *mut AnyObject = msg_send![event, window];
                let our_window = MAC_WINDOW.load(Ordering::SeqCst) as *mut AnyObject;
                if event_window == our_window {
                    toggle_fullscreen();
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

/// Run `f` with the live `*mut NSWindow` on the main thread (inline if already
/// there, otherwise marshalled via the Tauri app handle).
fn on_main<F>(f: F)
where
    F: FnOnce(*mut AnyObject) + Send + 'static,
{
    let run = move || {
        let ptr = MAC_WINDOW.load(Ordering::SeqCst);
        if ptr != 0 {
            f(ptr as *mut AnyObject);
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
