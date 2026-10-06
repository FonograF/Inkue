// Playback scrubber shown in the Inspector Time tab for audio and video cues.
// Reads live position from timingStore; drag-to-seek commits on mouseup.

import { useSeekDrag } from "../../hooks/useSeekDrag";
import type { CueTimeline } from "../../lib/timeline";
import { timelinePosition } from "../../lib/timeline";
import { useTimingStore } from "../../stores/timingStore";

function fmtMs(ms: number): string {
  const totalSec = Math.floor(ms / 1000);
  const min = Math.floor(totalSec / 60);
  const sec = totalSec % 60;
  const tenth = Math.floor((ms % 1000) / 100);
  return `${min}:${sec.toString().padStart(2, "0")}.${tenth}`;
}

interface Props {
  cueId: string;
  timeline: CueTimeline;
  cueState: string;
}

export function ScrubBar({ cueId, timeline, cueState }: Props) {
  const timing = useTimingStore((s) => s.timings[cueId]);
  const isInteractive = cueState === "running" || cueState === "paused";
  const liveMs = timelinePosition(timeline, timing?.action_elapsed_ms ?? 0);
  const { barRef, displayMs, dragging, onMouseDown } = useSeekDrag({
    cueId, timeline, liveMs, enabled: isInteractive,
  });
  const pct = Math.min(100, (displayMs / timeline.lengthMs) * 100);

  return (
    <div style={{ padding: "6px 0 8px" }}>
      {/* Track */}
      <div
        ref={barRef}
        onMouseDown={onMouseDown}
        style={{
          position: "relative",
          height: 6,
          background: "var(--wc-bg-surface)",
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
            background: isInteractive ? "var(--wc-accent)" : "var(--wc-border-strong)",
            borderRadius: 3,
            transition: dragging ? "none" : "width 80ms linear",
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
              background: "var(--wc-accent)",
              boxShadow: "0 0 0 2px var(--wc-bg-app)",
              pointerEvents: "none",
              transition: dragging ? "none" : "left 80ms linear",
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
          color: "var(--wc-text-muted)",
          fontVariantNumeric: "tabular-nums",
        }}
      >
        <span>{fmtMs(displayMs)}</span>
        <span>{fmtMs(timeline.lengthMs)}</span>
      </div>
    </div>
  );
}
