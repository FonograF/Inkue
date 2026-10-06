import { describe, expect, it } from "vitest";

import {
  TARGETING_PRESETS,
  countSupporting,
  loopCountFor,
  patchFor,
  planBatch,
  playCountOf,
  sharedValue,
  type CueJson,
} from "../batchEdit";
import { PLAY_COUNT_INFINITE } from "../types";

const base = { color: "none", continue_mode: "do_not_continue", is_disabled: false, pre_wait_ms: 0, post_wait_ms: 0 };

const audio: CueJson = {
  ...base, id: "a", cue_type: "audio", volume_db: -6, pan: 0, output_patch_id: null,
  fade_in_ms: null, fade_out_ms: null, loop_count: 0, slices: { markers: [], play_counts: [1] },
};
const video: CueJson = {
  ...base, id: "v", cue_type: "video", volume_db: 0, output_patch_id: null, output_id: null,
  fade_in_ms: null, fade_out_ms: null, video_fade_in_ms: null, video_fade_out_ms: null,
  loop_count: 2, hold_last_frame: false, slices: { markers: [], play_counts: [1] },
  layer_style: { layer: 3, opacity: 1, blend_mode: "normal" },
};
const image: CueJson = {
  ...base, id: "i", cue_type: "image", fade_in_ms: null, fade_out_ms: null, output_id: null,
  layer_style: { layer: null, opacity: 0.5, blend_mode: "screen" },
};
const memo: CueJson = { ...base, id: "m", cue_type: "memo", memo_text: "" };

describe("patchFor", () => {
  it("writes shared fields on every cue type", () => {
    for (const cue of [audio, video, image, memo]) {
      expect(patchFor({ kind: "color", color: "red" }, cue)).toEqual({ color: "red" });
    }
    expect(patchFor({ kind: "disabled", disabled: true }, memo)).toEqual({ is_disabled: true });
  });

  it("skips cues without the field", () => {
    expect(patchFor({ kind: "volume_set", db: -3 }, memo)).toBeNull();
    expect(patchFor({ kind: "pan", pan: 0.5 }, video)).toBeNull();
    expect(patchFor({ kind: "hold_last_frame", hold: true }, image)).toBeNull();
  });

  it("clamps volume and pan to the inspector's ranges", () => {
    expect(patchFor({ kind: "volume_set", db: 40 }, audio)).toEqual({ volume_db: 12 });
    expect(patchFor({ kind: "volume_adjust", deltaDb: -100 }, audio)).toEqual({ volume_db: -60 });
    expect(patchFor({ kind: "pan", pan: -3 }, audio)).toEqual({ pan: -1 });
  });

  it("adjusts volume relative to each cue's own level", () => {
    expect(patchFor({ kind: "volume_adjust", deltaDb: 3 }, audio)).toEqual({ volume_db: -3 });
    expect(patchFor({ kind: "volume_adjust", deltaDb: 3 }, video)).toEqual({ volume_db: 3 });
  });

  it("fades both sound and picture of a video, and an image's picture", () => {
    expect(patchFor({ kind: "fade_in", ms: 2000 }, video)).toEqual({ fade_in_ms: 2000, video_fade_in_ms: 2000 });
    expect(patchFor({ kind: "fade_out", ms: 1500.4 }, image)).toEqual({ fade_out_ms: 1500 });
  });

  it("removes a fade when its length is zero", () => {
    expect(patchFor({ kind: "fade_in", ms: 0 }, audio)).toEqual({ fade_in_ms: null });
  });

  it("keeps the rest of the layer style when changing opacity or blend", () => {
    expect(patchFor({ kind: "opacity", opacity: 0.25 }, video)).toEqual({
      layer_style: { layer: 3, opacity: 0.25, blend_mode: "normal" },
    });
    expect(patchFor({ kind: "blend_mode", mode: "add" }, image)).toEqual({
      layer_style: { layer: null, opacity: 0.5, blend_mode: "add" },
    });
  });

  it("never writes negative or fractional waits", () => {
    expect(patchFor({ kind: "pre_wait", ms: -5 }, audio)).toEqual({ pre_wait_ms: 0 });
    expect(patchFor({ kind: "post_wait", ms: 1250.6 }, audio)).toEqual({ post_wait_ms: 1251 });
  });
});

describe("play counts", () => {
  it("maps total plays to the backend's extra repetitions", () => {
    expect(loopCountFor(1)).toBe(0);
    expect(loopCountFor(4)).toBe(3);
    expect(loopCountFor(0)).toBe(0);
    expect(loopCountFor(PLAY_COUNT_INFINITE)).toBe(PLAY_COUNT_INFINITE);
  });

  it("reads them back", () => {
    expect(playCountOf(audio)).toBe(1);
    expect(playCountOf(video)).toBe(3);
    expect(playCountOf({ ...audio, loop_count: PLAY_COUNT_INFINITE })).toBe(PLAY_COUNT_INFINITE);
  });
});

describe("planBatch", () => {
  it("produces one edit per applicable cue", () => {
    const plan = planBatch({ kind: "volume_adjust", deltaDb: -6 }, [audio, memo, video]);
    expect(plan).toEqual([
      { cue_id: "a", properties: { volume_db: -12 } },
      { cue_id: "v", properties: { volume_db: -6 } },
    ]);
  });

  it("counts supporting cues", () => {
    expect(countSupporting("video_output", [audio, video, image, memo])).toBe(2);
    expect(countSupporting("color", [audio, video, image, memo])).toBe(4);
  });
});

describe("sharedValue", () => {
  it("returns the common value", () => {
    expect(sharedValue("color", [audio, memo], (c) => c.color)).toBe("none");
  });

  it("is undefined when cues differ or none supports the edit", () => {
    expect(sharedValue("volume_set", [audio, video], (c) => c.volume_db)).toBeUndefined();
    expect(sharedValue("pan", [memo], (c) => c.pan)).toBeUndefined();
  });

  it("ignores cues that do not support the edit", () => {
    expect(sharedValue("pan", [audio, memo], (c) => c.pan)).toBe(0);
  });
});

describe("TARGETING_PRESETS", () => {
  const labelsFor = (cues: CueJson[]) =>
    TARGETING_PRESETS.filter((p) => cues.some(p.appliesTo)).map((p) => p.label);

  it("offers fades only for cues with sound or picture", () => {
    expect(labelsFor([memo])).not.toContain("Fade Out");
    expect(labelsFor([image])).toContain("Fade Out & Stop");
  });

  it("offers Devamp only for cues with slices", () => {
    expect(labelsFor([image])).not.toContain("Devamp");
    expect(labelsFor([audio])).toContain("Devamp");
  });
});
