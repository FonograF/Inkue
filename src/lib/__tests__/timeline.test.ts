import { describe, expect, it } from "vitest";

import { cueTimeline, formatClock, msAtFraction, timelinePosition } from "../timeline";

describe("cueTimeline", () => {
  it("covers the whole cue when it plays once", () => {
    expect(cueTimeline({ duration_ms: 46_600, file_duration_ms: 46_600 })).toEqual({ lengthMs: 46_600, looping: false });
  });

  it("covers the trimmed length, not the file, when trimmed", () => {
    expect(cueTimeline({ duration_ms: 30_000, file_duration_ms: 46_600 })).toEqual({ lengthMs: 30_000, looping: false });
  });

  it("covers one pass of the file when looping a set number of times", () => {
    expect(cueTimeline({ duration_ms: 30_000, file_duration_ms: 10_000 })).toEqual({ lengthMs: 10_000, looping: true });
  });

  it("covers one pass of the file when looping forever", () => {
    expect(cueTimeline({ duration_ms: null, file_duration_ms: 10_000 })).toEqual({ lengthMs: 10_000, looping: true });
  });

  it("has no bar without a known length", () => {
    expect(cueTimeline({ duration_ms: null, file_duration_ms: null })).toBeNull();
    expect(cueTimeline({ duration_ms: 0, file_duration_ms: null })).toBeNull();
  });
});

describe("timelinePosition", () => {
  it("wraps each loop pass", () => {
    expect(timelinePosition({ lengthMs: 10_000, looping: true }, 23_500)).toBe(3_500);
  });

  it("stops at the end of a single pass", () => {
    expect(timelinePosition({ lengthMs: 10_000, looping: false }, 12_000)).toBe(10_000);
  });

  it("never goes negative", () => {
    expect(timelinePosition({ lengthMs: 10_000, looping: false }, -50)).toBe(0);
  });
});

describe("msAtFraction", () => {
  const timeline = { lengthMs: 192_000, looping: false };

  it("maps a point on the bar to a position in the cue", () => {
    expect(msAtFraction(timeline, 0.25)).toBe(48_000);
  });

  it("clamps points dragged past either end", () => {
    expect(msAtFraction(timeline, -0.3)).toBe(0);
    expect(msAtFraction(timeline, 1.7)).toBe(192_000);
  });
});

describe("formatClock", () => {
  it("prints minutes and seconds", () => {
    expect(formatClock(183_900)).toBe("3:03");
    expect(formatClock(0)).toBe("0:00");
  });
});
