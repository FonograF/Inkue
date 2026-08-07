// The TS curve maths mirrors `src-tauri/src/cue/curve.rs` so the editor draws
// exactly what the engine plays. These assert the same properties the Rust
// tests do — if one side drifts, one of these fails.

import { describe, it, expect } from "vitest";
import { sampleCurve, resolvedPoints, curvePath, curveY, bow, bendThrough, segmentAt } from "../curve";
import type { CurveKind, CurveShape } from "../types";

const shape = (kind: CurveKind, extra: Partial<CurveShape> = {}): CurveShape => ({
  kind,
  intensity: 0,
  points: [],
  bends: [],
  ...extra,
});

const ALL_KINDS: CurveKind[] = ["linear", "s_curve", "exponential", "parametric"];

describe("curve evaluation", () => {
  it("every kind starts at 0 and reaches 1", () => {
    for (const kind of ALL_KINDS) {
      const s = shape(kind, { intensity: 3, points: [{ t: 0.4, v: 0.8 }] });
      expect(sampleCurve(s, 0)).toBeCloseTo(0, 6);
      expect(sampleCurve(s, 1)).toBeCloseTo(1, 6);
    }
  });

  it("linear is the identity", () => {
    expect(sampleCurve(shape("linear"), 0.25)).toBeCloseTo(0.25, 6);
  });

  it("s-curve is symmetric about the midpoint", () => {
    const s = shape("s_curve");
    expect(sampleCurve(s, 0.5)).toBeCloseTo(0.5, 6);
    expect(sampleCurve(s, 0.25) + sampleCurve(s, 0.75)).toBeCloseTo(1, 6);
  });

  it("parametric intensity leans the curve each way", () => {
    expect(sampleCurve(shape("parametric", { intensity: 0 }), 0.5)).toBeCloseTo(0.5, 6);
    expect(sampleCurve(shape("parametric", { intensity: 3 }), 0.5)).toBeLessThan(0.5);
    expect(sampleCurve(shape("parametric", { intensity: -3 }), 0.5)).toBeGreaterThan(0.5);
  });

  it("passes exactly through a control point", () => {
    const s = shape("linear", { points: [{ t: 0.3, v: 0.9 }] });
    expect(sampleCurve(s, 0.3)).toBeCloseTo(0.9, 6);
  });

  it("a new point gives straight segments, not an invented curve", () => {
    // Adding a point asks for a corner. Shaping is Alt's job.
    const s = shape("linear", { points: [{ t: 0.5, v: 0.25 }] });
    expect(sampleCurve(s, 0.25)).toBeCloseTo(0.125, 6);
    expect(sampleCurve(s, 0.375)).toBeCloseTo(0.1875, 6);
    expect(sampleCurve(s, 0.75)).toBeCloseTo(0.625, 6);
  });

  it("linear points make straight segments", () => {
    const s = shape("linear", { points: [{ t: 0.5, v: 0.25 }] });
    expect(sampleCurve(s, 0.25)).toBeCloseTo(0.125, 6);
    expect(sampleCurve(s, 0.75)).toBeCloseTo(0.625, 6);
  });

  it("orders points and keeps the endpoints unremovable", () => {
    const s = shape("linear", {
      points: [
        { t: 0.75, v: 0.9 },
        { t: 0, v: 0.5 },
        { t: 0.25, v: 0.1 },
        { t: 1, v: 0.5 },
      ],
    });
    expect(resolvedPoints(s).map((p) => p.t)).toEqual([0, 0.25, 0.75, 1]);
    expect(resolvedPoints(s)[0].v).toBe(0);
    expect(resolvedPoints(s)[3].v).toBe(1);
  });
});

describe("bowed segments (Alt-drag)", () => {
  it("a bow lifts the segment without moving its ends", () => {
    const s = shape("linear", { bends: [0.5] });
    expect(sampleCurve(s, 0)).toBeCloseTo(0, 6);
    expect(sampleCurve(s, 1)).toBeCloseTo(1, 6);
    expect(sampleCurve(s, 0.5)).toBeGreaterThan(0.5);
  });

  it("a negative bow sags the segment", () => {
    expect(sampleCurve(shape("linear", { bends: [-0.5] }), 0.5)).toBeLessThan(0.5);
  });

  it("a bowed segment still lands on both control points", () => {
    const s = shape("linear", { points: [{ t: 0.4, v: 0.7 }], bends: [0.8, -0.8] });
    expect(sampleCurve(s, 0.4)).toBeCloseTo(0.7, 6);
    expect(sampleCurve(s, 0)).toBeCloseTo(0, 6);
    expect(sampleCurve(s, 1)).toBeCloseTo(1, 6);
  });

  it("a bowed segment never leaves its endpoints range", () => {
    for (const bend of [-1, -0.6, 0.6, 1]) {
      const s = shape("linear", { points: [{ t: 0.5, v: 0.5 }], bends: [bend, bend] });
      for (let i = 0; i <= 200; i++) {
        const v = sampleCurve(s, i / 200);
        expect(v).toBeGreaterThanOrEqual(0);
        expect(v).toBeLessThanOrEqual(1);
      }
    }
  });

  it("dragging puts the curve under the cursor", () => {
    for (const [local, target] of [[0.5, 0.8], [0.25, 0.1], [0.7, 0.9], [0.5, 0.2]]) {
      const s = shape("linear", { bends: [bendThrough(local, target)] });
      expect(sampleCurve(s, local)).toBeCloseTo(target, 1);
    }
  });

  it("an unbowed drag is the identity", () => {
    expect(bendThrough(0.5, 0.5)).toBeCloseTo(0, 6);
    expect(bow(0.37, 0)).toBeCloseTo(0.37, 6);
  });

  it("a short or absent bend list is harmless", () => {
    const s = shape("linear", { points: [{ t: 0.5, v: 0.5 }], bends: [] });
    expect(sampleCurve(s, 0.5)).toBeCloseTo(0.5, 6);
  });

  it("finds the segment a click landed in", () => {
    const s = shape("linear", { points: [{ t: 0.4, v: 0.5 }] });
    expect(segmentAt(s, 0.2)).toBe(0);
    expect(segmentAt(s, 0.7)).toBe(1);
  });
});

describe("curve drawing", () => {
  it("a rising curve climbs from bottom-left to top-right", () => {
    const path = curvePath(shape("linear"), 100, 50, 4);
    expect(path.startsWith("M0.00,50.00")).toBe(true);
    expect(path.endsWith("L100.00,0.00")).toBe(true);
  });

  it("a falling curve starts high and drops away", () => {
    // A fade-out reads as the value falling, not as progress climbing —
    // otherwise the Falling panel looks identical to the Rising one.
    const path = curvePath(shape("linear"), 100, 50, 4, true);
    expect(path.startsWith("M0.00,0.00")).toBe(true);
    expect(path.endsWith("L100.00,50.00")).toBe(true);
  });

  it("both directions place the same progress on opposite sides", () => {
    expect(curveY(0, 50)).toBe(50);
    expect(curveY(1, 50)).toBe(0);
    expect(curveY(0, 50, true)).toBe(0);
    expect(curveY(1, 50, true)).toBe(50);
  });
});
