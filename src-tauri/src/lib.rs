//! WinCue library root.  All modules are declared here and re-exported as needed.

pub mod commands;
pub mod cue;
pub mod engine;
pub mod machine_config;
pub mod preferences;
pub mod show;
pub mod state;

use std::sync::Arc;

use commands::{
    cue_cmds::{
        add_cue, add_cue_to_group, duplicate_cue, duplicate_cues,
        get_all_cues, get_cue, get_playhead,
        get_output_window_visible, get_waveform_peaks,
        group_cues, list_video_screens, move_cue, move_cues, preview_cue,
        move_to_top_level, remove_cue, remove_cues, remove_cue_from_group,
        set_audio_file, set_group_mode, set_image_file, set_playhead,
        set_video_file, stop_preview, toggle_output_window, ungroup, update_cue,
    },
    device_cmds::{get_output_patches, list_output_devices, refresh_devices, set_output_patch},
    preferences_cmds::{
        get_asio_output_pairs, get_available_backends, get_machine_audio_config,
        get_output_screen, get_preferences, list_audio_devices, list_system_fonts,
        preview_output_timer, set_output_screen, test_audio_device,
        update_audio_preferences, update_display_preferences,
        update_general_preferences, update_machine_audio_config,
    },
    transport_cmds::{
        go, hard_stop_all, pause_cue, resume_cue, seek_cue, set_master_volume, stop_all, stop_cue,
    },
    undo_cmds::{can_redo, can_undo, copy_cue, paste_cue, redo, undo},
    workspace_cmds::{get_workspace_info, load_workspace, new_workspace, save_workspace},
};
use engine::{AudioEngine, OutputEngine};
use state::AppState;
use tauri::Manager;

/// Build and run the Tauri application.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("wincue=info"),
    )
    .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            // ----------------------------------------------------------------
            // Initialise engines and managed state.
            // OutputEngine creates the persistent Win32 window + libmpv context
            // at startup (window is shown immediately — no first-GO freeze).
            // ----------------------------------------------------------------
            let machine_config = crate::machine_config::load();
            let audio_engine = AudioEngine::new(&machine_config).map_err(|e| {
                show_fatal_error(&format!("Audio engine failed to start:\n\n{e}"));
                Box::new(std::io::Error::other(e.to_string())) as Box<dyn std::error::Error>
            })?;
            let output_engine = Arc::new(
                OutputEngine::new(Arc::clone(&audio_engine))
                    .map_err(|e| {
                        show_fatal_error(&format!("Output engine failed to start:\n\n{e}"));
                        Box::new(std::io::Error::other(e.to_string())) as Box<dyn std::error::Error>
                    })?,
            );

            let app_state = AppState::new(audio_engine, Arc::clone(&output_engine));
            app.manage(app_state);

            // ----------------------------------------------------------------
            // Start the 30 fps event loop on a dedicated thread.
            // ----------------------------------------------------------------
            let handle = app.handle().clone();
            let a_engine = app.state::<AppState>().audio_engine.clone();
            let o_engine = Arc::clone(&output_engine);
            let workspace = app.state::<AppState>().workspace.clone();

            std::thread::Builder::new()
                .name("wincue-event-loop".to_string())
                .spawn(move || {
                    crate::show::event_loop::run(handle, a_engine, o_engine, workspace);
                })
                .expect("Failed to spawn event loop thread");

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Transport
            go,
            stop_all,
            hard_stop_all,
            stop_cue,
            pause_cue,
            resume_cue,
            seek_cue,
            set_master_volume,
            // Cues
            get_all_cues,
            get_cue,
            add_cue,
            remove_cue,
            remove_cues,
            move_cue,
            move_cues,
            duplicate_cue,
            duplicate_cues,
            group_cues,
            ungroup,
            set_group_mode,
            add_cue_to_group,
            remove_cue_from_group,
            move_to_top_level,
            update_cue,
            set_playhead,
            get_playhead,
            set_audio_file,
            set_video_file,
            set_image_file,
            get_waveform_peaks,
            list_video_screens,
            preview_cue,
            stop_preview,
            toggle_output_window,
            get_output_window_visible,
            // Undo / Redo / Copy / Paste
            undo,
            redo,
            can_undo,
            can_redo,
            copy_cue,
            paste_cue,
            // Workspace
            new_workspace,
            save_workspace,
            load_workspace,
            get_workspace_info,
            // Devices
            list_output_devices,
            get_output_patches,
            set_output_patch,
            refresh_devices,
            // Preferences
            get_preferences,
            get_machine_audio_config,
            update_machine_audio_config,
            update_audio_preferences,
            update_general_preferences,
            update_display_preferences,
            list_audio_devices,
            list_system_fonts,
            preview_output_timer,
            test_audio_device,
            get_available_backends,
            get_asio_output_pairs,
            get_output_screen,
            set_output_screen,
        ])
        .run(tauri::generate_context!())
        .expect("error while running WinCue");
}

/// Show a blocking error dialog — used when a fatal startup error occurs in
/// a release build where there is no console to read stderr from.
#[cfg(target_os = "windows")]
fn show_fatal_error(message: &str) {
    use std::ffi::OsStr;
    use std::iter::once;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

    let title: Vec<u16> = OsStr::new("WinCue — Startup Error")
        .encode_wide()
        .chain(once(0))
        .collect();
    let body: Vec<u16> = OsStr::new(message).encode_wide().chain(once(0)).collect();
    unsafe {
        MessageBoxW(0, body.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR);
    }
}

#[cfg(not(target_os = "windows"))]
fn show_fatal_error(message: &str) {
    eprintln!("FATAL: {message}");
}
