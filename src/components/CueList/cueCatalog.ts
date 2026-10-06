// Cue-list catalogues shared by the list view and its context menu: the
// creatable cue types, and the media file types a cue can be given.

import type { CueType } from "../../lib/types";
import { CUE_TYPE_COLORS } from "../../lib/types";
import { AUDIO_EXTS, VIDEO_EXTS, IMAGE_EXTS, MIDI_EXTS, extensionOf } from "../../lib/mediaTypes";
import { setAudioFile, setImageFile, setMidiFile, setVideoFile } from "../../lib/commands";

// Every cue type that can be created, in toolbar order. Colors come from the
// shared CUE_TYPE_COLORS map so the context menu and the Row 2 toolbar buttons
// in App.tsx never drift apart. Adding a new cue type needs one entry here.
export const CUE_TYPES: { type: CueType; label: string; color: string }[] = (
  [
    { type: "audio",    label: "Audio" },
    { type: "video",    label: "Video" },
    { type: "image",    label: "Image" },
    { type: "stop",     label: "Stop" },
    { type: "fade",     label: "Fade" },
    { type: "wait",     label: "Wait" },
    { type: "group",    label: "Group" },
    { type: "midi",     label: "MIDI" },
    { type: "midi_file", label: "MIDI File" },
    { type: "osc",      label: "OSC" },
    { type: "light",    label: "Light" },
    { type: "mic",      label: "Mic" },
    { type: "timecode", label: "Timecode" },
    { type: "text",     label: "Text" },
    { type: "memo",     label: "Memo" },
    // Command cues last: the toolbar groups them behind one button, but the
    // right-click "Add Cue" list is where you look when you want a specific
    // one, so they are spelled out here.
    { type: "start",    label: "Start" },
    { type: "pause",    label: "Pause" },
    { type: "resume",   label: "Resume" },
    { type: "load",     label: "Load" },
    { type: "reset",    label: "Reset" },
    { type: "goto",     label: "Goto" },
    { type: "arm",      label: "Arm" },
    { type: "disarm",   label: "Disarm" },
    { type: "script",   label: "Script" },
  ] as { type: CueType; label: string }[]
).map((c) => ({ ...c, color: CUE_TYPE_COLORS[c.type] }));

// Cue types that hold a media file, with the open-dialog filter for each.
export const FILE_FILTERS: Partial<Record<CueType, { name: string; extensions: string[] }>> = {
  audio: { name: "Audio Files", extensions: [...AUDIO_EXTS] },
  video: { name: "Video Files", extensions: [...VIDEO_EXTS] },
  image: { name: "Image Files", extensions: [...IMAGE_EXTS] },
  midi_file: { name: "MIDI Files", extensions: [...MIDI_EXTS] },
};

/** Cue types that own a file, as far as drop and "Assign … File…" go. */
export type MediaCueType = "audio" | "video" | "image" | "midi_file";

export const ASSIGN_FILE_LABELS: Record<MediaCueType, string> = {
  audio: "Audio",
  video: "Video",
  image: "Image",
  midi_file: "MIDI",
};

function isAudioPath(p: string) {
  return AUDIO_EXTS.has(extensionOf(p));
}
function isVideoPath(p: string) {
  return VIDEO_EXTS.has(extensionOf(p));
}
function isImagePath(p: string) {
  return IMAGE_EXTS.has(extensionOf(p));
}
function isMidiPath(p: string) {
  return MIDI_EXTS.has(extensionOf(p));
}
export function isMediaPath(p: string) {
  return isAudioPath(p) || isVideoPath(p) || isImagePath(p) || isMidiPath(p);
}
export function cueTypeForPath(p: string): MediaCueType {
  if (isVideoPath(p)) return "video";
  if (isImagePath(p)) return "image";
  if (isMidiPath(p)) return "midi_file";
  return "audio";
}
export async function setFileForCue(cueType: MediaCueType, cueId: string, path: string) {
  if (cueType === "video") await setVideoFile(cueId, path);
  else if (cueType === "image") await setImageFile(cueId, path);
  else if (cueType === "midi_file") await setMidiFile(cueId, path);
  else await setAudioFile(cueId, path);
}
export function basenameNoExt(p: string) {
  return (p.split(/[\\/]/).pop() ?? p).replace(/\.[^.]+$/, "");
}
