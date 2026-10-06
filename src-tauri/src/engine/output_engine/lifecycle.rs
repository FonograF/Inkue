//! Destroying an output the show no longer has.
//!
//! Order matters, because libmpv and the GL stack each have a rule:
//!
//! 1. The output leaves the registry, so voice lookups stop finding its slots,
//!    and its content stops — every cue on it resets through the normal
//!    `Completed` path.
//! 2. The render thread frees every mpv **render** context while its GL
//!    context is current (libmpv requires it before a core goes), then exits,
//!    dropping the GL context and surface.
//! 3. The event threads end (woken with `mpv_wakeup`).
//! 4. The window closes — after its GL surface is gone.
//! 5. The mpv **cores** are released by `Drop` with their last reference, so a
//!    call that looked a slot up a moment earlier never runs on a freed handle.
//!
//! Joining threads takes a moment: this runs on its own thread, never on the
//! show's.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::output::{self, Output};
use super::slot;

/// Tear `output` down (see the module doc).  Blocking.
pub(super) fn destroy(output: Arc<Output>) {
    let name = output.name();
    output::unregister(&output);
    output.hide();

    // 2. Render resources first: the render thread holds the GL context.
    output.shutdown.store(true, Ordering::Release);
    output.wake();

    // 1 + 3. Content stops, slot event threads end.
    for slot in output.slots_snapshot() {
        slot::retire(&slot);
    }
    output.events_stop.store(true, Ordering::Release);
    if let Some(overlay) = output.overlay.get() {
        // SAFETY: the overlay core is alive (the output holds it); this only
        // interrupts its event thread's mpv_wait_event.
        unsafe { (overlay.lib.mpv_wakeup)(overlay.ctx.0) };
    }
    for thread in output.take_threads() {
        let _ = thread.join();
    }

    // 4. The window, now that nothing draws into it.
    output.window.destroy();

    // 5. The overlay core goes with the output's last reference, the slot
    //    cores with theirs (their `retire` armed them).
    if let Some(overlay) = output.overlay.get() {
        overlay.destroy_on_drop.store(true, Ordering::Release);
    }
    log::info!("[output] '{name}' destroyed");
}

/// Run [`destroy`] on a thread of its own.
pub(super) fn destroy_in_background(output: Arc<Output>) {
    let spawned = std::thread::Builder::new()
        .name("inkue-output-teardown".into())
        .spawn(move || destroy(output));
    if let Err(e) = spawned {
        log::error!("[output] could not start the teardown thread: {e}");
    }
}
