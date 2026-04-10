// Zustand store: transport state (running cues, levels, master volume).

import { create } from "zustand";
import type { CueId } from "../lib/types";

interface RunningCueTime {
  cueId: CueId;
  elapsedMs: number;
  actionElapsedMs: number;
  remainingMs: number | null;
}


interface TransportState {
  runningCues: Map<CueId, RunningCueTime>;
  masterPeakL: number;
  masterPeakR: number;
  masterVolume: number; // 0.0–1.0

  // Actions
  updateCueTime: (data: RunningCueTime) => void;
  removeCueTime: (cueId: CueId) => void;
  updateMasterLevels: (peakL: number, peakR: number) => void;
  setMasterVolume: (vol: number) => void;
}

export const useTransportStore = create<TransportState>((set) => ({
  runningCues: new Map(),
  masterPeakL: 0,
  masterPeakR: 0,
  masterVolume: 1.0,

  updateCueTime: (data) =>
    set((prev) => {
      const next = new Map(prev.runningCues);
      next.set(data.cueId, data);
      return { runningCues: next };
    }),

  removeCueTime: (cueId) =>
    set((prev) => {
      const next = new Map(prev.runningCues);
      next.delete(cueId);
      return { runningCues: next };
    }),

  updateMasterLevels: (peakL, peakR) =>
    set({ masterPeakL: peakL, masterPeakR: peakR }),

  setMasterVolume: (vol) => set({ masterVolume: vol }),
}));
