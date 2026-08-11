// Prevents additional console window on Windows in release mode.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// Keep WebKitGTK off its DMABUF renderer unless the operator asks for it.
///
/// Symptom without this: a **completely blank window**. WebKitGTK's accelerated
/// compositing path asks for an EGL display, fails with
/// `Could not create default EGL display: EGL_BAD_PARAMETER. Aborting...` (once
/// per web process), and renders nothing at all — the app is running and
/// logging normally behind an empty white rectangle.
///
/// The cause is a mismatch between the WebKitGTK the AppImage carries (built on
/// Ubuntu 22.04) and the host's much newer Mesa. Reported on Arch Linux with an
/// Intel iGPU, 2026-08-11.
///
/// Disabling DMABUF drops WebKit to its shared-memory compositing path. That is
/// slightly slower, which costs Inkue very little — the UI deliberately avoids
/// continuous animation (see `PORTAGE.md`) — and "slightly slower" beats "blank"
/// on every machine where the fast path does not work.
///
/// Set `WEBKIT_DISABLE_DMABUF_RENDERER=0` to force the fast path back on.
#[cfg(target_os = "linux")]
fn disable_webkit_dmabuf_unless_overridden() {
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        // Sound here specifically because it is the first statement of `main`:
        // no thread has been spawned yet, and GTK/WebKit has not read the
        // environment. (`set_var` becomes `unsafe` in edition 2024.)
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
}

fn main() {
    #[cfg(target_os = "linux")]
    disable_webkit_dmabuf_unless_overridden();

    inkue_lib::run();
}
