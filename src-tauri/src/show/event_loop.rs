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

/// Target tick interval (~30 fps).
const TICK_MS: u64 = 33;

/// Entry point for the event loop thread.  Loops indefinitely.
pub fn run(
    handle: tauri::AppHandle,
    audio_engine: Arc<AudioEngine>,
    output_engine: Arc<OutputEngine>,
    workspace: Arc<Mutex<Workspace>>,
) {
    let mut auto_follow_pending: HashMap<CueId, Instant> = HashMap::new();

    loop {
        std::thread::sleep(Duration::from_millis(TICK_MS));
        tick(
            &handle,
            &audio_engine,
            &output_engine,
            &workspace,
            &mut auto_follow_pending,
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
    )
}

fn tick(
    handle: &tauri::AppHandle,
    audio_engine: &Arc<AudioEngine>,
    output_engine: &Arc<OutputEngine>,
    workspace: &Arc<Mutex<Workspace>>,
    auto_follow_pending: &mut HashMap<CueId, Instant>,
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

    let stop_fade_ms = ws.preferences.audio.default_fade_out_ms;
    let ws_patches = ws.output_patches.clone();
    let ws_default_patch = ws.default_output_patch_id;
    let ws_output_screen = ws.preferences.display.output_screen;

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
    let tick_ctx = make_context(audio_engine, output_engine, stop_fade_ms, ws_patches.clone(), ws_default_patch, ws_output_screen);
    for cue in cue_list.cues.iter_mut() {
        if cue.state() == CueState::Running {
            let _ = cue.tick(&tick_ctx);
        }
    }

    // ------------------------------------------------------------------
    // 6. Detect cue completions.
    // ------------------------------------------------------------------
    let mut newly_completed: Vec<(CueId, ContinueMode, Duration)> = Vec::new();

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
            let _ = cue.reset();
            newly_completed.push((id, cm, pw));
        }
    }

    // ------------------------------------------------------------------
    // 7. Collect cue-time-update data for still-running cues.
    // ------------------------------------------------------------------
    #[derive(Clone)]
    struct TimeSnapshot {
        cue_id: CueId,
        elapsed_ms: u64,
        action_elapsed_ms: u64,
        remaining_ms: Option<u64>,
    }

    let time_snapshots: Vec<TimeSnapshot> = cue_list
        .cues
        .iter()
        .filter(|c| c.state() == CueState::Running)
        .map(|cue| TimeSnapshot {
            cue_id: cue.id(),
            elapsed_ms: cue.elapsed().as_millis() as u64,
            action_elapsed_ms: cue.action_elapsed().as_millis() as u64,
            remaining_ms: cue.duration().map(|d| {
                d.as_millis()
                    .saturating_sub(cue.action_elapsed().as_millis()) as u64
            }),
        })
        .collect();

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
        let context = make_context(audio_engine, output_engine, stop_fade_ms, ws_patches.clone(), ws_default_patch, ws_output_screen);
        let mut transport = Transport::new(context);
        if let Ok(ids) = transport.go(cue_list) {
            go_triggered = ids;
            go_final_playhead = cue_list.playhead_cue_id;
        }
    }

    drop(ws);

    // ------------------------------------------------------------------
    // 10. Emit all events.
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

    for snap in &time_snapshots {
        let _ = handle.emit(
            "cue-time-update",
            serde_json::json!({
                "cue_id": snap.cue_id,
                "elapsed_ms": snap.elapsed_ms,
                "action_elapsed_ms": snap.action_elapsed_ms,
                "remaining_ms": snap.remaining_ms,
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
    // 11. Garbage-collect finished audio voices.
    // ------------------------------------------------------------------
    audio_engine.gc_voices();
}
