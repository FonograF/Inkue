// Zone (c) — Zustand store reducer logic (transport).
//
// The store is the frontend's source of truth for what is running and at what
// level. These reducers are pure and easy to get subtly wrong (map identity,
// the OSC-log ring cap, id monotonicity), so they are worth pinning down.

import { describe, it, expect, beforeEach } from "vitest";
import { useTransportStore } from "../transportStore";

function reset() {
  useTransportStore.setState({
    runningCues: new Map(),
    masterPeakL: 0,
    masterPeakR: 0,
    masterVolume: 1.0,
    oscActivityAt: null,
    oscLog: [],
  });
}

beforeEach(reset);

describe("transportStore", () => {
  it("updateCueTime inserts and removeCueTime deletes", () => {
    const s = () => useTransportStore.getState();
    s().updateCueTime({ cueId: "c1", elapsedMs: 100, actionElapsedMs: 50, remainingMs: 200 });
    expect(s().runningCues.get("c1")?.elapsedMs).toBe(100);

    s().removeCueTime("c1");
    expect(s().runningCues.has("c1")).toBe(false);
  });

  it("updateCueTime replaces an existing entry without growing the map", () => {
    const s = () => useTransportStore.getState();
    s().updateCueTime({ cueId: "c1", elapsedMs: 100, actionElapsedMs: 50, remainingMs: 200 });
    s().updateCueTime({ cueId: "c1", elapsedMs: 300, actionElapsedMs: 250, remainingMs: 0 });
    expect(s().runningCues.size).toBe(1);
    expect(s().runningCues.get("c1")?.elapsedMs).toBe(300);
  });

  it("updateCueTime returns a new Map reference (immutability for React)", () => {
    const s = () => useTransportStore.getState();
    const before = s().runningCues;
    s().updateCueTime({ cueId: "c1", elapsedMs: 1, actionElapsedMs: 1, remainingMs: null });
    expect(s().runningCues).not.toBe(before);
  });

  it("setMasterVolume and updateMasterLevels store values", () => {
    const s = () => useTransportStore.getState();
    s().setMasterVolume(0.42);
    s().updateMasterLevels(0.8, 0.6);
    expect(s().masterVolume).toBe(0.42);
    expect(s().masterPeakL).toBe(0.8);
    expect(s().masterPeakR).toBe(0.6);
  });

  it("addOscLog assigns monotonically increasing ids and caps the log at 100", () => {
    const s = () => useTransportStore.getState();
    for (let i = 0; i < 105; i++) {
      s().addOscLog({ ts: "00:00:00.000", addr: `/a/${i}`, args: [] });
    }
    const log = s().oscLog;
    expect(log.length).toBe(100);
    // Latest id is the 105th insertion; the ring kept the last 100 (ids 6..105).
    expect(log[log.length - 1].id).toBe(105);
    expect(log[0].id).toBe(6);
  });

  it("clearOscLog empties the log", () => {
    const s = () => useTransportStore.getState();
    s().addOscLog({ ts: "x", addr: "/a", args: [] });
    s().clearOscLog();
    expect(s().oscLog).toEqual([]);
  });
});
