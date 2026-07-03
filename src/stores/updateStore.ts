// Auto-update state — wraps tauri-plugin-updater.
//
// One store so the startup check, the About dialog button and the update
// dialog all share the same state machine:
//   idle → checking → available → downloading → ready-to-relaunch
//                   ↘ up-to-date / error

import { create } from "zustand";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export type UpdateStatus =
  | "idle"
  | "checking"
  | "up-to-date"
  | "available"
  | "downloading"
  | "installing"
  | "error";

interface UpdateState {
  status: UpdateStatus;
  version: string | null;
  notes: string | null;
  /** Download progress 0–1, or null while the total size is unknown. */
  progress: number | null;
  error: string | null;
  /** True once the user dismissed the dialog for this session. */
  dismissed: boolean;

  checkForUpdates: (opts?: { silent?: boolean }) => Promise<void>;
  downloadAndInstall: () => Promise<void>;
  dismiss: () => void;
}

let pendingUpdate: Update | null = null;

export const useUpdateStore = create<UpdateState>((set, get) => ({
  status: "idle",
  version: null,
  notes: null,
  progress: null,
  error: null,
  dismissed: false,

  checkForUpdates: async ({ silent = false } = {}) => {
    const { status } = get();
    if (status === "checking" || status === "downloading" || status === "installing") return;
    set({ status: "checking", error: null });
    try {
      const update = await check();
      if (update) {
        pendingUpdate = update;
        set({
          status: "available",
          version: update.version,
          notes: update.body ?? null,
          dismissed: false,
        });
      } else {
        pendingUpdate = null;
        set({ status: "up-to-date" });
      }
    } catch (e) {
      // The silent startup check must never surface an error dialog — no
      // network backstage is a normal situation for a show machine.
      if (silent) {
        console.warn("Update check failed:", e);
        set({ status: "idle" });
      } else {
        set({ status: "error", error: String(e) });
      }
    }
  },

  downloadAndInstall: async () => {
    if (!pendingUpdate) return;
    set({ status: "downloading", progress: null });
    let total: number | null = null;
    let received = 0;
    try {
      await pendingUpdate.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            total = event.data.contentLength ?? null;
            received = 0;
            set({ progress: total ? 0 : null });
            break;
          case "Progress":
            received += event.data.chunkLength;
            if (total) set({ progress: Math.min(received / total, 1) });
            break;
          case "Finished":
            set({ status: "installing", progress: 1 });
            break;
        }
      });
      await relaunch();
    } catch (e) {
      set({ status: "error", error: String(e) });
    }
  },

  dismiss: () => set({ dismissed: true }),
}));
