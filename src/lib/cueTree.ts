// Walking the cue tree the backend sends: a Group summary carries its
// children, so "does this target show a picture?" has to look inside it.

import type { CueSummary, CueType } from "./types";

/** Cue types that put a picture on a video output layer — what a crossfade
 *  dissolves away and what it can dissolve into. */
export const PICTURE_CUE_TYPES: readonly CueType[] = ["video", "image", "camera"];

/** Every cue of the tree, depth-first, each group before its children. */
export function flattenCues(cues: CueSummary[]): CueSummary[] {
  return cues.flatMap((c) => [c, ...(c.children ? flattenCues(c.children) : [])]);
}

/** True when the cue shows a picture itself or, for a group, through any of
 *  its descendants. */
export function showsPicture(cue: CueSummary): boolean {
  return PICTURE_CUE_TYPES.includes(cue.cue_type) || (cue.children ?? []).some(showsPicture);
}

/** True when the cue makes sound a Fade can drive: an audio or video cue
 *  (video carries a sound track), a live input, or a group holding one. */
export function makesSound(cue: CueSummary): boolean {
  return (
    cue.cue_type === "audio" ||
    cue.cue_type === "video" ||
    cue.cue_type === "mic" ||
    (cue.children ?? []).some(makesSound)
  );
}
