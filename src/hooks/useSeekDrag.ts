// Drag-to-seek on a cue's progress bar. The position follows the pointer while
// dragging; the seek is sent on release, and the bar then holds the requested
// position until live timing catches up (so it does not jump back for a tick).

import { useCallback, useEffect, useRef, useState } from "react";

import { seekCue } from "../lib/commands";
import type { CueTimeline } from "../lib/timeline";
import { msAtFraction } from "../lib/timeline";

/** Live timing within this distance of the requested seek means it landed. */
const SEEK_LANDED_TOLERANCE_MS = 250;
/** Stop holding the requested position if timing never reports it. */
const SEEK_HOLD_TIMEOUT_MS = 1500;

interface Options {
  cueId: string;
  timeline: CueTimeline | null;
  /** Current position on the bar, in ms, from live timing. */
  liveMs: number;
  enabled: boolean;
}

export function useSeekDrag({ cueId, timeline, liveMs, enabled }: Options) {
  const barRef = useRef<HTMLDivElement>(null);
  const [dragMs, setDragMs] = useState<number | null>(null);
  const [pendingMs, setPendingMs] = useState<number | null>(null);
  const detachRef = useRef<(() => void) | null>(null);

  useEffect(() => () => detachRef.current?.(), []);

  useEffect(() => {
    if (pendingMs === null) return;
    if (Math.abs(liveMs - pendingMs) < SEEK_LANDED_TOLERANCE_MS) {
      setPendingMs(null);
      return;
    }
    const timer = setTimeout(() => setPendingMs(null), SEEK_HOLD_TIMEOUT_MS);
    return () => clearTimeout(timer);
  }, [liveMs, pendingMs]);

  const msAtClientX = useCallback(
    (clientX: number): number | null => {
      const bar = barRef.current;
      if (!bar || !timeline) return null;
      const rect = bar.getBoundingClientRect();
      if (rect.width <= 0) return null;
      return msAtFraction(timeline, (clientX - rect.left) / rect.width);
    },
    [timeline],
  );

  const onMouseDown = useCallback(
    (e: React.MouseEvent) => {
      if (!enabled || e.button !== 0) return;
      const start = msAtClientX(e.clientX);
      if (start === null) return;
      e.preventDefault();
      e.stopPropagation();
      setDragMs(start);

      const onMove = (ev: MouseEvent) => {
        const ms = msAtClientX(ev.clientX);
        if (ms !== null) setDragMs(ms);
      };
      const onUp = (ev: MouseEvent) => {
        detach();
        const ms = msAtClientX(ev.clientX) ?? start;
        setDragMs(null);
        setPendingMs(ms);
        seekCue(cueId, ms).catch(console.error);
      };
      const detach = () => {
        document.removeEventListener("mousemove", onMove);
        document.removeEventListener("mouseup", onUp);
        detachRef.current = null;
      };
      detachRef.current?.();
      document.addEventListener("mousemove", onMove);
      document.addEventListener("mouseup", onUp);
      detachRef.current = detach;
    },
    [enabled, cueId, msAtClientX],
  );

  return {
    barRef,
    /** Position to draw: the pointer while dragging, else the requested or live one. */
    displayMs: dragMs ?? pendingMs ?? liveMs,
    dragging: dragMs !== null,
    onMouseDown,
    msAtClientX,
  };
}
