//! Tauri commands for reading and writing application preferences.

use std::f32::consts::PI;

use tauri::{AppHandle, Emitter, Manager, State};

use crate::{
    engine::device_manager::DeviceInfo,
    preferences::{AppPreferences, AudioPreferences, DisplayPreferences, GeneralPreferences, MachineAudioConfig},
    state::AppState,
};

/// Return the number of stereo output pairs the current ASIO engine stream
/// is using.  Call this after Apply to populate the pair selector.
///
/// Reads the channel count stored by the last successful `restart()`.
/// Returns 1 if the engine has not yet been switched to ASIO.
#[tauri::command]
pub fn get_asio_output_pairs(state: State<'_, AppState>) -> u32 {
    let ch = state.audio_engine.output_channels();
    (ch / 2).max(1)
}

/// Return the list of audio backends available on this platform.
///
/// Windows: `wasapi_shared`, `wasapi_exclusive`, and `asio` (when installed).
/// Mac / Linux: `system_default` — cpal picks CoreAudio / ALSA automatically.
#[tauri::command]
pub fn get_available_backends() -> Vec<String> {
    #[cfg(target_os = "windows")]
    {
        #[allow(unused_mut)]
        let mut backends = vec![
            "wasapi_shared".to_string(),
            "wasapi_exclusive".to_string(),
        ];
        #[cfg(feature = "asio-support")]
        if asio_drivers_installed() {
            backends.push("asio".to_string());
        }
        backends
    }
    #[cfg(not(target_os = "windows"))]
    vec!["system_default".to_string()]
}

/// Returns `true` when at least one ASIO driver is registered under
/// `HKEY_LOCAL_MACHINE\SOFTWARE\ASIO` (checked only when the
/// `asio-support` feature is enabled).
#[cfg(all(windows, feature = "asio-support"))]
fn asio_drivers_installed() -> bool {
    use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    hklm.open_subkey("SOFTWARE\\ASIO")
        .map(|k| k.enum_keys().count() > 0)
        .unwrap_or(false)
}

/// Enumerate installed ASIO drivers by reading the Windows registry directly.
///
/// Returns one `DeviceInfo` per subkey under `HKLM\SOFTWARE\ASIO`.
/// Channels and sample rate are left at defaults — ASIO drivers report their
/// actual capabilities only after they are opened.
#[cfg(all(windows, feature = "asio-support"))]
fn list_asio_drivers_from_registry() -> Vec<crate::engine::device_manager::DeviceInfo> {
    use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let Ok(key) = hklm.open_subkey("SOFTWARE\\ASIO") else {
        return vec![];
    };
    key.enum_keys()
        .filter_map(|k| k.ok())
        .map(|name| crate::engine::device_manager::DeviceInfo {
            id: name.clone(),
            name,
            channels: 2,
            sample_rate: 44100,
        })
        .collect()
}

/// Return the full preferences tree from the active workspace.
#[tauri::command]
pub fn get_preferences(state: State<'_, AppState>) -> Result<AppPreferences, String> {
    let ws = state.workspace.lock().map_err(|e| e.to_string())?;
    Ok(ws.preferences.clone())
}

/// Return the machine audio config from disk, normalised for the current build.
///
/// - Mac / Linux: any Windows-specific backend (`wasapi_*`, `asio`) → `system_default`.
/// - Windows without `--features asio-support`: `asio` → `wasapi_shared` so the
///   preferences UI shows a usable backend.  The file on disk is NOT rewritten, so
///   switching to `pnpm tauri:dev` (with ASIO) restores the real choice automatically.
#[tauri::command]
pub fn get_machine_audio_config() -> MachineAudioConfig {
    #[allow(unused_mut)]
    let mut config = crate::machine_config::load();
    #[cfg(not(target_os = "windows"))]
    if !matches!(config.backend, crate::preferences::AudioBackend::SystemDefault) {
        config.backend = crate::preferences::AudioBackend::SystemDefault;
    }
    #[cfg(all(windows, not(feature = "asio-support")))]
    if matches!(config.backend, crate::preferences::AudioBackend::Asio) {
        config.backend = crate::preferences::AudioBackend::WasapiShared;
    }
    config
}

/// Persist machine audio config to `%APPDATA%\Inkue\audio.json` and re-open the
/// audio engine on the new device.  Running cues keep playing — voices are
/// preserved across the restart and resume on the new output.
#[tauri::command]
pub fn update_machine_audio_config(
    config: MachineAudioConfig,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    crate::machine_config::save(&config).map_err(|e| e.to_string())?;

    // Record this as the operator's desired device (clears any auto-fallback +
    // its banner) and re-open the stream on it.
    state.audio_engine.apply_user_config(&config).map_err(|e| e.to_string())?;
    crate::health::clear("audio-device");

    let new_buffer_size = config.buffer_size;
    {
        let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
        // Keep the runtime buffer-size hint in sync with the new machine config.
        // Running cues are NOT reset: voices are preserved across the device
        // switch and keep playing on the new output.
        ws.preferences.audio.audio_buffer_size = new_buffer_size;
    }

    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(())
}

/// Overwrite the show-specific audio defaults (volume, fade) in the workspace.
/// Does not restart the engine — use `update_machine_audio_config` for hardware changes.
#[tauri::command]
pub fn update_audio_preferences(
    prefs: AudioPreferences,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    ws.preferences.audio = prefs;
    ws.mark_modified();
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(())
}

/// Overwrite the general section of preferences and mark the workspace modified.
///
/// Unlike audio preferences, no engine restart is needed.
#[tauri::command]
pub fn update_general_preferences(
    prefs: GeneralPreferences,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    ws.preferences.general = prefs;
    ws.mark_modified();
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(())
}

/// Return the current output screen index from display preferences.
///
/// `None` means floating windowed; `Some(n)` means fullscreen on monitor n.
#[tauri::command]
pub fn get_output_screen(state: State<'_, AppState>) -> Result<Option<u32>, String> {
    let ws = state.workspace.lock().map_err(|e| e.to_string())?;
    Ok(ws.preferences.display.output_screen)
}

/// Set the output screen index in display preferences and mark the workspace modified.
///
/// Pass `None` for floating windowed, `Some(n)` for fullscreen on monitor n.
#[tauri::command]
pub fn set_output_screen(
    screen: Option<u32>,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    ws.preferences.display.output_screen = screen;
    ws.mark_modified();
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(())
}

/// Overwrite the colour-theme and timer fields in display preferences and mark the workspace modified.
///
/// The frontend applies colour values as CSS variables; timer style is applied
/// immediately to the mpv OSD — no engine restart needed.
#[tauri::command]
pub fn update_display_preferences(
    prefs: DisplayPreferences,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let (font, font_size, position, margin, show_floating) = {
        let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
        // Preserve output_screen (managed by set_output_screen).
        ws.preferences.display.theme             = prefs.theme;
        ws.preferences.display.show_output_timer = prefs.show_output_timer;
        ws.preferences.display.timer_floating    = prefs.timer_floating;
        ws.preferences.display.timer_count_down    = prefs.timer_count_down;
        ws.preferences.display.timer_show_ms       = prefs.timer_show_ms;
        ws.preferences.display.timer_font          = prefs.timer_font;
        ws.preferences.display.timer_font_size     = prefs.timer_font_size;
        ws.preferences.display.timer_position      = prefs.timer_position;
        ws.preferences.display.timer_margin        = prefs.timer_margin;
        ws.preferences.display.cue_color_style     = prefs.cue_color_style;
        ws.mark_modified();
        (
            ws.preferences.display.timer_font.clone(),
            ws.preferences.display.timer_font_size,
            ws.preferences.display.timer_position,
            ws.preferences.display.timer_margin,
            ws.preferences.display.show_output_timer && ws.preferences.display.timer_floating,
        )
    };

    state.output_engine.set_timer_style(&font, font_size, position, margin);
    state.output_engine.set_floating_timer_visible(show_floating);
    // Clear any active preview — live cue timer takes over from here.
    state.output_engine.set_timer_preview(None);
    let _ = app_handle.emit("workspace-modified", serde_json::json!({}));
    Ok(())
}

/// Apply timer style settings immediately (without persisting) and show or hide
/// a preview placeholder on the output window.
///
/// Used by the preferences panel while the user adjusts timer settings.
/// `text = Some("00:00.000")` → show placeholder; `text = None` → clear preview.
/// Restoring the persisted style after cancel is the caller's responsibility.
#[tauri::command]
pub fn preview_output_timer(
    font: String,
    font_size: u32,
    position: crate::preferences::TimerPosition,
    margin: u32,
    text: Option<String>,
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<(), String> {
    state.output_engine.set_timer_style(&font, font_size, position, margin);
    state.output_engine.set_timer_preview(text);
    Ok(())
}

/// Enumerate all font family names installed on the system.
///
/// On Windows: uses GDI `EnumFontFamiliesExW` so names match exactly what
/// mpv's `osd-font` property accepts. Vertical-text (`@`-prefixed) families
/// are excluded.
/// On macOS / Linux: shells out to fontconfig's `fc-list`, which both mpv
/// (libass) and WebKit/WebView resolve font names through — so the names
/// returned are guaranteed to match what `osd-font` and the floating timer's
/// CSS `font-family` actually render. Returns an empty list if `fc-list`
/// isn't on PATH (the font field stays free-text in that case).
#[tauri::command]
pub fn list_system_fonts() -> Vec<String> {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::Graphics::Gdi::{
            CreateCompatibleDC, DeleteDC, EnumFontFamiliesExW,
            LOGFONTW, TEXTMETRICW,
        };

        unsafe extern "system" fn enum_cb(
            lpelfe: *const LOGFONTW,
            _: *const TEXTMETRICW,
            _: u32,
            lparam: isize,
        ) -> i32 {
            let list = &mut *(lparam as *mut Vec<String>);
            let face = (*lpelfe).lfFaceName;
            let len = face.iter().position(|&c| c == 0).unwrap_or(32);
            let name = String::from_utf16_lossy(&face[..len]);
            if !name.starts_with('@') {
                list.push(name);
            }
            1
        }

        let mut fonts: Vec<String> = Vec::new();
        unsafe {
            let hdc = CreateCompatibleDC(0);
            if hdc != 0 {
                let mut lf: LOGFONTW = std::mem::zeroed();
                lf.lfCharSet = 1;
                EnumFontFamiliesExW(
                    hdc,
                    &lf,
                    Some(enum_cb),
                    &mut fonts as *mut Vec<String> as isize,
                    0,
                );
                DeleteDC(hdc);
            }
        }
        fonts.sort_by_key(|a| a.to_lowercase());
        fonts.dedup();
        fonts
    }
    #[cfg(not(target_os = "windows"))]
    {
        let output = match std::process::Command::new("fc-list").arg(":").arg("family").output() {
            Ok(o) if o.status.success() => o,
            _ => return Vec::new(),
        };
        let mut fonts: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .flat_map(|line| line.split(','))
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty())
            .collect();
        fonts.sort_by_key(|a| a.to_lowercase());
        fonts.dedup();
        fonts
    }
}

/// Return all available audio output devices for the given backend.
#[tauri::command]
pub fn list_audio_devices(
    backend: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<DeviceInfo>, String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    use crate::preferences::AudioBackend;

    #[allow(unused_variables)] // used only in #[cfg(feature = "asio-support")] block
    let ab: AudioBackend = match backend.as_deref() {
        Some("wasapi_exclusive") => AudioBackend::WasapiExclusive,
        Some("asio") => AudioBackend::Asio,
        _ => AudioBackend::WasapiShared,
    };

    // For ASIO: cpal's output_devices() is unreliable (COM/thread issues).
    // Read driver names directly from the Windows registry instead.
    #[cfg(all(windows, feature = "asio-support"))]
    if matches!(ab, AudioBackend::Asio) {
        return Ok(list_asio_drivers_from_registry());
    }

    let host = cpal::default_host();

    let mut devices = Vec::new();
    if let Ok(iter) = host.output_devices() {
        for device in iter {
            let id = device.id().ok().map(|i| i.id().to_string()).unwrap_or_else(|| device.to_string());
            let name = device.to_string();
            let (channels, sample_rate) = device
                .default_output_config()
                .map(|c| (c.channels(), c.sample_rate()))
                .unwrap_or((2, 44100));
            devices.push(DeviceInfo { id, name, channels, sample_rate });
        }
    }

    // Fallback: if enumeration returned nothing, use the default device.
    if devices.is_empty() {
        if let Some(device) = host.default_output_device() {
            let id = device.id().ok().map(|i| i.id().to_string()).unwrap_or_else(|| device.to_string());
            let name = device.to_string();
            let (channels, sample_rate) = device
                .default_output_config()
                .map(|c| (c.channels(), c.sample_rate()))
                .unwrap_or((2, 44100));
            devices.push(DeviceInfo { id, name, channels, sample_rate });
        }
    }

    // Update engine's device manager cache.
    if let Ok(mut mgr) = state.audio_engine.device_manager.lock() {
        mgr.refresh_devices().ok();
    }

    #[cfg(target_os = "linux")]
    return Ok(crate::engine::device_manager::linux_devices(false, devices));
    #[cfg(not(target_os = "linux"))]
    Ok(devices)
}

/// Play a short 440 Hz sine-wave beep on the specified device and backend.
///
/// For WASAPI backends a temporary stream is opened directly on the selected
/// device — no need to Apply first.  For ASIO (exclusive, single-stream) the
/// beep is routed through the main engine instead.
#[tauri::command]
pub fn test_audio_device(
    device_id: String,
    backend: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    use crate::preferences::AudioBackend;
    let ab: AudioBackend = match backend.as_str() {
        "asio" => AudioBackend::Asio,
        "wasapi_exclusive" => AudioBackend::WasapiExclusive,
        _ => AudioBackend::WasapiShared,
    };

    // ASIO uses exclusive access — can't open a second stream alongside the
    // engine, so play through the existing engine voice path.
    if matches!(ab, AudioBackend::Asio) {
        let sample_rate = state.audio_engine.sample_rate();
        return play_beep_via_engine(sample_rate, &state);
    }

    // For WASAPI: open a temporary independent stream on the selected device.
    // Everything is done inside a thread so cpal's Stream (which contains raw
    // Win32 handles) never crosses a thread boundary.
    let device_id_opt = if device_id.is_empty() { None } else { Some(device_id) };
    play_beep_on_device(device_id_opt);
    Ok(())
}

/// Spawn a background thread that opens a WASAPI stream on `device_name` (or
/// the default device if `None`) and plays a 440 Hz beep for ~400 ms.
///
/// The cpal `Stream` is created and destroyed entirely inside the thread to
/// avoid Send issues with the underlying Win32 handles.
fn play_beep_on_device(device_name: Option<String>) {
    std::thread::spawn(move || {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };

        let host = cpal::default_host();
        let device = match &device_name {
            Some(name) => host
                .output_devices()
                .ok()
                .and_then(|mut it| it.find(|d| d.id().ok().map(|id| id.id() == *name).unwrap_or(false)))
                .or_else(|| host.default_output_device()),
            None => host.default_output_device(),
        };

        let device = match device { Some(d) => d, None => return };
        let config = match device.default_output_config() { Ok(c) => c, Err(_) => return };

        let sample_rate = config.sample_rate();
        let channels = config.channels() as usize;
        let samples = Arc::new(build_beep(sample_rate, channels));
        let pos = Arc::new(AtomicUsize::new(0));
        let (s_cb, p_cb) = (samples.clone(), pos.clone());

        let Ok(stream) = device.build_output_stream(
            config.into(),
            move |data: &mut [f32], _| {
                let start = p_cb.fetch_add(data.len(), Ordering::Relaxed);
                for (i, s) in data.iter_mut().enumerate() {
                    *s = s_cb.get(start + i).copied().unwrap_or(0.0);
                }
            },
            |_| {},
            None,
        ) else { return };

        if stream.play().is_ok() {
            std::thread::sleep(std::time::Duration::from_millis(600));
        }
        // stream drops here, closing the device
    });
}

/// Play a 440 Hz beep through the main audio engine (used for ASIO where only
/// one exclusive stream can be open at a time).
fn play_beep_via_engine(sample_rate: u32, state: &State<'_, AppState>) -> Result<(), String> {
    use crate::engine::voice::Voice;
    use std::sync::Arc;

    let samples = Arc::new(build_beep(sample_rate, 2));
    let voice = Voice::new(samples, 2, sample_rate, 1.0, 0.0);
    state
        .audio_engine
        .play_voice(voice)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Generate a 400 ms 440 Hz sine wave with 20 ms fade-in/out, interleaved for
/// `channels` output channels.
fn build_beep(sample_rate: u32, channels: usize) -> Vec<f32> {
    let n_frames = (sample_rate as f32 * 0.4) as usize;
    let fade = (sample_rate as f32 * 0.02) as usize;
    let mut buf = Vec::with_capacity(n_frames * channels);
    for i in 0..n_frames {
        let t = i as f32 / sample_rate as f32;
        let mut amp = (2.0 * PI * 440.0 * t).sin() * 0.4;
        if i < fade { amp *= i as f32 / fade as f32; }
        if i >= n_frames - fade { amp *= (n_frames - i) as f32 / fade as f32; }
        for _ in 0..channels { buf.push(amp); }
    }
    buf
}

/// Show the Preferences window (pre-created at startup, hidden by default).
///
/// Calling this a second time just brings the window to front.
#[tauri::command]
pub fn open_preferences_window(app_handle: AppHandle) -> Result<(), String> {
    let w = app_handle
        .get_webview_window("preferences")
        .ok_or("preferences window not found")?;
    w.show().map_err(|e| e.to_string())?;
    w.set_focus().map_err(|e| e.to_string())?;
    Ok(())
}
