// Curve evaluation, mirroring `src-tauri/src/cue/curve.rs`.
//
// The editor has to draw exactly what the engine will play, and asking the
// backend to sample on every pointer move would be a round-trip per frame. So
// the maths lives in both places — the same trade the layer compositor makes
// with its GLSL blend formulas. `__tests__/curve.test.ts` asserts the same
// properties the Rust tests do, so the two cannot drift silently.

import type { CurvePoint, CurveShape } from "./types";

/** The control points including the implicit endpoints, ordered and
 *  de-duplicated on `t` so no segment has zero width. */
export function resolvedPoints(shape: CurveShape): CurvePoint[] {
  const points: CurvePoint[] = [{ t: 0, v: 0 }];
  const interior = (shape.points ?? [])
    .filter((p) => p.t > 0 && p.t < 1)
    .sort((a, b) => a.t - b.t);
  for (const point of interior) {
    if (point.t > points[points.length - 1].t) points.push(point);
  }
  points.push({ t: 1, v: 1 });
  return points;
}

/** A power curve about the midpoint: 0 is linear, positive eases in. */
function parametric(t: number, intensity: number): number {
  const k = Math.max(-10, Math.min(10, intensity ?? 0));
  if (Math.abs(k) < 1e-9) return t;
  return k > 0 ? Math.pow(t, 1 + k) : 1 - Math.pow(1 - t, 1 - k);
}

/** How hard a full-scale bend pushes. Mirrors BOW_STRENGTH in curve.rs. */
const BOW_STRENGTH = 4;

/** Warp a segment's local parameter to bow it. 0 leaves it untouched;
 *  positive lifts the segment above its chord, negative sags it below.
 *  Monotone, and still lands exactly on both endpoints. */
export function bow(local: number, bend: number): number {
  if (Math.abs(bend) < 1e-9) return local;
  return parametric(local, -Math.max(-1, Math.min(1, bend)) * BOW_STRENGTH);
}

/** The bend that makes a segment pass through `target` at `local` — the
 *  inverse of `bow`, so Alt-drag puts the curve under the cursor. */
export function bendThrough(local: number, target: number): number {
  const EPSILON = 1e-4;
  const u = Math.max(EPSILON, Math.min(1 - EPSILON, local));
  const v = Math.max(EPSILON, Math.min(1 - EPSILON, target));
  const k =
    v > u
      ? 1 - Math.log(1 - v) / Math.log(1 - u)
      : Math.log(v) / Math.log(u) - 1;
  return Math.max(-1, Math.min(1, -k / BOW_STRENGTH));
}

/** The bow of segment `index`, tolerating a list of the wrong length. */
export function bendFor(shape: CurveShape, index: number): number {
  const bend = shape.bends?.[index] ?? 0;
  return Math.max(-1, Math.min(1, bend));
}

function throughPoints(shape: CurveShape, t: number): number {
  const points = resolvedPoints(shape);
  const index = points.findIndex((p) => p.t >= t);
  if (index <= 0) return points[0].v;
  const left = points[index - 1];
  const right = points[index];
  const span = right.t - left.t;
  if (span <= 0) return right.v;
  const local = bow((t - left.t) / span, bendFor(shape, index - 1));
  return left.v + (right.v - left.v) * local;
}

/** Index of the segment containing `t`, given the resolved point list. */
export function segmentAt(shape: CurveShape, t: number): number {
  const points = resolvedPoints(shape);
  const index = points.findIndex((p) => p.t >= t);
  return Math.max(0, (index <= 0 ? 1 : index) - 1);
}

/** Progress towards the target at normalised time `t`. */
export function sampleCurve(shape: CurveShape, t: number): number {
  const clamped = Math.max(0, Math.min(1, t));
  let value: number;
  switch (shape.kind) {
    case "linear":
      value = throughPoints(shape, clamped);
      break;
    case "s_curve":
      value = clamped * clamped * (3 - 2 * clamped);
      break;
    case "exponential": {
      const K = 5;
      value = Math.expm1(K * clamped) / Math.expm1(K);
      break;
    }
    case "parametric":
      value = parametric(clamped, shape.intensity);
      break;
    default:
      value = clamped;
  }
  return Math.max(0, Math.min(1, value));
}

/** Screen y for a progress value, in a box `height` tall.
 *
 *  Ascending (a rising fade) draws progress directly: it climbs from the
 *  bottom-left to the top-right. `descending` draws the *value* instead —
 *  starting high and falling away — which is how a fade-out actually reads,
 *  and how QLab draws its down shape. Same curve underneath, flipped. */
export function curveY(progress: number, height: number, descending = false): number {
  return descending ? progress * height : height - progress * height;
}

/** An SVG path for the curve, drawn in a `width` × `height` box. */
export function curvePath(
  shape: CurveShape,
  width: number,
  height: number,
  steps = 96,
  descending = false,
): string {
  const parts: string[] = [];
  for (let i = 0; i <= steps; i++) {
    const t = i / steps;
    const x = t * width;
    const y = curveY(sampleCurve(shape, t), height, descending);
    parts.push(`${i === 0 ? "M" : "L"}${x.toFixed(2)},${y.toFixed(2)}`);
  }
  return parts.join(" ");
}
