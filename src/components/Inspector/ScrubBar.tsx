// Playback scrubber shown in the Inspector Time tab for audio and video cues.
// Reads live position from timingStore; drag-to-seek commits on mouseup.

import { useCallback, useEffect, useRef, useState } from "react";
import { useTimingStore } from "../../stores/timingStore";
import { seekCue } from "../../lib/commands";

function fmtMs(ms: number): string {
  const totalSec = Math.floor(ms / 1000);
  const min = Math.floor(totalSec / 60);
  const sec = totalSec % 60;
  const tenth = Math.floor((ms % 1000) / 100);
  return `${min}:${sec.toString().padStart(2, "0")}.${tenth}`;
}

interface Props {
  cueId: string;
  durationMs: number;
  cueState: string;
  /** When set, progress wraps modulo this value so the bar resets each loop. */
  loopDurationMs?: number;
}

export function ScrubBar({ cueId, durationMs, cueState, loopDurationMs }: Props) {
  const timing = useTimingStore((s) => s.timings[cueId]);
  const barRef = useRef<HTMLDivElement>(null);

  const [isDragging, setIsDragging] = useState(false);
  const [dragMs, setDragMs] = useState(0);
  // Override displayed position after a seek until timing catches up.
  const [seekOverrideMs, setSeekOverrideMs] = useState<number | null>(null);

  const isInteractive = cueState === "running" || cueState === "paused";

  // Clear the seek override once live timing catches up.
  useEffect(() => {
    if (seekOverrideMs === null || !timing) return;
    if (Math.abs(timing.action_elapsed_ms - seekOverrideMs) < 250) {
      setSeekOverrideMs(null);
    }
  }, [timing?.action_elapsed_ms, seekOverrideMs]);

  const rawMs = timing?.action_elapsed_ms ?? 0;
  // For looping cues, show position within the current iteration.
  const liveMs = loopDurationMs && loopDurationMs > 0 ? rawMs % loopDurationMs : rawMs;
  const displayMs = isDragging ? dragMs : (seekOverrideMs ?? liveMs);
  const pct = durationMs > 0 ? Math.min(100, (displayMs / durationMs) * 100) : 0;

  const msFromMouseEvent = useCallback(
    (e: MouseEvent | React.MouseEvent): number => {
      const bar = barRef.current;
      if (!bar || durationMs <= 0) return 0;
      const rect = bar.getBoundingClientRect();
      const frac = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
      return Math.round(frac * durationMs);
    },
    [durationMs],
  );

  const handleMouseDown = useCallback(
    (e: React.MouseEvent) => {
      if (!isInteractive) return;
      e.preventDefault();
      const ms = msFromMouseEvent(e);
      setDragMs(ms);
      setIsDragging(true);

      const onMove = (ev: MouseEvent) => setDragMs(msFromMouseEvent(ev));
      const onUp = (ev: MouseEvent) => {
        const seekMs = msFromMouseEvent(ev);
        setIsDragging(false);
        setSeekOverrideMs(seekMs);
        seekCue(cueId, seekMs).catch(console.error);
        document.removeEventListener("mousemove", onMove);
        document.removeEventListener("mouseup", onUp);
      };
      document.addEventListener("mousemove", onMove);
      document.addEventListener("mouseup", onUp);
    },
    [isInteractive, cueId, msFromMouseEvent],
  );

  return (
    <div style={{ padding: "6px 0 8px" }}>
      {/* Track */}
      <div
        ref={barRef}
        onMouseDown={handleMouseDown}
        style={{
          position: "relative",
          height: 6,
          background: "#1e293b",
          borderRadius: 3,
          cursor: isInteractive ? "pointer" : "default",
          marginBottom: 5,
          userSelect: "none",
        }}
      >
        {/* Filled */}
        <div
          style={{
            position: "absolute",
            inset: 0,
            width: `${pct}%`,
            background: isInteractive ? "#3b82f6" : "#334155",
            borderRadius: 3,
            transition: isDragging ? "none" : "width 80ms linear",
          }}
        />
        {/* Thumb */}
        {isInteractive && (
          <div
            style={{
              position: "absolute",
              top: "50%",
              left: `${pct}%`,
              transform: "translate(-50%, -50%)",
              width: 12,
              height: 12,
              borderRadius: "50%",
              background: "#3b82f6",
              boxShadow: "0 0 0 2px #0f172a",
              pointerEvents: "none",
              transition: isDragging ? "none" : "left 80ms linear",
            }}
          />
        )}
      </div>

      {/* Time readout */}
      <div
        style={{
          display: "flex",
          justifyContent: "space-between",
          fontSize: 10,
          color: "#64748b",
          fontVariantNumeric: "tabular-nums",
        }}
      >
        <span>{fmtMs(displayMs)}</span>
        <span>{fmtMs(durationMs)}</span>
      </div>
    </div>
  );
}
