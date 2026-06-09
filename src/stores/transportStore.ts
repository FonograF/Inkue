// Zustand store: transport state (running cues, levels, master volume).

import { create } from "zustand";
import type { CueId } from "../lib/types";

export interface OscLogEntry {
  id: number;
  ts: string;       // HH:MM:SS.mmm
  addr: string;
  args: string[];
}

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
  /** performance.now() timestamp of the last OSC activity, or null. */
  oscActivityAt: number | null;
  /** Rolling log of received OSC messages (max 100 entries). */
  oscLog: OscLogEntry[];

  // Actions
  updateCueTime: (data: RunningCueTime) => void;
  removeCueTime: (cueId: CueId) => void;
  updateMasterLevels: (peakL: number, peakR: number) => void;
  setMasterVolume: (vol: number) => void;
  markOscActivity: () => void;
  addOscLog: (entry: Omit<OscLogEntry, "id">) => void;
  clearOscLog: () => void;
}

export const useTransportStore = create<TransportState>((set) => ({
  runningCues: new Map(),
  masterPeakL: 0,
  masterPeakR: 0,
  masterVolume: 1.0,
  oscActivityAt: null,
  oscLog: [],

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

  markOscActivity: () => set({ oscActivityAt: performance.now() }),

  addOscLog: (entry) =>
    set((prev) => {
      const id = (prev.oscLog[prev.oscLog.length - 1]?.id ?? 0) + 1;
      const next = [...prev.oscLog, { ...entry, id }];
      return { oscLog: next.length > 100 ? next.slice(-100) : next };
    }),

  clearOscLog: () => set({ oscLog: [] }),
}));
