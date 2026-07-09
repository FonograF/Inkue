//! Behavioural GO/stop coverage for the non-audio cue types.
//!
//! The existing unit tests verify each cue *builds* the right message/params;
//! these verify `go()`/`stop()` actually **drive the engine** — the layer the
//! engine seam unlocked. OSC is asserted end-to-end over a real loopback UDP
//! socket; Text/Image via recording engine doubles; Wait is pure timing (it had
//! zero tests before).
//!
//! Still hardware/manual (documented, not covered here): MIDI real send (needs a
//! MIDI port / loopMIDI), Video playback (needs libmpv + a display), and Mic
//! capture (needs a live input device).

mod common;

use std::net::UdpSocket;
use std::time::{Duration, Instant};

use common::{full_registry, recording_context, recording_context_with, EngineCall};
use inkue_lib::cue::group_cue::GroupCue;
use inkue_lib::cue::light_cue::{LightCue, ParamTarget};
use inkue_lib::cue::osc_cue::OscCue;
use inkue_lib::cue::osc_types::{OscArg, OscMessage};
use inkue_lib::cue::traits::{Cue, CueFactory};
use inkue_lib::cue::types::{CueState, CueType, GroupMode};
use inkue_lib::cue::wait_cue::WaitCueFactory;
use inkue_lib::engine::fixture::{builtin_fixture_types, PatchedFixture};
use inkue_lib::engine::osc_patch::OscPatch;

// ---------------------------------------------------------------------------
// Group cue completion + Playlist / StartRandom fire paths
// ---------------------------------------------------------------------------
// Uses WaitCue children — they stay Running for a real duration and, like every
// cue, do NOT self-complete (completion is driven externally), so these tests
// exercise the group's own child-reaping, which the top-level completion
// detector never does for nested children.

/// A short WaitCue with no auto-continue (so a group drives the sequencing).
fn short_wait(ms: u64) -> Box<dyn Cue> {
    WaitCueFactory
        .from_json(serde_json::json!({
            "id": uuid::Uuid::new_v4().to_string(),
            "wait_duration_ms": ms,
            "continue_mode": "do_not_continue",
        }))
        .unwrap()
}

fn running_children(g: &GroupCue) -> usize {
    g.children.iter().filter(|c| c.state() == CueState::Running).count()
}

/// Tick the group until `is_complete()` or `timeout`, recording the peak number
/// of simultaneously-running children.  Returns (completed, max_concurrent).
fn tick_group_until_complete(
    g: &mut GroupCue,
    ctx: &inkue_lib::cue::context::CueContext,
    timeout: Duration,
) -> (bool, usize) {
    let start = Instant::now();
    let mut max_concurrent = running_children(g);
    while start.elapsed() < timeout {
        let _ = g.tick(ctx);
        max_concurrent = max_concurrent.max(running_children(g));
        if g.is_complete() {
            return (true, max_concurrent);
        }
        std::thread::sleep(Duration::from_millis(3));
    }
    (g.is_complete(), max_concurrent)
}

#[test]
fn simultaneous_group_completes_after_children_finish() {
    // Regression: a Timeline/Simultaneous group left its finished children stuck
    // in Running forever (is_complete never fired), stalling the app.
    let (ctx, _rx, _log) = recording_context();
    let mut g = GroupCue::new();
    g.mode = GroupMode::Simultaneous;
    g.children.push(short_wait(20));
    g.children.push(short_wait(40));
    g.go(&ctx).unwrap();
    assert_eq!(running_children(&g), 2, "both children fire at once");
    assert!(!g.is_complete());

    let (done, _) = tick_group_until_complete(&mut g, &ctx, Duration::from_millis(500));
    assert!(done, "simultaneous group must complete once all children finish");
    assert_eq!(running_children(&g), 0, "no child left stuck Running");
}

#[test]
fn playlist_plays_one_child_at_a_time_and_completes() {
    let (ctx, _rx, _log) = recording_context();
    let mut g = GroupCue::new();
    g.mode = GroupMode::Playlist;
    for _ in 0..3 {
        g.children.push(short_wait(20));
    }
    g.go(&ctx).unwrap();
    assert_eq!(running_children(&g), 1, "playlist starts exactly one child");

    let (done, max) = tick_group_until_complete(&mut g, &ctx, Duration::from_millis(800));
    assert!(done, "non-looping playlist completes after playing every child");
    assert_eq!(max, 1, "playlist is exclusive — never two children at once");
}

#[test]
fn looping_playlist_never_completes_and_always_plays() {
    let (ctx, _rx, _log) = recording_context();
    let mut g = GroupCue::new();
    g.mode = GroupMode::Playlist;
    g.set_playlist_loop(true);
    g.children.push(short_wait(20));
    g.children.push(short_wait(20));
    g.go(&ctx).unwrap();

    // Run well past both children's total time: a looping playlist keeps going.
    let start = Instant::now();
    let mut ever_idle = false;
    while start.elapsed() < Duration::from_millis(200) {
        let _ = g.tick(&ctx);
        assert!(!g.is_complete(), "a looping playlist never completes on its own");
        if running_children(&g) == 0 {
            ever_idle = true;
        }
        std::thread::sleep(Duration::from_millis(3));
    }
    // It wrapped and kept playing rather than stopping at the end.
    assert!(running_children(&g) >= 1 || !ever_idle,
        "looping playlist should still be playing after wrapping");
}

/// A short WaitCue set to Auto-Follow (chains to the next child at action start).
fn af_wait(ms: u64) -> Box<dyn Cue> {
    WaitCueFactory
        .from_json(serde_json::json!({
            "id": uuid::Uuid::new_v4().to_string(),
            "wait_duration_ms": ms,
            "continue_mode": "auto_follow",
        }))
        .unwrap()
}

#[test]
fn sequential_group_with_overlapping_auto_follow_children_completes() {
    // Reproduces the tutorial's sub-section (#8): auto-follow children chain at
    // start so they overlap; the last (do_not_continue) is the shortest and
    // finishes first. The group must still reap the longer siblings and complete
    // — none may linger in Running.
    let (ctx, _rx, _log) = recording_context();
    let mut g = GroupCue::new();
    g.mode = GroupMode::Sequential;
    g.children.push(af_wait(60));
    g.children.push(af_wait(50));
    g.children.push(af_wait(40));
    g.children.push(short_wait(20)); // last child, do_not_continue, shortest
    g.go(&ctx).unwrap();
    assert_eq!(running_children(&g), 4, "auto-follow chain overlaps all children");

    let (done, _) = tick_group_until_complete(&mut g, &ctx, Duration::from_millis(700));
    assert!(done, "sequential group must complete after overlapping children finish");
    assert_eq!(running_children(&g), 0, "no child left stuck Running (regression #8)");
}

#[test]
fn start_random_fires_exactly_one_child_per_go() {
    let (ctx, _rx, _log) = recording_context();
    let mut g = GroupCue::new();
    g.mode = GroupMode::StartRandom;
    for _ in 0..4 {
        g.children.push(short_wait(40));
    }
    g.go(&ctx).unwrap();
    assert_eq!(running_children(&g), 1, "start-random fires one random child");

    // A GO while running fires another random child (still one at a time here,
    // since waits are short; the point is GO is absorbed, not that it stacks).
    assert!(g.absorbs_go(), "a running start-random group absorbs GO");
}

// ---------------------------------------------------------------------------
// Wait — pure state machine + elapsed freeze (previously 0 tests)
// ---------------------------------------------------------------------------

#[test]
fn wait_go_pause_resume_stop_state_machine() {
    let reg = full_registry();
    let (ctx, _rx, _log) = recording_context();
    let mut w = reg.create(&CueType::Wait).unwrap();

    assert_eq!(w.state(), CueState::Standby);
    w.go(&ctx).unwrap();
    assert_eq!(w.state(), CueState::Running);
    w.pause(&ctx).unwrap();
    assert_eq!(w.state(), CueState::Paused);
    w.resume(&ctx).unwrap();
    assert_eq!(w.state(), CueState::Running);
    w.stop(&ctx).unwrap();
    assert_eq!(w.state(), CueState::Standby);
}

#[test]
fn wait_pause_freezes_elapsed() {
    let reg = full_registry();
    let (ctx, _rx, _log) = recording_context();
    let mut w = reg.create(&CueType::Wait).unwrap();

    w.go(&ctx).unwrap();
    std::thread::sleep(Duration::from_millis(10));
    w.pause(&ctx).unwrap();
    let frozen = w.elapsed();
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(w.elapsed(), frozen, "elapsed must not advance while the Wait is paused");
    assert!(frozen >= Duration::from_millis(1), "time should have accrued before the pause");
}

// ---------------------------------------------------------------------------
// OSC — real UDP send over loopback
// ---------------------------------------------------------------------------

#[test]
fn osc_cue_go_sends_a_udp_datagram_to_its_patch() {
    let sock = UdpSocket::bind("127.0.0.1:0").expect("bind loopback socket");
    sock.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
    let port = sock.local_addr().unwrap().port();

    let patch = OscPatch::new("Test", "127.0.0.1", port);
    let patch_id = patch.id;
    let (ctx, _rx, _log) = recording_context_with(vec![patch], vec![], vec![]);

    let mut cue = OscCue::new();
    cue.messages.push(OscMessage {
        patch_id,
        address: "/test/vol".to_string(),
        args: vec![OscArg::Float(0.5)],
    });

    cue.go(&ctx).unwrap();

    let mut buf = [0u8; 1024];
    let n = sock
        .recv(&mut buf)
        .expect("OSC cue GO must actually transmit a UDP datagram to the patch target");
    assert!(n > 0, "received an empty datagram");
    assert!(
        buf[..n].windows(9).any(|w| w == b"/test/vol"),
        "the datagram must carry the OSC address"
    );
}

#[test]
fn osc_cue_go_with_unknown_patch_does_not_error() {
    // A message referencing a patch that is not in the workspace must be a
    // logged no-op, never a hard failure that aborts the whole GO.
    let (ctx, _rx, _log) = recording_context();
    let mut cue = OscCue::new();
    cue.messages.push(OscMessage {
        patch_id: uuid::Uuid::new_v4(),
        address: "/orphan".to_string(),
        args: vec![],
    });
    assert!(cue.go(&ctx).is_ok(), "an unresolved OSC patch must not fail GO");
}

// ---------------------------------------------------------------------------
// Text — GO drives the mpv overlay, Stop clears it
// ---------------------------------------------------------------------------

#[test]
fn text_cue_go_pushes_overlay_containing_the_text() {
    let reg = full_registry();
    let (ctx, _rx, log) = recording_context();

    let mut tj = reg.create(&CueType::Text).unwrap().serialize();
    tj["text"] = serde_json::json!("HELLO SHOW");
    let mut cue = reg.from_json(tj).unwrap();

    cue.go(&ctx).unwrap();

    let ass = log.lock().unwrap().iter().find_map(|c| match c {
        EngineCall::OutputTextOverlay { ass } => Some(ass.clone()),
        _ => None,
    });
    let ass = ass.expect("Text GO must call show_text_overlay");
    assert!(ass.contains("HELLO SHOW"), "overlay ASS must embed the text, got: {ass}");
}

#[test]
fn text_cue_stop_clears_the_overlay() {
    let reg = full_registry();
    let (ctx, _rx, log) = recording_context();

    let mut tj = reg.create(&CueType::Text).unwrap().serialize();
    tj["text"] = serde_json::json!("BYE");
    let mut cue = reg.from_json(tj).unwrap();

    cue.go(&ctx).unwrap();
    cue.stop(&ctx).unwrap();

    let cleared = log.lock().unwrap().iter().any(|c| matches!(c, EngineCall::OutputClearText));
    assert!(cleared, "Text stop must clear the overlay");
}

// ---------------------------------------------------------------------------
// Image — GO shows content flagged as an image
// ---------------------------------------------------------------------------

#[test]
fn image_cue_go_shows_content_as_image() {
    let reg = full_registry();
    let (ctx, _rx, log) = recording_context();

    let mut ij = reg.create(&CueType::Image).unwrap().serialize();
    ij["file_path"] = serde_json::json!("images/logo.png");
    let mut cue = reg.from_json(ij).unwrap();

    cue.go(&ctx).unwrap();

    let show = log.lock().unwrap().iter().find_map(|c| match c {
        EngineCall::OutputShowContent { path, is_image } => Some((path.clone(), *is_image)),
        _ => None,
    });
    let (path, is_image) = show.expect("Image GO must call show_content");
    assert!(is_image, "an Image cue must present content as an image");
    assert!(path.ends_with("logo.png"), "the media path is forwarded to show_content, got: {path}");
}

#[test]
fn image_cue_go_without_a_file_completes_instantly() {
    // No file assigned → no engine call, cue completes so the sequence advances.
    let reg = full_registry();
    let (ctx, _rx, log) = recording_context();
    let mut cue = reg.create(&CueType::Image).unwrap();

    cue.go(&ctx).unwrap();

    assert_eq!(cue.state(), CueState::Completed, "an empty Image cue completes instantly");
    let showed = log.lock().unwrap().iter().any(|c| matches!(c, EngineCall::OutputShowContent { .. }));
    assert!(!showed, "an Image cue with no file must not call show_content");
}

// ---------------------------------------------------------------------------
// Light (DMX) — GO submits a timed fade for each resolved fixture channel
// ---------------------------------------------------------------------------

#[test]
fn light_cue_go_submits_a_fade_to_the_patched_fixture() {
    let ftype = builtin_fixture_types()
        .into_iter()
        .next()
        .expect("at least one builtin fixture type");
    // base_address 1 (1-based) → 0-based channel 0 for the first parameter.
    let fixture = PatchedFixture::new("Wash", 1, 1, ftype);
    let fixture_id = fixture.id;

    let (ctx, _rx, log) = recording_context_with(vec![], vec![fixture], vec![]);

    let mut cue = LightCue::new();
    cue.targets.push(ParamTarget::Fixture {
        fixture_id: fixture_id.to_string(),
        param_index: 0,
        value: 1.0,
    });

    cue.go(&ctx).unwrap();

    let fade = log.lock().unwrap().iter().find_map(|c| match c {
        EngineCall::DmxSubmitFade { universe, channel, target_norm } => {
            Some((*universe, *channel, *target_norm))
        }
        _ => None,
    });
    let (universe, channel, target) = fade.expect("Light GO must submit a DMX fade for a patched target");
    assert_eq!(universe, 1, "fade routed to the fixture's universe");
    assert_eq!(channel, 0, "base address 1 resolves to 0-based channel 0 for param 0");
    assert!((target - 1.0).abs() < 1e-9, "the target value is forwarded to the fade");
}

#[test]
fn light_cue_go_with_unpatched_target_is_a_noop_not_a_crash() {
    let (ctx, _rx, log) = recording_context();
    let mut cue = LightCue::new();
    cue.targets.push(ParamTarget::Fixture {
        fixture_id: uuid::Uuid::new_v4().to_string(),
        param_index: 0,
        value: 0.5,
    });

    assert!(cue.go(&ctx).is_ok(), "an unresolved fixture must not fail GO");
    assert!(
        log.lock().unwrap().iter().all(|c| !matches!(c, EngineCall::DmxSubmitFade { .. })),
        "an unpatched fixture target must submit no fade"
    );
}

// ---------------------------------------------------------------------------
// Fade — pan fade (QLab crosspoint fade → Inkue pan fade)
// ---------------------------------------------------------------------------

#[test]
fn fade_stop_at_end_queues_its_targets_after_completing() {
    use inkue_lib::cue::fade_cue::FadeCue;
    use uuid::Uuid;

    let (ctx, _rx, _log) = recording_context();
    let target = Uuid::new_v4();
    let vid = Uuid::new_v4();

    let mut fade = FadeCue::new();
    fade.target_cue_ids = vec![target];
    fade.target_volume_db = -60.0;
    fade.stop_at_end = true;
    fade.fade_duration_ms = 20;

    fade.go(&ctx).unwrap();
    fade.set_fade_voices(vec![(vid, 1.0, 0.0)], false, 0, 0);

    // Before completion, nothing to stop.
    assert!(fade.take_fade_stop_targets().is_empty());

    std::thread::sleep(Duration::from_millis(60));
    fade.tick(&ctx).unwrap();

    // Once the fade finished, it hands its target cue ids to the event loop.
    let targets = fade.take_fade_stop_targets();
    assert_eq!(targets, vec![target], "a stop_at_end fade must queue its targets to be stopped");
    // Drained exactly once — no re-stopping on later ticks.
    assert!(fade.take_fade_stop_targets().is_empty(), "targets are drained, not repeated");
}

#[test]
fn pan_only_fade_moves_voice_pan_and_leaves_gain_untouched() {
    use inkue_lib::cue::fade_cue::FadeCue;
    use uuid::Uuid;

    let (ctx, _rx, log) = recording_context();
    let vid = Uuid::new_v4();

    let mut fade = FadeCue::new();
    fade.target_cue_ids = vec![Uuid::new_v4()];
    fade.target_pan = Some(1.0); // pan fully right
    fade.fade_volume = false;    // pan-only: must NOT move the level
    fade.fade_duration_ms = 50;

    fade.go(&ctx).unwrap();
    // Transport injects (voice_id, start_gain, start_pan) after go().
    fade.set_fade_voices(vec![(vid, 0.5, -1.0)], false, 0, 0);

    std::thread::sleep(Duration::from_millis(80)); // past the fade duration
    fade.tick(&ctx).unwrap();

    let calls = log.lock().unwrap();
    let last_pan = calls.iter().rev().find_map(|c| match c {
        EngineCall::AudioSetPan { pan } => Some(*pan),
        _ => None,
    });
    assert!(last_pan.is_some(), "a pan fade must drive set_voice_pan");
    assert!(
        (last_pan.unwrap() - 1.0).abs() < 1e-3,
        "pan should reach +1 (right), got {last_pan:?}",
    );
    assert!(
        !calls.iter().any(|c| matches!(c, EngineCall::AudioSetGain { .. })),
        "a pan-only fade must not change the voice gain",
    );
}
