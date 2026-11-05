// Path maths for the Fade Cue's path editor — a mirror of the flattening in
// `src-tauri/src/engine/output_engine/motion.rs` (`CompiledPath::new` and
// `catmull_rom`), so the curve you draw is the curve the engine plays.
// `__tests__/path.test.ts` and motion.rs's `shared_path_vector` pin the same
// numbers: if one side drifts, one of them fails.

import type { PathParam } from "./types";

/** `[x, y]` in path units (for translation: fractions of the output, +Y up). */
export type PathPoint = [number, number];

/** Samples per span of a smoothed path — motion.rs `SPLINE_SAMPLES_PER_SPAN`. */
export const SPLINE_SAMPLES_PER_SPAN = 32;

/** Points closer than this (metric units) are one — motion.rs `POINT_EPSILON`. */
const POINT_EPSILON = 1e-9;

/** How many output pixels (per pixel of output height) one unit of `param`
 *  spans — motion.rs `MotionParam::metric_scale`. */
export function metricScale(param: PathParam, aspect: number): number {
  return param === "translation_x" ? aspect : 1;
}

const distance = (a: PathPoint, b: PathPoint) => Math.hypot(b[0] - a[0], b[1] - a[1]);

/** Centripetal Catmull-Rom (α = 0.5) through `points`, reflected end points,
 *  flattened — motion.rs `catmull_rom`. `points` must have at least 3 entries. */
function catmullRom(points: PathPoint[]): PathPoint[] {
  const n = points.length;
  const reflect = (a: PathPoint, b: PathPoint): PathPoint => [2 * a[0] - b[0], 2 * a[1] - b[1]];
  const controls = [reflect(points[0], points[1]), ...points, reflect(points[n - 1], points[n - 2])];
  const out: PathPoint[] = [];
  for (let span = 0; span < n - 1; span++) {
    const [p0, p1, p2, p3] = controls.slice(span, span + 4);
    const knot = (ti: number, a: PathPoint, b: PathPoint) => ti + Math.sqrt(Math.max(distance(a, b), POINT_EPSILON));
    const t0 = 0;
    const t1 = knot(t0, p0, p1);
    const t2 = knot(t1, p1, p2);
    const t3 = knot(t2, p2, p3);
    for (let k = 0; k < SPLINE_SAMPLES_PER_SPAN; k++) {
      const t = t1 + ((t2 - t1) * k) / SPLINE_SAMPLES_PER_SPAN;
      const lerp = (a: PathPoint, b: PathPoint, ta: number, tb: number): PathPoint => {
        const w = (t - ta) / (tb - ta);
        return [a[0] + (b[0] - a[0]) * w, a[1] + (b[1] - a[1]) * w];
      };
      const a1 = lerp(p0, p1, t0, t1);
      const a2 = lerp(p1, p2, t1, t2);
      const a3 = lerp(p2, p3, t2, t3);
      const b1 = lerp(a1, a2, t0, t2);
      const b2 = lerp(a2, a3, t1, t3);
      out.push(lerp(b1, b2, t1, t2));
    }
  }
  out.push(points[n - 1]);
  return out;
}

/** The path as the engine travels it, as a polyline in path units: the
 *  points measured in output pixels (`aspect` = width / height), repeated
 *  points merged, smoothed when asked. */
export function flattenPath(
  points: PathPoint[],
  smooth: boolean,
  aspect: number,
  xParam: PathParam = "translation_x",
  yParam: PathParam = "translation_y",
): PathPoint[] {
  const sx = metricScale(xParam, aspect);
  const sy = metricScale(yParam, aspect);
  const metric: PathPoint[] = [];
  for (const [x, y] of points) {
    const p: PathPoint = [x * sx, y * sy];
    const last = metric[metric.length - 1];
    if (!last || distance(last, p) > POINT_EPSILON) metric.push(p);
  }
  const vertices = smooth && metric.length > 2 ? catmullRom(metric) : metric;
  return vertices.map(([x, y]) => [x / sx, y / sy]);
}

/** The point of a polyline nearest `p`, as the index of the segment it lies
 *  on (for inserting a point into the path). */
export function nearestSegment(points: PathPoint[], p: PathPoint): number {
  let best = 0;
  let bestDistance = Infinity;
  for (let i = 0; i < points.length - 1; i++) {
    const [a, b] = [points[i], points[i + 1]];
    const dx = b[0] - a[0];
    const dy = b[1] - a[1];
    const lengthSq = dx * dx + dy * dy;
    const u = lengthSq > 0 ? Math.max(0, Math.min(1, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / lengthSq)) : 0;
    const d = distance(p, [a[0] + dx * u, a[1] + dy * u]);
    if (d < bestDistance) {
      bestDistance = d;
      best = i;
    }
  }
  return best;
}
