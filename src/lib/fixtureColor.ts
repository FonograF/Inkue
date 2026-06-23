// Shared helpers for fixture colour mixing (Dashboard + Light Cue inspector).

const clamp01 = (v: number) => Math.min(1, Math.max(0, v));
const to255 = (v: number) => Math.round(clamp01(v) * 255);
const hex2 = (n: number) => n.toString(16).padStart(2, "0");

/** Normalised r,g,b (0–1) → "#rrggbb". */
export function rgbToHex(r: number, g: number, b: number): string {
  return `#${hex2(to255(r))}${hex2(to255(g))}${hex2(to255(b))}`;
}

/** "#rrggbb" → normalised [r, g, b] (0–1). */
export function hexToRgb(hex: string): [number, number, number] {
  const n = parseInt(hex.slice(1), 16);
  return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255];
}

/** Index of the first parameter of a given kind, or -1. */
export function paramIndexOfKind(params: { kind: string }[], kind: string): number {
  return params.findIndex((p) => p.kind === kind);
}
