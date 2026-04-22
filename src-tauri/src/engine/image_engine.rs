//! [`ImageEngine`] — manages persistent output surface windows for Image Cues.
//!
//! Each screen (identified by its monitor index) gets a single persistent
//! [`WebviewWindow`] labelled `output-surface-{index}` (or
//! `output-surface-float` for the floating case).  The window is created lazily
//! on the first image cue that targets it and kept alive for the lifetime of
//! the application, so consecutive image cues on the same screen never flicker
//! due to window destruction and re-creation.
//!
//! # Data flow
//!
//! 1. **`show_voice()`** — stores a [`VoiceEntry`] and either:
//!    - creates the surface window (first use): the React component polls
//!      [`get_surface_current_voice`] on mount to fetch the initial image.
//!    - emits a `surface-show-image` Tauri event to the existing window.
//!
//! 2. **`hide_voice()`** — emits `surface-hide-image`; the React component runs
//!    a CSS fade-out and then calls `report_image_faded_out` so the event loop
//!    can detect cue completion.
//!
//! 3. **`gc_voice()`** — removes the voice entry.  If no other voices remain
//!    for the surface the window is hidden (but not destroyed) to avoid leaving
//!    a black window on screen.  It will be re-shown on the next `show_voice`.

use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::Serialize;
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use uuid::Uuid;

/// Unique identifier for one image display voice.
pub type ImageVoiceId = Uuid;

/// Status event produced by an image surface window.
#[derive(Debug, Clone)]
pub enum ImageStatus {
    /// The surface completed its fade-out transition.
    FadedOut { voice_id: ImageVoiceId },
}

/// Key identifying a persistent output surface (one per monitor target).
#[derive(Hash, Eq, PartialEq, Clone, Copy, Debug)]
enum SurfaceKey {
    Screen(u32),
    Floating,
}

impl SurfaceKey {
    fn label(self) -> String {
        match self {
            SurfaceKey::Screen(i) => format!("output-surface-{i}"),
            SurfaceKey::Floating => "output-surface-float".to_string(),
        }
    }
}

/// Data for one active image voice.
struct VoiceEntry {
    data_url: String,
    fade_in_ms: u32,
    surface_key: SurfaceKey,
}

/// Persistent surface window state.
struct SurfaceInfo {
    window_label: String,
    /// Voice currently shown (used by `get_surface_current_voice` for the
    /// initial-load polling race on window creation).
    current_voice_id: Option<ImageVoiceId>,
}

/// Payload returned to a surface window when it first mounts.
#[derive(Debug, Serialize, Clone)]
pub struct VoiceInitData {
    pub voice_id: String,
    pub data_url: String,
    pub fade_in_ms: u32,
}

struct ImageEngineInner {
    voices: HashMap<ImageVoiceId, VoiceEntry>,
    surfaces: HashMap<SurfaceKey, SurfaceInfo>,
}

/// Manages all persistent image output surface windows.
pub struct ImageEngine {
    app_handle: tauri::AppHandle,
    inner: Mutex<ImageEngineInner>,
    status_tx: crossbeam_channel::Sender<ImageStatus>,
    status_rx: Mutex<crossbeam_channel::Receiver<ImageStatus>>,
}

impl ImageEngine {
    /// Create a new [`ImageEngine`] bound to the given Tauri [`AppHandle`].
    pub fn new(app_handle: tauri::AppHandle) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        Self {
            app_handle,
            inner: Mutex::new(ImageEngineInner {
                voices: HashMap::new(),
                surfaces: HashMap::new(),
            }),
            status_tx: tx,
            status_rx: Mutex::new(rx),
        }
    }

    /// Display an image on the target screen.
    ///
    /// Creates the surface window on first use; for subsequent calls on the
    /// same screen the existing window receives a `surface-show-image` event.
    pub fn show_voice(
        &self,
        file_path: &std::path::Path,
        screen_index: Option<u32>,
        fade_in_ms: u32,
    ) -> Result<ImageVoiceId> {
        let bytes = std::fs::read(file_path)
            .map_err(|e| anyhow!("ImageEngine: cannot read {:?}: {e}", file_path))?;

        let mime = mime_for_path(file_path);
        let data_url = format!("data:{mime};base64,{}", STANDARD.encode(&bytes));

        let voice_id = Uuid::new_v4();
        let surface_key = screen_index
            .map(SurfaceKey::Screen)
            .unwrap_or(SurfaceKey::Floating);

        let mut inner = self.inner.lock().unwrap();
        inner.voices.insert(
            voice_id,
            VoiceEntry { data_url: data_url.clone(), fade_in_ms, surface_key },
        );

        if let Some(surface) = inner.surfaces.get_mut(&surface_key) {
            // Surface window already exists — update current voice and send event.
            surface.current_voice_id = Some(voice_id);
            let label = surface.window_label.clone();
            drop(inner);

            let app = self.app_handle.clone();
            std::thread::spawn(move || {
                if let Some(win) = app.get_webview_window(&label) {
                    // Re-show in case the window was hidden after a previous stop.
                    let _ = win.show();
                    win.emit(
                        "surface-show-image",
                        serde_json::json!({
                            "voice_id": voice_id,
                            "data_url": data_url,
                            "fade_in_ms": fade_in_ms,
                        }),
                    )
                    .ok();
                }
                // Refocus main so keyboard shortcuts (GO, STOP) still work.
                if let Some(main) = app.get_webview_window("main") {
                    let _ = main.set_focus();
                }
            });
        } else {
            // First use — create the persistent surface window.
            let label = surface_key.label();
            inner.surfaces.insert(
                surface_key,
                SurfaceInfo { window_label: label.clone(), current_voice_id: Some(voice_id) },
            );
            drop(inner);

            let app = self.app_handle.clone();
            std::thread::spawn(move || {
                let window = match WebviewWindowBuilder::new(
                    &app,
                    &label,
                    WebviewUrl::App("".into()),
                )
                .decorations(false)
                .always_on_top(true)
                .fullscreen(false)
                .skip_taskbar(true)
                .focused(false)
                .visible(false)
                .build()
                {
                    Ok(w) => w,
                    Err(e) => {
                        log::warn!("ImageEngine: failed to create surface window: {e}");
                        return;
                    }
                };

                if let Err(e) = position_window_on_screen(&window, screen_index) {
                    log::warn!("ImageEngine: position_window_on_screen: {e}");
                }
                if let Err(e) = window.show() {
                    log::warn!("ImageEngine: window.show(): {e}");
                }
                // React will call get_surface_current_voice on mount to fetch
                // the initial image; no event needed here.

                if let Some(main) = app.get_webview_window("main") {
                    let _ = main.set_focus();
                }
            });
        }

        Ok(voice_id)
    }

    /// Return the current voice data for a surface window, called by the
    /// React component on mount to handle the window-creation timing race.
    pub fn get_surface_current_voice(&self, surface_label: &str) -> Option<VoiceInitData> {
        let inner = self.inner.lock().unwrap();
        let surface = inner.surfaces.values().find(|s| s.window_label == surface_label)?;
        let vid = surface.current_voice_id?;
        let entry = inner.voices.get(&vid)?;
        Some(VoiceInitData {
            voice_id: vid.to_string(),
            data_url: entry.data_url.clone(),
            fade_in_ms: entry.fade_in_ms,
        })
    }

    /// Emit the `surface-hide-image` event to fade out the given voice.
    pub fn hide_voice(&self, voice_id: ImageVoiceId, fade_ms: u32) -> Result<()> {
        let inner = self.inner.lock().unwrap();
        if let Some(entry) = inner.voices.get(&voice_id) {
            if let Some(surface) = inner.surfaces.get(&entry.surface_key) {
                if let Some(win) = self.app_handle.get_webview_window(&surface.window_label) {
                    win.emit(
                        "surface-hide-image",
                        serde_json::json!({ "voice_id": voice_id, "fade_ms": fade_ms }),
                    )
                    .ok();
                }
            }
        }
        Ok(())
    }

    /// Remove voice data.  The surface window is hidden (not closed) when it
    /// has no remaining active voices.
    pub fn gc_voice(&self, voice_id: ImageVoiceId) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(entry) = inner.voices.remove(&voice_id) {
            let has_more = inner
                .voices
                .values()
                .any(|v| v.surface_key == entry.surface_key);

            if let Some(surface) = inner.surfaces.get_mut(&entry.surface_key) {
                if surface.current_voice_id == Some(voice_id) {
                    surface.current_voice_id = None;
                }
                if !has_more {
                    let label = surface.window_label.clone();
                    let app = self.app_handle.clone();
                    std::thread::spawn(move || {
                        if let Some(win) = app.get_webview_window(&label) {
                            let _ = win.hide();
                        }
                    });
                }
            }
        }
    }

    /// Drain all pending status events (called once per event-loop tick).
    pub fn drain_status(&self) -> Vec<ImageStatus> {
        let rx = self.status_rx.lock().unwrap();
        let mut out = Vec::new();
        while let Ok(s) = rx.try_recv() {
            out.push(s);
        }
        out
    }

    /// Push a status event (called by the `report_image_faded_out` command).
    pub fn push_status(&self, status: ImageStatus) {
        let _ = self.status_tx.send(status);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn mime_for_path(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("bmp") => "image/bmp",
        Some("svg") => "image/svg+xml",
        _ => "image/png",
    }
}

fn position_window_on_screen(
    window: &tauri::WebviewWindow,
    screen_index: Option<u32>,
) -> Result<()> {
    let monitors = window
        .available_monitors()
        .map_err(|e| anyhow!("ImageEngine: available_monitors: {e}"))?;

    let mut sorted: Vec<_> = monitors.into_iter().collect();
    sorted.sort_by(|a, b| {
        let a_primary = a.position().x == 0 && a.position().y == 0;
        let b_primary = b.position().x == 0 && b.position().y == 0;
        b_primary.cmp(&a_primary).then(a.position().x.cmp(&b.position().x))
    });

    if let Some(idx) = screen_index {
        if let Some(monitor) = sorted.get(idx as usize) {
            let pos = monitor.position();
            let size = monitor.size();
            window
                .set_position(tauri::PhysicalPosition::new(pos.x, pos.y))
                .map_err(|e| anyhow!("{e}"))?;
            window
                .set_size(tauri::PhysicalSize::new(size.width, size.height))
                .map_err(|e| anyhow!("{e}"))?;
            window.set_fullscreen(true).map_err(|e| anyhow!("{e}"))?;
            return Ok(());
        }
    }

    // Floating window — reasonable default.
    window
        .set_position(tauri::PhysicalPosition::new(100i32, 100i32))
        .map_err(|e| anyhow!("{e}"))?;
    window
        .set_size(tauri::PhysicalSize::new(1280u32, 720u32))
        .map_err(|e| anyhow!("{e}"))?;
    Ok(())
}
