//! Inkue library root.  All modules are declared here and re-exported as needed.

pub mod bundled_fonts;
pub mod commands;
pub mod cue;
pub mod engine;
pub mod health;
pub mod logger;
pub mod machine_config;
pub mod preferences;
pub mod qlab_import;
pub mod recovery;
pub mod show;
pub mod state;

use std::sync::Arc;

use commands::{
    cue_cmds::{
        add_cue, add_cue_to_group, duplicate_cue, duplicate_cues,
        get_all_cues, get_cue, get_playhead,
        get_output_window_visible, get_waveform_peaks, get_normalize_db,
        set_live_level, set_live_crosspoint,
        get_media_thumbnail, get_video_filmstrip, get_video_filmstrip_range, group_cues, identify_output_screen,
        list_camera_devices, list_video_screens, move_cue, move_cues, preview_cue,
        clear_cue_numbers, renumber_cues, renumber_selected_cues,
        move_to_top_level, remove_cue, remove_cues, remove_cue_from_group,
        set_audio_file, set_group_mode, set_playlist_loop, set_image_file, set_midi_file,
        set_playhead, set_video_file, stop_preview, toggle_output_window, ungroup, update_cue,
    },
    cue_list_cmds::{
        add_cue_list, get_cue_lists, remove_cue_list, rename_cue_list,
        set_active_cue_list, set_cue_list_mode,
    },
    device_cmds::{
        get_output_patches, list_input_devices, list_output_devices, refresh_devices,
        open_mixer_window, remove_output_patch, set_default_output_patch, set_output_patch,
        set_output_patch_gain,
    },
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
    midi_cmds::{
        clear_midi_learn, get_cue_midi_trigger, get_midi_trigger_config, learn_midi_trigger,
        list_midi_input_ports, list_midi_output_ports, send_midi_test, set_cue_midi_trigger,
        set_midi_trigger_config,
    },
    network_cmds::{get_network_config, list_network_interfaces, set_network_config},
    osc_cmds::{
        add_osc_patch, get_osc_config, list_osc_patches, remove_osc_patch,
        send_osc_test, set_osc_config, update_osc_patch,
    },
    preferences_cmds::{
        clear_test_pattern, get_asio_output_pairs, get_available_backends,
        get_machine_audio_config, get_output_screen, get_output_transform, get_preferences,
        list_audio_devices, list_system_fonts, open_preferences_window, preview_output_timer,
        set_output_screen, set_output_transform, show_test_pattern, test_audio_device,
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
    workspace_cmds::{
        collect_and_save_workspace, get_workspace_info, import_qlab_workspace, load_workspace,
        new_workspace, save_workspace,
    },
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
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
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

            // Neither engine failing is fatal.  A `setup()` that returns Err
            // tears the process down after the window is already on screen —
            // the app "flashes and disappears" with the reason going only to
            // stderr, which nobody sees when launching from a desktop icon or
            // an AppImage.  Both engines degrade instead, and the reason lands
            // in the health banner where the operator can act on it.
            let audio_engine = match AudioEngine::new(&machine_config) {
                Ok(engine) => engine,
                Err(e) => {
                    log::error!("[startup] no audio output device ({e}) — running silent");
                    AudioEngine::new_silent(&machine_config)
                }
            };
            let output_engine = Arc::new(
                match OutputEngine::new(Arc::clone(&audio_engine), app.handle().clone()) {
                    Ok(engine) => engine,
                    Err(e) => {
                        log::error!("[startup] video output unavailable ({e}) — running headless");
                        // The banner is one line: it carries the fix, not the
                        // diagnosis — the full error (searched paths, GL/window
                        // failure) is in the log the operator can open.
                        crate::health::set(crate::health::HealthAlert::new(
                            "video-output",
                            crate::health::HealthLevel::Error,
                            format!("Video output unavailable — {}", install_libmpv_hint()),
                        ));
                        OutputEngine::new_headless(
                            Arc::clone(&audio_engine),
                            app.handle().clone(),
                        )
                    }
                },
            );

            // Pin all network traffic to the configured interface (must run
            // before the OSC server and any DMX sink binds a socket).
            crate::engine::net_interface::apply(&crate::machine_config::load_network());

            let osc_config = crate::machine_config::load_osc();
            crate::engine::osc_feedback::apply(
                osc_config.feedback_enabled,
                osc_config.feedback_host.clone(),
                osc_config.feedback_port,
                osc_config.feedback_progress_hz,
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
                    .name("inkue-dmx-monitor".to_string())
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
            let tc_receiver = app.state::<AppState>().tc_receiver.clone();
            let tc_event_rx = tc_receiver.lock().ok()
                .and_then(|opt| opt.as_ref().map(|r| r.subscribe()));

            // Start the per-cue MIDI trigger listener if this machine has it on.
            let midi_listener = app.state::<AppState>().midi_listener.clone();
            {
                let config = crate::machine_config::load_midi_trigger_config();
                if config.enabled {
                    if let Ok(mut slot) = midi_listener.lock() {
                        *slot = Some(Arc::new(
                            crate::engine::midi_trigger::MidiTriggerListener::new(config.port),
                        ));
                    }
                }
            }
            let midi_listener_for_loop = midi_listener.clone();

            std::thread::Builder::new()
                .name("inkue-event-loop".to_string())
                .spawn(move || {
                    crate::show::event_loop::run(
                        handle, a_engine, o_engine, d_engine, workspace, tc_event_rx,
                        midi_listener_for_loop,
                        tc_receiver,
                    );
                })
                .expect("Failed to spawn event loop thread");

            // ----------------------------------------------------------------
            // Crash-recovery autosave: snapshot unsaved work every few seconds.
            // ----------------------------------------------------------------
            {
                let ws = app.state::<AppState>().workspace.clone();
                std::thread::Builder::new()
                    .name("inkue-autosave".to_string())
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
                    .name("inkue-log-emitter".to_string())
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
                    .name("inkue-device-watchdog".to_string())
                    .spawn(move || {
                        use std::sync::atomic::Ordering;
                        use tauri::Emitter;
                        use crate::health::{self, HealthAlert, HealthLevel};

                        /// Watchdog ticks between two retries when the machine
                        /// has no usable output device at all (2 s per tick).
                        const RETRY_EVERY_TICKS: u32 = 5;

                        let mut last_seq = 0u64;
                        let mut last_count = engine.callback_count();
                        let mut ticks_since_retry = 0u32;
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
                                // Recover on first detection — a device lost
                                // mid-show must switch over now, not in 10 s —
                                // then only every RETRY_EVERY_TICKS, so a
                                // machine with no usable device at all does not
                                // re-enumerate (and log) on every tick.
                                let attempt =
                                    ticks_since_retry == 0 || ticks_since_retry >= RETRY_EVERY_TICKS;
                                ticks_since_retry = if attempt { 1 } else { ticks_since_retry + 1 };

                                match h.desired_device.clone() {
                                    Some(dev) => {
                                        let took_over =
                                            attempt && engine.fall_back_to_default().is_some();
                                        if took_over {
                                            last_count = engine.callback_count();
                                        }
                                        health::set(HealthAlert::new(
                                            "audio-device",
                                            HealthLevel::Error,
                                            if took_over {
                                                format!("Audio device \"{dev}\" lost — switched to the default device")
                                            } else {
                                                format!("Audio device \"{dev}\" unavailable — no audio output")
                                            },
                                        ));
                                    }
                                    // Nothing to fall back to: the machine has no
                                    // usable output device (silent startup, or the
                                    // last one was unplugged).  Audio returns by
                                    // itself once an interface is plugged in.
                                    None => {
                                        if attempt && engine.retry_output_stream() {
                                            last_count = engine.callback_count();
                                            health::clear("audio-device");
                                        } else {
                                            health::set(HealthAlert::new(
                                                "audio-device",
                                                HealthLevel::Error,
                                                "No audio output device available",
                                            ));
                                        }
                                    }
                                }
                            } else if h.in_fallback {
                                let dev = h.desired_device.clone().unwrap_or_default();
                                if h.desired_present {
                                    health::set(
                                        HealthAlert::new(
                                            "audio-device",
                                            HealthLevel::Warning,
                                            format!("Audio device \"{dev}\" is back"),
                                        )
                                        .with_action("restore_audio_device", "Switch back"),
                                    );
                                } else {
                                    health::set(HealthAlert::new(
                                        "audio-device",
                                        HealthLevel::Error,
                                        format!(
                                            "Audio device \"{dev}\" lost — playing on the default device"
                                        ),
                                    ));
                                }
                            } else {
                                // Healthy: arm an immediate recovery attempt
                                // for the next failure.
                                ticks_since_retry = 0;
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
            renumber_cues,
            renumber_selected_cues,
            clear_cue_numbers,
            duplicate_cue,
            duplicate_cues,
            group_cues,
            ungroup,
            set_group_mode,
            set_playlist_loop,
            add_cue_to_group,
            remove_cue_from_group,
            move_to_top_level,
            update_cue,
            set_playhead,
            get_playhead,
            set_audio_file,
            set_video_file,
            set_image_file,
            set_midi_file,
            get_waveform_peaks,
            get_normalize_db,
            set_live_level,
            set_live_crosspoint,
            list_video_screens,
            identify_output_screen,
            list_camera_devices,
            preview_cue,
            stop_preview,
            toggle_output_window,
            get_output_window_visible,
            get_media_thumbnail,
            get_video_filmstrip,
            get_video_filmstrip_range,
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
            import_qlab_workspace,
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
            remove_output_patch,
            set_default_output_patch,
            set_output_patch_gain,
            open_mixer_window,
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
            get_output_transform,
            set_output_transform,
            show_test_pattern,
            clear_test_pattern,
            // MIDI
            list_midi_output_ports,
            send_midi_test,
            list_midi_input_ports,
            get_midi_trigger_config,
            set_midi_trigger_config,
            get_cue_midi_trigger,
            set_cue_midi_trigger,
            learn_midi_trigger,
            clear_midi_learn,
            // Network
            list_network_interfaces,
            get_network_config,
            set_network_config,
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
        .expect("error while running Inkue");
}

/// Per-OS instruction for restoring video output, appended to the health banner.
///
/// libmpv is bundled on Windows and inside the macOS `.app`, so there it points
/// at a broken install.  On Linux it is a system dependency the `.deb` pulls in
/// but the AppImage cannot declare — the one case where the operator genuinely
/// has to install something.
fn install_libmpv_hint() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        "install libmpv (Arch: mpv · Debian/Ubuntu: libmpv2 · Fedora: mpv-libs) and restart"
    }
    #[cfg(not(target_os = "linux"))]
    {
        "reinstall Inkue to restore the bundled libmpv"
    }
}
