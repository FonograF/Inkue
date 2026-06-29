//! Background event loop running at ~30 fps.
//!
//! This task bridges the engines and the Tauri frontend:
//! - Drains [`AudioStatus`] messages from the audio engine's ring buffer.
//! - Drains [`OutputStatus`] messages from the output engine's channel.
//! - Marks cues as completed when their voice ends.
//! - Applies video duration updates to the owning cue.
//! - Fires Auto-Continue chains (Post-Wait based).
//! - Emits `cue-state-changed`, `cue-time-update`, and `master-level`
//!   Tauri events so the UI stays in sync without polling.
//! - Calls [`AudioEngine::gc_voices`] to release stopped audio voice memory.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::Emitter;

use crate::{
    cue::{
        context::{CueContext, CueEvent},
        types::{ContinueMode, CueId, CueState, CueType},
    },
    engine::{
        output_engine::{OutputEngine, OutputStatus},
        ring_command::AudioStatus,
        timecode_types::TcEvent,
        AudioEngine, DmxEngine,
    },
    show::{transport::Transport, workspace::Workspace},
};

/// Target tick interval for the main event loop (~30 fps).
const TICK_MS: u64 = 33;
/// Timer overlay refresh interval — fast enough for smooth millisecond display.
const TIMER_TICK_MS: u64 = 16;
/// If the output stream produces no callback for this long, the audio is frozen
/// (device lost / mid-switch) and running audio cues are paused so their
/// timeline does not drift past the frozen audio.  Above a planned switch's
/// ~tens-of-ms gap, below a perceptible delay.
const AUDIO_FREEZE_MS: u64 = 250;

/// Entry point for the event loop thread.  Loops indefinitely.
pub fn run(
    handle: tauri::AppHandle,
    audio_engine: Arc<AudioEngine>,
    output_engine: Arc<OutputEngine>,
    dmx_engine: Arc<DmxEngine>,
    workspace: Arc<Mutex<Workspace>>,
    tc_rx: Option<crossbeam_channel::Receiver<TcEvent>>,
) {
    // Spawn a dedicated thread that refreshes the OSD timer overlay at ~60 fps.
    // This is independent of the main 30 fps tick so the millisecond display
    // stays smooth even when the workspace lock is briefly held by a command.
    {
        let ws2 = Arc::clone(&workspace);
        let oe2 = Arc::clone(&output_engine);
        std::thread::Builder::new()
            .name("inkue-timer-refresh".into())
            .spawn(move || timer_refresh_loop(ws2, oe2))
            .expect("Failed to spawn timer refresh thread");
    }

    // Maps a completed cue's ID to the deadline and the ID of its cue list.
    let mut auto_follow_pending: HashMap<CueId, (Instant, uuid::Uuid)> = HashMap::new();
    // Last TC position seen — for the TC dispatcher monotone guard.
    let mut prev_tc_frame: Option<u64> = None;
    // Per-group snapshot: (active_child_id, any_child_running).
    // Used to detect inner-sequence progress and emit cue-list-refresh.
    let mut prev_group_state: HashMap<CueId, (Option<CueId>, bool)> = HashMap::new();
    // Cue sets tracked for OSC feedback (compared each tick to detect changes).
    let mut prev_running_cues: Vec<CueId>  = Vec::new();
    let mut prev_playhead_cue: Option<CueId> = None;
    // Fingerprint of the full cue list (number+name). Sending on change.
    let mut prev_cue_list_hash: u64 = 0;
    // Audio-freeze guard state (device-loss timeline pause).
    let mut last_cb_count: u64 = audio_engine.callback_count();
    let mut cb_last_advance: Instant = Instant::now();
    let mut audio_frozen = false;
    let mut auto_paused: std::collections::HashSet<CueId> = std::collections::HashSet::new();

    loop {
        std::thread::sleep(Duration::from_millis(TICK_MS));
        tick(
            &handle,
            &audio_engine,
            &output_engine,
            &dmx_engine,
            &workspace,
            tc_rx.as_ref(),
            &mut prev_tc_frame,
            &mut auto_follow_pending,
            &mut prev_group_state,
            &mut prev_running_cues,
            &mut prev_playhead_cue,
            &mut prev_cue_list_hash,
            &mut last_cb_count,
            &mut cb_last_advance,
            &mut audio_frozen,
            &mut auto_paused,
        );
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn make_context(
    audio_engine: &Arc<AudioEngine>,
    output_engine: &Arc<OutputEngine>,
    dmx_engine: &Arc<DmxEngine>,
    stop_fade_ms: u32,
    output_patches: Vec<crate::engine::device_manager::OutputPatch>,
    default_patch_id: Option<uuid::Uuid>,
    output_screen: Option<u32>,
    osc_patches: Vec<crate::engine::osc_patch::OscPatch>,
    fixtures: Vec<crate::engine::fixture::PatchedFixture>,
    fixture_groups: Vec<crate::engine::fixture::FixtureGroup>,
    input_patches: Vec<crate::engine::audio_input::InputPatch>,
    audio_buffer_size: u32,
) -> CueContext {
    let (tx, _rx) = crossbeam_channel::unbounded::<CueEvent>();
    CueContext::new(
        audio_engine.clone(),
        output_engine.clone(),
        tx,
        stop_fade_ms,
        output_patches,
        default_patch_id,
        output_screen,
        osc_patches,
        dmx_engine.clone(),
        fixtures,
        fixture_groups,
        input_patches,
        audio_buffer_size,
    )
}

/// Collect `cue-time-update` snapshots recursively, including children of
/// running Group cues.
fn collect_time_snapshots(cues: &[Box<dyn crate::cue::traits::Cue>]) -> Vec<(CueId, u64, u64, Option<u64>)> {
    let mut result = Vec::new();
    for cue in cues {
        if cue.state() == CueState::Running || cue.state() == CueState::Paused {
            result.push((
                cue.id(),
                cue.elapsed().as_millis() as u64,
                cue.action_elapsed().as_millis() as u64,
                cue.duration().map(|d| d.as_millis().saturating_sub(cue.action_elapsed().as_millis()) as u64),
            ));
        }
        if let Some(children) = cue.child_cues() {
            result.extend(collect_time_snapshots(children));
        }
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn tick(
    handle: &tauri::AppHandle,
    audio_engine: &Arc<AudioEngine>,
    output_engine: &Arc<OutputEngine>,
    dmx_engine: &Arc<DmxEngine>,
    workspace: &Arc<Mutex<Workspace>>,
    tc_rx:           Option<&crossbeam_channel::Receiver<TcEvent>>,
    prev_tc_frame:   &mut Option<u64>,
    auto_follow_pending: &mut HashMap<CueId, (Instant, uuid::Uuid)>,
    prev_group_state:    &mut HashMap<CueId, (Option<CueId>, bool)>,
    prev_running_cues:   &mut Vec<CueId>,
    prev_playhead_cue:   &mut Option<CueId>,
    prev_cue_list_hash:  &mut u64,
    last_cb_count:       &mut u64,
    cb_last_advance:     &mut Instant,
    audio_frozen:        &mut bool,
    auto_paused:         &mut std::collections::HashSet<CueId>,
) {
    // ------------------------------------------------------------------
    // 0. Drain incoming timecode events and fire TC-triggered cues.
    // ------------------------------------------------------------------
    if let Some(rx) = tc_rx {
        // Collect all pending TC events without blocking.
        let mut latest_pos = None;
        while let Ok(ev) = rx.try_recv() {
            match ev {
                TcEvent::Position(pos) | TcEvent::Started(pos) => {
                    latest_pos = Some(pos);
                }
                TcEvent::Stopped => {
                    // On-Stop policy is applied per-list below when ws is locked.
                    let _ = handle.emit("timecode-stopped", serde_json::json!({}));
                }
            }
        }

        if let Some(pos) = latest_pos {
            let abs_frame = pos.to_frame_number();
            // Only process when the position has actually advanced (monotone guard).
            let advanced = prev_tc_frame.map(|prev| abs_frame > prev).unwrap_or(true);
            // Jump backwards: re-arm all triggers.
            let jumped_back = prev_tc_frame.map(|prev| abs_frame < prev).unwrap_or(false);

            *prev_tc_frame = Some(abs_frame);

            // Emit the current position for the UI status widget.
            let _ = handle.emit("timecode", serde_json::json!({
                "h": pos.hours, "m": pos.minutes, "s": pos.seconds, "f": pos.frames,
                "rate": pos.rate.to_string(),
            }));

            if advanced || jumped_back {
                if let Ok(mut ws) = workspace.try_lock() {
                    for cl in &mut ws.cue_lists {
                        if !cl.tc_config.enabled { continue; }

                        // Re-arm on jump back.
                        if jumped_back { cl.tc_last_triggered_frame = u64::MAX; }

                        // Fire every cue whose trigger sits in (last_triggered_frame, abs_frame].
                        let triggers: Vec<(CueId, u64)> = cl.tc_triggers.iter()
                            .filter_map(|(&id, trigger)| {
                                let tf = trigger.position.to_frame_number();
                                if tf <= abs_frame && tf > cl.tc_last_triggered_frame {
                                    Some((id, tf))
                                } else {
                                    None
                                }
                            })
                            .collect();

                        if triggers.is_empty() { continue; }
                        cl.tc_last_triggered_frame = abs_frame;

                        for (cue_id, _) in triggers {
                            // Verify the cue exists, then release the borrow before emitting.
                            if cl.get_mut_recursive(&cue_id).is_none() { continue };
                            let _ = handle.emit("cue-state-changed", serde_json::json!({
                                "cue_id": cue_id, "old_state": "standby", "new_state": "running",
                            }));
                        }
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // 1. Drain the audio status ring buffer.
    // ------------------------------------------------------------------
    let audio_statuses = audio_engine.drain_status();

    let mut completed_voice_ids: Vec<CueId> = Vec::new();
    let mut master_peak_l = 0.0_f32;
    let mut master_peak_r = 0.0_f32;
    let mut has_master = false;

    for s in audio_statuses {
        match s {
            AudioStatus::Completed { voice_id } => {
                completed_voice_ids.push(voice_id);
            }
            AudioStatus::MasterLevels { peak_l, peak_r } => {
                master_peak_l = master_peak_l.max(peak_l);
                master_peak_r = master_peak_r.max(peak_r);
                has_master = true;
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------
    // 2. Drain the output engine status channel.
    // ------------------------------------------------------------------
    let output_statuses = output_engine.drain_status();

    let mut video_duration_updates: Vec<(CueId, Duration)> = Vec::new();
    let mut emit_workspace_modified = false;

    for s in output_statuses {
        match s {
            OutputStatus::Completed { voice_id } => {
                completed_voice_ids.push(voice_id);
                output_engine.gc_voice(voice_id);
            }
            OutputStatus::Duration { voice_id, duration_ms } => {
                video_duration_updates.push((voice_id, Duration::from_millis(duration_ms)));
                emit_workspace_modified = true;
            }
            OutputStatus::Error { voice_id, message } => {
                log::warn!("Output voice {voice_id} error: {message}");
            }
        }
    }

    // Emit master-level whenever there is any active signal.
    if has_master {
        let _ = handle.emit(
            "master-level",
            serde_json::json!({ "peak_l": master_peak_l, "peak_r": master_peak_r }),
        );
    }

    // ------------------------------------------------------------------
    // 3. Lock the workspace (non-blocking; skip tick if a command holds it).
    // ------------------------------------------------------------------
    let mut ws = match workspace.try_lock() {
        Ok(w) => w,
        Err(_) => return,
    };

    let stop_fade_ms      = ws.preferences.audio.default_fade_out_ms;
    let ws_patches        = ws.output_patches.clone();
    let ws_default_patch  = ws.default_output_patch_id;
    let ws_output_screen  = ws.preferences.display.output_screen;
    let ws_osc_patches    = ws.osc_patches.clone();
    let ws_fixtures       = ws.fixtures.clone();
    let ws_fixture_groups = ws.fixture_groups.clone();
    let ws_input_patches  = ws.input_patches.clone();
    let ws_buffer_size    = ws.preferences.audio.audio_buffer_size;
    let active_list_id    = ws.active_cue_list_id;

    if ws.cue_lists.is_empty() {
        return;
    }

    let tick_ctx = make_context(audio_engine, output_engine, dmx_engine, stop_fade_ms, ws_patches.clone(), ws_default_patch, ws_output_screen, ws_osc_patches.clone(), ws_fixtures.clone(), ws_fixture_groups.clone(), ws_input_patches.clone(), ws_buffer_size);

    // ------------------------------------------------------------------
    // 3b. Audio-freeze guard.  When the output stream stops producing
    //     callbacks (device pulled, or briefly during a switch), pause every
    //     running audio cue so its wall-clock timeline freezes in sync with
    //     the frozen (preserved) audio — otherwise the clock keeps advancing,
    //     hits `duration`, and the cue completes while its audio is still
    //     queued and unstoppable.  Resume them when callbacks return.
    // ------------------------------------------------------------------
    let cb = audio_engine.callback_count();
    if cb != *last_cb_count {
        *last_cb_count = cb;
        *cb_last_advance = Instant::now();
    }
    let frozen_now = cb_last_advance.elapsed() >= Duration::from_millis(AUDIO_FREEZE_MS);

    let mut just_paused:  Vec<CueId> = Vec::new();
    let mut just_resumed: Vec<CueId> = Vec::new();
    if frozen_now && !*audio_frozen {
        for cl in ws.cue_lists.iter_mut() {
            for cue in cl.cues.iter_mut() {
                if cue.state() == CueState::Running && cue.playing_voice_id().is_some()
                    && cue.pause(&tick_ctx).is_ok()
                {
                    auto_paused.insert(cue.id());
                    just_paused.push(cue.id());
                }
            }
        }
    } else if !frozen_now && *audio_frozen {
        for cl in ws.cue_lists.iter_mut() {
            for cue in cl.cues.iter_mut() {
                if !(auto_paused.contains(&cue.id()) && cue.state() == CueState::Paused) {
                    continue;
                }
                // Video: mpv (its own clock) kept playing during the ~250 ms
                // detection window while the paired audio voice was frozen, so
                // they desynced.  Re-anchor the audio voice to mpv's *actual*
                // position (time-pos) — without moving the picture — so audio
                // catches up precisely before playback resumes.
                if cue.cue_type() == CueType::Video {
                    tick_ctx.output_engine.resync_audio_to_video();
                }
                if cue.resume(&tick_ctx).is_ok() {
                    just_resumed.push(cue.id());
                }
            }
        }
        auto_paused.clear();
    }
    *audio_frozen = frozen_now;

    // ------------------------------------------------------------------
    // 4. Apply video duration updates — search every cue list.
    // ------------------------------------------------------------------
    'duration_update: for (voice_id, duration) in &video_duration_updates {
        for cl in ws.cue_lists.iter_mut() {
            for cue in cl.cues.iter_mut() {
                if cue.playing_voice_id() == Some(*voice_id) {
                    cue.set_runtime_duration(*duration);
                    continue 'duration_update;
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // 5-9. Per-list: tick, completion, auto-continue/follow, GO.
    //      We collect cue list IDs first to avoid borrow issues.
    // ------------------------------------------------------------------
    let cue_list_ids: Vec<uuid::Uuid> = ws.cue_lists.iter().map(|cl| cl.id).collect();

    // Aggregate results across all lists for event emission.
    let mut all_newly_completed: Vec<(CueId, ContinueMode, Duration)> = Vec::new();
    let mut all_time_snapshots:  Vec<(CueId, u64, u64, Option<u64>)>  = Vec::new();
    let mut all_go_triggered:    Vec<CueId>                            = Vec::new();
    let mut all_go_stopped:      Vec<CueId>                            = Vec::new();
    let mut all_seq_group_playheads: Vec<Option<CueId>>                = Vec::new();
    let mut group_child_changed  = false;

    // Lists whose auto-continue chain needs a GO this tick.
    let mut should_go_lists: std::collections::HashSet<uuid::Uuid> = std::collections::HashSet::new();

    // Resolve pending auto-follow delays and collect which lists need a GO.
    let now = Instant::now();
    auto_follow_pending.retain(|_id, (due, list_id)| {
        if now >= *due {
            should_go_lists.insert(*list_id);
            false
        } else {
            true
        }
    });

    for &list_id in &cue_list_ids {
        // 5. Tick all Running cues.
        if let Some(cl) = ws.cue_list_by_id_mut(list_id) {
            for cue in cl.cues.iter_mut() {
                if cue.state() == CueState::Running {
                    let _ = cue.tick(&tick_ctx);
                }
            }
        }

        // 5b. Release the outer Playhead once a Sequential group has fired its
        //     last child (overlapping children may still be playing out).  This
        //     covers Auto-Continue/Follow reaching the last child without a GO;
        //     the transport handles the manual-GO case synchronously.
        let release_ph = ws.cue_list_by_id(list_id).and_then(|cl| {
            cl.playhead_cue_id.filter(|ph| {
                cl.cues.iter().any(|c| c.id() == *ph && c.released_playhead())
            })
        });
        if release_ph.is_some() {
            if let Some(cl) = ws.cue_list_by_id_mut(list_id) {
                cl.advance_playhead();
            }
            let ph = ws.cue_list_by_id(list_id).and_then(|cl| cl.playhead_cue_id);
            all_seq_group_playheads.push(ph);
        }

        // 6. Detect completions.
        let mut newly_completed: Vec<(CueId, ContinueMode, Duration)> = Vec::new();
        let mut advance_playhead_ids: Vec<CueId> = Vec::new();

        if let Some(cl) = ws.cue_list_by_id_mut(list_id) {
            let current_playhead = cl.playhead_cue_id;
            for cue in cl.cues.iter_mut() {
                if cue.state() != CueState::Running {
                    continue;
                }
                let voice_done = cue
                    .playing_voice_id()
                    .map(|vid| completed_voice_ids.contains(&vid))
                    .unwrap_or(false);
                let time_done = cue.duration().map(|d| cue.action_elapsed() >= d).unwrap_or(false);
                let group_done = cue.is_complete();
                if voice_done || time_done || group_done {
                    let id = cue.id();
                    let cm = cue.continue_mode();
                    let pw = cue.post_wait();
                    if cue.holds_playhead() && current_playhead == Some(id) {
                        advance_playhead_ids.push(id);
                    }
                    let _ = cue.reset();
                    newly_completed.push((id, cm, pw));
                }
            }
        }

        // Advance playhead for sequential groups that held it.
        let mut playhead_advanced = false;
        if let Some(cl) = ws.cue_list_by_id_mut(list_id) {
            for id in &advance_playhead_ids {
                if cl.playhead_cue_id == Some(*id) {
                    cl.advance_playhead();
                    playhead_advanced = true;
                }
            }
        }
        if playhead_advanced {
            let ph = ws.cue_list_by_id(list_id).and_then(|cl| cl.playhead_cue_id);
            all_seq_group_playheads.push(ph);
        }

        // 7. Time snapshots.
        if let Some(cl) = ws.cue_list_by_id(list_id) {
            all_time_snapshots.extend(collect_time_snapshots(&cl.cues));
        }

        // 8. Auto-Continue / Auto-Follow detection.
        let delayed_ac_ids: Vec<CueId> = ws
            .cue_list_by_id(list_id)
            .map(|cl| {
                cl.cues
                    .iter()
                    .filter(|c| {
                        c.state() == CueState::Running
                            && c.continue_mode() == ContinueMode::AutoContinue
                            && !c.is_auto_continue_fired()
                            && c.is_action_started()
                            && c.action_elapsed() >= c.post_wait()
                    })
                    .map(|c| c.id())
                    .collect()
            })
            .unwrap_or_default();

        if let Some(cl) = ws.cue_list_by_id_mut(list_id) {
            for id in &delayed_ac_ids {
                if let Some(cue) = cl.cues.iter_mut().find(|c| c.id() == *id) {
                    cue.mark_auto_continue_fired();
                }
            }
        }
        if !delayed_ac_ids.is_empty() {
            should_go_lists.insert(list_id);
        }

        for (cue_id, cm, pw) in &newly_completed {
            if *cm == ContinueMode::AutoFollow {
                if pw.is_zero() {
                    should_go_lists.insert(list_id);
                } else {
                    auto_follow_pending.insert(*cue_id, (Instant::now() + *pw, list_id));
                }
            }
        }

        all_newly_completed.extend(newly_completed);

        // Group-child change detection (for cue-list-refresh event).
        if let Some(cl) = ws.cue_list_by_id(list_id) {
            let current: Vec<(CueId, Option<CueId>, bool)> = cl
                .cues
                .iter()
                .filter(|c| c.child_cues().is_some() && c.state() == CueState::Running)
                .map(|c| {
                    let active = c.active_child_id();
                    let any_running = c
                        .child_cues()
                        .map(|ch| ch.iter().any(|child| child.state() == CueState::Running))
                        .unwrap_or(false);
                    (c.id(), active, any_running)
                })
                .collect();
            for (id, active, any_running) in &current {
                let (prev_active, prev_running) =
                    prev_group_state.get(id).copied().unwrap_or((None, false));
                if *active != prev_active || *any_running != prev_running {
                    group_child_changed = true;
                }
            }
            for (id, active, any_running) in current {
                prev_group_state.insert(id, (active, any_running));
            }
        }
    }

    // 9. Fire Auto-Continue / Auto-Follow GO for each list that needs it.
    for &list_id in &should_go_lists {
        if let Some(cl) = ws.cue_list_by_id_mut(list_id) {
            let context = make_context(
                audio_engine, output_engine, dmx_engine, stop_fade_ms,
                ws_patches.clone(), ws_default_patch, ws_output_screen, ws_osc_patches.clone(), ws_fixtures.clone(), ws_fixture_groups.clone(), ws_input_patches.clone(), ws_buffer_size,
            );
            let mut transport = Transport::new(context);
            if let Ok(result) = transport.go(cl) {
                all_go_triggered.extend(result.triggered);
                all_go_stopped.extend(result.stopped);
            }
        }
    }

    // Capture final playhead for the GO event (active list only, matching QLab).
    let go_final_playhead: Option<CueId> = if !all_go_triggered.is_empty() {
        ws.active_cue_list().and_then(|cl| cl.playhead_cue_id)
    } else {
        None
    };

    // ------------------------------------------------------------------
    // 10. Detect running-cue-set / playhead changes for OSC feedback
    //     (active cue list only — matches QLab OSC behavior).
    // ------------------------------------------------------------------
    let (running_payload, playhead_payload, cue_list_payload) =
        if let Some(active_cl) = ws.cue_list_by_id(active_list_id) {
            let running_now: Vec<(CueId, String, String)> = all_running_cues_info(&active_cl.cues);
            let playhead_now = active_cl
                .playhead_cue_id
                .and_then(|ph_id| find_cue_info(&active_cl.cues, ph_id));

            let running_ids: Vec<CueId> = running_now.iter().map(|(id, _, _)| *id).collect();
            let running_p: Option<Vec<(String, String)>> = if running_ids != *prev_running_cues {
                *prev_running_cues = running_ids;
                Some(running_now.into_iter().map(|(_, n, name)| (n, name)).collect())
            } else {
                None
            };

            let playhead_p: Option<(String, String)> = {
                let id = playhead_now.as_ref().map(|(id, _, _)| *id);
                if id != *prev_playhead_cue || crate::engine::osc_feedback::is_playhead_requested() {
                    *prev_playhead_cue = id;
                    Some(playhead_now.map(|(_, n, name)| (n, name)).unwrap_or_default())
                } else {
                    None
                }
            };

            let all_cues = all_cues_flat(&active_cl.cues);
            let cue_list_hash = fingerprint_cue_list(&all_cues);
            let cue_list_p: Option<Vec<(String, String)>> =
                if cue_list_hash != *prev_cue_list_hash
                    || crate::engine::osc_feedback::is_cue_list_requested()
                {
                    *prev_cue_list_hash = cue_list_hash;
                    Some(all_cues)
                } else {
                    None
                };

            (running_p, playhead_p, cue_list_p)
        } else {
            (None, None, None)
        };

    // Rename for clarity in the rest of the function.
    let newly_completed   = all_newly_completed;
    let time_snapshots    = all_time_snapshots;
    let go_triggered      = all_go_triggered;
    let go_stopped        = all_go_stopped;

    drop(ws);

    if let Some(cues) = running_payload {
        crate::engine::osc_feedback::send_running(&cues);
    }
    if let Some((number, name)) = playhead_payload {
        crate::engine::osc_feedback::send_playhead(&number, &name);
    }
    if let Some(cues) = cue_list_payload {
        crate::engine::osc_feedback::send_cue_list(&cues);
    }

    // ------------------------------------------------------------------
    // 11. Emit all events.
    // ------------------------------------------------------------------

    for (cue_id, _, _) in &newly_completed {
        let _ = handle.emit(
            "cue-state-changed",
            serde_json::json!({
                "cue_id": cue_id,
                "old_state": "running",
                "new_state": "standby",
            }),
        );
    }

    // Audio-freeze pause / resume (device-loss timeline guard).
    for cue_id in &just_paused {
        let _ = handle.emit("cue-state-changed", serde_json::json!({
            "cue_id": cue_id, "old_state": "running", "new_state": "paused",
        }));
    }
    for cue_id in &just_resumed {
        let _ = handle.emit("cue-state-changed", serde_json::json!({
            "cue_id": cue_id, "old_state": "paused", "new_state": "running",
        }));
    }

    // Emit playhead-moved for each sequential-group completion advance.
    for new_ph in &all_seq_group_playheads {
        let _ = handle.emit("playhead-moved", serde_json::json!({ "cue_id": new_ph }));
    }

    for (cue_id, elapsed_ms, action_elapsed_ms, remaining_ms) in &time_snapshots {
        let _ = handle.emit(
            "cue-time-update",
            serde_json::json!({
                "cue_id": cue_id,
                "elapsed_ms": elapsed_ms,
                "action_elapsed_ms": action_elapsed_ms,
                "remaining_ms": remaining_ms,
            }),
        );
    }

    // Emit cue-list-refresh when a sequential group's active child changed.
    if group_child_changed {
        let _ = handle.emit("cue-list-refresh", serde_json::json!({}));
    }

    for stopped_id in &go_stopped {
        let _ = handle.emit(
            "cue-state-changed",
            serde_json::json!({
                "cue_id": stopped_id,
                "old_state": "running",
                "new_state": "standby",
            }),
        );
    }

    if !go_triggered.is_empty() {
        if let Some(phid) = go_final_playhead {
            let _ = handle.emit("playhead-moved", serde_json::json!({ "cue_id": phid }));
        }
        for triggered_id in &go_triggered {
            let _ = handle.emit(
                "cue-state-changed",
                serde_json::json!({
                    "cue_id": triggered_id,
                    "old_state": "standby",
                    "new_state": "running",
                }),
            );
        }
    }

    if emit_workspace_modified {
        let _ = handle.emit("workspace-modified", serde_json::json!({}));
    }

    // ------------------------------------------------------------------
    // 12. Garbage-collect finished audio voices.
    // ------------------------------------------------------------------
    audio_engine.gc_voices();
}

// ---------------------------------------------------------------------------
// Fast timer refresh (runs on its own thread at ~60 fps)
// ---------------------------------------------------------------------------

/// Runs on a dedicated thread.  Reads the current running-cue position at
/// `TIMER_TICK_MS` intervals and updates the mpv OSD timer overlay.
///
/// Using a separate thread (rather than doing it inside the 30 fps main tick)
/// lets the millisecond display update smoothly without coupling the refresh
/// rate to all the other, heavier work the main tick performs.
fn timer_refresh_loop(workspace: Arc<Mutex<Workspace>>, output_engine: Arc<OutputEngine>) {
    loop {
        std::thread::sleep(Duration::from_millis(TIMER_TICK_MS));

        // Non-blocking lock — skip this frame if a command handler holds the lock.
        let Ok(ws) = workspace.try_lock() else { continue; };

        let show     = ws.preferences.display.show_output_timer;
        let floating = ws.preferences.display.timer_floating;
        let countdn  = ws.preferences.display.timer_count_down;
        let show_ms  = ws.preferences.display.timer_show_ms;

        // Preview mode overrides live cue time — show placeholder regardless of
        // whether a cue is playing or the show_output_timer setting.
        let preview   = output_engine.get_timer_preview();
        let live_text = ws.active_cue_list()
            .and_then(|cl| first_running_timer_text(&cl.cues, countdn, show_ms));
        let text = if preview.is_some() { preview } else if show { live_text.clone() } else { None };
        drop(ws); // release workspace lock before calling into mpv

        if show && floating {
            // Floating mode: drive the Win32 window, silence the OSD.
            output_engine.set_output_timer(None);
            output_engine.update_floating_timer(text.as_deref());
        } else {
            // Normal mode: drive the OSD, clear the floating window.
            output_engine.set_output_timer(text.as_deref());
            output_engine.update_floating_timer(None);
        }
    }
}

/// Find the first running cue with time data (recursive — checks group children)
/// and format its position as a timer string.
fn first_running_timer_text(
    cues: &[Box<dyn crate::cue::traits::Cue>],
    count_down: bool,
    show_ms: bool,
) -> Option<String> {
    for cue in cues {
        if cue.state() == CueState::Running {
            let ms = if count_down {
                let remaining = cue.duration()?.as_millis()
                    .saturating_sub(cue.action_elapsed().as_millis());
                remaining as u64
            } else {
                cue.action_elapsed().as_millis() as u64
            };
            return Some(format_timer(ms, show_ms));
        }
        if let Some(children) = cue.child_cues() {
            if let Some(text) = first_running_timer_text(children, count_down, show_ms) {
                return Some(text);
            }
        }
    }
    None
}

/// Return `(id, number, name)` for the cue with the given ID (recursive lookup).
fn find_cue_info(
    cues: &[Box<dyn crate::cue::traits::Cue>],
    target: CueId,
) -> Option<(CueId, String, String)> {
    for cue in cues {
        if cue.id() == target {
            return Some((
                cue.id(),
                cue.number().unwrap_or("").to_owned(),
                cue.name().to_owned(),
            ));
        }
        if let Some(children) = cue.child_cues() {
            if let Some(found) = find_cue_info(children, target) {
                return Some(found);
            }
        }
    }
    None
}

/// Collect `(id, number, name)` for every running cue (recursive, ordered).
fn all_running_cues_info(
    cues: &[Box<dyn crate::cue::traits::Cue>],
) -> Vec<(CueId, String, String)> {
    let mut out = Vec::new();
    collect_running(cues, &mut out);
    out
}

fn collect_running(
    cues: &[Box<dyn crate::cue::traits::Cue>],
    out: &mut Vec<(CueId, String, String)>,
) {
    for cue in cues {
        if cue.state() == CueState::Running {
            out.push((
                cue.id(),
                cue.number().unwrap_or("").to_owned(),
                cue.name().to_owned(),
            ));
        }
        if let Some(children) = cue.child_cues() {
            collect_running(children, out);
        }
    }
}

/// Collect `(number, name)` for every cue in display order (recursive).
fn all_cues_flat(cues: &[Box<dyn crate::cue::traits::Cue>]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    collect_all_flat(cues, &mut out);
    out
}

fn collect_all_flat(
    cues: &[Box<dyn crate::cue::traits::Cue>],
    out: &mut Vec<(String, String)>,
) {
    for cue in cues {
        out.push((
            cue.number().unwrap_or("").to_owned(),
            cue.name().to_owned(),
        ));
        if let Some(children) = cue.child_cues() {
            collect_all_flat(children, out);
        }
    }
}

/// Cheap fingerprint of the full cue list (number + name pairs).
fn fingerprint_cue_list(cues: &[(String, String)]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    cues.hash(&mut h);
    h.finish()
}

fn format_timer(ms: u64, show_ms: bool) -> String {
    let total_secs = ms / 1000;
    let mins  = total_secs / 60;
    let secs  = total_secs % 60;
    if show_ms {
        let millis = ms % 1000;
        format!("{mins:02}:{secs:02}.{millis:03}")
    } else {
        format!("{mins:02}:{secs:02}")
    }
}
