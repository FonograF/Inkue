import { describe, it, expect } from "vitest";
import { flattenCues, makesSound, showsPicture } from "../cueTree";
import type { CueSummary, CueType } from "../types";

const cue = (id: string, cue_type: CueType, children?: CueSummary[]): CueSummary => ({
  id,
  cue_type,
  name: id,
  number: null,
  notes: "",
  state: "standby",
  continue_mode: "do_not_continue",
  color: "none",
  pre_wait_ms: 0,
  post_wait_ms: 0,
  duration_ms: null,
  file_path: null,
  is_loading: false,
  is_disabled: false,
  is_broken: false,
  is_warning: false,
  file_duration_ms: null,
  children,
});

describe("flattenCues", () => {
  it("lists nested cues after their group", () => {
    const tree = [cue("g", "group", [cue("a", "audio"), cue("h", "group", [cue("v", "video")])]), cue("m", "memo")];
    expect(flattenCues(tree).map((c) => c.id)).toEqual(["g", "a", "h", "v", "m"]);
  });
});

describe("showsPicture", () => {
  it("is true for video, image and camera cues", () => {
    for (const type of ["video", "image", "camera"] as CueType[]) {
      expect(showsPicture(cue("x", type))).toBe(true);
    }
  });

  it("is false for a cue with no picture", () => {
    expect(showsPicture(cue("a", "audio"))).toBe(false);
  });

  it("finds a picture deep inside a group", () => {
    expect(showsPicture(cue("g", "group", [cue("h", "group", [cue("i", "image")])]))).toBe(true);
  });

  it("is false for a group of sounds", () => {
    expect(showsPicture(cue("g", "group", [cue("a", "audio")]))).toBe(false);
  });
});

describe("makesSound", () => {
  it("counts a video's sound track", () => {
    expect(makesSound(cue("v", "video"))).toBe(true);
  });

  it("is false for an image", () => {
    expect(makesSound(cue("i", "image"))).toBe(false);
  });

  it("finds sound inside a group", () => {
    expect(makesSound(cue("g", "group", [cue("a", "audio")]))).toBe(true);
  });
});
