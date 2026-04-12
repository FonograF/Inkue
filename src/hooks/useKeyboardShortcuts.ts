// Global keyboard shortcut handler, mirroring QLab's key bindings.

import { useEffect, useRef } from "react";
import { confirm } from "@tauri-apps/plugin-dialog";
import {
  go,
  hardStopAll,
  stopAll,
  stopCue,
  pauseCue,
  resumeCue,
  addCue,
  removeCue,
  duplicateCue,
  undo,
  redo,
  copyCue,
  pasteCue,
  setPlayhead,
} from "../lib/commands";
import { useWorkspaceStore } from "../stores/workspaceStore";

export function useKeyboardShortcuts(
  onRefresh: () => void,
  onOpenPreferences?: () => void,
  onSave?: () => void,
  onOpen?: () => void,
  onToggleInspector?: () => void,
  onGoto?: () => void,
) {
  const lastEscapeRef = useRef<number>(0);
  const lastGoRef = useRef<number>(0);
  const { selectedCueId, generalPrefs } = useWorkspaceStore();

  useEffect(() => {
    const handler = async (e: KeyboardEvent) => {
      // Ignore shortcuts when typing in an input / textarea.
      const target = e.target as HTMLElement;
      if (
        target.tagName === "INPUT" ||
        target.tagName === "TEXTAREA" ||
        target.isContentEditable
      ) {
        return;
      }

      switch (e.key) {
        case " ": {
          // Space → GO (with double-GO protection)
          e.preventDefault();
          const now = Date.now();
          const protection = generalPrefs.double_go_protection_ms;
          if (protection > 0 && now - lastGoRef.current < protection) break;
          lastGoRef.current = now;
          await go().catch(console.error);
          onRefresh();
          break;
        }
        case "Escape": {
          // Single Escape → Stop All; double Escape → Hard Stop All
          const now = Date.now();
          if (now - lastEscapeRef.current < 500) {
            await hardStopAll().catch(console.error);
          } else {
            await stopAll().catch(console.error);
          }
          lastEscapeRef.current = now;
          onRefresh();
          break;
        }
        case "s":
        case "S": {
          if (e.ctrlKey) {
            e.preventDefault();
            onSave?.();
          } else if (selectedCueId) {
            await stopCue(selectedCueId).catch(console.error);
            onRefresh();
          }
          break;
        }
        case "o":
        case "O": {
          if (e.ctrlKey) {
            e.preventDefault();
            onOpen?.();
          }
          break;
        }
        case "i":
        case "I": {
          if (e.ctrlKey) {
            e.preventDefault();
            onToggleInspector?.();
          }
          break;
        }
        case "p":
        case "P":
        case "[": {
          if (!e.ctrlKey && selectedCueId) {
            await pauseCue(selectedCueId).catch(console.error);
            onRefresh();
          }
          break;
        }
        case "]": {
          if (selectedCueId) {
            await resumeCue(selectedCueId).catch(console.error);
            onRefresh();
          }
          break;
        }
        case ",": {
          if (e.ctrlKey) {
            e.preventDefault();
            onOpenPreferences?.();
          }
          break;
        }
        case "g":
        case "G": {
          if (!e.ctrlKey && !e.shiftKey && !e.altKey) {
            e.preventDefault();
            onGoto?.();
          }
          break;
        }
        case "ArrowUp": {
          if (e.ctrlKey) {
            e.preventDefault();
            const { cues, playheadCueId } = useWorkspaceStore.getState();
            const idx = cues.findIndex((c) => c.id === playheadCueId);
            const prevCue = idx > 0 ? cues[idx - 1] : cues[0];
            if (prevCue) {
              await setPlayhead(prevCue.id).catch(console.error);
              onRefresh();
            }
          }
          break;
        }
        case "ArrowDown": {
          if (e.ctrlKey) {
            e.preventDefault();
            const { cues, playheadCueId } = useWorkspaceStore.getState();
            const idx = cues.findIndex((c) => c.id === playheadCueId);
            const nextCue = idx < cues.length - 1 ? cues[idx + 1] : cues[cues.length - 1];
            if (nextCue) {
              await setPlayhead(nextCue.id).catch(console.error);
              onRefresh();
            }
          }
          break;
        }
        case "n":
        case "N": {
          if (e.ctrlKey) {
            e.preventDefault();
            await addCue("audio").catch(console.error);
            onRefresh();
          }
          break;
        }
        case "d":
        case "D": {
          if (e.ctrlKey && selectedCueId) {
            e.preventDefault();
            await duplicateCue(selectedCueId).catch(console.error);
            onRefresh();
          }
          break;
        }
        case "z":
        case "Z": {
          if (e.ctrlKey && e.shiftKey) {
            // Ctrl+Shift+Z → Redo (alternative to Ctrl+Y)
            e.preventDefault();
            await redo().catch(console.error);
            onRefresh();
          } else if (e.ctrlKey) {
            // Ctrl+Z → Undo
            e.preventDefault();
            await undo().catch(console.error);
            onRefresh();
          }
          break;
        }
        case "y":
        case "Y": {
          if (e.ctrlKey) {
            e.preventDefault();
            await redo().catch(console.error);
            onRefresh();
          }
          break;
        }
        case "c":
        case "C": {
          if (e.ctrlKey && selectedCueId) {
            e.preventDefault();
            await copyCue(selectedCueId).catch(console.error);
          }
          break;
        }
        case "v":
        case "V": {
          if (e.ctrlKey) {
            e.preventDefault();
            await pasteCue(selectedCueId).catch(console.error);
            onRefresh();
          }
          break;
        }
        case "Delete":
        case "Backspace": {
          if (selectedCueId && e.ctrlKey === false) {
            if (generalPrefs.confirm_before_delete) {
              const ok = await confirm("Delete this cue?", { title: "Confirm Delete", kind: "warning" });
              if (!ok) break;
            }
            await removeCue(selectedCueId).catch(console.error);
            onRefresh();
          }
          break;
        }
        default:
          break;
      }
    };

    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [selectedCueId, generalPrefs, onRefresh, onOpenPreferences, onSave, onOpen, onToggleInspector, onGoto]);
}
