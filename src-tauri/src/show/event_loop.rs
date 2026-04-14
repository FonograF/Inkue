//! Background event loop running at ~30 fps.
//!
//! This task bridges the engines and the Tauri frontend:
//! - Drains [`AudioStatus`] messages from the audio engine's ring buffer.
//! - Drains [`VideoStatus`] messages from the video engine's channel.
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
        ring_command::AudioStatus,
        video_engine::{VideoEngine, VideoStatus},
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
    video_engine: Arc<VideoEngine>,
    workspace: Arc<Mutex<Workspace>>,
) {
    // Tracks Auto-Follow completions waiting for their Post-Wait timer.
    let mut auto_follow_pending: HashMap<CueId, Instant> = HashMap::new();

    loop {
        std::thread::sleep(Duration::from_millis(TICK_MS));
        tick(
            &handle,
            &audio_engine,
            &video_engine,
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
    video_engine: &Arc<VideoEngine>,
    stop_fade_ms: u32,
    output_patches: Vec<crate::engine::device_manager::OutputPatch>,
    default_patch_id: Option<uuid::Uuid>,
    audio_device_id: Option<String>,
    audio_backend: crate::preferences::AudioBackend,
) -> CueContext {
    // The receiver is intentionally dropped here; events from within the loop
    // are handled directly by reading status from the ring buffers / channels.
    let (tx, _rx) = crossbeam_channel::unbounded::<CueEvent>();
    CueContext::new(audio_engine.clone(), video_engine.clone(), tx, stop_fade_ms, output_patches, default_patch_id, audio_device_id, audio_backend)
}

fn tick(
    handle: &tauri::AppHandle,
    audio_engine: &Arc<AudioEngine>,
    video_engine: &Arc<VideoEngine>,
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
    // 2. Drain the video status channel.
    // ------------------------------------------------------------------
    let video_statuses = video_engine.drain_status();

    let mut video_duration_updates: Vec<(CueId, Duration)> = Vec::new();
    let mut emit_workspace_modified = false;

    for s in video_statuses {
        match s {
            VideoStatus::Completed { voice_id } => {
                // Merge video completions into the same list as audio ones so
                // the completion detection below stays uniform.
                completed_voice_ids.push(voice_id);
                video_engine.gc_voice(voice_id);
            }
            VideoStatus::Duration { voice_id, duration_ms } => {
                // Collect duration updates; we'll apply them after acquiring
                // the workspace lock.
                video_duration_updates.push((voice_id, Duration::from_millis(duration_ms)));
                emit_workspace_modified = true;
            }
            VideoStatus::Error { voice_id, message } => {
                log::warn!("Video voice {voice_id} error: {message}");
            }
        }
    }

    // ------------------------------------------------------------------
    // 3. Merge video peak levels into the master meter.
    //    mpv's lavfi `astats` filter exposes per-frame peak metadata via the
    //    `af-metadata` property, giving real dBFS values rather than the
    //    configured playback volume.  Both L and R channels are read.
    // ------------------------------------------------------------------
    let (vid_l, vid_r) = video_engine.current_levels();
    if vid_l > 0.0 || vid_r > 0.0 {
        master_peak_l = master_peak_l.max(vid_l);
        master_peak_r = master_peak_r.max(vid_r);
        has_master = true;
    }

    // Emit master-level whenever there is any active signal (audio or video).
    if has_master {
        let _ = handle.emit(
            "master-level",
            serde_json::json!({ "peak_l": master_peak_l, "peak_r": master_peak_r }),
        );
    }

    // ------------------------------------------------------------------
    // 4. Lock the workspace (non-blocking; skip tick if a command holds it).
    // ------------------------------------------------------------------
    let mut ws = match workspace.try_lock() {
        Ok(w) => w,
        Err(_) => return,
    };

    let stop_fade_ms = ws.preferences.audio.default_fade_out_ms;
    // Snapshot the patch table and audio prefs before taking the mutable cue-list borrow.
    let ws_patches = ws.output_patches.clone();
    let ws_default_patch = ws.default_output_patch_id;
    let ws_audio_device = ws.preferences.audio.device_id.clone();
    let ws_audio_backend = ws.preferences.audio.backend.clone();

    let cue_list = match ws.active_cue_list_mut() {
        Some(cl) => cl,
        None => return,
    };

    // ------------------------------------------------------------------
    // 5. Apply video duration updates to cues.
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
    // 6. Tick all Running cues so they can handle pre-wait transitions.
    //    (Must happen before the completion check so that a cue that
    //    completes its pre-wait and immediately finishes is detected.)
    // ------------------------------------------------------------------
    let tick_ctx = make_context(audio_engine, video_engine, stop_fade_ms, ws_patches.clone(), ws_default_patch, ws_audio_device.clone(), ws_audio_backend.clone());
    for cue in cue_list.cues.iter_mut() {
        if cue.state() == CueState::Running {
            let _ = cue.tick(&tick_ctx);
        }
    }

    // ------------------------------------------------------------------
    // 7. Detect cue completions.
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
            newly_completed.push((id, cm, pw));
        }
    }

    // ------------------------------------------------------------------
    // 8. Collect cue-time-update data for still-running cues.
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
    // 9. Auto-Continue (delayed) / Auto-Follow detection.
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
    // 10. Fire Auto-Continue / Auto-Follow GO.
    // ------------------------------------------------------------------
    let mut go_triggered: Vec<CueId> = Vec::new();
    let mut go_final_playhead: Option<CueId> = None;

    if should_go {
        let context = make_context(audio_engine, video_engine, stop_fade_ms, ws_patches.clone(), ws_default_patch, ws_audio_device.clone(), ws_audio_backend.clone());
        let mut transport = Transport::new(context);
        if let Ok(ids) = transport.go(cue_list) {
            go_triggered = ids;
            go_final_playhead = cue_list.playhead_cue_id;
        }
    }

    // Release the workspace lock BEFORE emitting any events.
    drop(ws);

    // ------------------------------------------------------------------
    // 11. Emit all events now that the workspace lock is released.
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

    // Emit workspace-modified if any video duration arrived (updates duration column).
    if emit_workspace_modified {
        let _ = handle.emit("workspace-modified", serde_json::json!({}));
    }

    // ------------------------------------------------------------------
    // 12. Garbage-collect finished audio voices.
    // ------------------------------------------------------------------
    audio_engine.gc_voices();
}
