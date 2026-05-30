//! Shared media decoding: extract an audio track to interleaved f32 samples.
//!
//! Used by both [`AudioCue`](super::audio_cue::AudioCue) (audio files) and
//! [`VideoCue`](super::video_cue::VideoCue) (the audio track of a video
//! container).  Selecting the *first audio track* — rather than the container's
//! default track — is what lets the same decoder serve an `.mp4` whose default
//! track is video.

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
/// - `Err(..)` on an I/O or decode failure.
pub fn decode_audio_track(path: &Path) -> Result<Option<(Vec<f32>, u16, u32)>> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("Cannot open media file: {}", path.display()))?;

    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .with_context(|| format!("Unsupported media format: {}", path.display()))?;

    let mut format = probed.format;

    // Pick the first track that is actually audio.  Audio tracks always report a
    // sample rate; video/subtitle tracks do not.  This is what makes the decoder
    // work on a video container whose default track is the video stream.
    let track = match format
        .tracks()
        .iter()
        .find(|t| t.codec_params.sample_rate.is_some())
    {
        Some(t) => t,
        None => return Ok(None), // No audio track (silent video).
    };

    let track_id = track.id;
    let codec_params = track.codec_params.clone();

    let channels = codec_params.channels.map(|c| c.count() as u16).unwrap_or(2);
    let sample_rate = codec_params.sample_rate.unwrap_or(44100);

    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &DecoderOptions::default())
        .with_context(|| "Failed to create audio decoder")?;

    // Pre-allocate from the known frame count so the Vec does not repeatedly
    // double and copy the whole (potentially large) buffer while decoding.
    let estimated_samples = codec_params
        .n_frames
        .map(|n| n as usize * channels as usize)
        .unwrap_or(44_100 * 2 * 60); // fall back to ~1 min stereo
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

        let decoded = decoder.decode(&packet)?;

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
            // Any other format (S24, F64, U16, U24, …) via symphonia conversion.
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
