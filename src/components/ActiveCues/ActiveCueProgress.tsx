// Progress line along the bottom of an Active-panel row. On a cue that can
// seek, hovering it thickens the line and shows a knob plus the time under the
// pointer; click or drag to move playback there (sent on release).
//
// The knob and the time label are portalled to <body> at fixed positions taken
// from the bar's on-screen rect: inside the Active panel (a scroll box) a knob
// overhanging the last row would add a scrollbar and be clipped, where it
// should sit over the cue list header below.

import { useState } from "react";
import { createPortal } from "react-dom";

import { useSeekDrag } from "../../hooks/useSeekDrag";
import type { CueTimeline } from "../../lib/timeline";
import { formatClock } from "../../lib/timeline";

interface Props {
  cueId: string;
  timeline: CueTimeline;
  /** Live position on the bar, in ms. */
  positionMs: number;
  seekable: boolean;
  color: string;
}

/** Height of the invisible strip that catches the pointer. */
const HIT_HEIGHT_PX = 10;
const KNOB_SIZE_PX = 10;
/** Above the app's panels and headers, below its menus and dialogs. */
const OVERLAY_Z_INDEX = 9000;

export function ActiveCueProgress({ cueId, timeline, positionMs, seekable, color }: Props) {
  const [hoverMs, setHoverMs] = useState<number | null>(null);
  const { barRef, displayMs, dragging, onMouseDown, msAtClientX } = useSeekDrag({
    cueId, timeline, liveMs: positionMs, enabled: seekable,
  });

  const active = seekable && (hoverMs !== null || dragging);
  const fraction = Math.min(1, displayMs / timeline.lengthMs);
  const labelMs = dragging ? displayMs : hoverMs;
  // Measured while the overlay shows: the bar moves when the panel scrolls.
  const barRect = active ? barRef.current?.getBoundingClientRect() ?? null : null;

  return (
    <div
      ref={barRef}
      onMouseDown={onMouseDown}
      onMouseMove={(e) => seekable && setHoverMs(msAtClientX(e.clientX))}
      onMouseLeave={() => setHoverMs(null)}
      style={{
        position: "absolute", left: 0, right: 0, bottom: 0, height: HIT_HEIGHT_PX,
        zIndex: 2, cursor: seekable ? "pointer" : "default",
        pointerEvents: seekable ? "auto" : "none",
      }}
    >
      <div
        style={{
          position: "absolute", left: 0, bottom: 0, width: "100%",
          height: active ? 4 : 2,
          background: active ? "rgba(255,255,255,0.08)" : "transparent",
          transition: "height 80ms ease-out",
        }}
      >
        <div
          style={{
            position: "absolute", inset: 0,
            transform: `scaleX(${fraction})`, transformOrigin: "left",
            background: color, willChange: "transform",
          }}
        />
      </div>

      {barRect && createPortal(
        <>
          <div
            style={{
              position: "fixed",
              left: barRect.left + fraction * barRect.width,
              // Centred on the thickened (4 px) line.
              top: barRect.bottom - 2,
              width: KNOB_SIZE_PX, height: KNOB_SIZE_PX, borderRadius: "50%",
              transform: "translate(-50%, -50%)",
              background: color, boxShadow: "0 0 0 2px var(--wc-bg-app)",
              zIndex: OVERLAY_Z_INDEX, pointerEvents: "none",
            }}
          />
          {labelMs !== null && (
            <div
              style={{
                position: "fixed",
                left: clamp(barRect.left + (labelMs / timeline.lengthMs) * barRect.width, barRect.left + 18, barRect.right - 18),
                top: barRect.bottom - (HIT_HEIGHT_PX - 2),
                transform: "translate(-50%, -100%)",
                padding: "0 4px", borderRadius: 3, lineHeight: "13px",
                background: "var(--wc-bg-app)", border: "1px solid var(--wc-border-strong)",
                color: "var(--wc-text)", fontFamily: "monospace", fontSize: 10,
                whiteSpace: "nowrap", zIndex: OVERLAY_Z_INDEX, pointerEvents: "none",
              }}
            >
              {formatClock(labelMs)}
            </div>
          )}
        </>,
        document.body,
      )}
    </div>
  );
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}
