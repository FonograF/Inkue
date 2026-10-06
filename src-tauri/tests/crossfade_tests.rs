//! Zone (🔴 core) — Crossfade: a Fade Cue that dissolves its targets into
//! another visual cue.
//!
//! Drives `Transport::go` plus the event loop's per-tick crossfade step
//! (`show::crossfade::advance_pending`) against the recording engine doubles.
//! The output double's `CrossfadeSim` stands in for the picture: a linked
//! dissolve either starts at once (content already on screen) or waits for
//! `reveal_all()` (a video decoding its first frame).

mod common;

use std::time::{Duration, Instant};

use common::{
    full_registry, recording_context, recording_context_with_crossfades, CallLog, CrossfadeSim,
    EngineCall,
};
use inkue_lib::cue::context::CueContext;
use inkue_lib::cue::fade_cue::FadeCue;
use inkue_lib::cue::group_cue::GroupCue;
use inkue_lib::cue::registry::CueRegistry;
use inkue_lib::cue::traits::Cue;
use inkue_lib::cue::types::{ContinueMode, CueType, GroupMode};
use inkue_lib::show::crossfade;
use inkue_lib::show::cue_list::CueList;
use inkue_lib::show::transport::{GoResult, Transport};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn visual(reg: &CueRegistry, cue_type: CueType, file: &str) -> Box<dyn Cue> {
    let mut json = reg.create(&cue_type).unwrap().serialize();
    json["file_path"] = serde_json::json!(file);
    reg.from_json(json).unwrap()
}

fn video(reg: &CueRegistry, file: &str) -> Box<dyn Cue> {
    visual(reg, CueType::Video, file)
}

/// A crossfading Fade with `stop_at_end` **off**: whatever it dissolves away
/// must still go, because a crossfade replaces its targets.
fn crossfade_fade(targets: Vec<Uuid>, into: Uuid, ms: u64) -> FadeCue {
    let mut fade = FadeCue::new();
    fade.target_cue_ids = targets;
    fade.crossfade_into = Some(into);
    fade.fade_duration_ms = ms;
    fade.stop_at_end = false;
    fade.set_continue_mode(ContinueMode::DoNotContinue);
    fade
}

fn list_of(cues: Vec<Box<dyn Cue>>) -> CueList {
    let mut list = CueList::new("T");
    for c in cues {
        list.push(c);
    }
    list
}

fn go(transport: &mut Transport, list: &mut CueList, id: Uuid) -> GoResult {
    list.playhead_cue_id = Some(id);
    transport.go(list).unwrap()
}

/// What the event loop does every tick for these cues: tick every running
/// cue, then push the crossfades that had to wait.
fn run_for(list: &mut CueList, ctx: &CueContext, duration: Duration) -> Vec<Uuid> {
    let mut started = Vec::new();
    let begin = Instant::now();
    while begin.elapsed() < duration {
        for cue in list.cues.iter_mut() {
            if cue.is_running() {
                let _ = cue.tick(ctx);
            }
        }
        started.extend(crossfade::advance_pending(list, ctx));
        std::thread::sleep(Duration::from_millis(3));
    }
    started
}

fn links(log: &CallLog) -> Vec<(u32, usize)> {
    log.lock()
        .unwrap()
        .iter()
        .filter_map(|c| match c {
            EngineCall::OutputLinkCrossfade { duration_ms, outgoing } => Some((*duration_ms, *outgoing)),
            _ => None,
        })
        .collect()
}

fn gains(log: &CallLog) -> Vec<f32> {
    log.lock()
        .unwrap()
        .iter()
        .filter_map(|c| match c {
            EngineCall::AudioSetGain { gain } => Some(*gain),
            _ => None,
        })
        .collect()
}

fn count(log: &CallLog, wanted: fn(&EngineCall) -> bool) -> usize {
    log.lock().unwrap().iter().filter(|c| wanted(c)).count()
}

fn fade_stops(list: &mut CueList, fade_id: Uuid) -> Vec<Uuid> {
    list.get_mut(&fade_id).unwrap().take_fade_stop_targets()
}

/// A, B and a Fade dissolving A into B; A is on screen.
struct Scene {
    list: CueList,
    transport: Transport,
    ctx: CueContext,
    log: CallLog,
    sim: CrossfadeSim,
    /// The incoming picture takes a while to appear (`sim.reveal_all()`).
    picture_waits: bool,
    a: Uuid,
    b: Uuid,
    fade: Uuid,
}

fn scene(fade_ms: u64, picture_waits: bool, video_audio: bool) -> Scene {
    let reg = full_registry();
    let (ctx, _rx, log, sim) = recording_context_with_crossfades(video_audio);
    let a = video(&reg, "video/a.mp4");
    let b = video(&reg, "video/b.mp4");
    let (a_id, b_id) = (a.id(), b.id());
    let fade = crossfade_fade(vec![a_id], b_id, fade_ms);
    let fade_id = fade.id();
    let mut transport = Transport::new(ctx.clone());
    let mut list = list_of(vec![a, b, Box::new(fade)]);
    go(&mut transport, &mut list, a_id);
    Scene { list, transport, ctx, log, sim, picture_waits, a: a_id, b: b_id, fade: fade_id }
}

impl Scene {
    fn go_fade(&mut self) -> GoResult {
        let result = go(&mut self.transport, &mut self.list, self.fade);
        if !self.picture_waits {
            self.sim.reveal_all();
        }
        result
    }

    fn action_elapsed(&self) -> Duration {
        self.list.get(&self.fade).unwrap().action_elapsed()
    }
}

// ---------------------------------------------------------------------------
// Starting and linking
// ---------------------------------------------------------------------------

#[test]
fn crossfade_starts_the_incoming_cue_and_reports_it_as_triggered() {
    let mut s = scene(1500, false, false);
    let result = s.go_fade();
    assert!(s.list.get(&s.b).unwrap().is_running(), "the Fade starts the cue it dissolves into");
    assert!(
        result.triggered.contains(&s.b),
        "the UI only refreshes a cue it is told about: {:?}",
        result.triggered,
    );
    assert_eq!(links(&s.log), vec![(1500, 1)], "one dissolve, with the fade's duration and one picture away");
}

#[test]
fn every_outgoing_picture_is_dissolved_away() {
    let reg = full_registry();
    let (ctx, _rx, log, _sim) = recording_context_with_crossfades(false);
    let a1 = video(&reg, "video/a1.mp4");
    let a2 = visual(&reg, CueType::Image, "img/a2.png");
    let b = video(&reg, "video/b.mp4");
    let (a1_id, a2_id, b_id) = (a1.id(), a2.id(), b.id());
    let fade = crossfade_fade(vec![a1_id, a2_id], b_id, 500);
    let fade_id = fade.id();
    let mut transport = Transport::new(ctx);
    let mut list = list_of(vec![a1, a2, b, Box::new(fade)]);
    go(&mut transport, &mut list, a1_id);
    go(&mut transport, &mut list, a2_id);
    go(&mut transport, &mut list, fade_id);

    assert_eq!(links(&log), vec![(500, 2)], "both targets leave in the same dissolve");
}

#[test]
fn a_crossfade_with_nothing_on_screen_still_brings_the_cue_in() {
    // The target is not playing: there is nothing to dissolve away, but the
    // incoming picture must still fade in over the fade time, not cut in.
    let reg = full_registry();
    let (ctx, _rx, log, _sim) = recording_context_with_crossfades(false);
    let a = video(&reg, "video/a.mp4");
    let b = video(&reg, "video/b.mp4");
    let fade = crossfade_fade(vec![a.id()], b.id(), 800);
    let fade_id = fade.id();
    let mut transport = Transport::new(ctx);
    let mut list = list_of(vec![a, b, Box::new(fade)]);
    go(&mut transport, &mut list, fade_id);

    assert_eq!(links(&log), vec![(800, 0)]);
}

#[test]
fn a_fade_without_crossfade_links_nothing() {
    let reg = full_registry();
    let (ctx, _rx, log) = recording_context();
    let a = video(&reg, "video/a.mp4");
    let a_id = a.id();
    let mut fade = FadeCue::new();
    fade.target_cue_ids = vec![a_id];
    let fade_id = fade.id();
    let mut transport = Transport::new(ctx);
    let mut list = list_of(vec![a, Box::new(fade)]);
    go(&mut transport, &mut list, a_id);
    go(&mut transport, &mut list, fade_id);

    assert!(links(&log).is_empty());
}

// ---------------------------------------------------------------------------
// The clock follows the picture
// ---------------------------------------------------------------------------

#[test]
fn the_fade_waits_for_the_incoming_picture() {
    let mut s = scene(40, true, false);
    s.go_fade();
    let ctx = s.ctx.clone();
    run_for(&mut s.list, &ctx, Duration::from_millis(60));
    assert_eq!(s.action_elapsed(), Duration::ZERO, "no picture yet: the dissolve has not started");
    assert!(fade_stops(&mut s.list, s.fade).is_empty(), "nor ended");

    s.sim.reveal_all();
    run_for(&mut s.list, &ctx, Duration::from_millis(80));
    assert!(s.action_elapsed() >= Duration::from_millis(40), "first frame: the fade runs");
    assert_eq!(fade_stops(&mut s.list, s.fade), vec![s.a], "and removes what it dissolved away");
}

#[test]
fn the_outgoing_picture_is_left_to_the_output_engine() {
    // Two clocks animating one layer would fight: the engine owns the picture.
    let mut s = scene(20, false, false);
    s.go_fade();
    let ctx = s.ctx.clone();
    run_for(&mut s.list, &ctx, Duration::from_millis(60));
    assert_eq!(count(&s.log, |c| matches!(c, EngineCall::OutputSetOpacity { .. })), 0);
}

#[test]
fn a_crossfade_removes_what_it_dissolved_away_even_without_stop_at_end() {
    let mut s = scene(20, false, false);
    s.go_fade();
    let ctx = s.ctx.clone();
    run_for(&mut s.list, &ctx, Duration::from_millis(60));
    let stops = fade_stops(&mut s.list, s.fade);
    assert_eq!(stops, vec![s.a], "a crossfade replaces its targets");
    assert!(!stops.contains(&s.b), "and never stops the cue it brought in");
}

#[test]
fn the_incoming_sound_rises_from_silence_and_the_outgoing_sound_falls_to_it() {
    let mut s = scene(20, false, true);
    s.go_fade();
    assert!(gains(&s.log).contains(&0.0), "the incoming sound starts silent, under the still-dark picture");
    let ctx = s.ctx.clone();
    run_for(&mut s.list, &ctx, Duration::from_millis(80));
    let all = gains(&s.log);
    assert!(all.iter().any(|g| (*g - 1.0).abs() < 1e-3), "the incoming sound reaches its level: {all:?}");
    let last_two: Vec<f32> = all.iter().rev().take(2).copied().collect();
    assert!(
        last_two.iter().any(|g| g.abs() < 1e-3),
        "and the outgoing sound lands on silence: {last_two:?}",
    );
}

// ---------------------------------------------------------------------------
// Falling back to a plain fade
// ---------------------------------------------------------------------------

#[test]
fn crossfading_into_a_cue_already_on_screen_falls_back_to_a_plain_fade() {
    // There is no entrance left to dissolve; restarting its fade from black
    // would flash it off and on.
    let mut s = scene(20, false, false);
    let b = s.b;
    go(&mut s.transport, &mut s.list, b);
    s.go_fade();
    assert!(links(&s.log).is_empty());
    let ctx = s.ctx.clone();
    run_for(&mut s.list, &ctx, Duration::from_millis(60));
    assert!(
        count(&s.log, |c| matches!(c, EngineCall::OutputSetOpacity { .. })) > 0,
        "the target is faded like an ordinary Fade",
    );
}

#[test]
fn crossfading_into_a_cue_that_is_not_visual_falls_back_to_a_plain_fade() {
    let reg = full_registry();
    let (ctx, _rx, log, _sim) = recording_context_with_crossfades(false);
    let a = video(&reg, "video/a.mp4");
    let memo = reg.create(&CueType::Memo).unwrap();
    let (a_id, memo_id) = (a.id(), memo.id());
    let fade = crossfade_fade(vec![a_id], memo_id, 30);
    let fade_id = fade.id();
    let mut transport = Transport::new(ctx.clone());
    let mut list = list_of(vec![a, memo, Box::new(fade)]);
    go(&mut transport, &mut list, a_id);
    let result = go(&mut transport, &mut list, fade_id);

    assert!(links(&log).is_empty());
    assert!(!result.triggered.contains(&memo_id), "and the Fade does not start it");
    run_for(&mut list, &ctx, Duration::from_millis(60));
    assert!(count(&log, |c| matches!(c, EngineCall::OutputSetOpacity { .. })) > 0);
}

#[test]
fn an_abandoned_crossfade_no_longer_stops_its_targets() {
    // The incoming cue left mid-way (stopped, failed): the outgoing picture is
    // back on screen, so the Fade must not cut it at its end.
    let mut s = scene(60, false, false);
    s.go_fade();
    let ctx = s.ctx.clone();
    run_for(&mut s.list, &ctx, Duration::from_millis(15));
    s.sim.vanish_all();
    run_for(&mut s.list, &ctx, Duration::from_millis(100));
    assert!(fade_stops(&mut s.list, s.fade).is_empty());
}

// ---------------------------------------------------------------------------
// Pre-waits
// ---------------------------------------------------------------------------

#[test]
fn an_incoming_cue_with_a_pre_wait_is_linked_once_it_has_a_picture() {
    let reg = full_registry();
    let (ctx, _rx, log, sim) = recording_context_with_crossfades(false);
    let a = video(&reg, "video/a.mp4");
    let mut b = video(&reg, "video/b.mp4");
    b.set_pre_wait(Duration::from_millis(40));
    let (a_id, b_id) = (a.id(), b.id());
    let fade = crossfade_fade(vec![a_id], b_id, 30);
    let fade_id = fade.id();
    let mut transport = Transport::new(ctx.clone());
    let mut list = list_of(vec![a, b, Box::new(fade)]);
    go(&mut transport, &mut list, a_id);
    let result = go(&mut transport, &mut list, fade_id);

    assert!(result.triggered.contains(&b_id), "the incoming cue starts on the Fade's GO");
    assert!(links(&log).is_empty(), "but has no picture during its pre-wait");
    run_for(&mut list, &ctx, Duration::from_millis(80));
    assert_eq!(links(&log), vec![(30, 1)], "linked as soon as its picture exists");
    sim.reveal_all();
    run_for(&mut list, &ctx, Duration::from_millis(80));
    assert_eq!(fade_stops(&mut list, fade_id), vec![a_id]);
}

#[test]
fn a_fade_with_a_pre_wait_starts_its_crossfade_after_it() {
    let reg = full_registry();
    let (ctx, _rx, log, sim) = recording_context_with_crossfades(false);
    let a = video(&reg, "video/a.mp4");
    let b = video(&reg, "video/b.mp4");
    let (a_id, b_id) = (a.id(), b.id());
    let mut fade = crossfade_fade(vec![a_id], b_id, 30);
    fade.set_pre_wait(Duration::from_millis(40));
    let fade_id = fade.id();
    let mut transport = Transport::new(ctx.clone());
    let mut list = list_of(vec![a, b, Box::new(fade)]);
    go(&mut transport, &mut list, a_id);
    let result = go(&mut transport, &mut list, fade_id);

    assert!(!list.get(&b_id).unwrap().is_running(), "nothing happens during the Fade's pre-wait");
    assert!(!result.triggered.contains(&b_id));
    let started = run_for(&mut list, &ctx, Duration::from_millis(80));
    assert_eq!(started, vec![b_id], "the event loop starts it when the pre-wait ends");
    assert_eq!(links(&log).len(), 1);
    sim.reveal_all();
    run_for(&mut list, &ctx, Duration::from_millis(80));
    assert_eq!(fade_stops(&mut list, fade_id), vec![a_id]);
}

// ---------------------------------------------------------------------------
// Pause, stop
// ---------------------------------------------------------------------------

#[test]
fn pausing_the_fade_holds_the_dissolve_and_resuming_releases_it() {
    let mut s = scene(500, false, false);
    s.go_fade();
    let ctx = s.ctx.clone();
    let fade = s.list.get_mut(&s.fade).unwrap();
    fade.pause(&ctx).unwrap();
    fade.resume(&ctx).unwrap();
    let holds: Vec<bool> = s
        .log
        .lock()
        .unwrap()
        .iter()
        .filter_map(|c| match c {
            EngineCall::OutputHoldCrossfade { paused } => Some(*paused),
            _ => None,
        })
        .collect();
    assert_eq!(holds, vec![true, false], "picture and sound pause together");
}

#[test]
fn stopping_the_fade_releases_the_dissolve() {
    let mut s = scene(500, false, false);
    s.go_fade();
    let ctx = s.ctx.clone();
    s.list.get_mut(&s.fade).unwrap().stop(&ctx).unwrap();
    assert_eq!(count(&s.log, |c| matches!(c, EngineCall::OutputReleaseCrossfade)), 1);
}

// ---------------------------------------------------------------------------
// Groups
// ---------------------------------------------------------------------------

fn group_of(children: Vec<Box<dyn Cue>>) -> GroupCue {
    let mut group = GroupCue::new();
    group.mode = GroupMode::Simultaneous;
    group.children = children;
    group
}

#[test]
fn a_group_holding_a_video_is_dissolved_as_a_whole() {
    let reg = full_registry();
    let (ctx, _rx, log, sim) = recording_context_with_crossfades(false);
    let group = group_of(vec![video(&reg, "video/a.mp4"), visual(&reg, CueType::Image, "img/logo.png")]);
    let b = video(&reg, "video/b.mp4");
    let (group_id, b_id) = (group.id(), b.id());
    let fade = crossfade_fade(vec![group_id], b_id, 20);
    let fade_id = fade.id();
    let mut transport = Transport::new(ctx.clone());
    let mut list = list_of(vec![Box::new(group), b, Box::new(fade)]);
    go(&mut transport, &mut list, group_id);
    go(&mut transport, &mut list, fade_id);

    assert_eq!(links(&log), vec![(20, 2)], "both pictures of the group dissolve away");
    sim.reveal_all();
    run_for(&mut list, &ctx, Duration::from_millis(60));
    assert_eq!(fade_stops(&mut list, fade_id), vec![group_id], "and the group goes as one");
}

#[test]
fn a_plain_fade_on_a_group_fades_its_pictures() {
    // Regression: a group's pictures were never faded — the transport only
    // looked for pictures on visual leaf cues.
    let reg = full_registry();
    let (ctx, _rx, log) = recording_context();
    let group = group_of(vec![video(&reg, "video/a.mp4")]);
    let group_id = group.id();
    let mut fade = FadeCue::new();
    fade.target_cue_ids = vec![group_id];
    fade.fade_duration_ms = 20;
    let fade_id = fade.id();
    let mut transport = Transport::new(ctx.clone());
    let mut list = list_of(vec![Box::new(group), Box::new(fade)]);
    go(&mut transport, &mut list, group_id);
    go(&mut transport, &mut list, fade_id);
    run_for(&mut list, &ctx, Duration::from_millis(60));

    assert!(count(&log, |c| matches!(c, EngineCall::OutputSetOpacity { .. })) > 0);
}

#[test]
fn the_incoming_cue_is_never_stopped_even_when_listed_as_a_target() {
    let mut s = scene(20, false, false);
    let (a, b, fade) = (s.a, s.b, s.fade);
    if let Some(cue) = s.list.get_mut(&fade) {
        let mut json = cue.serialize();
        json["target_cue_ids"] = serde_json::json!([a.to_string(), b.to_string()]);
        json["stop_at_end"] = serde_json::json!(true);
        let rebuilt = full_registry().from_json(json).unwrap();
        let index = s.list.index_of(&fade).unwrap();
        s.list.cues[index] = rebuilt;
    }
    s.go_fade();
    let ctx = s.ctx.clone();
    run_for(&mut s.list, &ctx, Duration::from_millis(60));
    assert_eq!(fade_stops(&mut s.list, s.fade), vec![a]);
}
