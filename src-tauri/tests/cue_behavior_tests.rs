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
use std::time::Duration;

use common::{full_registry, recording_context, recording_context_with, EngineCall};
use inkue_lib::cue::light_cue::{LightCue, ParamTarget};
use inkue_lib::cue::osc_cue::OscCue;
use inkue_lib::cue::osc_types::{OscArg, OscMessage};
use inkue_lib::cue::traits::Cue;
use inkue_lib::cue::types::{CueState, CueType};
use inkue_lib::engine::fixture::{builtin_fixture_types, PatchedFixture};
use inkue_lib::engine::osc_patch::OscPatch;

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
