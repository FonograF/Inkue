//! In-app logging backend.
//!
//! Fans the single `log` global out to three sinks: stderr (dev convenience), a
//! size-rotated file in the per-user config dir (`%APPDATA%/WinCue/logs/`), and
//! an in-memory ring buffer the UI reads via [`recent`].  The buffer + file mean
//! an operator can see "what went wrong" without a terminal — logs for the user.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use chrono::Local;
use log::{Level, LevelFilter, Log, Metadata, Record};
use serde::Serialize;

use crate::machine_config::config_base_dir;

/// Most recent lines kept in memory for the in-app viewer.
const RING_CAPACITY: usize = 2000;
/// Rotate the log file once it grows past this size (one backup is kept).
const ROTATE_BYTES: u64 = 5 * 1024 * 1024;

/// One formatted log record, as shown in the in-app viewer.
#[derive(Debug, Clone, Serialize)]
pub struct LogLine {
    pub ts: String,
    pub level: String,
    pub target: String,
    pub message: String,
}

static RING: OnceLock<Mutex<VecDeque<LogLine>>> = OnceLock::new();
static FILE: OnceLock<Mutex<Option<File>>> = OnceLock::new();
/// Bumped on every accepted record so the UI-emitter thread can fire an event
/// only when there is genuinely something new (no idle event spam).
pub static SEQ: AtomicU64 = AtomicU64::new(0);

/// Directory holding the log files.
pub fn logs_dir() -> PathBuf {
    config_base_dir().join("WinCue").join("logs")
}

fn log_path() -> PathBuf {
    logs_dir().join("wincue.log")
}

struct WincueLogger {
    level: LevelFilter,
}

impl Log for WincueLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        // WinCue's own records up to the configured level, plus warnings/errors
        // from anywhere (so a dependency blowing up is never silently dropped).
        (metadata.level() <= self.level && metadata.target().starts_with("wincue"))
            || metadata.level() <= Level::Warn
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = LogLine {
            ts: Local::now().format("%H:%M:%S%.3f").to_string(),
            level: record.level().to_string(),
            target: record.target().to_string(),
            message: record.args().to_string(),
        };

        eprintln!("[{}] {:5} {}: {}", line.ts, line.level, line.target, line.message);

        if let Some(m) = FILE.get() {
            if let Ok(mut guard) = m.lock() {
                if let Some(f) = guard.as_mut() {
                    let _ = writeln!(
                        f, "{} {:5} {} {}", line.ts, line.level, line.target, line.message
                    );
                }
            }
        }

        if let Some(r) = RING.get() {
            if let Ok(mut q) = r.lock() {
                if q.len() >= RING_CAPACITY {
                    q.pop_front();
                }
                q.push_back(line);
            }
        }

        SEQ.fetch_add(1, Ordering::Relaxed);
    }

    fn flush(&self) {
        if let Some(m) = FILE.get() {
            if let Ok(mut g) = m.lock() {
                if let Some(f) = g.as_mut() {
                    let _ = f.flush();
                }
            }
        }
    }
}

/// Install the WinCue logger as the global `log` backend.  Call once at startup.
pub fn init() {
    let level = match std::env::var("RUST_LOG").ok().as_deref() {
        Some(s) if s.contains("trace") => LevelFilter::Trace,
        Some(s) if s.contains("debug") => LevelFilter::Debug,
        _ => LevelFilter::Info,
    };

    RING.get_or_init(|| Mutex::new(VecDeque::with_capacity(RING_CAPACITY)));
    FILE.get_or_init(|| Mutex::new(open_log_file()));

    if log::set_boxed_logger(Box::new(WincueLogger { level })).is_ok() {
        log::set_max_level(level);
    }
}

/// Open the log file for appending, rotating it first if it is over the cap.
/// On any failure the logger degrades gracefully to stderr + ring buffer only.
fn open_log_file() -> Option<File> {
    let dir = logs_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let path = log_path();
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > ROTATE_BYTES {
            let _ = std::fs::rename(&path, dir.join("wincue.log.1"));
        }
    }
    OpenOptions::new().create(true).append(true).open(&path).ok()
}

/// The most recent `limit` log lines, oldest first.
pub fn recent(limit: usize) -> Vec<LogLine> {
    RING.get()
        .and_then(|r| r.lock().ok())
        .map(|q| {
            let start = q.len().saturating_sub(limit);
            q.iter().skip(start).cloned().collect()
        })
        .unwrap_or_default()
}

/// Clear the in-memory ring buffer and truncate the on-disk log file.
pub fn clear() {
    if let Some(r) = RING.get() {
        if let Ok(mut q) = r.lock() {
            q.clear();
        }
    }
    if let Some(m) = FILE.get() {
        if let Ok(mut guard) = m.lock() {
            *guard = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(log_path())
                .ok();
        }
    }
}
