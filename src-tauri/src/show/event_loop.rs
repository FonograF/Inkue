//! Background event loop running at ~30 fps.
//!
//! This task bridges the real-time audio engine and the Tauri frontend:
//! - Drains [`AudioStatus`] messages from the engine's ring buffer.
//! - Marks cues as completed when their audio voice ends.
//! - Fires Auto-Continue chains (Post-Wait based).
//! - Emits `cue-state-changed`, `cue-time-update`, and `master-level`
//!   Tauri events so the UI stays in sync without polling.
//! - Calls [`AudioEngine::gc_voices`] to release stopped voice memory.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::Emitter;

use crate::{
    cue::{
        context::{CueContext, CueEvent},
        types::{ContinueMode, CueId, CueState},
    },
    engine::{ring_command::AudioStatus, AudioEngine},
    show::{transport::Transport, workspace::Workspace},
};

/// Target tick interval (~30 fps).
const TICK_MS: u64 = 33;

/// Entry point for the event loop thread.  Loops indefinitely.
pub fn run(
    handle: tauri::AppHandle,
    audio_engine: Arc<AudioEngine>,
    workspace: Arc<Mutex<Workspace>>,
) {
    // Tracks which Running cues have already had their Auto-Continue fired
    // this execution cycle.  Cleared when the cue resets/stops.
    let mut auto_continue_fired: HashSet<CueId> = HashSet::new();

    loop {
        std::thread::sleep(Duration::from_millis(TICK_MS));
        tick(&handle, &audio_engine, &workspace, &mut auto_continue_fired);
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn make_context(engine: &Arc<AudioEngine>) -> CueContext {
    // The receiver is intentionally dropped here; events from within the loop
    // are handled directly by reading AudioStatus from the ring buffer.
    let (tx, _rx) = crossbeam_channel::unbounded::<CueEvent>();
    CueContext::new(engine.clone(), tx)
}

fn tick(
    handle: &tauri::AppHandle,
    engine: &Arc<AudioEngine>,
    workspace: &Arc<Mutex<Workspace>>,
    auto_continue_fired: &mut HashSet<CueId>,
) {
    // ------------------------------------------------------------------
    // 1. Drain the audio status ring buffer.
    // ------------------------------------------------------------------
    let statuses = engine.drain_status();

    let mut completed_voice_ids: Vec<CueId> = Vec::new();
    let mut master_peak_l = 0.0_f32;
    let mut master_peak_r = 0.0_f32;
    let mut has_master = false;

    for s in statuses {
        match s {
            AudioStatus::Completed { voice_id } => {
                completed_voice_ids.push(voice_id);
            }
            AudioStatus::MasterLevels { peak_l, peak_r } => {
                // Keep the maximum across multiple callbacks this tick.
                master_peak_l = master_peak_l.max(peak_l);
                master_peak_r = master_peak_r.max(peak_r);
                has_master = true;
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------
    // 2. Emit master-level event (even when 0, to allow meter decay in UI).
    // ------------------------------------------------------------------
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

    let cue_list = match ws.active_cue_list_mut() {
        Some(cl) => cl,
        None => return,
    };

    // ------------------------------------------------------------------
    // 3b. Tick all Running cues so they can handle pre-wait transitions.
    //     (Must happen before the completion check so that a cue that
    //     completes its pre-wait and immediately finishes is detected.)
    // ------------------------------------------------------------------
    let tick_ctx = make_context(engine);
    for cue in cue_list.cues.iter_mut() {
        if cue.state() == CueState::Running {
            let _ = cue.tick(&tick_ctx);
        }
    }

    // ------------------------------------------------------------------
    // 4. Detect cue completions.
    //    Each entry: (cue_id, continue_mode, post_wait).
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

        if voice_done || time_done {
            let id = cue.id();
            let cm = cue.continue_mode();
            let pw = cue.post_wait();
            let _ = cue.reset();
            auto_continue_fired.remove(&id);
            newly_completed.push((id, cm, pw));
        }
    }

    // ------------------------------------------------------------------
    // 5. Collect cue-time-update data for still-running cues.
    //    Snapshots are taken while the lock is held; events are emitted
    //    AFTER the lock is released so the workspace is free for GO.
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
    // 6. Auto-Continue / Auto-Follow detection.
    // ------------------------------------------------------------------
    let mut should_go = false;

    // Auto-Continue on still-running cues: fire when action_elapsed >= post_wait.
    for cue in cue_list.cues.iter() {
        if cue.state() != CueState::Running {
            continue;
        }
        if cue.continue_mode() == ContinueMode::AutoContinue
            && !auto_continue_fired.contains(&cue.id())
            && cue.is_action_started()
            && cue.action_elapsed() >= cue.post_wait()
        {
            auto_continue_fired.insert(cue.id());
            should_go = true;
        }
    }

    // Auto-Follow: fire the next cue the moment THIS cue finishes playing.
    // (transport.go() does NOT chain immediately for running cues, so it falls
    // through to here.)
    for (_cue_id, cm, _pw) in &newly_completed {
        if *cm == ContinueMode::AutoFollow {
            should_go = true;
        }
    }

    // ------------------------------------------------------------------
    // 7. Fire Auto-Continue (workspace still locked — needed for GO).
    // ------------------------------------------------------------------
    let mut go_result: Option<(CueId, Option<CueId>)> = None; // (triggered_id, new_playhead_id)

    if should_go {
        let context = make_context(engine);
        let mut transport = Transport::new(context);
        if let Ok(Some(triggered_id)) = transport.go(cue_list) {
            go_result = Some((triggered_id, cue_list.playhead_cue_id));
        }
    }

    // Release the workspace lock BEFORE emitting any events.
    // This keeps the mutex free so that `go()` commands never block on a tick.
    drop(ws);

    // ------------------------------------------------------------------
    // 8. Emit all events now that the workspace lock is released.
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

    if let Some((triggered_id, new_phid)) = go_result {
        if let Some(phid) = new_phid {
            let _ = handle.emit("playhead-moved", serde_json::json!({ "cue_id": phid }));
        }
        let _ = handle.emit(
            "cue-state-changed",
            serde_json::json!({
                "cue_id": triggered_id,
                "old_state": "standby",
                "new_state": "running",
            }),
        );
    }

    // ------------------------------------------------------------------
    // 9. Garbage-collect finished voices from the pool.
    // ------------------------------------------------------------------
    engine.gc_voices();
}
