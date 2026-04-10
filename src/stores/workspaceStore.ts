// Zustand store: workspace data, cue list, selection, and playhead.

import { create } from "zustand";
import type { CueId, CueSummary, WorkspaceInfo } from "../lib/types";
import { getAllCues, getPlayhead, getWorkspaceInfo } from "../lib/commands";

interface WorkspaceState {
  cues: CueSummary[];
  selectedCueId: CueId | null;
  playheadCueId: CueId | null;
  workspaceInfo: WorkspaceInfo | null;

  // Actions
  refreshCues: () => Promise<void>;
  refreshWorkspaceInfo: () => Promise<void>;
  setSelectedCueId: (id: CueId | null) => void;
  setPlayheadCueId: (id: CueId | null) => void;
  updateCueState: (cueId: CueId, state: CueSummary["state"]) => void;
}

export const useWorkspaceStore = create<WorkspaceState>((set, _get) => ({
  cues: [],
  selectedCueId: null,
  playheadCueId: null,
  workspaceInfo: null,

  refreshCues: async () => {
    try {
      const cues = await getAllCues();
      const playheadCueId = await getPlayhead();
      set({ cues, playheadCueId });
    } catch (e) {
      console.error("Failed to refresh cues:", e);
    }
  },

  refreshWorkspaceInfo: async () => {
    try {
      const info = await getWorkspaceInfo();
      set({ workspaceInfo: info });
    } catch (e) {
      console.error("Failed to refresh workspace info:", e);
    }
  },

  setSelectedCueId: (id) => set({ selectedCueId: id }),

  setPlayheadCueId: (id) => set({ playheadCueId: id }),

  updateCueState: (cueId, state) => {
    set((prev) => ({
      cues: prev.cues.map((c) => (c.id === cueId ? { ...c, state } : c)),
    }));
  },
}));
