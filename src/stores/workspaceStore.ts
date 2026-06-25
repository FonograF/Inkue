// Zustand store: workspace data, cue list, selection, and playhead.

import { create } from "zustand";
import type { CueId, CueListSummary, CueSummary, CueValidation, DisplayPreferences, GeneralPreferences, WorkspaceInfo } from "../lib/types";
import { DEFAULT_DISPLAY_PREFS, DEFAULT_GENERAL_PREFS } from "../lib/types";
import { checkWorkspace, getAllCues, getCueLists, getPlayhead, getPreferences, getWorkspaceInfo } from "../lib/commands";

interface WorkspaceState {
  cues: CueSummary[];
  cueLists: CueListSummary[];
  activeCueListId: string | null;
  selectedCueId: CueId | null;
  /** All cues currently highlighted (multi-selection). Always includes selectedCueId when non-null. */
  selectedCueIds: CueId[];
  playheadCueId: CueId | null;
  workspaceInfo: WorkspaceInfo | null;
  generalPrefs: GeneralPreferences;
  displayPrefs: DisplayPreferences;
  /** Latest preflight results (one entry per cue with problems). */
  validation: CueValidation[];
  /** IDs of cues with at least one error-severity problem (drives the row badge). */
  brokenCueIds: Set<CueId>;

  // Actions
  refreshCues: () => Promise<void>;
  refreshValidation: () => Promise<void>;
  refreshCueLists: () => Promise<void>;
  setCueLists: (lists: CueListSummary[], activeId: string) => void;
  refreshWorkspaceInfo: () => Promise<void>;
  setSelectedCueId: (id: CueId | null) => void;
  setSelectedCueIds: (ids: CueId[]) => void;
  setPlayheadCueId: (id: CueId | null) => void;
  updateCueState: (cueId: CueId, state: CueSummary["state"]) => void;
  loadGeneralPrefs: () => Promise<void>;
  setGeneralPrefs: (p: GeneralPreferences) => void;
  loadDisplayPrefs: () => Promise<void>;
  setDisplayPrefs: (p: DisplayPreferences) => void;
}

export const useWorkspaceStore = create<WorkspaceState>((set, _get) => ({
  cues: [],
  cueLists: [],
  activeCueListId: null,
  selectedCueId: null,
  selectedCueIds: [],
  playheadCueId: null,
  workspaceInfo: null,
  validation: [],
  brokenCueIds: new Set<CueId>(),
  generalPrefs: DEFAULT_GENERAL_PREFS,
  displayPrefs: { ...DEFAULT_DISPLAY_PREFS, output_screen: null, show_output_timer: false, timer_floating: false, timer_count_down: false, timer_font: "DSEG7 Classic", timer_font_size: 120, timer_position: "center" as const, timer_show_ms: false, timer_margin: 50, theme: "system" as const },

  refreshCues: async () => {
    try {
      const cues = await getAllCues();
      const playheadCueId = await getPlayhead();
      set({ cues, playheadCueId });
    } catch (e) {
      console.error("Failed to refresh cues:", e);
    }
  },

  refreshValidation: async () => {
    try {
      const validation = await checkWorkspace();
      const brokenCueIds = new Set<CueId>(
        validation
          .filter((v) => v.issues.some((i) => i.severity === "error"))
          .map((v) => v.cue_id),
      );
      set({ validation, brokenCueIds });
    } catch (e) {
      console.error("Failed to validate workspace:", e);
    }
  },

  refreshCueLists: async () => {
    try {
      const cueLists = await getCueLists();
      set((prev) => {
        // Keep the active ID only if it still exists in the new list.
        const validId = cueLists.some((cl) => cl.id === prev.activeCueListId)
          ? prev.activeCueListId
          : (cueLists[0]?.id ?? null);
        return { cueLists, activeCueListId: validId };
      });
    } catch (e) {
      console.error("Failed to refresh cue lists:", e);
    }
  },

  setCueLists: (lists, activeId) => set({ cueLists: lists, activeCueListId: activeId }),

  refreshWorkspaceInfo: async () => {
    try {
      const info = await getWorkspaceInfo();
      set({ workspaceInfo: info });
    } catch (e) {
      console.error("Failed to refresh workspace info:", e);
    }
  },

  setSelectedCueId: (id) => set({ selectedCueId: id }),

  setSelectedCueIds: (ids) => set({ selectedCueIds: ids }),

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

  loadDisplayPrefs: async () => {
    try {
      const prefs = await getPreferences();
      set({ displayPrefs: { ...DEFAULT_DISPLAY_PREFS, ...prefs.display } });
    } catch (e) {
      console.error("Failed to load display preferences:", e);
    }
  },

  setDisplayPrefs: (p) => set({ displayPrefs: p }),
}));
