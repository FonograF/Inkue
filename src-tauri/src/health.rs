//! Cross-cutting runtime health registry.
//!
//! Hardware / network faults that the operator must see — a disconnected audio
//! interface, an absent MIDI port — are published here as keyed alerts and shown
//! as a non-blocking banner.  Mirrors [`crate::logger`]: a global keyed registry
//! plus a `SEQ` counter so the watchdog thread emits a UI event only on change.
//!
//! Alerts are **idempotent** by key: a watchdog can re-assert the same alert every
//! tick without spamming the UI (only a real change bumps `SEQ`).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthLevel {
    Error,
    Warning,
    Info,
}

/// One active runtime problem, shown as a banner row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HealthAlert {
    /// Stable key so the alert can be set/cleared idempotently (e.g. "audio-device").
    pub key: String,
    pub level: HealthLevel,
    pub message: String,
    /// Action id the UI maps to a button + command (e.g. "restore_audio_device").
    pub action: Option<String>,
    /// Label for that action button.
    pub action_label: Option<String>,
}

impl HealthAlert {
    pub fn new(key: impl Into<String>, level: HealthLevel, message: impl Into<String>) -> Self {
        Self { key: key.into(), level, message: message.into(), action: None, action_label: None }
    }

    pub fn with_action(mut self, action: impl Into<String>, label: impl Into<String>) -> Self {
        self.action = Some(action.into());
        self.action_label = Some(label.into());
        self
    }
}

static ALERTS: OnceLock<Mutex<BTreeMap<String, HealthAlert>>> = OnceLock::new();
/// Bumped on every real change so the watchdog can emit a UI event only when needed.
pub static SEQ: AtomicU64 = AtomicU64::new(0);

fn registry() -> &'static Mutex<BTreeMap<String, HealthAlert>> {
    ALERTS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// Insert or replace an alert.  No-op (no `SEQ` bump) when identical to the
/// current value for that key — so re-asserting every watchdog tick is free.
pub fn set(alert: HealthAlert) {
    if let Ok(mut m) = registry().lock() {
        if m.get(&alert.key) == Some(&alert) {
            return;
        }
        m.insert(alert.key.clone(), alert);
        SEQ.fetch_add(1, Ordering::Relaxed);
    }
}

/// Remove an alert by key.  No-op if it was not present.
pub fn clear(key: &str) {
    if let Ok(mut m) = registry().lock() {
        if m.remove(key).is_some() {
            SEQ.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// All currently-active alerts.
pub fn snapshot() -> Vec<HealthAlert> {
    registry().lock().map(|m| m.values().cloned().collect()).unwrap_or_default()
}
