// The span a playing cue's progress bar covers, and the conversions between a
// point on that bar and a seek position — shared by every scrubber so the
// cue list, the Active panel and the inspector agree on where a cue is.

import type { CueSummary } from "./types";

export interface CueTimeline {
  /** Length of the bar in ms: one loop iteration when looping, else the whole cue. */
  lengthMs: number;
  /** The bar restarts at every iteration. */
  looping: boolean;
}

/** The cue's bar, or null when it has no known length. */
export function cueTimeline(cue: Pick<CueSummary, "duration_ms" | "file_duration_ms">): CueTimeline | null {
  const fileMs = cue.file_duration_ms;
  const totalMs = cue.duration_ms;
  // Infinite loop (no total) or a total longer than one pass of the file.
  const looping = fileMs != null && (totalMs == null || fileMs < totalMs);
  const lengthMs = looping ? fileMs : totalMs;
  if (lengthMs == null || lengthMs <= 0) return null;
  return { lengthMs, looping };
}

/** Where `elapsedMs` of action time sits on the bar, in ms. */
export function timelinePosition(timeline: CueTimeline, elapsedMs: number): number {
  const elapsed = Math.max(0, elapsedMs);
  return timeline.looping ? elapsed % timeline.lengthMs : Math.min(elapsed, timeline.lengthMs);
}

/** The seek position under a point `fraction` (0–1) along the bar. */
export function msAtFraction(timeline: CueTimeline, fraction: number): number {
  return Math.round(Math.min(1, Math.max(0, fraction)) * timeline.lengthMs);
}

/** `m:ss` — the readout next to a scrubber. */
export function formatClock(ms: number): string {
  const totalSeconds = Math.floor(Math.max(0, ms) / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  return `${minutes}:${String(totalSeconds % 60).padStart(2, "0")}`;
}
