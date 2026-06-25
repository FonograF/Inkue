//! WinCue library root.  All modules are declared here and re-exported as needed.

pub mod bundled_fonts;
pub mod commands;
pub mod cue;
pub mod engine;
pub mod health;
pub mod logger;
pub mod machine_config;
pub mod preferences;
pub mod recovery;
pub mod show;
pub mod state;

use std::sync::Arc;

use commands::{
    cue_cmds::{
        add_cue, add_cue_to_group, duplicate_cue, duplicate_cues,
        get_all_cues, get_cue, get_playhead,
        get_output_window_visible, get_waveform_peaks, get_normalize_db,
        group_cues, list_video_screens, move_cue, move_cues, preview_cue,
        move_to_top_level, remove_cue, remove_cues, remove_cue_from_group,
        set_audio_file, set_group_mode, set_image_file, set_playhead,
        set_video_file, stop_preview, toggle_output_window, ungroup, update_cue,
    },
    cue_list_cmds::{
        add_cue_list, get_cue_lists, remove_cue_list, rename_cue_list,
        set_active_cue_list, set_cue_list_mode,
    },
    device_cmds::{get_output_patches, list_input_devices, list_output_devices, refresh_devices, set_output_patch},
    input_cmds::{add_input_patch, list_input_patches, remove_input_patch, update_input_patch},
    timecode_cmds::{
        get_tc_config, set_tc_config, get_tc_position,
        list_tc_midi_input_ports,
        get_cue_tc_trigger, set_cue_tc_trigger,
        get_cuelist_tc_config, set_cuelist_tc_config,
    },
    light_cmds::{
        add_fixture, add_fixture_group, capture_live_targets, dmx_clear_fixtures, dmx_get_blackout,
        dmx_get_outputs, dmx_get_snapshot, dmx_set_blackout, dmx_set_channel, dmx_set_fixture_param,
        dmx_set_outputs, dmx_test_fixture, get_fixture_conflicts, list_fixtures, list_fixture_groups,
        list_builtin_fixture_types, remove_fixture, remove_fixture_group, update_fixture,
        update_fixture_group,
    },
    midi_cmds::{list_midi_output_ports, send_midi_test},
    osc_cmds::{
        add_osc_patch, get_osc_config, list_osc_patches, remove_osc_patch,
        send_osc_test, set_osc_config, update_osc_patch,
    },
    preferences_cmds::{
        get_asio_output_pairs, get_available_backends, get_machine_audio_config,
        get_output_screen, get_preferences, list_audio_devices, list_system_fonts,
        open_preferences_window, preview_output_timer, set_output_screen, test_audio_device,
        update_audio_preferences, update_display_preferences,
        update_general_preferences, update_machine_audio_config,
    },
    transport_cmds::{
        go, go_cue, hard_stop_all, pause_cue, resume_cue, seek_cue,
        set_master_volume, stop_all, stop_cue,
    },
    undo_cmds::{can_redo, can_undo, copy_cue, paste_cue, redo, undo},
    health_cmds::{get_health_alerts, restore_audio_device},
    log_cmds::{clear_logs, get_recent_logs, open_logs_folder},
    preflight_cmds::{check_workspace, relink_media},
    recovery_cmds::{check_recovery, discard_recovery, restore_recovery},
    workspace_cmds::{collect_and_save_workspace, get_workspace_info, load_workspace, new_workspace, save_workspace},
};
use engine::{AudioEngine, DmxEngine, OscServer, OutputEngine};
use state::AppState;
use tauri::Manager;

/// Build and run the Tauri application.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Custom logger: stderr + rotating file in the config dir + in-memory ring
    // buffer for the in-app log viewer.  RUST_LOG=debug/trace still bumps the level.
    crate::logger::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .on_window_event(|window, event| {
            // When the main window is destroyed, the Win32 output-window thread
            // and the audio / event-loop threads keep the process alive
            // indefinitely.  Force-exit so the OS cleans everything up.
            if matches!(event, tauri::WindowEvent::Destroyed) && window.label() == "main" {
                // Deliberate close (whatever the save/discard choice): drop the
                // crash-recovery snapshot so the next launch does not offer to
                // restore work the operator already decided about.
                crate::recovery::delete();
                std::process::exit(0);
            }
        })
        .setup(|app| {
            // ----------------------------------------------------------------
            // Initialise engines and managed state.
            // OutputEngine creates the persistent Win32 window + libmpv context
            // at startup (window is shown immediately — no first-GO freeze).
            // ----------------------------------------------------------------
            crate::bundled_fonts::ensure_installed();
            let machine_config = crate::machine_config::load();
            // Inject the machine's buffer size into the workspace AudioPreferences
            // so CueContext can pass it to ensure_input_feed for Mic Cues.
            {
                // AppState is not yet created; we stash it into a global so the
                // .setup() callback can read it.  Simpler: just store it and apply
                // it after app.manage() below.
            }
            let startup_buffer_size = machine_config.buffer_size;
            let audio_engine = AudioEngine::new(&machine_config).map_err(|e| {
                show_fatal_error(&format!("Audio engine failed to start:\n\n{e}"));
                Box::new(std::io::Error::other(e.to_string())) as Box<dyn std::error::Error>
            })?;
            let output_engine = Arc::new(
                OutputEngine::new(Arc::clone(&audio_engine), app.handle().clone())
                    .map_err(|e| {
                        show_fatal_error(&format!("Output engine failed to start:\n\n{e}"));
                        Box::new(std::io::Error::other(e.to_string())) as Box<dyn std::error::Error>
                    })?,
            );

            let osc_config = crate::machine_config::load_osc();
            crate::engine::osc_feedback::apply(
                osc_config.feedback_enabled,
                osc_config.feedback_host.clone(),
                osc_config.feedback_port,
            );
            let app_handle_osc = app.handle().clone();
            let osc_server = Arc::new(OscServer::start(osc_config, app_handle_osc));

            // DMX lighting engine — owns its own ~40Hz output thread.
            let dmx_engine = Arc::new(DmxEngine::new());

            // Timecode receiver — start with no config (MTC, default port).
            // The operator configures it via Preferences.
            let tc_config = crate::machine_config::load_tc_config();
            let tc_receiver = if tc_config.enabled {
                Some(crate::engine::timecode_receiver::TimecodeReceiver::new(
                    tc_config.receiver_config.clone(),
                ))
            } else {
                None
            };

            let app_state = AppState::new(
                audio_engine,
                Arc::clone(&output_engine),
                Arc::clone(&osc_server),
                Arc::clone(&dmx_engine),
                tc_receiver,
            );
            app.manage(app_state);
            // Inject the machine buffer size into the runtime audio prefs so that
            // the CueContext can forward it to ensure_input_feed for Mic Cues.
            {
                if let Ok(mut ws) = app.state::<AppState>().workspace.lock() {
                    ws.preferences.audio.audio_buffer_size = startup_buffer_size;
                }
            }

            // DMX monitor: push live universe values to the UI (event, not poll),
            // ~20 fps and only when the values actually change.
            {
                let monitor_handle = app.handle().clone();
                let dmx = Arc::clone(&dmx_engine);
                std::thread::Builder::new()
                    .name("wincue-dmx-monitor".to_string())
                    .spawn(move || {
                        use tauri::Emitter;
                        let mut last: Vec<commands::light_cmds::DmxUniverseSnapshot> = Vec::new();
                        loop {
                            std::thread::sleep(std::time::Duration::from_millis(50));
                            let snap = commands::light_cmds::snapshot_dto(&dmx);
                            if snap != last {
                                let _ = monitor_handle.emit("dmx-monitor", &snap);
                                last = snap;
                            }
                        }
                    })
                    .expect("Failed to spawn DMX monitor thread");
            }

            // ----------------------------------------------------------------
            // Start the 30 fps event loop on a dedicated thread.
            // ----------------------------------------------------------------
            let handle = app.handle().clone();
            let a_engine = app.state::<AppState>().audio_engine.clone();
            let o_engine = Arc::clone(&output_engine);
            let d_engine = Arc::clone(&dmx_engine);
            let workspace = app.state::<AppState>().workspace.clone();
            // Subscribe to TC events for the dispatcher in the event loop.
            let tc_event_rx = app.state::<AppState>()
                .tc_receiver.lock().ok()
                .and_then(|opt| opt.as_ref().map(|r| r.subscribe()));

            std::thread::Builder::new()
                .name("wincue-event-loop".to_string())
                .spawn(move || {
                    crate::show::event_loop::run(handle, a_engine, o_engine, d_engine, workspace, tc_event_rx);
                })
                .expect("Failed to spawn event loop thread");

            // ----------------------------------------------------------------
            // Crash-recovery autosave: snapshot unsaved work every few seconds.
            // ----------------------------------------------------------------
            {
                let ws = app.state::<AppState>().workspace.clone();
                std::thread::Builder::new()
                    .name("wincue-autosave".to_string())
                    .spawn(move || {
                        enum Action { Write(String), Clear, Idle }
                        // u64::MAX forces a first evaluation; the pristine
                        // "Untitled" workspace (is_modified == false) yields Clear.
                        let mut last_rev: u64 = u64::MAX;
                        // Only track files written by THIS session. A pre-existing
                        // recovery file belongs to the previous session — it must
                        // not be deleted here (the user hasn't responded to the
                        // recovery prompt yet). It is removed by discard_recovery()
                        // or by the clean-exit WindowEvent::Destroyed handler.
                        let mut on_disk = false;
                        loop {
                            std::thread::sleep(std::time::Duration::from_secs(3));
                            let action = match ws.lock() {
                                Ok(w) => {
                                    if !w.is_modified {
                                        last_rev = w.revision;
                                        Action::Clear
                                    } else if w.revision != last_rev {
                                        last_rev = w.revision;
                                        match w.to_recovery_json() {
                                            Ok(json) => Action::Write(json),
                                            Err(e) => {
                                                log::warn!("[autosave] serialize failed: {e}");
                                                Action::Idle
                                            }
                                        }
                                    } else {
                                        Action::Idle
                                    }
                                }
                                Err(_) => Action::Idle,
                            };
                            match action {
                                Action::Write(json) => match crate::recovery::write(&json) {
                                    Ok(()) => on_disk = true,
                                    Err(e) => log::warn!("[autosave] write failed: {e}"),
                                },
                                Action::Clear => {
                                    if on_disk {
                                        crate::recovery::delete();
                                        on_disk = false;
                                    }
                                }
                                Action::Idle => {}
                            }
                        }
                    })
                    .expect("Failed to spawn autosave thread");
            }

            // ----------------------------------------------------------------
            // Log viewer feed: tell the UI when new log lines are available so
            // the in-app viewer can live-tail without polling.  Fires at most
            // ~2×/s and only when the log sequence actually advanced.
            // ----------------------------------------------------------------
            {
                let handle = app.handle().clone();
                std::thread::Builder::new()
                    .name("wincue-log-emitter".to_string())
                    .spawn(move || {
                        use std::sync::atomic::Ordering;
                        use tauri::Emitter;
                        let mut last = 0u64;
                        loop {
                            std::thread::sleep(std::time::Duration::from_millis(500));
                            let seq = crate::logger::SEQ.load(Ordering::Relaxed);
                            if seq != last {
                                last = seq;
                                let _ = handle.emit("logs-updated", ());
                            }
                        }
                    })
                    .expect("Failed to spawn log-emitter thread");
            }

            // ----------------------------------------------------------------
            // Device watchdog: detect a lost audio output device mid-show, fall
            // back to the system default to keep the show audible, and surface a
            // non-blocking banner (with a "restore" action when the device
            // returns).  In the healthy steady state this is just an atomic read.
            // ----------------------------------------------------------------
            {
                let handle = app.handle().clone();
                let engine = app.state::<AppState>().audio_engine.clone();
                std::thread::Builder::new()
                    .name("wincue-device-watchdog".to_string())
                    .spawn(move || {
                        use std::sync::atomic::Ordering;
                        use tauri::Emitter;
                        use crate::health::{self, HealthAlert, HealthLevel};

                        let mut last_seq = 0u64;
                        let mut last_count = engine.callback_count();
                        // Hysteresis: only report "device is back" after it has
                        // appeared in enumeration for this many consecutive ticks
                        // (avoids a false-positive flash when PipeWire is slow to
                        // remove a just-unplugged device from its node list).
                        let mut desired_present_streak: u32 = 0;
                        const DESIRED_PRESENT_MIN_TICKS: u32 = 2;
                        loop {
                            std::thread::sleep(std::time::Duration::from_secs(2));

                            // Heartbeat: if the output callback stopped firing over
                            // the last tick, the stream is dead even if cpal raised
                            // no error (device-loss detection, kind-agnostic).
                            let count = engine.callback_count();
                            let stalled = count == last_count;
                            last_count = count;

                            let h = engine.audio_health();
                            let failed = h.failed || stalled;
                            if failed && !h.in_fallback {
                                if h.desired_device.is_some() {
                                    let lost = engine.fall_back_to_default().unwrap_or_default();
                                    health::set(HealthAlert::new(
                                        "audio-device",
                                        HealthLevel::Error,
                                        format!(
                                            "Audio device \"{lost}\" lost — switched to the default device"
                                        ),
                                    ));
                                } else {
                                    health::set(HealthAlert::new(
                                        "audio-device",
                                        HealthLevel::Error,
                                        "Default audio device unavailable",
                                    ));
                                }
                            } else if h.in_fallback {
                                let dev = h.desired_device.clone().unwrap_or_default();
                                if h.desired_present {
                                    desired_present_streak += 1;
                                } else {
                                    desired_present_streak = 0;
                                }
                                if desired_present_streak >= DESIRED_PRESENT_MIN_TICKS {
                                    health::set(
                                        HealthAlert::new(
                                            "audio-device",
                                            HealthLevel::Warning,
                                            format!("Audio device \"{dev}\" is back — cues are paused"),
                                        )
                                        .with_action("restore_audio_device", "Switch back & resume"),
                                    );
                                } else {
                                    health::set(HealthAlert::new(
                                        "audio-device",
                                        HealthLevel::Error,
                                        format!("Audio device \"{dev}\" lost — cues paused"),
                                    ));
                                }
                            } else {
                                desired_present_streak = 0;
                                health::clear("audio-device");
                            }

                            let seq = health::SEQ.load(Ordering::Relaxed);
                            if seq != last_seq {
                                last_seq = seq;
                                let _ = handle.emit("health-changed", ());
                            }
                        }
                    })
                    .expect("Failed to spawn device-watchdog thread");
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Transport
            go,
            go_cue,
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
            get_normalize_db,
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
            collect_and_save_workspace,
            check_recovery,
            restore_recovery,
            discard_recovery,
            check_workspace,
            relink_media,
            get_recent_logs,
            clear_logs,
            open_logs_folder,
            get_health_alerts,
            restore_audio_device,
            // Cue Lists
            get_cue_lists,
            add_cue_list,
            remove_cue_list,
            rename_cue_list,
            set_active_cue_list,
            set_cue_list_mode,
            // Timecode
            get_tc_config,
            set_tc_config,
            get_tc_position,
            list_tc_midi_input_ports,
            get_cue_tc_trigger,
            set_cue_tc_trigger,
            get_cuelist_tc_config,
            set_cuelist_tc_config,
            // Devices
            list_output_devices,
            list_input_devices,
            list_input_patches,
            add_input_patch,
            update_input_patch,
            remove_input_patch,
            get_output_patches,
            set_output_patch,
            refresh_devices,
            // Preferences
            get_preferences,
            get_machine_audio_config,
            update_machine_audio_config,
            open_preferences_window,
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
            // MIDI
            list_midi_output_ports,
            send_midi_test,
            // OSC
            list_osc_patches,
            add_osc_patch,
            update_osc_patch,
            remove_osc_patch,
            get_osc_config,
            set_osc_config,
            send_osc_test,
            // DMX / Lighting
            dmx_set_outputs,
            dmx_get_outputs,
            dmx_set_channel,
            dmx_set_blackout,
            dmx_get_blackout,
            dmx_get_snapshot,
            dmx_test_fixture,
            dmx_set_fixture_param,
            dmx_clear_fixtures,
            capture_live_targets,
            list_builtin_fixture_types,
            list_fixtures,
            add_fixture,
            update_fixture,
            remove_fixture,
            get_fixture_conflicts,
            list_fixture_groups,
            add_fixture_group,
            update_fixture_group,
            remove_fixture_group,
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
