//! Tauri commands backing the in-app log viewer.

use std::path::Path;

use crate::logger::{self, LogLine};

/// Return the most recent log lines (default 500).
#[tauri::command]
pub fn get_recent_logs(limit: Option<usize>) -> Vec<LogLine> {
    logger::recent(limit.unwrap_or(500))
}

/// Clear the in-memory log buffer and truncate the log file.
#[tauri::command]
pub fn clear_logs() -> Result<(), String> {
    logger::clear();
    Ok(())
}

/// Reveal the logs folder in the OS file manager.
#[tauri::command]
pub fn open_logs_folder() -> Result<(), String> {
    let dir = logger::logs_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    open_path(&dir).map_err(|e| e.to_string())
}

/// Open `path` with the platform's default file manager / handler.
fn open_path(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    let mut cmd = std::process::Command::new("explorer");
    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("open");
    #[cfg(target_os = "linux")]
    let mut cmd = std::process::Command::new("xdg-open");

    cmd.arg(path).spawn()?;
    Ok(())
}
