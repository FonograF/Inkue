//! The monitors an output can go fullscreen on.
//!
//! Windows asks the OS directly (`EnumDisplayMonitors`, callable from any
//! thread).  Linux and macOS go through the Tauri main window, and from any
//! thread but the main one that call **waits for the main thread** — so a GO
//! fired by the event loop (which holds the workspace lock) could deadlock
//! against a command that runs on the main thread and waits for that very lock.
//! There, the list is cached: refreshed inline whenever the main thread asks,
//! and every couple of seconds by a watcher thread that holds no lock at all.

use super::types::ScreenInfo;

/// Enumerate all connected monitors.  Index 0 is always the primary.
pub(super) fn list(app_handle: &tauri::AppHandle) -> Vec<ScreenInfo> {
    #[cfg(target_os = "windows")]
    {
        let _ = app_handle;
        windows_screens()
    }
    #[cfg(not(target_os = "windows"))]
    {
        cache::list(app_handle)
    }
}

/// Start keeping the screen list fresh (no-op on Windows).  Call from the main
/// thread at startup: it also fills the list right away.
pub(super) fn watch(app_handle: &tauri::AppHandle) {
    #[cfg(target_os = "windows")]
    let _ = app_handle;
    #[cfg(not(target_os = "windows"))]
    cache::watch(app_handle);
}

/// Put the primary first, then left to right, and number them in that order.
fn normalise(mut screens: Vec<ScreenInfo>) -> Vec<ScreenInfo> {
    screens.sort_by(|a, b| b.is_primary.cmp(&a.is_primary).then(a.x.cmp(&b.x)));
    for (i, s) in screens.iter_mut().enumerate() {
        s.index = i as u32;
    }
    screens
}

#[cfg(target_os = "windows")]
fn windows_screens() -> Vec<ScreenInfo> {
    let mut screens: Vec<ScreenInfo> = Vec::new();
    unsafe {
        use windows_sys::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, MONITORINFO};
        extern "system" fn cb(
            hmon: windows_sys::Win32::Graphics::Gdi::HMONITOR,
            _hdc: windows_sys::Win32::Graphics::Gdi::HDC,
            _rect: *mut windows_sys::Win32::Foundation::RECT,
            data: windows_sys::Win32::Foundation::LPARAM,
        ) -> windows_sys::Win32::Foundation::BOOL {
            unsafe {
                let list = &mut *(data as *mut Vec<ScreenInfo>);
                let mut mi: MONITORINFO = std::mem::zeroed();
                mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
                if GetMonitorInfoW(hmon, &mut mi) != 0 {
                    let r = mi.rcMonitor;
                    list.push(ScreenInfo {
                        index: list.len() as u32,
                        width: (r.right - r.left) as u32,
                        height: (r.bottom - r.top) as u32,
                        x: r.left,
                        y: r.top,
                        is_primary: (mi.dwFlags & 1) != 0,
                    });
                }
                1
            }
        }
        EnumDisplayMonitors(0, std::ptr::null(), Some(cb), &mut screens as *mut Vec<ScreenInfo> as isize);
    }
    normalise(screens)
}

#[cfg(not(target_os = "windows"))]
mod cache {
    use std::sync::{Mutex, OnceLock};
    use std::thread::ThreadId;
    use std::time::Duration;

    use super::{normalise, ScreenInfo};

    /// How often the watcher refreshes the list (a monitor plugged in mid-show
    /// is seen within this delay).
    const REFRESH: Duration = Duration::from_secs(2);

    static SCREENS: Mutex<Vec<ScreenInfo>> = Mutex::new(Vec::new());
    static MAIN_THREAD: OnceLock<ThreadId> = OnceLock::new();

    pub(super) fn watch(app_handle: &tauri::AppHandle) {
        if MAIN_THREAD.set(std::thread::current().id()).is_err() {
            return; // already watching
        }
        store(enumerate(app_handle));
        let app = app_handle.clone();
        let spawned = std::thread::Builder::new()
            .name("inkue-screen-watch".into())
            .spawn(move || loop {
                std::thread::sleep(REFRESH);
                store(enumerate(&app));
            });
        if let Err(e) = spawned {
            log::warn!("[screens] watcher not started ({e}) — the list refreshes from the UI only");
        }
    }

    pub(super) fn list(app_handle: &tauri::AppHandle) -> Vec<ScreenInfo> {
        if MAIN_THREAD.get() == Some(&std::thread::current().id()) {
            let fresh = enumerate(app_handle);
            store(fresh.clone());
            return fresh;
        }
        SCREENS.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Keep the last good list when an enumeration comes back empty (the main
    /// window is closing, or a transient error) — a machine always has a screen.
    fn store(screens: Vec<ScreenInfo>) {
        if screens.is_empty() {
            return;
        }
        if let Ok(mut cached) = SCREENS.lock() {
            *cached = screens;
        }
    }

    fn enumerate(app_handle: &tauri::AppHandle) -> Vec<ScreenInfo> {
        use tauri::Manager;
        let Some(win) = app_handle.get_webview_window("main") else { return Vec::new() };
        let all = win.available_monitors().unwrap_or_default();
        let primary_pos = win.primary_monitor().ok().flatten().map(|p| *p.position());
        let screens = all
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let pos = m.position();
                let size = m.size();
                ScreenInfo {
                    index: i as u32,
                    width: size.width,
                    height: size.height,
                    x: pos.x,
                    y: pos.y,
                    is_primary: primary_pos.map(|pp| pp.x == pos.x && pp.y == pos.y).unwrap_or(i == 0),
                }
            })
            .collect();
        normalise(screens)
    }
}

/// Resolve a configured screen index against the connected screens.
///
/// Returns `(target, missing)`:
/// - `target` — the screen to go fullscreen on (`None` = floating window);
///   when the configured index is absent, falls back to the primary display.
/// - `missing` — `true` when a screen was configured but is not connected.
pub(super) fn resolve(screens: &[ScreenInfo], screen_index: Option<u32>) -> (Option<ScreenInfo>, bool) {
    match screen_index {
        None => (None, false),
        Some(idx) => match screens.iter().find(|s| s.index == idx) {
            Some(s) => (Some(s.clone()), false),
            None => (screens.first().cloned(), true),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screens(n: u32) -> Vec<ScreenInfo> {
        (0..n)
            .map(|i| ScreenInfo {
                index: i,
                width: 1920,
                height: 1080,
                x: (i as i32) * 1920,
                y: 0,
                is_primary: i == 0,
            })
            .collect()
    }

    #[test]
    fn resolve_screen_none_is_floating() {
        assert_eq!(resolve(&screens(2), None), (None, false));
    }

    #[test]
    fn resolve_screen_found() {
        let (target, missing) = resolve(&screens(2), Some(1));
        assert!(!missing);
        assert_eq!(target.unwrap().index, 1);
    }

    #[test]
    fn resolve_screen_missing_falls_back_to_primary() {
        let (target, missing) = resolve(&screens(2), Some(4));
        assert!(missing);
        assert_eq!(target.unwrap().index, 0);
    }

    #[test]
    fn resolve_screen_missing_with_no_screens() {
        let (target, missing) = resolve(&[], Some(1));
        assert!(missing);
        assert!(target.is_none());
    }

    #[test]
    fn the_primary_comes_first_then_left_to_right() {
        let mut list = screens(3);
        list[0].is_primary = false;
        list[2].is_primary = true;
        let ordered = normalise(list);
        assert!(ordered[0].is_primary);
        assert_eq!(ordered[0].x, 3840);
        assert_eq!(ordered.iter().map(|s| s.index).collect::<Vec<_>>(), vec![0, 1, 2]);
        assert!(ordered[1].x < ordered[2].x);
    }
}
