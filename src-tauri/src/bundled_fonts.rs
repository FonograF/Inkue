//! Installs the timer's bundled font (DSEG7 Classic, SIL OFL 1.1 — see
//! `vendor/fonts/DSEG-LICENSE.txt`) into the OS's per-user font directory.
//!
//! Once installed there, it resolves by family name exactly like any other
//! system font — both mpv's OSD (`osd-font`, via fontconfig/GDI/CoreText)
//! and the floating timer's WebView (CSS `font-family`) pick it up with no
//! further code path needed. This keeps font resolution unified across all
//! three OS instead of requiring a separate embedding mechanism per surface.

const FONT_BYTES: &[u8] = include_bytes!("../../vendor/fonts/DSEG7Classic-Regular.ttf");
const FONT_FILE_NAME: &str = "DSEG7Classic-Regular.ttf";

/// Default `timer_font` family name, as embedded in the font itself.
pub const FONT_FAMILY: &str = "DSEG7 Classic";

/// Copy the bundled font into the user's font directory if it isn't already
/// there with matching content. Cheap and idempotent — safe to call on every
/// launch.
pub fn ensure_installed() {
    let Some(dir) = user_font_dir() else {
        log::warn!("[fonts] could not determine user font directory");
        return;
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::warn!("[fonts] could not create font dir {dir:?}: {e}");
        return;
    }
    let dest = dir.join(FONT_FILE_NAME);
    let already_current = std::fs::read(&dest)
        .map(|existing| existing == FONT_BYTES)
        .unwrap_or(false);
    if already_current {
        return;
    }
    if let Err(e) = std::fs::write(&dest, FONT_BYTES) {
        log::warn!("[fonts] could not write bundled font to {dest:?}: {e}");
        return;
    }
    log::info!("[fonts] installed bundled font: {}", dest.display());
    register_font(&dest);
}

#[cfg(target_os = "linux")]
fn user_font_dir() -> Option<std::path::PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|h| std::path::PathBuf::from(h).join(".local/share/fonts"))
}

#[cfg(target_os = "macos")]
fn user_font_dir() -> Option<std::path::PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|h| std::path::PathBuf::from(h).join("Library/Fonts"))
}

#[cfg(target_os = "windows")]
fn user_font_dir() -> Option<std::path::PathBuf> {
    std::env::var("LOCALAPPDATA")
        .ok()
        .map(|h| std::path::PathBuf::from(h).join(r"Microsoft\Windows\Fonts"))
}

/// Make a freshly-written font file visible without requiring a logout/reboot.
#[cfg(target_os = "linux")]
fn register_font(_dest: &std::path::Path) {
    // fontconfig caches the family list; force a rescan of just our dir.
    if let Some(dir) = user_font_dir() {
        let _ = std::process::Command::new("fc-cache").arg("-f").arg(&dir).output();
    }
}

#[cfg(target_os = "macos")]
fn register_font(_dest: &std::path::Path) {
    // CoreText's font server watches ~/Library/Fonts and registers new files
    // dropped into it automatically — no explicit refresh call needed.
}

#[cfg(target_os = "windows")]
fn register_font(dest: &std::path::Path) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Graphics::Gdi::AddFontResourceW;

    let wide: Vec<u16> = dest.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    unsafe {
        AddFontResourceW(wide.as_ptr());
    }

    // Per-user registration (no admin rights needed, Windows 10 1809+) so the
    // font survives a reboot without re-running this installer.
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(fonts_key) = hkcu.create_subkey(r"Software\Microsoft\Windows NT\CurrentVersion\Fonts").map(|(k, _)| k) {
        let _ = fonts_key.set_value(&format!("{FONT_FAMILY} (TrueType)"), &FONT_FILE_NAME.to_string());
    }
}
