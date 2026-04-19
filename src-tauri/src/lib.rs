//! WinCue library root.  All modules are declared here and re-exported as needed.

pub mod commands;
pub mod cue;
pub mod engine;
pub mod preferences;
pub mod show;
pub mod state;

use std::sync::Arc;

use commands::{
    cue_cmds::{
        add_cue, duplicate_cue, get_all_cues, get_cue, get_image_surface_data, get_playhead,
        get_waveform_peaks, list_video_screens, move_cue, preview_cue, remove_cue,
        report_image_faded_out, set_audio_file, set_image_file, set_playhead, set_video_file,
        stop_preview, update_cue,
    },
    device_cmds::{get_output_patches, list_output_devices, refresh_devices, set_output_patch},
    preferences_cmds::{
        get_asio_output_pairs, get_available_backends, get_preferences, list_audio_devices,
        test_audio_device, update_audio_preferences, update_general_preferences,
    },
    transport_cmds::{
        go, hard_stop_all, pause_cue, resume_cue, set_master_volume, stop_all, stop_cue,
    },
    undo_cmds::{can_redo, can_undo, copy_cue, paste_cue, redo, undo},
    workspace_cmds::{get_workspace_info, load_workspace, new_workspace, save_workspace},
};
use engine::{AudioEngine, ImageEngine, VideoEngine};
use state::AppState;
use tauri::Manager;
/// Build and run the Tauri application.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Default to info-level for the wincue crate so mpv renderer messages are
    // visible in the terminal without needing to set RUST_LOG manually.
    // Override with: RUST_LOG=debug pnpm tauri dev
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
            // VideoEngine creates its own native Win32 window + libmpv context.
            // ----------------------------------------------------------------
            let audio_engine = AudioEngine::new()?;
            let video_engine = Arc::new(
                VideoEngine::new(Arc::clone(&audio_engine))
                    .map_err(|e| Box::new(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
                        as Box<dyn std::error::Error>)?,
            );
            let image_engine = Arc::new(ImageEngine::new(app.handle().clone()));

            let app_state = AppState::new(
                audio_engine,
                Arc::clone(&video_engine),
                Arc::clone(&image_engine),
            );
            app.manage(app_state);

            // ----------------------------------------------------------------
            // Start the 30 fps event loop on a dedicated thread.
            // ----------------------------------------------------------------
            let handle = app.handle().clone();
            let a_engine = app.state::<AppState>().audio_engine.clone();
            let v_engine = Arc::clone(&video_engine);
            let i_engine = Arc::clone(&image_engine);
            let workspace = app.state::<AppState>().workspace.clone();

            std::thread::Builder::new()
                .name("wincue-event-loop".to_string())
                .spawn(move || {
                    crate::show::event_loop::run(handle, a_engine, v_engine, i_engine, workspace);
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
            set_master_volume,
            // Cues
            get_all_cues,
            get_cue,
            add_cue,
            remove_cue,
            move_cue,
            duplicate_cue,
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
            get_image_surface_data,
            report_image_faded_out,
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
            update_audio_preferences,
            update_general_preferences,
            list_audio_devices,
            test_audio_device,
            get_available_backends,
            get_asio_output_pairs,
        ])
        .run(tauri::generate_context!())
        .expect("error while running WinCue");
}
