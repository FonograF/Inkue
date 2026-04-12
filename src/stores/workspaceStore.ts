// Zustand store: workspace data, cue list, selection, and playhead.

import { create } from "zustand";
import type { CueId, CueSummary, GeneralPreferences, WorkspaceInfo } from "../lib/types";
import { DEFAULT_GENERAL_PREFS } from "../lib/types";
import { getAllCues, getPlayhead, getPreferences, getWorkspaceInfo } from "../lib/commands";

interface WorkspaceState {
  cues: CueSummary[];
  selectedCueId: CueId | null;
  playheadCueId: CueId | null;
  workspaceInfo: WorkspaceInfo | null;
  generalPrefs: GeneralPreferences;

  // Actions
  refreshCues: () => Promise<void>;
  refreshWorkspaceInfo: () => Promise<void>;
  setSelectedCueId: (id: CueId | null) => void;
  setPlayheadCueId: (id: CueId | null) => void;
  updateCueState: (cueId: CueId, state: CueSummary["state"]) => void;
  loadGeneralPrefs: () => Promise<void>;
  setGeneralPrefs: (p: GeneralPreferences) => void;
}

export const useWorkspaceStore = create<WorkspaceState>((set, _get) => ({
  cues: [],
  selectedCueId: null,
  playheadCueId: null,
  workspaceInfo: null,
  generalPrefs: DEFAULT_GENERAL_PREFS,

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

  loadGeneralPrefs: async () => {
    try {
      const prefs = await getPreferences();
      set({ generalPrefs: { ...DEFAULT_GENERAL_PREFS, ...prefs.general } });
    } catch (e) {
      console.error("Failed to load general preferences:", e);
    }
  },

  setGeneralPrefs: (p) => set({ generalPrefs: p }),
}));
