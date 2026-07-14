// Hook that subscribes to backend Tauri events and updates the Zustand stores.
// Set up once at the App level.

import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { useWorkspaceStore } from "../stores/workspaceStore";
import { useTransportStore } from "../stores/transportStore";
import { useTimingStore } from "../stores/timingStore";
import { go, stopAll, hardStopAll, setPlayhead, pauseCue, resumeCue } from "../lib/commands";
import type {
  CueListsChangedEvent,
  CueStateChangedEvent,
  CueTimeUpdateEvent,
  DeviceChangedEvent,
  OutputKeyEvent,
  PlayheadMovedEvent,
} from "../lib/types";

interface TauriEventsOptions {
  onLoadError?: (cueId: string, error: string) => void;
}

export function useTauriEvents({ onLoadError }: TauriEventsOptions = {}) {
  const { refreshCues, refreshWorkspaceInfo, refreshValidation, refreshHealth, loadDisplayPrefs, loadGeneralPrefs, setPlayheadCueId, updateCueState, setCueLists } =
    useWorkspaceStore();
  const { updateMasterLevels, markOscActivity, addOscLog } = useTransportStore();
  const { setTiming, clearTiming } = useTimingStore();

  useEffect(() => {
    let cancelled = false;
    const unlisteners: (() => void)[] = [];
    // Re-validate the workspace after edits settle (avoids running the full
    // preflight — incl. MIDI port enumeration — on every keystroke).
    let validationTimer: ReturnType<typeof setTimeout> | undefined;
    const scheduleValidation = () => {
      if (validationTimer) clearTimeout(validationTimer);
      validationTimer = setTimeout(() => { void refreshValidation(); }, 1200);
    };

    const setup = async () => {
      unlisteners.push(
        await listen<CueStateChangedEvent>("cue-state-changed", (e) => {
          updateCueState(e.payload.cue_id, e.payload.new_state);
          // Only clear timing when the cue fully stops — not on pause, so the
          // progress bar and inspector counter freeze at the paused position.
          if (e.payload.new_state === "standby" || e.payload.new_state === "completed") {
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
        await listen<CueListsChangedEvent>("cue-lists-changed", async (e) => {
          setCueLists(e.payload.cue_lists, e.payload.active_cue_list_id);
          await refreshCues();
        })
      );

      unlisteners.push(
        await listen("workspace-modified", async () => {
          await refreshCues();
          await refreshWorkspaceInfo();
          scheduleValidation();
        })
      );

      unlisteners.push(
        await listen("cue-list-refresh", async () => {
          await refreshCues();
        })
      );

      unlisteners.push(
        await listen("health-changed", async () => {
          await refreshHealth();
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

      unlisteners.push(
        await listen<{ cue_id: string; error: string }>("cue-load-error", (e) => {
          onLoadError?.(e.payload.cue_id, e.payload.error);
        })
      );

      unlisteners.push(
        await listen("preferences-applied", async () => {
          await loadDisplayPrefs();
          await loadGeneralPrefs();
        })
      );

      unlisteners.push(
        await listen<OutputKeyEvent>("output-keydown", (e) => {
          // Keys pressed while the output window has focus — replay them into
          // the regular window-level shortcut handler.
          window.dispatchEvent(
            new KeyboardEvent("keydown", {
              key: e.payload.key,
              ctrlKey: e.payload.ctrl,
              altKey: e.payload.alt,
              shiftKey: e.payload.shift,
              metaKey: e.payload.meta,
            })
          );
        })
      );

      unlisteners.push(
        await listen("osc-activity", () => {
          markOscActivity();
        })
      );

      unlisteners.push(
        await listen<{ addr: string; args: string[] }>("osc-debug", (e) => {
          const now = new Date();
          const ts = now.toTimeString().slice(0, 8) + "." + String(now.getMilliseconds()).padStart(3, "0");
          addOscLog({ ts, addr: e.payload.addr, args: e.payload.args });
        })
      );

      unlisteners.push(
        await listen<{ command: string; cue_number?: string; seek_mode?: string; value?: number }>("osc-command", async (e) => {
          const { command, cue_number, seek_mode, value } = e.payload;
          try {
            switch (command) {
              case "go":
                await go();
                break;
              case "stop_all":
                await stopAll();
                break;
              case "hard_stop_all":
                await hardStopAll();
                break;
              case "pause_all": {
                const running = useWorkspaceStore.getState().cues.filter(c => c.state === "running");
                for (const c of running) {
                  const { pauseCue } = await import("../lib/commands");
                  await pauseCue(c.id);
                }
                break;
              }
              case "resume_all": {
                const paused = useWorkspaceStore.getState().cues.filter(c => c.state === "paused");
                for (const c of paused) {
                  const { resumeCue } = await import("../lib/commands");
                  await resumeCue(c.id);
                }
                break;
              }
              case "cue_go": {
                const target = useWorkspaceStore.getState().cues.find(c => c.number === cue_number);
                if (target) {
                  await setPlayhead(target.id);
                  await go();
                }
                break;
              }
              case "cue_select": {
                const target = useWorkspaceStore.getState().cues.find(c => c.number === cue_number);
                if (target) await setPlayhead(target.id);
                break;
              }
              case "cue_stop": {
                const target = useWorkspaceStore.getState().cues.find(c => c.number === cue_number);
                if (target) {
                  const { stopCue } = await import("../lib/commands");
                  await stopCue(target.id);
                }
                break;
              }
              case "cue_seek": {
                const target = useWorkspaceStore.getState().cues.find(c => c.number === cue_number);
                if (!target || value == null) break;
                // Positions are clip-relative (same convention as the scrub
                // bar): one loop iteration when looping, else total duration.
                const fileDur = target.file_duration_ms ?? null;
                const totalDur = target.duration_ms ?? null;
                const isLooping = fileDur != null && (totalDur == null || fileDur < totalDur);
                const clipDur = isLooping ? fileDur : (totalDur ?? fileDur);
                let posMs: number;
                if (seek_mode === "percent") {
                  if (clipDur == null) break; // live feed — nothing to map
                  posMs = Math.max(0, Math.min(1, value)) * clipDur;
                } else if (seek_mode === "relative") {
                  const current = useTimingStore.getState().timings[target.id]?.action_elapsed_ms ?? 0;
                  posMs = current + value * 1000;
                } else {
                  posMs = value * 1000;
                }
                posMs = Math.max(0, clipDur != null ? Math.min(posMs, clipDur) : posMs);
                const { seekCue } = await import("../lib/commands");
                await seekCue(target.id, Math.round(posMs));
                break;
              }
              case "pause_toggle": {
                const cues = useWorkspaceStore.getState().cues;
                const running = cues.filter(c => c.state === "running");
                const paused  = cues.filter(c => c.state === "paused");
                if (running.length > 0) {
                  for (const c of running) await pauseCue(c.id);
                } else if (paused.length > 0) {
                  for (const c of paused) await resumeCue(c.id);
                }
                break;
              }
              case "select_next": {
                const { cues, playheadCueId } = useWorkspaceStore.getState();
                const idx = cues.findIndex(c => c.id === playheadCueId);
                const next = idx >= 0 ? cues[idx + 1] : cues[0];
                if (next) await setPlayhead(next.id);
                break;
              }
              case "select_previous": {
                const { cues, playheadCueId } = useWorkspaceStore.getState();
                const idx = cues.findIndex(c => c.id === playheadCueId);
                if (idx === -1) {
                  // Playhead past end — go to last cue
                  const last = cues[cues.length - 1];
                  if (last) await setPlayhead(last.id);
                } else if (idx > 0) {
                  await setPlayhead(cues[idx - 1].id);
                }
                break;
              }
            }
          } catch (err) {
            console.error("OSC command error:", err);
          }
        })
      );
    };

    setup()
      .then(() => {
        // The cleanup below can run while the listen() promises are still
        // pending (React StrictMode dev double-mount): those listeners land in
        // the array *after* the sweep and would stay subscribed forever — every
        // backend event then fires its handler twice (seen as F9 hiding and
        // instantly re-showing the output window). Sweep again once setup
        // settles.
        if (cancelled) unlisteners.splice(0).forEach((u) => u());
      })
      .catch(console.error);

    return () => {
      cancelled = true;
      if (validationTimer) clearTimeout(validationTimer);
      unlisteners.splice(0).forEach((u) => u());
    };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps
}
