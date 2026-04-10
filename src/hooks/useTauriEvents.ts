// Hook that subscribes to backend Tauri events and updates the Zustand stores.
// Set up once at the App level.

import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { useWorkspaceStore } from "../stores/workspaceStore";
import { useTransportStore } from "../stores/transportStore";
import { useTimingStore } from "../stores/timingStore";
import type {
  CueStateChangedEvent,
  CueTimeUpdateEvent,
  DeviceChangedEvent,
  PlayheadMovedEvent,
} from "../lib/types";

export function useTauriEvents() {
  const { refreshCues, refreshWorkspaceInfo, setPlayheadCueId, updateCueState } =
    useWorkspaceStore();
  const { updateMasterLevels } = useTransportStore();
  const { setTiming, clearTiming } = useTimingStore();

  useEffect(() => {
    const unlisteners: (() => void)[] = [];

    const setup = async () => {
      unlisteners.push(
        await listen<CueStateChangedEvent>("cue-state-changed", (e) => {
          updateCueState(e.payload.cue_id, e.payload.new_state);
          if (e.payload.new_state !== "running") {
            clearTiming(e.payload.cue_id);
          }
        })
      );

      unlisteners.push(
        await listen<CueTimeUpdateEvent>("cue-time-update", (e) => {
          setTiming(e.payload.cue_id, {
            elapsed_ms: e.payload.elapsed_ms,
            action_elapsed_ms: e.payload.action_elapsed_ms,
            remaining_ms: e.payload.remaining_ms,
          });
        })
      );

      unlisteners.push(
        await listen<PlayheadMovedEvent>("playhead-moved", (e) => {
          setPlayheadCueId(e.payload.cue_id);
        })
      );

      unlisteners.push(
        await listen("workspace-modified", async () => {
          await refreshCues();
          await refreshWorkspaceInfo();
        })
      );

      unlisteners.push(
        await listen<{ peak_l: number; peak_r: number }>("master-level", (e) => {
          updateMasterLevels(e.payload.peak_l, e.payload.peak_r);
        })
      );

      unlisteners.push(
        await listen<DeviceChangedEvent>("device-changed", () => {
          // Optionally re-fetch device list; handled by device settings panel.
        })
      );
    };

    setup().catch(console.error);

    return () => {
      unlisteners.forEach((u) => u());
    };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps
}
