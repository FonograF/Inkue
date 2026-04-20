//! [`ImageEngine`] — manages image display surface windows for Image Cues.
//!
//! Each "voice" is a Tauri [`WebviewWindow`] that loads the same React app
//! as the main window, but detects its label prefix (`"image-surface-"`) and
//! renders an `<ImageSurface>` component instead.  The image data is delivered
//! as a base64 data URL via the `get_image_surface_data` Tauri command, which
//! the surface calls on mount to avoid the window-creation timing race.

use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use uuid::Uuid;

/// Unique identifier for one image display voice (one surface window).
pub type ImageVoiceId = Uuid;

/// Status event produced by an image surface window and consumed by the
/// event loop to drive cue lifecycle transitions.
#[derive(Debug, Clone)]
pub enum ImageStatus {
    /// The surface completed its fade-out CSS transition.  The window can
    /// now be garbage-collected and the owning cue may be marked completed.
    FadedOut { voice_id: ImageVoiceId },
}

/// Pending display data stored until the surface window calls
/// `get_image_surface_data` on mount.
struct ImageVoiceData {
    /// Base64-encoded image as a `data:<mime>;base64,<data>` URI.
    data_url: String,
    /// Fade-in duration in milliseconds (0 = instant).
    fade_in_ms: u32,
    /// Label of the Tauri WebviewWindow backing this voice.
    window_label: String,
}

/// Manages all active image surface windows.
pub struct ImageEngine {
    app_handle: tauri::AppHandle,
    voices: Mutex<HashMap<ImageVoiceId, ImageVoiceData>>,
    status_tx: crossbeam_channel::Sender<ImageStatus>,
    status_rx: Mutex<crossbeam_channel::Receiver<ImageStatus>>,
}

impl ImageEngine {
    /// Create a new [`ImageEngine`] bound to the given Tauri [`AppHandle`].
    pub fn new(app_handle: tauri::AppHandle) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        Self {
            app_handle,
            voices: Mutex::new(HashMap::new()),
            status_tx: tx,
            status_rx: Mutex::new(rx),
        }
    }

    /// Display an image file on the target screen.
    ///
    /// Reads the file, encodes it as a base64 data URL, creates a fullscreen
    /// [`WebviewWindow`] on the target monitor, and stores the data URL until
    /// the surface window requests it via `get_image_surface_data`.
    ///
    /// Returns the voice ID that identifies this display instance.
    pub fn show_voice(
        &self,
        file_path: &std::path::Path,
        screen_index: Option<u32>,
        fade_in_ms: u32,
    ) -> Result<ImageVoiceId> {
        let bytes = std::fs::read(file_path)
            .map_err(|e| anyhow!("ImageEngine: cannot read {:?}: {e}", file_path))?;

        let mime = mime_for_path(file_path);
        let encoded = STANDARD.encode(&bytes);
        let data_url = format!("data:{mime};base64,{encoded}");

        let voice_id = Uuid::new_v4();
        let label = format!("image-surface-{voice_id}");

        // Register the voice data up front so `get_image_surface_data` can be
        // served as soon as the surface window's React code mounts, even though
        // the window itself is created asynchronously below.
        self.voices.lock().unwrap().insert(
            voice_id,
            ImageVoiceData {
                data_url,
                fade_in_ms,
                window_label: label.clone(),
            },
        );

        // `WebviewWindowBuilder::build()` must not run on the main thread (the
        // Tao event loop must be free to service the window-creation request).
        // Sync Tauri commands may execute on the main thread, so we dispatch
        // window creation to a background OS thread and return the voice id
        // immediately — GO stays non-blocking.
        let app_handle = self.app_handle.clone();
        std::thread::spawn(move || {
            let window = match WebviewWindowBuilder::new(
                &app_handle,
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
        });

        Ok(voice_id)
    }

    /// Retrieve the pending display data for a surface window.
    ///
    /// Called by the `get_image_surface_data` Tauri command from the
    /// surface window's React component on mount.  Returns a JSON object
    /// `{ "data_url": "…", "fade_in_ms": N }`.
    pub fn get_surface_data(&self, voice_id: ImageVoiceId) -> Result<serde_json::Value> {
        let voices = self.voices.lock().unwrap();
        let data = voices
            .get(&voice_id)
            .ok_or_else(|| anyhow!("ImageEngine: unknown voice_id {voice_id}"))?;
        Ok(serde_json::json!({
            "data_url": data.data_url,
            "fade_in_ms": data.fade_in_ms,
        }))
    }

    /// Emit the `hide-image` event to the surface window, triggering a CSS
    /// fade-out.  The surface will call `report_image_faded_out` when done.
    pub fn hide_voice(&self, voice_id: ImageVoiceId, fade_ms: u32) -> Result<()> {
        let voices = self.voices.lock().unwrap();
        if let Some(data) = voices.get(&voice_id) {
            if let Some(win) = self.app_handle.get_webview_window(&data.window_label) {
                win.emit("hide-image", serde_json::json!({ "fade_ms": fade_ms }))
                    .map_err(|e| anyhow!("ImageEngine: hide_voice emit: {e}"))?;
            }
        }
        Ok(())
    }

    /// Close and remove the surface window for the given voice.
    ///
    /// Idempotent — safe to call even if the voice was already gc'd.
    pub fn gc_voice(&self, voice_id: ImageVoiceId) {
        let mut voices = self.voices.lock().unwrap();
        if let Some(data) = voices.remove(&voice_id) {
            // `close()` blocks waiting on the main-thread event loop. Dispatch
            // on a worker thread so callers on the transport thread are never
            // stalled.
            let app_handle = self.app_handle.clone();
            let label = data.window_label;
            std::thread::spawn(move || {
                if let Some(win) = app_handle.get_webview_window(&label) {
                    let _ = win.close();
                }
            });
        }
    }

    /// Drain all pending status events.  Called once per event-loop tick.
    pub fn drain_status(&self) -> Vec<ImageStatus> {
        let rx = self.status_rx.lock().unwrap();
        let mut out = Vec::new();
        while let Ok(s) = rx.try_recv() {
            out.push(s);
        }
        out
    }

    /// Push a status event.  Called by the `report_image_faded_out` command.
    pub fn push_status(&self, status: ImageStatus) {
        let _ = self.status_tx.send(status);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Determine the MIME type for a file by its extension.
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

/// Position and resize a surface window to cover the given monitor.
///
/// Mirrors the monitor-selection logic in `VideoEngine::list_screens` so that
/// `screen_index = 0` maps to the primary monitor, `screen_index = 1` to the
/// next one, and so on.  `None` leaves the window as a 1280×720 floating frame.
fn position_window_on_screen(
    window: &tauri::WebviewWindow,
    screen_index: Option<u32>,
) -> Result<()> {
    let monitors = window
        .available_monitors()
        .map_err(|e| anyhow!("ImageEngine: available_monitors: {e}"))?;

    // Sort: primary first, then by x position (matches VideoEngine ordering).
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

    // Floating window — reasonable default size.
    window
        .set_position(tauri::PhysicalPosition::new(100i32, 100i32))
        .map_err(|e| anyhow!("{e}"))?;
    window
        .set_size(tauri::PhysicalSize::new(1280u32, 720u32))
        .map_err(|e| anyhow!("{e}"))?;
    Ok(())
}
