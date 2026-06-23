//! Shared media decoding: extract an audio track to interleaved f32 samples.
//!
//! Used by both [`AudioCue`](super::audio_cue::AudioCue) (audio files) and
//! [`VideoCue`](super::video_cue::VideoCue) (the audio track of a video
//! container).
//!
//! Decode chain:
//! 1. Symphonia with gapless enabled  (handles most MP3/WAV/FLAC/OGG/AAC)
//! 2. Symphonia with gapless disabled (handles MP3s with malformed Xing/LAME headers)
//! 3. libmpv → temp WAV → symphonia   (handles anything ffmpeg can read: MP2,
//!    unusual MPEG encodings, edge-case containers)

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Decode the first audio track of `path` to interleaved f32 samples.
///
/// Returns:
/// - `Ok(Some((samples, channels, sample_rate)))` when an audio track is found
///   and decoded,
/// - `Ok(None)` when the container has **no** audio track (e.g. a silent video),
/// - `Err(..)` on an I/O or decode failure after all fallbacks are exhausted.
pub fn decode_audio_track(path: &Path) -> Result<Option<(Vec<f32>, u16, u32)>> {
    // Try symphonia first (two attempts: gapless on, then off).
    match decode_with_symphonia(path) {
        Ok(r) => return Ok(r),
        Err(e) => {
            log::warn!(
                "Symphonia could not decode '{}': {e}. Trying libmpv fallback.",
                path.display()
            );
        }
    }

    // Fallback: transcode via libmpv (ffmpeg) → temp WAV → re-read with symphonia.
    decode_via_mpv(path)
}

// ---------------------------------------------------------------------------
// Symphonia decoder (steps 1 & 2)
// ---------------------------------------------------------------------------

/// Probe and decode `path` using symphonia only — no libmpv fallback.
///
/// Separating this from [`decode_audio_track`] ensures that [`decode_via_mpv`]
/// can call it on the temp WAV without risking infinite recursion.
fn decode_with_symphonia(path: &Path) -> Result<Option<(Vec<f32>, u16, u32)>> {
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = {
        let file = std::fs::File::open(path)
            .with_context(|| format!("Cannot open media file: {}", path.display()))?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        match symphonia::default::get_probe().format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        ) {
            Ok(p) => p,
            Err(_) => {
                // Some MP3s have malformed Xing/LAME gapless headers — retry without.
                let file2 = std::fs::File::open(path)
                    .with_context(|| format!("Cannot open media file: {}", path.display()))?;
                let mss2 = MediaSourceStream::new(Box::new(file2), Default::default());
                let mut hint2 = Hint::new();
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    hint2.with_extension(ext);
                }
                symphonia::default::get_probe()
                    .format(
                        &hint2,
                        mss2,
                        &FormatOptions { enable_gapless: false, ..Default::default() },
                        &MetadataOptions::default(),
                    )
                    .with_context(|| format!("Unsupported media format: {}", path.display()))?
            }
        }
    };

    let mut format = probed.format;

    // Pick the first track that actually reports a sample rate (audio tracks do;
    // video/subtitle tracks do not).
    let track = match format
        .tracks()
        .iter()
        .find(|t| t.codec_params.sample_rate.is_some())
    {
        Some(t) => t,
        None => return Ok(None),
    };

    let track_id = track.id;
    let codec_params = track.codec_params.clone();

    let channels = codec_params.channels.map(|c| c.count() as u16).unwrap_or(2);
    let sample_rate = codec_params.sample_rate.unwrap_or(44100);

    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &DecoderOptions::default())
        .with_context(|| "Failed to create audio decoder")?;

    let estimated_samples = codec_params
        .n_frames
        .map(|n| n as usize * channels as usize)
        .unwrap_or(44_100 * 2 * 60);
    let mut samples: Vec<f32> = Vec::with_capacity(estimated_samples);

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(_)) => break,
            Err(symphonia::core::errors::Error::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(e) => return Err(anyhow!("Decode error: {e}")),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            // Malformed frame — skip and continue rather than aborting.
            Err(symphonia::core::errors::Error::DecodeError(e)) => {
                log::warn!("Skipping malformed audio frame in {}: {e}", path.display());
                continue;
            }
            Err(e) => return Err(anyhow!("Decode error: {e}")),
        };

        let n_frames = decoded.frames();
        let n_ch = decoded.spec().channels.count();
        samples.reserve(n_frames * n_ch);

        match decoded {
            AudioBufferRef::F32(buf) => {
                for frame in 0..n_frames {
                    for ch in 0..n_ch {
                        samples.push(buf.chan(ch)[frame]);
                    }
                }
            }
            AudioBufferRef::S16(buf) => {
                for frame in 0..n_frames {
                    for ch in 0..n_ch {
                        samples.push(buf.chan(ch)[frame] as f32 / i16::MAX as f32);
                    }
                }
            }
            AudioBufferRef::S32(buf) => {
                for frame in 0..n_frames {
                    for ch in 0..n_ch {
                        samples.push(buf.chan(ch)[frame] as f32 / i32::MAX as f32);
                    }
                }
            }
            AudioBufferRef::U8(buf) => {
                for frame in 0..n_frames {
                    for ch in 0..n_ch {
                        samples.push(buf.chan(ch)[frame] as f32 / 128.0 - 1.0);
                    }
                }
            }
            other => {
                let mut f32_buf = other.make_equivalent::<f32>();
                other.convert(&mut f32_buf);
                let n_frames = f32_buf.frames();
                let n_ch = f32_buf.spec().channels.count();
                for frame in 0..n_frames {
                    for ch in 0..n_ch {
                        samples.push(f32_buf.chan(ch)[frame]);
                    }
                }
            }
        }
    }

    samples.shrink_to_fit();
    Ok(Some((samples, channels, sample_rate)))
}

// ---------------------------------------------------------------------------
// libmpv fallback decoder (step 3)
// ---------------------------------------------------------------------------

/// Decode `path` by having libmpv transcode it to a temp WAV file, then reading
/// that WAV with symphonia.
///
/// libmpv delegates to ffmpeg internally and can handle formats symphonia cannot
/// (MP2, unusual MPEG variants, some AAC-in-MP3 wrappers, etc.).
fn decode_via_mpv(path: &Path) -> Result<Option<(Vec<f32>, u16, u32)>> {
    use crate::engine::mpv_sys::{MpvLib, MPV_EVENT_END_FILE, MPV_EVENT_SHUTDOWN};
    use std::ffi::CString;

    let mpv = MpvLib::load().context("libmpv not available for audio fallback")?;

    let tmp_path = std::env::temp_dir()
        .join(format!("wincue_audio_{}.wav", uuid::Uuid::new_v4().simple()));

    let decode_result: Result<()> = (|| {
        unsafe {
            #[cfg(not(target_os = "windows"))]
            libc::setlocale(libc::LC_NUMERIC, c"C".as_ptr());

            let ctx = (mpv.mpv_create)();
            if ctx.is_null() {
                return Err(anyhow!("mpv_create returned null"));
            }

            // Helper: set a string option before initialize.
            let set = |key: &str, val: &str| {
                if let (Ok(k), Ok(v)) = (CString::new(key), CString::new(val)) {
                    (mpv.mpv_set_option_string)(ctx, k.as_ptr(), v.as_ptr());
                }
            };

            set("video", "no");
            set("vo", "null");
            set("ao", "pcm");
            set("audio-channels", "stereo");

            // Forward slashes required — libmpv on Windows does not always accept backslashes.
            let tmp_str = tmp_path
                .to_str()
                .ok_or_else(|| anyhow!("temp path is not valid UTF-8"))?
                .replace('\\', "/");
            set("ao-pcm-file", &tmp_str);

            if (mpv.mpv_initialize)(ctx) < 0 {
                (mpv.mpv_terminate_destroy)(ctx);
                return Err(anyhow!("mpv_initialize failed"));
            }

            let path_str = path
                .to_str()
                .ok_or_else(|| anyhow!("file path is not valid UTF-8"))?;
            let cmd = CString::new("loadfile").unwrap();
            let arg = CString::new(path_str)?;
            let null: *const std::ffi::c_char = std::ptr::null();
            let argv = [cmd.as_ptr(), arg.as_ptr(), null];
            (mpv.mpv_command)(ctx, argv.as_ptr());

            // Block until playback ends (up to 5 minutes).
            loop {
                let ev = (mpv.mpv_wait_event)(ctx, 300.0);
                if ev.is_null() {
                    break;
                }
                match (*ev).event_id {
                    MPV_EVENT_END_FILE | MPV_EVENT_SHUTDOWN => break,
                    _ => {}
                }
            }

            (mpv.mpv_terminate_destroy)(ctx);
        }
        Ok(())
    })();

    if let Err(e) = decode_result {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }

    // Read the WAV that mpv wrote, using the pure-symphonia path (no recursion).
    let wav_result = decode_with_symphonia(&tmp_path);
    let _ = std::fs::remove_file(&tmp_path);

    wav_result.with_context(|| {
        format!(
            "libmpv transcoded '{}' but the resulting WAV could not be read",
            path.display()
        )
    })
}
