// Single source of truth for the media file extensions Inkue accepts.
//
// Used both for open-dialog filters and for detecting a dropped/linked file's
// cue type. Keep this in sync with the decoder features in
// `src-tauri/Cargo.toml` (symphonia) — e.g. AIFF (`aif`/`aiff`) is enabled there.

export const AUDIO_EXTENSIONS = [
  "wav", "mp3", "flac", "ogg", "aac", "m4a", "aif", "aiff", "aifc",
] as const;

export const VIDEO_EXTENSIONS = [
  "mp4", "m4v", "webm", "mov", "mkv", "avi", "ogv",
] as const;

export const IMAGE_EXTENSIONS = [
  "png", "jpg", "jpeg", "gif", "webp", "bmp", "svg",
] as const;

export const AUDIO_EXTS = new Set<string>(AUDIO_EXTENSIONS);
export const VIDEO_EXTS = new Set<string>(VIDEO_EXTENSIONS);
export const IMAGE_EXTS = new Set<string>(IMAGE_EXTENSIONS);

/** Lowercased extension of a path (without the dot), or "" if none. */
export function extensionOf(path: string): string {
  return path.split(".").pop()?.toLowerCase() ?? "";
}
