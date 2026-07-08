//! Zone (a) — audio decode path (`cue::media_decode` / `AudioCue::decode_file`).
//!
//! Covers the symphonia decode chain against generated WAV fixtures (all
//! dependency-free) plus, when present, real user-supplied compressed files.
//! The focus is the risks that degrade a show silently: a corrupt/truncated
//! file must never panic, and sample rate / channel count must survive decode
//! intact (a wrong SR desyncs every downstream duration computation).

mod common;

use std::io::Write;

use common::*;
use inkue_lib::cue::audio_cue::AudioCue;
use inkue_lib::cue::media_decode::decode_audio_track;

fn is_non_silent(samples: &[f32]) -> bool {
    samples.iter().any(|s| s.abs() > 0.01)
}

#[test]
fn decode_generated_wav_stereo_48k() {
    let dir = temp_dir("dec48");
    let path = dir.join("tone.wav");
    write_wav_pcm16(&path, &sine(440.0, 1.0, 48_000, 2), 2, 48_000);

    let (samples, channels, sr) = AudioCue::decode_file(&path).expect("decode should succeed");
    assert_eq!(channels, 2, "stereo file must report 2 channels");
    assert_eq!(sr, 48_000, "sample rate must survive decode");
    let frames = samples.len() / channels as usize;
    // ±1 frame tolerance for encoder/decoder boundary handling.
    assert!(
        (frames as i64 - 48_000).abs() <= 1,
        "1 s @ 48 kHz should decode ~48000 frames, got {frames}"
    );
    assert!(is_non_silent(&samples), "decoded tone must not be silent");
}

#[test]
fn decode_wav_44100_preserves_sr() {
    let dir = temp_dir("dec441");
    let path = dir.join("tone.wav");
    write_wav_pcm16(&path, &sine(1000.0, 0.5, 44_100, 2), 2, 44_100);
    let (_s, _c, sr) = AudioCue::decode_file(&path).expect("decode");
    assert_eq!(sr, 44_100);
}

#[test]
fn decode_wav_96000_preserves_sr() {
    let dir = temp_dir("dec96");
    let path = dir.join("tone.wav");
    write_wav_pcm16(&path, &sine(1000.0, 0.25, 96_000, 2), 2, 96_000);
    let (_s, _c, sr) = AudioCue::decode_file(&path).expect("decode");
    assert_eq!(sr, 96_000);
}

#[test]
fn decode_wav_nonstandard_sr_22050() {
    let dir = temp_dir("dec22");
    let path = dir.join("tone.wav");
    write_wav_pcm16(&path, &sine(500.0, 0.5, 22_050, 1), 1, 22_050);
    let (_s, channels, sr) = AudioCue::decode_file(&path).expect("decode");
    assert_eq!(sr, 22_050, "non-standard SR must be preserved verbatim");
    assert_eq!(channels, 1);
}

#[test]
fn decode_wav_mono() {
    let dir = temp_dir("decmono");
    let path = dir.join("mono.wav");
    write_wav_pcm16(&path, &sine(440.0, 0.5, 48_000, 1), 1, 48_000);
    let (samples, channels, _sr) = AudioCue::decode_file(&path).expect("decode");
    assert_eq!(channels, 1, "mono file must report 1 channel");
    assert!(is_non_silent(&samples));
}

#[test]
fn decode_wav_float32_branch() {
    // Exercises the IEEE-float (format 3) → F32 buffer path in media_decode.
    let dir = temp_dir("decf32");
    let path = dir.join("float.wav");
    write_wav_float32(&path, &sine(440.0, 0.5, 48_000, 2), 2, 48_000);
    let (samples, channels, sr) = AudioCue::decode_file(&path).expect("decode float wav");
    assert_eq!(channels, 2);
    assert_eq!(sr, 48_000);
    assert!(is_non_silent(&samples));
}

#[test]
fn decode_truncated_wav_does_not_panic() {
    // A valid header whose data is cut short mid-stream must degrade
    // gracefully (partial samples or a clean error), never panic.
    let dir = temp_dir("dectrunc");
    let path = dir.join("trunc.wav");
    write_wav_pcm16(&path, &sine(440.0, 1.0, 48_000, 2), 2, 48_000);

    // Chop the file to ~1/3 of its size, cutting the PCM data mid-stream.
    let bytes = std::fs::read(&path).unwrap();
    let cut = bytes.len() / 3;
    std::fs::write(&path, &bytes[..cut]).unwrap();

    let path2 = path.clone();
    let result = with_timeout(60, "decode truncated wav", move || decode_audio_track(&path2));
    match result {
        Ok(Some((samples, _c, _sr))) => {
            let full_frames = 48_000usize;
            assert!(
                samples.len() / 2 < full_frames,
                "truncated file should yield fewer than the declared frames"
            );
        }
        Ok(None) => {} // no audio track recovered — acceptable
        Err(_) => {}   // clean error — acceptable
    }
}

#[test]
fn decode_garbage_file_errors_without_panic() {
    let dir = temp_dir("decgarbage");
    let path = dir.join("garbage.wav");
    let mut f = std::fs::File::create(&path).unwrap();
    // Random-ish bytes with a .wav extension: neither symphonia nor mpv can
    // make sense of it, so both fallbacks must fail to a clean Err.
    let junk: Vec<u8> = (0..4096).map(|i| ((i * 37 + 11) % 251) as u8).collect();
    f.write_all(&junk).unwrap();
    drop(f);

    let path2 = path.clone();
    let result = with_timeout(90, "decode garbage", move || decode_audio_track(&path2));
    assert!(result.is_err(), "garbage input must return Err, got {result:?}");
}

#[test]
fn decode_empty_file_errors_without_panic() {
    let dir = temp_dir("decempty");
    let path = dir.join("empty.wav");
    std::fs::write(&path, []).unwrap();

    let path2 = path.clone();
    let result = with_timeout(90, "decode empty", move || decode_audio_track(&path2));
    assert!(result.is_err(), "empty input must return Err, got {result:?}");
}

#[test]
fn decode_missing_file_errors() {
    let path = temp_dir("decmissing").join("nope.wav");
    let result = decode_audio_track(&path);
    assert!(result.is_err(), "missing file must return Err");
}

// ---------------------------------------------------------------------------
// Real compressed fixtures — run only when the user has supplied files under
// tests/fixtures/audio/.  Absent files → the test is skipped (printed), not failed.
// ---------------------------------------------------------------------------

fn decode_real_fixture(ext: &str) {
    let Some(path) = find_fixture(ext) else {
        eprintln!(
            "[skip] no .{ext} fixture in {} — drop a short real file there to cover this codec",
            fixtures_audio_dir().display()
        );
        return;
    };
    let path2 = path.clone();
    let result = with_timeout(120, &format!("decode {ext}"), move || decode_audio_track(&path2));
    let decoded = result.unwrap_or_else(|e| panic!("failed to decode {}: {e}", path.display()));
    let (samples, channels, sr) = decoded.expect("fixture should contain an audio track");
    assert!(sr > 0, "{ext}: sample rate must be > 0");
    assert!(channels >= 1, "{ext}: at least one channel");
    assert!(!samples.is_empty(), "{ext}: decoded sample buffer must be non-empty");
    assert!(
        is_non_silent(&samples),
        "{ext}: fixture decoded to pure silence — is the file valid?"
    );
}

#[test]
fn decode_real_flac() {
    decode_real_fixture("flac");
}

#[test]
fn decode_real_mp3() {
    decode_real_fixture("mp3");
}

#[test]
fn decode_real_ogg() {
    decode_real_fixture("ogg");
}

#[test]
fn decode_real_m4a_aac() {
    // AAC ships either in an MP4 container (.m4a) or as a raw ADTS stream (.aac);
    // exercise whichever the user provided.
    if find_fixture("m4a").is_some() {
        decode_real_fixture("m4a");
    } else {
        decode_real_fixture("aac");
    }
}
