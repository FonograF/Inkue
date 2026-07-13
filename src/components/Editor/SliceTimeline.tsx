// Large slice-editing timeline for the clip editor dock: a canvas background
// (audio waveform or video filmstrip, painted by the caller), draggable trim
// markers, and QLab-style slice markers with per-segment play-count badges.
//
// Interactions:
// - drag the blue (start) / orange (end) markers to trim
// - double-click adds a slice marker at the cursor
// - drag a slice marker to move it; right-click removes it
// - click a segment badge to edit its play count ("inf" or ∞ = vamp)

import { useEffect, useRef, useState } from "react";
import type { SliceList } from "../../lib/types";
import { PLAY_COUNT_INFINITE } from "../../lib/types";
import type { TrimPainter, TrimView } from "../Inspector/TrimStrip";
import { TRIM_END_COLOR, TRIM_START_COLOR, useCanvasWidth } from "../Inspector/TrimStrip";

const SLICE_COLOR = "#facc15";

function drawTrimMarker(
  ctx: CanvasRenderingContext2D,
  x: number,
  height: number,
  color: string,
) {
  ctx.strokeStyle = color;
  ctx.lineWidth = 2;
  ctx.beginPath();
  ctx.moveTo(x, 0);
  ctx.lineTo(x, height);
  ctx.stroke();
  ctx.fillStyle = color;
  ctx.beginPath();
  ctx.moveTo(x - 6, 0);
  ctx.lineTo(x + 6, 0);
  ctx.lineTo(x, 10);
  ctx.closePath();
  ctx.fill();
}

function drawSliceMarker(ctx: CanvasRenderingContext2D, x: number, height: number) {
  ctx.strokeStyle = SLICE_COLOR;
  ctx.lineWidth = 1.5;
  ctx.setLineDash([5, 3]);
  ctx.beginPath();
  ctx.moveTo(x, 0);
  ctx.lineTo(x, height);
  ctx.stroke();
  ctx.setLineDash([]);
  // Grip triangle at the bottom edge.
  ctx.fillStyle = SLICE_COLOR;
  ctx.beginPath();
  ctx.moveTo(x - 5, height);
  ctx.lineTo(x + 5, height);
  ctx.lineTo(x, height - 8);
  ctx.closePath();
  ctx.fill();
}

type DragTarget =
  | { kind: "start" }
  | { kind: "end" }
  | { kind: "slice"; index: number };

export function SliceTimeline({
  durationMs,
  startMs,
  endMs,
  slices,
  height,
  paint,
  paintKey,
  onCommitStart,
  onCommitEnd,
  onSlicesChange,
  dragPreview,
  onZoomDetail,
  onViewChange,
}: {
  durationMs: number;
  startMs: number | null;
  endMs: number | null;
  slices: SliceList;
  height: number;
  paint: TrimPainter;
  paintKey: unknown;
  onCommitStart: (ms: number | null) => void;
  onCommitEnd: (ms: number | null) => void;
  onSlicesChange: (s: SliceList) => void;
  /** Optional frame preview above the cursor while dragging (video). */
  dragPreview?: (ms: number) => React.ReactNode;
  /** Called when the operator zooms past 2× — load higher-resolution data. */
  onZoomDetail?: () => void;
  /** Reports every visible-window change (`null` = whole clip) so the owner
   *  can stream in window-matched preview data. */
  onViewChange?: (view: TrimView | null) => void;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const canvasWidth = useCanvasWidth(canvasRef);
  const [dragging, setDragging] = useState<DragTarget | null>(null);
  const [localStartMs, setLocalStartMs] = useState<number | null>(null);
  const [localEndMs, setLocalEndMs] = useState<number | null>(null);
  const [localMarkers, setLocalMarkers] = useState<number[] | null>(null);
  const [cursor, setCursor] = useState<{ x: number; width: number } | null>(null);
  const [editingBadge, setEditingBadge] = useState<number | null>(null);
  /** Visible time window; null = the whole clip. */
  const [viewWindow, setViewWindow] = useState<{ s: number; e: number } | null>(null);

  // Reset transient drag/edit state only when the slice *content* actually
  // changes — keying on the object identity would fire on every parent
  // render, cancelling drags in progress and closing the badge editor.
  const slicesKey = `${slices.markers.join(",")}|${slices.play_counts.join(",")}`;
  useEffect(() => {
    setLocalStartMs(null);
    setLocalEndMs(null);
    setLocalMarkers(null);
    setEditingBadge(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [paintKey, slicesKey]);

  // A different clip resets the zoom (but data-detail swaps must not).
  useEffect(() => {
    setViewWindow(null);
    onViewChange?.(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [durationMs]);

  const effStartMs = localStartMs ?? startMs ?? 0;
  const effEndMs = localEndMs ?? endMs ?? durationMs;
  const markers = localMarkers ?? slices.markers;

  const view = viewWindow ?? { s: 0, e: durationMs };
  const viewSpan = Math.max(1, view.e - view.s);
  const zoomFactor = durationMs > 0 ? durationMs / viewSpan : 1;
  /** Narrowest window: 200 ms (or the clip, for very short files). */
  const minSpan = Math.min(200, durationMs);

  const changeView = (next: { s: number; e: number } | null) => {
    setViewWindow(next);
    onViewChange?.(next ? { startMs: next.s, endMs: next.e } : null);
  };

  const applyZoom = (factor: number, anchorMs: number) => {
    if (durationMs <= 0) return;
    const span = Math.min(durationMs, Math.max(minSpan, viewSpan / factor));
    // Keep the time under the anchor at the same on-screen ratio.
    const ratio = (anchorMs - view.s) / viewSpan;
    let s = anchorMs - ratio * span;
    s = Math.max(0, Math.min(s, durationMs - span));
    const next = { s, e: s + span };
    changeView(next.s <= 0 && next.e >= durationMs ? null : next);
    if (durationMs / span >= 2) onZoomDetail?.();
  };

  const panBy = (deltaMs: number) => {
    if (!viewWindow) return;
    let s = viewWindow.s + deltaMs;
    s = Math.max(0, Math.min(s, durationMs - viewSpan));
    changeView({ s, e: s + viewSpan });
  };

  // Wheel: zoom centered on the cursor; Shift+wheel pans. Native listener —
  // React's synthetic wheel handlers are passive, so preventDefault (needed
  // to stop scroll chaining) only works here.
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const rect = canvas.getBoundingClientRect();
      const relX = Math.max(0, Math.min(e.clientX - rect.left, rect.width));
      const anchorMs = view.s + (relX / rect.width) * viewSpan;
      if (e.shiftKey) {
        panBy((e.deltaY > 0 ? 1 : -1) * viewSpan * 0.15);
      } else {
        applyZoom(e.deltaY < 0 ? 1.3 : 1 / 1.3, anchorMs);
      }
    };
    canvas.addEventListener("wheel", onWheel, { passive: false });
    return () => canvas.removeEventListener("wheel", onWheel);
  });

  // ── Painting ─────────────────────────────────────────────────────────────
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const rect = canvas.getBoundingClientRect();
    const W = rect.width || 600;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(W * dpr);
    canvas.height = Math.round(height * dpr);
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

    const toX = (ms: number) => ((ms - view.s) / viewSpan) * W;
    paint(ctx, W, height, toX(effStartMs), toX(effEndMs), { startMs: view.s, endMs: view.e });
    for (const m of markers) {
      if (m > effStartMs && m < effEndMs && m >= view.s && m <= view.e) {
        drawSliceMarker(ctx, toX(m), height);
      }
    }
    drawTrimMarker(ctx, toX(effStartMs), height, TRIM_START_COLOR);
    drawTrimMarker(ctx, toX(effEndMs), height, TRIM_END_COLOR);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [paint, paintKey, effStartMs, effEndMs, markers, durationMs, height, view.s, view.e, canvasWidth]);

  // ── Coordinate helpers ────────────────────────────────────────────────────
  const xToMs = (clientX: number): number => {
    const canvas = canvasRef.current;
    if (!canvas || durationMs === 0) return 0;
    const rect = canvas.getBoundingClientRect();
    const relX = Math.max(0, Math.min(clientX - rect.left, rect.width));
    return Math.max(0, Math.min(view.s + (relX / rect.width) * viewSpan, durationMs));
  };

  const hitTest = (clientX: number): DragTarget | null => {
    const canvas = canvasRef.current;
    if (!canvas || durationMs === 0) return null;
    const rect = canvas.getBoundingClientRect();
    const x = clientX - rect.left;
    const W = rect.width;
    const toX = (ms: number) => ((ms - view.s) / viewSpan) * W;

    // Slice markers first (thinner target, higher precision need).
    let best: { index: number; dist: number } | null = null;
    markers.forEach((m, i) => {
      const d = Math.abs(x - toX(m));
      if (d < 8 && (!best || d < best.dist)) best = { index: i, dist: d };
    });
    if (best !== null) return { kind: "slice", index: (best as { index: number }).index };

    const dStart = Math.abs(x - toX(effStartMs));
    const dEnd = Math.abs(x - toX(effEndMs));
    if (dStart <= dEnd && dStart < 14) return { kind: "start" };
    if (dEnd < 14) return { kind: "end" };
    return null;
  };

  // ── Mouse interactions ────────────────────────────────────────────────────
  const handleMouseDown = (e: React.MouseEvent<HTMLCanvasElement>) => {
    if (e.button !== 0) return;
    const target = hitTest(e.clientX);
    if (!target) return;
    setDragging(target);
    const rect = canvasRef.current!.getBoundingClientRect();
    setCursor({ x: e.clientX - rect.left, width: rect.width });
    if (target.kind === "slice" && localMarkers === null) {
      setLocalMarkers([...slices.markers]);
    }
  };

  const handleMouseMove = (e: React.MouseEvent<HTMLCanvasElement>) => {
    if (!dragging) return;
    const ms = xToMs(e.clientX);
    const rect = canvasRef.current!.getBoundingClientRect();
    setCursor({
      x: Math.max(0, Math.min(e.clientX - rect.left, rect.width)),
      width: rect.width,
    });
    if (dragging.kind === "start") {
      setLocalStartMs(Math.max(0, Math.min(ms, effEndMs - 50)));
    } else if (dragging.kind === "end") {
      setLocalEndMs(Math.min(durationMs, Math.max(ms, effStartMs + 50)));
    } else {
      setLocalMarkers((prev) => {
        const next = [...(prev ?? slices.markers)];
        next[dragging.index] = Math.round(
          Math.max(effStartMs + 20, Math.min(ms, effEndMs - 20)),
        );
        return next;
      });
    }
  };

  const handleMouseUp = () => {
    setCursor(null);
    if (!dragging) return;
    if (dragging.kind === "start" && localStartMs !== null) {
      const ms = Math.round(localStartMs);
      onCommitStart(ms <= 0 ? null : ms);
    } else if (dragging.kind === "end" && localEndMs !== null) {
      const ms = Math.round(localEndMs);
      onCommitEnd(ms >= durationMs ? null : ms);
    } else if (dragging.kind === "slice" && localMarkers !== null) {
      commitMarkers(localMarkers);
    }
    setDragging(null);
  };

  const handleDoubleClick = (e: React.MouseEvent<HTMLCanvasElement>) => {
    if (hitTest(e.clientX)) return; // don't add on top of an existing marker
    const ms = Math.round(xToMs(e.clientX));
    if (ms <= effStartMs + 20 || ms >= effEndMs - 20) return;
    // Insert the marker in sorted position; the segment it splits keeps its
    // play count on both halves (adjust afterwards as needed).
    const k = slices.markers.filter((m) => m < ms).length;
    const markers2 = [...slices.markers];
    markers2.splice(k, 0, ms);
    const counts2 = [...slices.play_counts];
    while (counts2.length < slices.markers.length + 1) counts2.push(1);
    counts2.splice(k, 0, counts2[k] ?? 1);
    onSlicesChange({ markers: markers2, play_counts: counts2 });
  };

  const handleContextMenu = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const target = hitTest(e.clientX);
    if (target?.kind !== "slice") return;
    e.preventDefault();
    const next = slices.markers.filter((_, i) => i !== target.index);
    const counts = [...slices.play_counts];
    counts.splice(target.index, 1); // the segment ending at this marker merges away
    onSlicesChange({ markers: next, play_counts: counts });
  };

  /** Commit dragged markers: sort by position and permute the per-segment
   *  counts the same way (a drag that crosses a neighbour swaps segments). */
  const commitMarkers = (raw: number[]) => {
    const order = raw.map((_, i) => i).sort((a, b) => raw[a] - raw[b]);
    const sorted = order.map((i) => raw[i]);
    const oldCounts = [...slices.play_counts];
    while (oldCounts.length < raw.length + 1) oldCounts.push(1);
    const counts = order.map((i) => oldCounts[i]);
    counts.push(oldCounts[raw.length]); // the final segment's count
    onSlicesChange({ markers: sorted, play_counts: counts });
  };

  // ── Segment badges ────────────────────────────────────────────────────────
  const setCount = (index: number, count: number) => {
    const counts = [...slices.play_counts];
    while (counts.length < slices.markers.length + 1) counts.push(1);
    counts[index] = count;
    onSlicesChange({ markers: slices.markers, play_counts: counts });
    setEditingBadge(null);
  };

  const segments: { midMs: number; count: number; index: number }[] = [];
  if (durationMs > 0 && markers.length > 0) {
    const bounds = [effStartMs, ...markers.filter((m) => m > effStartMs && m < effEndMs), effEndMs];
    for (let i = 0; i < bounds.length - 1; i++) {
      segments.push({
        midMs: (bounds[i] + bounds[i + 1]) / 2,
        count: slices.play_counts[i] ?? 1,
        index: i,
      });
    }
  }

  const draggedMs =
    dragging?.kind === "start" ? effStartMs
    : dragging?.kind === "end" ? effEndMs
    : dragging?.kind === "slice" ? (markers[dragging.index] ?? 0)
    : 0;

  const fmtViewS = (ms: number) => (ms / 1000).toFixed(zoomFactor > 20 ? 3 : 1);

  return (
    <div style={{ position: "relative" }}>
      {/* Zoom controls + visible range */}
      <div style={{ display: "flex", alignItems: "center", gap: 6, marginBottom: 3 }}>
        <span style={{ fontSize: 10, color: "var(--wc-text-faint)" }}>
          Wheel: zoom · Shift+wheel: pan
        </span>
        <span style={{ flex: 1 }} />
        {zoomFactor > 1.01 && (
          <span style={{ fontSize: 10, color: "var(--wc-text-muted)", fontVariantNumeric: "tabular-nums" }}>
            {fmtViewS(view.s)}s – {fmtViewS(view.e)}s ({zoomFactor.toFixed(1)}×)
          </span>
        )}
        <button style={zoomBtn} title="Zoom out"
          onClick={() => applyZoom(1 / 1.6, (view.s + view.e) / 2)}>−</button>
        <button style={zoomBtn} title="Zoom in"
          onClick={() => applyZoom(1.6, (view.s + view.e) / 2)}>+</button>
        <button
          style={{ ...zoomBtn, width: "auto", padding: "0 8px", opacity: viewWindow ? 1 : 0.4 }}
          title="Show the whole clip"
          disabled={!viewWindow}
          onClick={() => changeView(null)}
        >
          Fit
        </button>
      </div>

      {dragging && cursor && dragPreview && (
        <div
          style={{
            position: "absolute",
            left: Math.max(105, Math.min(cursor.x, cursor.width - 105)),
            top: -6,
            transform: "translate(-50%, -100%)",
            zIndex: 10,
            pointerEvents: "none",
          }}
        >
          {dragPreview(draggedMs)}
        </div>
      )}

      <canvas
        ref={canvasRef}
        style={{
          width: "100%",
          height,
          display: "block",
          borderRadius: 4,
          cursor: dragging ? "ew-resize" : "default",
        }}
        onMouseDown={handleMouseDown}
        onMouseMove={handleMouseMove}
        onMouseUp={handleMouseUp}
        onMouseLeave={handleMouseUp}
        onDoubleClick={handleDoubleClick}
        onContextMenu={handleContextMenu}
      />

      {/* Per-segment play-count badges (only when sliced). */}
      {segments.length > 1 && (
        <div style={{ position: "relative", height: 24, marginTop: 2 }}>
          {segments.map((seg) => {
            const leftPct = ((seg.midMs - view.s) / viewSpan) * 100;
            if (leftPct < 2 || leftPct > 98) return null; // outside the zoomed view
            const isVamp = seg.count === PLAY_COUNT_INFINITE;
            return editingBadge === seg.index ? (
              <input
                key={seg.index}
                autoFocus
                defaultValue={isVamp ? "inf" : String(seg.count)}
                onFocus={(e) => e.target.select()}
                onKeyDown={(e) => {
                  if (e.key === "Enter") (e.target as HTMLInputElement).blur();
                  if (e.key === "Escape") setEditingBadge(null);
                }}
                onBlur={(e) => {
                  const v = e.target.value.trim().toLowerCase();
                  if (v === "inf" || v === "∞" || v === "0") {
                    setCount(seg.index, PLAY_COUNT_INFINITE);
                  } else {
                    const n = parseInt(v, 10);
                    setCount(seg.index, Number.isNaN(n) || n < 1 ? 1 : n);
                  }
                }}
                style={{
                  position: "absolute",
                  left: `${leftPct}%`,
                  transform: "translateX(-50%)",
                  width: 44,
                  textAlign: "center",
                  fontSize: 11,
                  background: "var(--wc-bg-surface)",
                  border: `1px solid ${SLICE_COLOR}`,
                  borderRadius: 3,
                  color: "var(--wc-text)",
                  padding: "1px 2px",
                }}
              />
            ) : (
              <button
                key={seg.index}
                onClick={() => setEditingBadge(seg.index)}
                title={
                  isVamp
                    ? "Vamp — loops until a Devamp Cue releases it. Click to edit."
                    : `Plays ${seg.count}× . Click to edit (type "inf" for a vamp).`
                }
                style={{
                  position: "absolute",
                  left: `${leftPct}%`,
                  transform: "translateX(-50%)",
                  minWidth: 28,
                  fontSize: 11,
                  fontWeight: isVamp ? 700 : 400,
                  background: isVamp ? SLICE_COLOR : "var(--wc-bg-surface)",
                  color: isVamp ? "#1c1917" : "var(--wc-text)",
                  border: "1px solid var(--wc-border-strong)",
                  borderRadius: 3,
                  padding: "1px 6px",
                  cursor: "pointer",
                }}
              >
                {isVamp ? "∞" : `×${seg.count}`}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}

const zoomBtn: React.CSSProperties = {
  width: 22,
  height: 18,
  lineHeight: 1,
  fontSize: 12,
  background: "var(--wc-bg-surface)",
  border: "1px solid var(--wc-border-strong)",
  borderRadius: 3,
  color: "var(--wc-text)",
  cursor: "pointer",
  padding: 0,
};
