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
        types::{ContinueMode, CueId, CueState},
    },
    engine::{
        output_engine::{OutputEngine, OutputStatus},
        ring_command::AudioStatus,
        AudioEngine,
    },
    show::{transport::Transport, workspace::Workspace},
};

/// Target tick interval for the main event loop (~30 fps).
const TICK_MS: u64 = 33;
/// Timer overlay refresh interval — fast enough for smooth millisecond display.
const TIMER_TICK_MS: u64 = 16;

/// Entry point for the event loop thread.  Loops indefinitely.
pub fn run(
    handle: tauri::AppHandle,
    audio_engine: Arc<AudioEngine>,
    output_engine: Arc<OutputEngine>,
    workspace: Arc<Mutex<Workspace>>,
) {
    // Spawn a dedicated thread that refreshes the OSD timer overlay at ~60 fps.
    // This is independent of the main 30 fps tick so the millisecond display
    // stays smooth even when the workspace lock is briefly held by a command.
    {
        let ws2 = Arc::clone(&workspace);
        let oe2 = Arc::clone(&output_engine);
        std::thread::Builder::new()
            .name("wincue-timer-refresh".into())
            .spawn(move || timer_refresh_loop(ws2, oe2))
            .expect("Failed to spawn timer refresh thread");
    }

    let mut auto_follow_pending: HashMap<CueId, Instant> = HashMap::new();
    // Per-group snapshot: (active_child_id, any_child_running).
    // Used to detect inner-sequence progress and emit cue-list-refresh.
    let mut prev_group_state: HashMap<CueId, (Option<CueId>, bool)> = HashMap::new();
    // Cue sets tracked for OSC feedback (compared each tick to detect changes).
    let mut prev_running_cues: Vec<CueId>  = Vec::new();
    let mut prev_playhead_cue: Option<CueId> = None;
    // Fingerprint of the full cue list (number+name). Sending on change.
    let mut prev_cue_list_hash: u64 = 0;

    loop {
        std::thread::sleep(Duration::from_millis(TICK_MS));
        tick(
            &handle,
            &audio_engine,
            &output_engine,
            &workspace,
            &mut auto_follow_pending,
            &mut prev_group_state,
            &mut prev_running_cues,
            &mut prev_playhead_cue,
            &mut prev_cue_list_hash,
        );
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn make_context(
    audio_engine: &Arc<AudioEngine>,
    output_engine: &Arc<OutputEngine>,
    stop_fade_ms: u32,
    output_patches: Vec<crate::engine::device_manager::OutputPatch>,
    default_patch_id: Option<uuid::Uuid>,
    output_screen: Option<u32>,
    osc_patches: Vec<crate::engine::osc_patch::OscPatch>,
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

fn tick(
    handle: &tauri::AppHandle,
    audio_engine: &Arc<AudioEngine>,
    output_engine: &Arc<OutputEngine>,
    workspace: &Arc<Mutex<Workspace>>,
    auto_follow_pending: &mut HashMap<CueId, Instant>,
    prev_group_state:    &mut HashMap<CueId, (Option<CueId>, bool)>,
    prev_running_cues:   &mut Vec<CueId>,
    prev_playhead_cue:   &mut Option<CueId>,
    prev_cue_list_hash:  &mut u64,
) {
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
    let cue_list = match ws.active_cue_list_mut() {
        Some(cl) => cl,
        None => return,
    };

    // ------------------------------------------------------------------
    // 4. Apply video duration updates to cues.
    // ------------------------------------------------------------------
    for (voice_id, duration) in &video_duration_updates {
        for cue in cue_list.cues.iter_mut() {
            if cue.playing_voice_id() == Some(*voice_id) {
                cue.set_runtime_duration(*duration);
                break;
            }
        }
    }

    // ------------------------------------------------------------------
    // 5. Tick all Running cues so they can handle pre-wait transitions.
    // ------------------------------------------------------------------
    let tick_ctx = make_context(audio_engine, output_engine, stop_fade_ms, ws_patches.clone(), ws_default_patch, ws_output_screen, ws_osc_patches.clone());
    for cue in cue_list.cues.iter_mut() {
        if cue.state() == CueState::Running {
            let _ = cue.tick(&tick_ctx);
        }
    }

    // ------------------------------------------------------------------
    // 6. Detect cue completions.
    // ------------------------------------------------------------------
    let mut newly_completed: Vec<(CueId, ContinueMode, Duration)> = Vec::new();
    // Sequential groups that held the playhead and just completed need the
    // playhead advanced here (the transport skipped advance_playhead() earlier).
    let mut advance_playhead_ids: Vec<CueId> = Vec::new();

    let current_playhead = cue_list.playhead_cue_id;

    for cue in cue_list.cues.iter_mut() {
        if cue.state() != CueState::Running {
            continue;
        }

        let voice_done = cue
            .playing_voice_id()
            .map(|vid| completed_voice_ids.contains(&vid))
            .unwrap_or(false);

        let time_done = cue
            .duration()
            .map(|d| cue.action_elapsed() >= d)
            .unwrap_or(false);

        // Group cues signal completion via is_complete() rather than voice/time.
        let group_done = cue.is_complete();

        if voice_done || time_done || group_done {
            let id = cue.id();
            let cm = cue.continue_mode();
            let pw = cue.post_wait();
            // If this cue held the playhead, schedule a playhead advance.
            if cue.holds_playhead() && current_playhead == Some(id) {
                advance_playhead_ids.push(id);
            }
            let _ = cue.reset();
            newly_completed.push((id, cm, pw));
        }
    }

    // Advance the playhead for sequential groups that held it and just finished.
    let mut playhead_advanced = false;
    for id in &advance_playhead_ids {
        if cue_list.playhead_cue_id == Some(*id) {
            cue_list.advance_playhead();
            playhead_advanced = true;
        }
    }

    // ------------------------------------------------------------------
    // 7. Collect cue-time-update data — recursive so running children of
    //    Group cues also get progress updates.
    // ------------------------------------------------------------------
    // (cue_id, elapsed_ms, action_elapsed_ms, remaining_ms)
    let time_snapshots = collect_time_snapshots(&cue_list.cues);

    // ------------------------------------------------------------------
    // 8. Auto-Continue / Auto-Follow detection.
    // ------------------------------------------------------------------
    let mut should_go = false;

    let delayed_ac_ids: Vec<CueId> = cue_list
        .cues
        .iter()
        .filter(|c| {
            c.state() == CueState::Running
                && c.continue_mode() == ContinueMode::AutoContinue
                && !c.is_auto_continue_fired()
                && c.is_action_started()
                && c.action_elapsed() >= c.post_wait()
        })
        .map(|c| c.id())
        .collect();

    for id in &delayed_ac_ids {
        if let Some(cue) = cue_list.cues.iter_mut().find(|c| c.id() == *id) {
            cue.mark_auto_continue_fired();
        }
    }

    if !delayed_ac_ids.is_empty() {
        should_go = true;
    }

    for (cue_id, cm, pw) in &newly_completed {
        if *cm == ContinueMode::AutoFollow {
            if pw.is_zero() {
                should_go = true;
            } else {
                auto_follow_pending.insert(*cue_id, Instant::now() + *pw);
            }
        }
    }

    let now = Instant::now();
    auto_follow_pending.retain(|_id, due| {
        if now >= *due {
            should_go = true;
            false
        } else {
            true
        }
    });

    // ------------------------------------------------------------------
    // 9. Fire Auto-Continue / Auto-Follow GO.
    // ------------------------------------------------------------------
    let mut go_triggered: Vec<CueId> = Vec::new();
    let mut go_final_playhead: Option<CueId> = None;

    if should_go {
        let context = make_context(audio_engine, output_engine, stop_fade_ms, ws_patches.clone(), ws_default_patch, ws_output_screen, ws_osc_patches.clone());
        let mut transport = Transport::new(context);
        if let Ok(ids) = transport.go(cue_list) {
            go_triggered = ids;
            go_final_playhead = cue_list.playhead_cue_id;
        }
    }

    // Capture the new playhead position for sequential-group completion advances
    // before releasing the workspace lock.
    let seq_group_new_playhead: Option<Option<CueId>> = if playhead_advanced {
        Some(cue_list.playhead_cue_id)
    } else {
        None
    };

    // Detect inner-sequence changes in group cues so the frontend can update
    // the inner-playhead display without waiting for a user GO press.
    // We track (active_child_id, any_child_running) per group.
    let mut group_child_changed = false;
    {
        // Only track groups that are currently Running — the frontend derives the
        // pre-fire (Standby) state from outerPlayheadId + children list directly.
        let current: Vec<(CueId, Option<CueId>, bool)> = cue_list
            .cues
            .iter()
            .filter(|c| c.child_cues().is_some() && c.state() == CueState::Running)
            .map(|c| {
                let active = c.active_child_id();
                let any_running = c.child_cues()
                    .map(|ch| ch.iter().any(|child| child.state() == CueState::Running))
                    .unwrap_or(false);
                (c.id(), active, any_running)
            })
            .collect();
        for (id, active, any_running) in &current {
            let (prev_active, prev_running) = prev_group_state
                .get(id)
                .copied()
                .unwrap_or((None, false));
            if *active != prev_active || *any_running != prev_running {
                group_child_changed = true;
            }
        }
        for (id, active, any_running) in current {
            prev_group_state.insert(id, (active, any_running));
        }
    }

    // ------------------------------------------------------------------
    // 10. Detect running-cue-set / playhead changes for OSC feedback.
    // ------------------------------------------------------------------
    let running_now: Vec<(CueId, String, String)> = all_running_cues_info(&cue_list.cues);
    let playhead_now = cue_list.playhead_cue_id
        .and_then(|ph_id| find_cue_info(&cue_list.cues, ph_id));

    let running_ids: Vec<CueId> = running_now.iter().map(|(id, _, _)| *id).collect();
    let running_payload: Option<Vec<(String, String)>> = if running_ids != *prev_running_cues {
        *prev_running_cues = running_ids;
        Some(running_now.into_iter().map(|(_, n, name)| (n, name)).collect())
    } else {
        None
    };

    let playhead_payload: Option<(String, String)> = {
        let id = playhead_now.as_ref().map(|(id, _, _)| *id);
        if id != *prev_playhead_cue {
            *prev_playhead_cue = id;
            Some(playhead_now.map(|(_, n, name)| (n, name)).unwrap_or_default())
        } else {
            None
        }
    };

    let all_cues = all_cues_flat(&cue_list.cues);
    let cue_list_hash = fingerprint_cue_list(&all_cues);
    let cue_list_payload: Option<Vec<(String, String)>> =
        if cue_list_hash != *prev_cue_list_hash
            || crate::engine::osc_feedback::is_cue_list_requested()
        {
            *prev_cue_list_hash = cue_list_hash;
            Some(all_cues)
        } else {
            None
        };

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

    // Emit playhead-moved when the event loop advanced the playhead for a
    // completed sequential group.
    if let Some(new_ph) = seq_group_new_playhead {
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

    // Emit cue-list-refresh when a sequential group's active child changed
    // (e.g., a timed child completed and the inner playhead advanced).
    if group_child_changed {
        let _ = handle.emit("cue-list-refresh", serde_json::json!({}));
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
