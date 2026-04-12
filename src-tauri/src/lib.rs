//! WinCue library root.  All modules are declared here and re-exported as needed.

pub mod commands;
pub mod cue;
pub mod engine;
pub mod preferences;
pub mod show;
pub mod state;

use commands::{
    cue_cmds::{
        add_cue, duplicate_cue, get_all_cues, get_cue, get_playhead, get_waveform_peaks,
        move_cue, preview_cue, remove_cue, set_audio_file, set_playhead, stop_preview,
        update_cue,
    },
    device_cmds::{get_output_patches, list_output_devices, refresh_devices, set_output_patch},
    preferences_cmds::{get_asio_output_pairs, get_available_backends, get_preferences, list_audio_devices, test_audio_device, update_audio_preferences, update_general_preferences},
    transport_cmds::{go, hard_stop_all, pause_cue, resume_cue, set_master_volume, stop_all, stop_cue},
    undo_cmds::{can_redo, can_undo, copy_cue, paste_cue, redo, undo},
    workspace_cmds::{get_workspace_info, load_workspace, new_workspace, save_workspace},
};
use state::AppState;
use tauri::Manager;

/// Build and run the Tauri application.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();

    let app_state = AppState::new().expect("Failed to initialise application state");

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(app_state)
        .setup(|app| {
            let handle = app.handle().clone();
            let engine = app.state::<AppState>().audio_engine.clone();
            let workspace = app.state::<AppState>().workspace.clone();

            std::thread::Builder::new()
                .name("wincue-event-loop".to_string())
                .spawn(move || {
                    crate::show::event_loop::run(handle, engine, workspace);
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
            get_waveform_peaks,
            preview_cue,
            stop_preview,
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
