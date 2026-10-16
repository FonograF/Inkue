// `lib/path.ts` mirrors the path flattening in
// `src-tauri/src/engine/output_engine/motion.rs`, so the path editor draws the
// curve the engine plays. The first test pins the exact numbers motion.rs's
// `shared_path_vector` asserts — if one side drifts, one of them fails.

import { describe, it, expect } from "vitest";
import { flattenPath, nearestSegment } from "../path";

describe("path flattening", () => {
  it("matches the engine on the shared vector", () => {
    const v = flattenPath([[-0.4, -0.3], [0.35, -0.3], [-0.4, 0.3], [0.4, 0.35]], true, 16 / 9);
    expect(v).toHaveLength(97);
    const expected: [number, [number, number]][] = [
      [0, [-0.4, -0.3]],
      [16, [0.06452616993973369, -0.33498517066005307]],
      [48, [-0.02504220802072924, -0.003799651069123458]],
      [80, [-0.09559465867115714, 0.35864230399818736]],
      [96, [0.4, 0.35]],
    ];
    for (const [index, [x, y]] of expected) {
      expect(v[index][0]).toBeCloseTo(x, 12);
      expect(v[index][1]).toBeCloseTo(y, 12);
    }
  });

  it("passes through every point when smoothed", () => {
    const points: [number, number][] = [[-0.3, -0.2], [0, 0.2], [0.3, -0.2]];
    const v = flattenPath(points, true, 16 / 9);
    for (const p of points) {
      const nearest = Math.min(...v.map((q) => Math.hypot(q[0] - p[0], q[1] - p[1])));
      expect(nearest).toBeLessThan(1e-9);
    }
  });

  it("keeps straight segments when not smoothed, merging repeats", () => {
    expect(flattenPath([[0, 0], [0.1, 0], [0.1, 0], [0.2, 0.1]], false, 16 / 9)).toEqual([
      [0, 0], [0.1, 0], [0.2, 0.1],
    ]);
  });

  it("two points stay a straight line even when smoothed", () => {
    expect(flattenPath([[0, 0], [0.3, 0]], true, 16 / 9)).toEqual([[0, 0], [0.3, 0]]);
  });
});

describe("nearest segment", () => {
  it("finds the span a click lands on", () => {
    const points: [number, number][] = [[0, 0], [1, 0], [1, 1]];
    expect(nearestSegment(points, [0.5, 0.05])).toBe(0);
    expect(nearestSegment(points, [0.95, 0.6])).toBe(1);
  });
});
