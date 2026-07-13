//! Media thumbnail generation for the inspector previews.
//!
//! A throwaway libmpv context with `vo=image` decodes one frame headlessly and
//! writes it as a JPEG — one code path covers video **and** image files (every
//! format the playback engine accepts previews identically). Thumbnails are
//! cached on disk keyed by path + size + mtime, so a file is only decoded
//! once until it changes.

use std::collections::hash_map::DefaultHasher;
use std::ffi::CString;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use base64::Engine as _;

use super::mpv_sys::{MpvLib, MPV_EVENT_END_FILE, MPV_EVENT_SHUTDOWN};

/// Thumbnail width in pixels (height follows the aspect ratio).
const THUMB_WIDTH: u32 = 400;
/// Raw-file fallback cap: images the browser can show natively (e.g. SVG,
/// which mpv cannot rasterise without librsvg) are sent as-is below this size.
const RAW_FALLBACK_MAX_BYTES: u64 = 10 * 1024 * 1024;

fn cache_dir() -> PathBuf {
    crate::machine_config::config_base_dir()
        .join("Inkue")
        .join("thumbnails")
}

/// Cache key for a media file: hash of absolute path + size + mtime, so an
/// edited/replaced file gets a fresh thumbnail while the stale JPEG ages out.
fn cache_key(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let mut h = DefaultHasher::new();
    path.hash(&mut h);
    meta.len().hash(&mut h);
    if let Ok(modified) = meta.modified() {
        if let Ok(d) = modified.duration_since(std::time::UNIX_EPOCH) {
            d.as_secs().hash(&mut h);
        }
    }
    Some(format!("{:016x}.jpg", h.finish()))
}

fn jpeg_data_url(bytes: &[u8]) -> String {
    format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// Return a `data:` URL thumbnail for a media file, generating and caching it
/// on first request.
///
/// `seek_into` picks a representative frame ~15 % into the file (videos —
/// frame 0 is often black); still images always use their single frame.
pub fn media_thumbnail(lib: &MpvLib, path: &Path, seek_into: bool) -> Result<String> {
    let key = cache_key(path)
        .ok_or_else(|| anyhow!("file not accessible: {}", path.display()))?;
    let cached = cache_dir().join(&key);
    if let Ok(bytes) = std::fs::read(&cached) {
        return Ok(jpeg_data_url(&bytes));
    }

    match render_one_frame(lib, path, seek_into) {
        Ok(bytes) => {
            let _ = std::fs::create_dir_all(cache_dir());
            let _ = std::fs::write(&cached, &bytes);
            Ok(jpeg_data_url(&bytes))
        }
        // mpv cannot rasterise everything the WebView can (SVG): fall back to
        // the raw file for browser-native image formats.
        Err(e) => raw_image_fallback(path).ok_or(e),
    }
}

fn raw_image_fallback(path: &Path) -> Option<String> {
    let mime = match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("bmp") => "image/bmp",
        _ => return None,
    };
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > RAW_FALLBACK_MAX_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    ))
}

/// Filmstrip for the video trimmer: `tiles` frames evenly spread across the
/// file, as JPEG `data:` URLs in playback order. Disk-cached like thumbnails.
///
/// `tile_width` sizes each frame (the trimmer strip uses small tiles, the
/// drag scrub-preview a denser strip of larger ones).
pub fn video_filmstrip(
    lib: &MpvLib,
    path: &Path,
    tiles: usize,
    tile_width: u32,
) -> Result<Vec<String>> {
    let tiles = tiles.clamp(2, 48);
    let tile_width = tile_width.clamp(80, 640);
    let key = cache_key(path)
        .ok_or_else(|| anyhow!("file not accessible: {}", path.display()))?;
    let stem = key.trim_end_matches(".jpg").to_string();
    let tile_path =
        |i: usize| cache_dir().join(format!("{stem}-strip{tiles}w{tile_width}-{i}.jpg"));

    // Cache hit only when every tile is present (a partial strip regenerates).
    let cached: Vec<Vec<u8>> = (0..tiles)
        .map(|i| std::fs::read(tile_path(i)))
        .collect::<std::io::Result<_>>()
        .unwrap_or_default();
    if cached.len() == tiles {
        return Ok(cached.iter().map(|b| jpeg_data_url(b)).collect());
    }

    let duration = super::output_engine::OutputEngine::probe_duration(lib, path)
        .ok_or_else(|| anyhow!("could not probe duration of {}", path.display()))?;
    // `sstep` skips this many seconds after every displayed frame, so with
    // `frames=tiles` the strip covers ~the whole file.
    let step_secs = (duration.as_secs_f64() / tiles as f64).max(0.1);

    let out_dir = std::env::temp_dir().join(format!("inkue-strip-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&out_dir)?;
    let result = render_frames_into(lib, path, &out_dir, &[
        ("frames", &tiles.to_string()),
        ("sstep", &format!("{step_secs:.3}")),
        ("vf", &format!("scale={tile_width}:-2")),
    ]);
    let frames = match result {
        Ok(f) => f,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&out_dir);
            return Err(e);
        }
    };
    let _ = std::fs::remove_dir_all(&out_dir);
    if frames.is_empty() {
        return Err(anyhow!("mpv produced no frames for {}", path.display()));
    }

    let _ = std::fs::create_dir_all(cache_dir());
    for (i, bytes) in frames.iter().enumerate() {
        let _ = std::fs::write(tile_path(i), bytes);
    }
    Ok(frames.iter().map(|b| jpeg_data_url(b)).collect())
}

/// Decode one frame of `path` into a JPEG via a throwaway `vo=image` context.
/// Filmstrip over a time range, for the zoomed clip editor: `tiles` frames
/// evenly spread across `[start_s, end_s]`.  Cached on a half-second grid so
/// nearby zoom windows reuse the same tiles.
pub fn video_filmstrip_range(
    lib: &MpvLib,
    path: &Path,
    start_s: f64,
    end_s: f64,
    tiles: usize,
    tile_width: u32,
) -> Result<Vec<String>> {
    let tiles = tiles.clamp(2, 24);
    let tile_width = tile_width.clamp(80, 640);
    if end_s <= start_s || start_s < 0.0 || !start_s.is_finite() || !end_s.is_finite() {
        return Err(anyhow!("invalid filmstrip range {start_s}..{end_s}"));
    }
    let key = cache_key(path)
        .ok_or_else(|| anyhow!("file not accessible: {}", path.display()))?;
    let stem = key.trim_end_matches(".jpg").to_string();
    // Half-second grid keys: zooming/panning small amounts hits the cache.
    let (gs, ge) = ((start_s * 2.0).round() as i64, (end_s * 2.0).round() as i64);
    let tile_path = |i: usize| {
        cache_dir().join(format!("{stem}-r{gs}-{ge}x{tiles}w{tile_width}-{i}.jpg"))
    };

    let cached: Vec<Vec<u8>> = (0..tiles)
        .map(|i| std::fs::read(tile_path(i)))
        .collect::<std::io::Result<_>>()
        .unwrap_or_default();
    if cached.len() == tiles {
        return Ok(cached.iter().map(|b| jpeg_data_url(b)).collect());
    }

    let step_secs = ((end_s - start_s) / tiles as f64).max(0.033);
    let out_dir = std::env::temp_dir().join(format!("inkue-strip-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&out_dir)?;
    let result = render_frames_into(lib, path, &out_dir, &[
        ("frames", &tiles.to_string()),
        ("start", &format!("{start_s:.3}")),
        ("sstep", &format!("{step_secs:.3}")),
        ("vf", &format!("scale={tile_width}:-2")),
    ]);
    let frames = match result {
        Ok(f) => f,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&out_dir);
            return Err(e);
        }
    };
    let _ = std::fs::remove_dir_all(&out_dir);
    if frames.is_empty() {
        return Err(anyhow!("mpv produced no frames for {}", path.display()));
    }

    let _ = std::fs::create_dir_all(cache_dir());
    for (i, bytes) in frames.iter().enumerate() {
        let _ = std::fs::write(tile_path(i), bytes);
    }
    Ok(frames.iter().map(|b| jpeg_data_url(b)).collect())
}

fn render_one_frame(lib: &MpvLib, path: &Path, seek_into: bool) -> Result<Vec<u8>> {
    // Unique temp dir per request: vo=image names files 00000001.jpg, so
    // concurrent generations must not share an outdir.
    let out_dir = std::env::temp_dir().join(format!("inkue-thumb-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&out_dir)?;
    let result = render_one_frame_into(lib, path, seek_into, &out_dir);
    let _ = std::fs::remove_dir_all(&out_dir);
    result
}

fn render_one_frame_into(
    lib: &MpvLib,
    path: &Path,
    seek_into: bool,
    out_dir: &Path,
) -> Result<Vec<u8>> {
    let scale = format!("scale={THUMB_WIDTH}:-2");
    let mut opts: Vec<(&str, &str)> = vec![("frames", "1"), ("vf", &scale)];
    if seek_into {
        opts.push(("start", "15%"));
    }
    render_frames_into(lib, path, out_dir, &opts)?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("mpv produced no frame for {}", path.display()))
}

/// Run a throwaway `vo=image` mpv over `path` with the given extra options and
/// return the produced JPEGs in playback order.
fn render_frames_into(
    lib: &MpvLib,
    path: &Path,
    out_dir: &Path,
    extra_opts: &[(&str, &str)],
) -> Result<Vec<Vec<u8>>> {
    let cs = |s: &str| CString::new(s).expect("no interior NUL in literal");
    let opt = |ctx: *mut std::ffi::c_void, k: &str, v: &str| {
        let (k, v) = (cs(k), cs(v));
        unsafe { (lib.mpv_set_option_string)(ctx, k.as_ptr(), v.as_ptr()) };
    };

    unsafe {
        let ctx = (lib.mpv_create)();
        if ctx.is_null() {
            return Err(anyhow!("mpv_create() returned null for thumbnail"));
        }

        opt(ctx, "vo", "image");
        opt(ctx, "vo-image-format", "jpg");
        opt(
            ctx,
            "vo-image-outdir",
            &out_dir.to_string_lossy().replace('\\', "/"),
        );
        opt(ctx, "audio", "no");
        opt(ctx, "hwdec", "no");
        opt(ctx, "untimed", "yes");
        for (k, v) in extra_opts {
            opt(ctx, k, v);
        }

        if (lib.mpv_initialize)(ctx) < 0 {
            (lib.mpv_terminate_destroy)(ctx);
            return Err(anyhow!("mpv_initialize() failed for thumbnail"));
        }

        let path_str = path.to_string_lossy().replace('\\', "/");
        let path_cstr = match CString::new(path_str.as_str()) {
            Ok(c) => c,
            Err(_) => {
                (lib.mpv_terminate_destroy)(ctx);
                return Err(anyhow!("path contains NUL byte"));
            }
        };
        let cmd = cs("loadfile");
        let replace = cs("replace");
        let args: [*const std::ffi::c_char; 4] = [
            cmd.as_ptr(),
            path_cstr.as_ptr(),
            replace.as_ptr(),
            std::ptr::null(),
        ];
        (lib.mpv_command)(ctx, args.as_ptr());

        // `frames=N` plays exactly N frames then ends the file — the JPEGs are
        // written before END_FILE fires. Generous deadline: a filmstrip does
        // several keyframe seeks through the file.
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let event = (lib.mpv_wait_event)(ctx, remaining.as_secs_f64().max(0.01));
            if event.is_null() {
                break;
            }
            let id = (*event).event_id;
            if id == MPV_EVENT_END_FILE || id == MPV_EVENT_SHUTDOWN {
                break;
            }
            if Instant::now() >= deadline {
                break;
            }
        }
        (lib.mpv_terminate_destroy)(ctx);
    }

    // vo=image names files 00000001.jpg, 00000002.jpg, … — lexicographic
    // order is playback order.
    let mut produced: Vec<PathBuf> = std::fs::read_dir(out_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|e| e == "jpg").unwrap_or(false))
        .collect();
    produced.sort();
    produced
        .iter()
        .map(|p| std::fs::read(p).map_err(Into::into))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_is_stable_for_unchanged_file() {
        let dir = std::env::temp_dir().join("inkue-thumb-test-stable");
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("a.bin");
        std::fs::write(&f, b"hello").unwrap();
        assert_eq!(cache_key(&f), cache_key(&f));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cache_key_changes_when_size_changes() {
        let dir = std::env::temp_dir().join("inkue-thumb-test-size");
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("a.bin");
        std::fs::write(&f, b"hello").unwrap();
        let k1 = cache_key(&f);
        std::fs::write(&f, b"hello world, longer content").unwrap();
        let k2 = cache_key(&f);
        assert_ne!(k1, k2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cache_key_differs_per_path() {
        let dir = std::env::temp_dir().join("inkue-thumb-test-path");
        let _ = std::fs::create_dir_all(&dir);
        let (fa, fb) = (dir.join("a.bin"), dir.join("b.bin"));
        std::fs::write(&fa, b"same").unwrap();
        std::fs::write(&fb, b"same").unwrap();
        assert_ne!(cache_key(&fa), cache_key(&fb));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cache_key_none_for_missing_file() {
        assert_eq!(cache_key(Path::new("Z:/definitely/not/here.mp4")), None);
    }

    #[test]
    fn jpeg_data_url_encodes_base64() {
        assert_eq!(jpeg_data_url(b"\xff\xd8"), "data:image/jpeg;base64,/9g=");
    }

    #[test]
    fn raw_fallback_rejects_unknown_extensions() {
        assert!(raw_image_fallback(Path::new("C:/x/clip.mp4")).is_none());
        assert!(raw_image_fallback(Path::new("C:/x/track.wav")).is_none());
    }
}
