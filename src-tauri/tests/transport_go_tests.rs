//! Zone (🔴 core) — Transport GO orchestration.
//!
//! Exercises `Transport::go` end-to-end: playhead advance, Auto-Follow /
//! Auto-Continue chaining, Stop-Cue action ordering, and Sequential-Group
//! hold/absorb. This is the show's trigger logic — previously untestable
//! because `CueContext` required concrete engines (an audio device + a GL/mpv
//! window). The `engine_traits` seam lets us inject inert engine doubles here,
//! so the logic runs with no hardware.
//!
//! MemoCue completes instantly in `go()` (Running → Completed); WaitCue stays
//! Running — that difference is what these scenarios lean on.

mod common;

use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::Receiver;

use common::{recording_context, full_registry, CallLog, EngineCall};
use inkue_lib::cue::context::{CueContext, CueEvent};
use inkue_lib::cue::devamp_cue::DevampCue;
use inkue_lib::cue::fade_cue::FadeCue;
use inkue_lib::cue::group_cue::GroupCue;
use inkue_lib::cue::registry::CueRegistry;
use inkue_lib::cue::traits::Cue;
use inkue_lib::cue::types::{ContinueMode, CueType, GroupMode};
use inkue_lib::show::cue_list::CueList;
use inkue_lib::show::transport::Transport;

fn test_context() -> (CueContext, Receiver<CueEvent>) {
    let (ctx, rx, _log) = recording_context();
    (ctx, rx)
}

fn memo(reg: &CueRegistry, name: &str, mode: ContinueMode, post_wait: Duration) -> Box<dyn Cue> {
    let mut c = reg.create(&CueType::Memo).unwrap();
    c.set_name(name.to_string());
    c.set_continue_mode(mode);
    c.set_post_wait(post_wait);
    c
}

fn list_of(cues: Vec<Box<dyn Cue>>) -> CueList {
    let mut list = CueList::new("T");
    for c in cues {
        list.push(c);
    }
    list
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn go_on_empty_playhead_returns_empty() {
    let (ctx, _rx) = test_context();
    let mut transport = Transport::new(ctx);
    let mut list = CueList::new("empty");
    let result = transport.go(&mut list).unwrap();
    assert!(result.triggered.is_empty());
    assert!(result.stopped.is_empty());
}

#[test]
fn go_triggers_playhead_cue_and_advances() {
    let reg = full_registry();
    let (ctx, _rx) = test_context();
    let mut transport = Transport::new(ctx);

    let m1 = memo(&reg, "1", ContinueMode::DoNotContinue, Duration::ZERO);
    let m2 = memo(&reg, "2", ContinueMode::DoNotContinue, Duration::ZERO);
    let id1 = m1.id();
    let id2 = m2.id();
    let mut list = list_of(vec![m1, m2]);

    let result = transport.go(&mut list).unwrap();
    assert_eq!(result.triggered, vec![id1], "only the playhead cue fires (no continue)");
    assert_eq!(list.playhead_cue_id, Some(id2), "playhead advances to the next cue");
}

#[test]
fn auto_follow_chains_through_instant_cues() {
    let reg = full_registry();
    let (ctx, _rx) = test_context();
    let mut transport = Transport::new(ctx);

    let m1 = memo(&reg, "1", ContinueMode::AutoFollow, Duration::ZERO);
    let m2 = memo(&reg, "2", ContinueMode::AutoFollow, Duration::ZERO);
    let m3 = memo(&reg, "3", ContinueMode::DoNotContinue, Duration::ZERO);
    let (id1, id2, id3) = (m1.id(), m2.id(), m3.id());
    let mut list = list_of(vec![m1, m2, m3]);

    let result = transport.go(&mut list).unwrap();
    assert_eq!(
        result.triggered,
        vec![id1, id2, id3],
        "Auto-Follow chains through the two instant cues and stops at DoNotContinue"
    );
    assert_eq!(list.playhead_cue_id, None, "playhead ends past the last cue");
}

#[test]
fn auto_continue_with_zero_postwait_chains_once() {
    let reg = full_registry();
    let (ctx, _rx) = test_context();
    let mut transport = Transport::new(ctx);

    let m1 = memo(&reg, "1", ContinueMode::AutoContinue, Duration::ZERO);
    let m2 = memo(&reg, "2", ContinueMode::DoNotContinue, Duration::ZERO);
    let (id1, id2) = (m1.id(), m2.id());
    let mut list = list_of(vec![m1, m2]);

    let result = transport.go(&mut list).unwrap();
    assert_eq!(result.triggered, vec![id1, id2], "zero post-wait Auto-Continue fires the next cue now");
}

#[test]
fn auto_continue_with_nonzero_postwait_defers_the_chain() {
    let reg = full_registry();
    let (ctx, _rx) = test_context();
    let mut transport = Transport::new(ctx);

    // Non-zero post-wait: the chain is the event loop's job, not this synchronous GO.
    let m1 = memo(&reg, "1", ContinueMode::AutoContinue, Duration::from_millis(100));
    let m2 = memo(&reg, "2", ContinueMode::DoNotContinue, Duration::ZERO);
    let id1 = m1.id();
    let mut list = list_of(vec![m1, m2]);

    let result = transport.go(&mut list).unwrap();
    assert_eq!(result.triggered, vec![id1], "a post-wait > 0 must not chain within the same GO");
}

#[test]
fn stop_cue_stops_running_cues_and_reports_them() {
    let reg = full_registry();
    let (ctx, _rx) = test_context();
    let mut transport = Transport::new(ctx);

    // A Wait cue stays Running after GO; a default Stop cue (empty targets)
    // stops everything running when it fires.
    let wait = reg.create(&CueType::Wait).unwrap();
    let stop = reg.create(&CueType::Stop).unwrap();
    let wait_id = wait.id();
    let mut list = list_of(vec![wait, stop]);

    // GO #1 starts the Wait (now Running).
    transport.go(&mut list).unwrap();
    assert!(list.get(&wait_id).unwrap().is_running(), "wait cue should be running after GO");

    // GO #2 fires the Stop cue, which stops the running Wait.
    let result = transport.go(&mut list).unwrap();
    assert!(
        result.stopped.contains(&wait_id),
        "the Stop cue must report the wait it stopped, got {:?}",
        result.stopped
    );
    assert!(!list.get(&wait_id).unwrap().is_running(), "the wait cue must no longer be running");
}

#[test]
fn stop_cue_with_specific_target_spares_the_others() {
    let reg = full_registry();
    let (ctx, _rx) = test_context();
    let mut transport = Transport::new(ctx);

    let wait_a = reg.create(&CueType::Wait).unwrap();
    let wait_b = reg.create(&CueType::Wait).unwrap();
    let (id_a, id_b) = (wait_a.id(), wait_b.id());

    // A Stop cue that targets only wait A (built via the real serializer).
    let mut stop_json = reg.create(&CueType::Stop).unwrap().serialize();
    stop_json["target_cue_ids"] = serde_json::json!([id_a.to_string()]);
    let stop = reg.from_json(stop_json).unwrap();

    let mut list = list_of(vec![wait_a, wait_b, stop]);

    transport.go(&mut list).unwrap(); // A running
    transport.go(&mut list).unwrap(); // B running
    let result = transport.go(&mut list).unwrap(); // Stop targets A only

    assert_eq!(result.stopped, vec![id_a], "only the targeted cue is stopped");
    assert!(!list.get(&id_a).unwrap().is_running(), "A must be stopped");
    assert!(list.get(&id_b).unwrap().is_running(), "B must keep running");
}

#[test]
fn sequential_group_holds_the_outer_playhead() {
    let reg = full_registry();
    let (ctx, _rx) = test_context();
    let mut transport = Transport::new(ctx);

    // Build a Sequential group with two memo children via the real serializer.
    let children: Vec<serde_json::Value> = (0..2)
        .map(|i| memo(&reg, &format!("child {i}"), ContinueMode::DoNotContinue, Duration::ZERO).serialize())
        .collect();
    let mut group = reg.create(&CueType::Group).unwrap();
    group.set_group_mode(GroupMode::Sequential);
    let mut gj = group.serialize();
    gj["children"] = serde_json::json!(children);
    let group = reg.from_json(gj).unwrap();
    let group_id = group.id();

    let mut list = list_of(vec![group]);

    // First GO fires the group's first child; because more children remain, the
    // sequential group holds the outer playhead on itself.
    let result = transport.go(&mut list).unwrap();
    assert_eq!(result.triggered, vec![group_id], "the group is the triggered cue");
    assert_eq!(
        list.playhead_cue_id,
        Some(group_id),
        "a running Sequential group with children left must retain the outer playhead"
    );

    // Second GO is absorbed by the group (fires its next child) rather than
    // advancing the outer playhead to a sibling.
    let result2 = transport.go(&mut list).unwrap();
    assert_eq!(result2.triggered, vec![group_id], "the second GO is absorbed by the group");
}

/// An Audio cue with a decoded buffer preloaded, so `go()` actually plays it
/// through the (recording) audio engine and assigns it a voice id.
fn preloaded_audio(reg: &CueRegistry) -> Box<dyn Cue> {
    let mut c = reg.create(&CueType::Audio).unwrap();
    // Long duration so nothing auto-completes by wall clock during the test.
    c.accept_preloaded_audio(Arc::new(vec![0.0f32; 4800]), 1, 48_000, Duration::from_secs(30));
    c
}

fn set_gain_count(log: &CallLog) -> usize {
    log.lock()
        .unwrap()
        .iter()
        .filter(|c| matches!(c, EngineCall::AudioSetGain { .. }))
        .count()
}

#[test]
fn fade_targeting_a_group_fades_every_child_voice() {
    // Regression: a Fade cue targeting a Group did nothing — the transport only
    // looked at top-level cues and only knew how to read a single Audio voice,
    // so a Group (which owns one voice per child) contributed no voices and the
    // fade had nothing to drive. It now collects the group's voices recursively.
    let reg = full_registry();
    let (ctx, _rx, log) = recording_context();
    let tick_ctx = ctx.clone(); // shares the same recording engine double + log
    let mut transport = Transport::new(ctx);

    // A Simultaneous group with two audio children (both play at once).
    let mut group = GroupCue::new();
    group.mode = GroupMode::Simultaneous;
    group.children.push(preloaded_audio(&reg));
    group.children.push(preloaded_audio(&reg));
    let group_id = group.id();

    // A Fade cue targeting the group: fade the volume down to silence.
    let mut fade = FadeCue::new();
    fade.target_cue_ids = vec![group_id];
    fade.target_volume_db = -60.0;
    fade.fade_duration_ms = 30;
    let fade_id = fade.id();

    let mut list = list_of(vec![Box::new(group), Box::new(fade)]);

    // GO the group: both children play and get voice ids.
    transport.go(&mut list).unwrap();
    // GO the fade: the transport injects the group's child voices into it.
    transport.go(&mut list).unwrap();

    let before = set_gain_count(&log);

    // Tick the fade to completion; it interpolates gain on every injected voice.
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_millis(200) {
        if let Some(fc) = list.get_mut(&fade_id) {
            let _ = fc.tick(&tick_ctx);
        }
        std::thread::sleep(Duration::from_millis(3));
    }

    let applied = set_gain_count(&log) - before;
    assert!(
        applied >= 2,
        "a fade on a group must drive set_voice_gain on every child voice \
         (got {applied} gain updates — 0 would mean the group contributed no voices)"
    );
}

#[test]
fn devamp_forwards_to_every_target_voice_with_mode() {
    // GO on a Devamp cue must resolve its targets' voices and forward the
    // devamp (with the stop-at-end flag) to the audio engine — including the
    // voices of a Group's children, like Fade does.
    let reg = full_registry();
    let (ctx, _rx, log) = recording_context();
    let mut transport = Transport::new(ctx);

    let mut group = GroupCue::new();
    group.mode = GroupMode::Simultaneous;
    group.children.push(preloaded_audio(&reg));
    group.children.push(preloaded_audio(&reg));
    let group_id = group.id();

    let mut devamp = DevampCue::new();
    devamp.target_cue_ids = vec![group_id];
    devamp.stop_at_end = true;

    let mut list = list_of(vec![Box::new(group), Box::new(devamp)]);

    transport.go(&mut list).unwrap(); // group: children play
    transport.go(&mut list).unwrap(); // devamp: forward to the voices

    let devamps: Vec<bool> = log
        .lock()
        .unwrap()
        .iter()
        .filter_map(|c| match c {
            EngineCall::AudioDevamp { stop_at_end } => Some(*stop_at_end),
            _ => None,
        })
        .collect();
    assert_eq!(
        devamps,
        vec![true, true],
        "one devamp per child voice, carrying the stop-at-end flag"
    );
}

#[test]
fn devamp_without_running_target_is_a_no_op() {
    let reg = full_registry();
    let (ctx, _rx, log) = recording_context();
    let mut transport = Transport::new(ctx);

    let idle_audio = preloaded_audio(&reg); // never GO'd — no voice
    let mut devamp = DevampCue::new();
    devamp.target_cue_ids = vec![idle_audio.id()];

    // Playhead starts on the devamp: put it first.
    let mut list = list_of(vec![Box::new(devamp), idle_audio]);
    transport.go(&mut list).unwrap();

    let count = log
        .lock()
        .unwrap()
        .iter()
        .filter(|c| matches!(c, EngineCall::AudioDevamp { .. }))
        .count();
    assert_eq!(count, 0, "an idle target contributes no voices — nothing to devamp");
}
