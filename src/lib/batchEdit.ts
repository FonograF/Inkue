// Multi-cue editing: turns one operator intent ("give these cues a 2 s fade
// in", "+3 dB") into a per-cue property patch.
//
// Applicability is read from each cue's own serialised JSON rather than from a
// table of cue types: a cue takes part in an edit exactly when it has the
// field, so a new cue type joins the right edits without touching this file.

import type { CueEdit, CueProperties } from "./commands";
import type { BlendMode, ContinueMode, CueColor, CueType, LayerStyle } from "./types";
import { DEFAULT_LAYER_STYLE, PLAY_COUNT_INFINITE } from "./types";

/** A cue as `get_cue` returns it: its full serialised form. */
export type CueJson = Record<string, unknown> & { id: string };

export const VOLUME_MIN_DB = -60;
export const VOLUME_MAX_DB = 12;

export type BatchEdit =
  | { kind: "color"; color: CueColor }
  | { kind: "continue_mode"; mode: ContinueMode }
  | { kind: "disabled"; disabled: boolean }
  | { kind: "pre_wait"; ms: number }
  | { kind: "post_wait"; ms: number }
  | { kind: "volume_set"; db: number }
  | { kind: "volume_adjust"; deltaDb: number }
  | { kind: "pan"; pan: number }
  | { kind: "output_patch"; patchId: string | null }
  | { kind: "fade_in"; ms: number }
  | { kind: "fade_out"; ms: number }
  | { kind: "play_count"; count: number }
  | { kind: "video_output"; outputId: string | null }
  | { kind: "opacity"; opacity: number }
  | { kind: "blend_mode"; mode: BlendMode }
  | { kind: "hold_last_frame"; hold: boolean };

export type BatchEditKind = BatchEdit["kind"];

/** Fields each kind of edit writes; a cue takes part when it has any of them. */
const FIELDS: Record<BatchEditKind, string[]> = {
  color: ["color"],
  continue_mode: ["continue_mode"],
  disabled: ["is_disabled"],
  pre_wait: ["pre_wait_ms"],
  post_wait: ["post_wait_ms"],
  volume_set: ["volume_db"],
  volume_adjust: ["volume_db"],
  pan: ["pan"],
  output_patch: ["output_patch_id"],
  // A Video Cue has both a sound fade and a picture fade; an Image Cue's
  // fade_in_ms is its picture fade. "Fade in" means all of them.
  fade_in: ["fade_in_ms", "video_fade_in_ms"],
  fade_out: ["fade_out_ms", "video_fade_out_ms"],
  play_count: ["loop_count"],
  video_output: ["output_id"],
  opacity: ["layer_style"],
  blend_mode: ["layer_style"],
  hold_last_frame: ["hold_last_frame"],
};

/** `true` when `cue` has a field `kind` writes. */
export function supports(kind: BatchEditKind, cue: CueJson): boolean {
  return FIELDS[kind].some((field) => field in cue);
}

/** How many of `cues` an edit of `kind` would touch. */
export function countSupporting(kind: BatchEditKind, cues: CueJson[]): number {
  return cues.filter((cue) => supports(kind, cue)).length;
}

/** The property patch `edit` makes on `cue`, or null when it does not apply. */
export function patchFor(edit: BatchEdit, cue: CueJson): CueProperties | null {
  if (!supports(edit.kind, cue)) return null;
  const fill = (value: unknown): CueProperties =>
    Object.fromEntries(FIELDS[edit.kind].filter((field) => field in cue).map((field) => [field, value]));

  switch (edit.kind) {
    case "color": return fill(edit.color);
    case "continue_mode": return fill(edit.mode);
    case "disabled": return fill(edit.disabled);
    case "pre_wait": return fill(wholeMs(edit.ms));
    case "post_wait": return fill(wholeMs(edit.ms));
    case "volume_set": return fill(clampVolume(edit.db));
    case "volume_adjust": return adjustVolume(cue, edit.deltaDb);
    case "pan": return fill(clamp(edit.pan, -1, 1));
    case "output_patch": return fill(edit.patchId);
    // No fade at all is `null`; a zero-length fade is not a fade.
    case "fade_in": return fill(edit.ms > 0 ? wholeMs(edit.ms) : null);
    case "fade_out": return fill(edit.ms > 0 ? wholeMs(edit.ms) : null);
    case "play_count": return fill(loopCountFor(edit.count));
    case "video_output": return fill(edit.outputId);
    case "opacity": return { layer_style: { ...layerStyleOf(cue), opacity: clamp(edit.opacity, 0, 1) } };
    case "blend_mode": return { layer_style: { ...layerStyleOf(cue), blend_mode: edit.mode } };
    case "hold_last_frame": return fill(edit.hold);
  }
}

/** One `CueEdit` per cue the edit applies to. */
export function planBatch(edit: BatchEdit, cues: CueJson[]): CueEdit[] {
  return cues.flatMap((cue) => {
    const properties = patchFor(edit, cue);
    return properties ? [{ cue_id: cue.id, properties }] : [];
  });
}

/**
 * The value every supporting cue shares for `read`, or undefined when they
 * differ (or none supports the edit) — drives the menu's check marks and the
 * dialogs' starting values.
 */
export function sharedValue<T>(
  kind: BatchEditKind,
  cues: CueJson[],
  read: (cue: CueJson) => T,
): T | undefined {
  const values = cues.filter((cue) => supports(kind, cue)).map(read);
  if (values.length === 0) return undefined;
  const first = JSON.stringify(values[0]);
  return values.every((v) => JSON.stringify(v) === first) ? values[0] : undefined;
}

/** Total plays (1 = once, PLAY_COUNT_INFINITE = forever) of a media cue. */
export function playCountOf(cue: CueJson): number {
  const loops = typeof cue.loop_count === "number" ? cue.loop_count : 0;
  return loops >= PLAY_COUNT_INFINITE ? PLAY_COUNT_INFINITE : loops + 1;
}

/** The backend's `loop_count` (extra repetitions) for a total play count. */
export function loopCountFor(count: number): number {
  if (count >= PLAY_COUNT_INFINITE) return PLAY_COUNT_INFINITE;
  return Math.max(0, Math.round(count) - 1);
}

export function layerStyleOf(cue: CueJson): LayerStyle {
  return { ...DEFAULT_LAYER_STYLE, ...((cue.layer_style as Partial<LayerStyle> | undefined) ?? {}) };
}

/** Cue types that can be created aimed at the selection, with their presets. */
export interface TargetingPreset {
  label: string;
  cueType: CueType;
  properties?: CueProperties;
  /** Shown only when at least one selected cue passes this test. */
  appliesTo: (cue: CueJson) => boolean;
}

const makesOutput = (cue: CueJson) =>
  "volume_db" in cue || "layer_style" in cue || cue.cue_type === "group";
const hasSlices = (cue: CueJson) => "slices" in cue;
const anyCue = () => true;

export const TARGETING_PRESETS: TargetingPreset[] = [
  { label: "Fade Out & Stop", cueType: "fade", properties: { name: "Fade Out & Stop", stop_at_end: true }, appliesTo: makesOutput },
  { label: "Fade Out", cueType: "fade", properties: { name: "Fade Out" }, appliesTo: makesOutput },
  { label: "Stop", cueType: "stop", appliesTo: anyCue },
  { label: "Devamp", cueType: "devamp", appliesTo: hasSlices },
  { label: "Start", cueType: "start", appliesTo: anyCue },
  { label: "Pause", cueType: "pause", appliesTo: anyCue },
  { label: "Resume", cueType: "resume", appliesTo: anyCue },
  { label: "Load", cueType: "load", appliesTo: anyCue },
  { label: "Reset", cueType: "reset", appliesTo: anyCue },
  { label: "Arm", cueType: "arm", appliesTo: anyCue },
  { label: "Disarm", cueType: "disarm", appliesTo: anyCue },
];

function adjustVolume(cue: CueJson, deltaDb: number): CueProperties | null {
  if (typeof cue.volume_db !== "number") return null;
  return { volume_db: clampVolume(cue.volume_db + deltaDb) };
}

function clampVolume(db: number): number {
  return clamp(db, VOLUME_MIN_DB, VOLUME_MAX_DB);
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function wholeMs(ms: number): number {
  return Math.max(0, Math.round(ms));
}
