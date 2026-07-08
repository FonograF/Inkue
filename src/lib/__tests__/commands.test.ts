// Zone (c) — command wrapper argument-shape contract.
//
// Mocks the Tauri invoke bridge and asserts each wrapper forwards the exact
// command name and argument object the Rust side expects — including the
// non-obvious normalisation logic (default positions, null coalescing, rounding)
// that could silently drift and send a malformed payload.

import { describe, it, expect, beforeEach, vi } from "vitest";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));

import * as cmd from "../commands";

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
});

describe("command wrappers forward the correct name + args", () => {
  it("goCue → go_cue { cueId }", async () => {
    await cmd.goCue("cue-1");
    expect(invokeMock).toHaveBeenCalledWith("go_cue", { cueId: "cue-1" });
  });

  it("seekCue → seek_cue { cueId, positionMs }", async () => {
    await cmd.seekCue("cue-1", 1234);
    expect(invokeMock).toHaveBeenCalledWith("seek_cue", { cueId: "cue-1", positionMs: 1234 });
  });

  it("addCue defaults position to -1 (append)", async () => {
    await cmd.addCue("audio");
    expect(invokeMock).toHaveBeenCalledWith("add_cue", { cueType: "audio", position: -1 });
  });

  it("moveCues forwards a null beforeId verbatim", async () => {
    await cmd.moveCues(["a", "b"], null);
    expect(invokeMock).toHaveBeenCalledWith("move_cues", { ids: ["a", "b"], beforeId: null });
  });

  it("pasteCue with no arg coalesces to afterCueId: null", async () => {
    await cmd.pasteCue();
    expect(invokeMock).toHaveBeenCalledWith("paste_cue", { afterCueId: null });
  });

  it("updateCue nests the partial properties object", async () => {
    await cmd.updateCue("cue-1", { volume_db: -6, pan: 0.5 } as never);
    expect(invokeMock).toHaveBeenCalledWith("update_cue", {
      cueId: "cue-1",
      properties: { volume_db: -6, pan: 0.5 },
    });
  });

  it("previewCue rounds fractional millisecond markers to integers", async () => {
    await cmd.previewCue("cue-1", 12.7, 40.2);
    expect(invokeMock).toHaveBeenCalledWith("preview_cue", {
      cueId: "cue-1",
      startMs: 13,
      endMs: 40,
    });
  });

  it("previewCue passes null markers when omitted", async () => {
    await cmd.previewCue("cue-1");
    expect(invokeMock).toHaveBeenCalledWith("preview_cue", {
      cueId: "cue-1",
      startMs: null,
      endMs: null,
    });
  });

  it("listAudioDevices coalesces an omitted backend to null", async () => {
    invokeMock.mockResolvedValue([]);
    await cmd.listAudioDevices();
    expect(invokeMock).toHaveBeenCalledWith("list_audio_devices", { backend: null });
  });

  it("setAudioFile → set_audio_file { cueId, filePath }", async () => {
    await cmd.setAudioFile("cue-1", "audio/track.wav");
    expect(invokeMock).toHaveBeenCalledWith("set_audio_file", {
      cueId: "cue-1",
      filePath: "audio/track.wav",
    });
  });

  it("setOutputPatch forwards the full patch definition", async () => {
    invokeMock.mockResolvedValue("patch-id");
    await cmd.setOutputPatch(null, "Main", "dev-1", [0, 1]);
    expect(invokeMock).toHaveBeenCalledWith("set_output_patch", {
      patchId: null,
      name: "Main",
      deviceId: "dev-1",
      channels: [0, 1],
    });
  });
});
