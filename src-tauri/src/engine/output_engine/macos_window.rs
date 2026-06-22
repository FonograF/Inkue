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
const NS_WINDOW_STYLE_MASK_BORDERLESS: usize = 0;
const NS_BACKING_STORE_BUFFERED: usize = 2;
/// `NSFloatingWindowLevel` — above normal windows so the output sits on top of
/// the control UI when both share a display.
const NS_FLOATING_WINDOW_LEVEL: isize = 3;
/// `NSWindowCollectionBehaviorCanJoinAllSpaces` (1 << 0).
const NS_COLLECTION_CAN_JOIN_ALL_SPACES: usize = 1 << 0;
/// `NSWindowCollectionBehaviorFullScreenAuxiliary` (1 << 8) — lets the borderless
/// output coexist over another app's native-fullscreen space.
const NS_COLLECTION_FULLSCREEN_AUXILIARY: usize = 1 << 8;

const INITIAL_WIDTH: f64 = 1920.0;
const INITIAL_HEIGHT: f64 = 1080.0;

/// Raw `*mut NSWindow` (as `usize`), retained for the app's lifetime.  0 = none.
static MAC_WINDOW: AtomicUsize = AtomicUsize::new(0);
/// Whether the window currently fills a whole screen (set by screen placement /
/// fullscreen toggle).
static MAC_FULLSCREEN: AtomicBool = AtomicBool::new(false);
/// Windowed frame saved before a fullscreen toggle, restored on toggle-back.
static MAC_SAVED_FRAME: Mutex<Option<(f64, f64, f64, f64)>> = Mutex::new(None);
/// App handle used to marshal control calls onto the main thread.
static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();

// ---------------------------------------------------------------------------
// Public API (called from render.rs / OutputEngine)
// ---------------------------------------------------------------------------

/// Create the borderless output `NSWindow` and return the raw handles + initial
/// size (logical pixels) for the render thread's `glutin` surface.
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
        let _: () = msg_send![window, setFrame: frame, display: true];
        MAC_FULLSCREEN.store(true, Ordering::SeqCst);
        super::render::set_surface_size(frame.size.width as u32, frame.size.height as u32);
    });
}

/// Toggle the window between its saved windowed frame and fullscreen on its
/// current screen — the macOS counterpart of winit's `Fullscreen::Borderless`.
pub(super) fn toggle_fullscreen() {
    on_main(|window| unsafe {
        if MAC_FULLSCREEN.load(Ordering::SeqCst) {
            if let Some((x, y, w, h)) = *MAC_SAVED_FRAME.lock().unwrap() {
                let rect = NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
                let _: () = msg_send![window, setFrame: rect, display: true];
                super::render::set_surface_size(w as u32, h as u32);
            }
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
                let _: () = msg_send![window, setFrame: frame, display: true];
                super::render::set_surface_size(
                    frame.size.width as u32,
                    frame.size.height as u32,
                );
            }
            MAC_FULLSCREEN.store(true, Ordering::SeqCst);
        }
    });
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

/// Build the NSWindow on the current (main) thread; store it and return the
/// `contentView` pointer + initial size.
fn build_window() -> (usize, u32, u32) {
    unsafe {
        let rect = NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(INITIAL_WIDTH, INITIAL_HEIGHT),
        );
        // alloc/init are memory-management-family selectors: objc2 requires
        // `msg_send_id!` (not `msg_send!`) so the +1 retain is tracked.
        let alloc: Allocated<AnyObject> = msg_send_id![class!(NSWindow), alloc];
        let window: Retained<AnyObject> = msg_send_id![
            alloc,
            initWithContentRect: rect,
            styleMask: NS_WINDOW_STYLE_MASK_BORDERLESS,
            backing: NS_BACKING_STORE_BUFFERED,
            defer: false
        ];
        // Raw pointer to the (heap-stable) NSWindow; the `forget` below leaks the
        // retain so the window outlives this Retained and lives for the whole app.
        let window_ptr: *mut AnyObject = (&*window as *const AnyObject) as *mut AnyObject;

        // Keep alive forever; closing must not deallocate it.
        let _: () = msg_send![window_ptr, setReleasedWhenClosed: false];
        // Drag the borderless window by its background, like the winit path.
        let _: () = msg_send![window_ptr, setMovableByWindowBackground: true];
        let _: () = msg_send![window_ptr, setLevel: NS_FLOATING_WINDOW_LEVEL];
        let behavior: usize =
            NS_COLLECTION_CAN_JOIN_ALL_SPACES | NS_COLLECTION_FULLSCREEN_AUXILIARY;
        let _: () = msg_send![window_ptr, setCollectionBehavior: behavior];
        let _: () = msg_send![window_ptr, setOpaque: true];

        // Paint the window black behind the GL surface so there is never a white
        // flash between show and the first committed frame.
        let black: *mut AnyObject = msg_send![class!(NSColor), blackColor];
        let _: () = msg_send![window_ptr, setBackgroundColor: black];

        let view: *mut AnyObject = msg_send![window_ptr, contentView];

        MAC_WINDOW.store(window_ptr as usize, Ordering::SeqCst);
        std::mem::forget(window);
        log::info!("[macos-window] NSWindow created (borderless, {INITIAL_WIDTH}x{INITIAL_HEIGHT})");

        (view as usize, INITIAL_WIDTH as u32, INITIAL_HEIGHT as u32)
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
