// Global keyboard shortcut handler, mirroring QLab's key bindings.

import { useEffect, useRef } from "react";
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
} from "../lib/commands";
import { useWorkspaceStore } from "../stores/workspaceStore";

export function useKeyboardShortcuts(
  onRefresh: () => void,
  onOpenPreferences?: () => void,
) {
  const lastEscapeRef = useRef<number>(0);
  const { selectedCueId } = useWorkspaceStore();

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
          // Space → GO
          e.preventDefault();
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
          if (!e.ctrlKey && selectedCueId) {
            await stopCue(selectedCueId).catch(console.error);
            onRefresh();
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
  }, [selectedCueId, onRefresh, onOpenPreferences]);
}
