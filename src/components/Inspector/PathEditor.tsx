// Drawn path editor for a Path-mode Fade Cue (QLab's 2D path canvas).
//
// The stage is drawn at the output's aspect ratio with a margin around it,
// since a path may leave the screen. Click empty space to add a point at the
// end; Alt-click inserts one into the nearest span; drag a point to move it;
// select one and press Delete to remove it. The line is the curve the engine
// plays: `lib/path.ts` mirrors the Rust flattening.

import { useRef, useState } from "react";
import type { PathGeometry } from "../../lib/types";
import { flattenPath, nearestSegment, type PathPoint } from "../../lib/path";

/** Path units shown either side of the centre: the stage (±0.5) plus a margin. */
const VIEW_HALF = 0.75;
/** Grab distance for a point, in SVG pixels. */
const HIT_RADIUS = 8;

const smallButton = {
  padding: "2px 8px",
  background: "var(--wc-bg-surface)",
  border: "1px solid var(--wc-border-strong)",
  borderRadius: 4,
  color: "var(--wc-text-secondary)",
  fontSize: 11,
  cursor: "pointer",
} as const;

export function PathEditor({
  path,
  aspect,
  width = 260,
  onChange,
}: {
  path: PathGeometry;
  /** Output width / height. */
  aspect: number;
  width?: number;
  onChange: (next: PathGeometry) => void;
}) {
  const svgRef = useRef<SVGSVGElement | null>(null);
  const [selected, setSelected] = useState<number | null>(null);
  const [dragging, setDragging] = useState<number | null>(null);
  const height = Math.round(width / aspect);
  const points = path.points;

  // Path units ↔ SVG pixels (+Y up on screen, like the output).
  const toX = (x: number) => ((x + VIEW_HALF) / (2 * VIEW_HALF)) * width;
  const toY = (y: number) => ((VIEW_HALF - y) / (2 * VIEW_HALF)) * height;
  const toPath = (event: React.PointerEvent): PathPoint => {
    const rect = svgRef.current?.getBoundingClientRect();
    if (!rect) return [0, 0];
    const fx = (event.clientX - rect.left) / rect.width;
    const fy = (event.clientY - rect.top) / rect.height;
    const round = (v: number) => Math.round(v * 1000) / 1000;
    return [round(fx * 2 * VIEW_HALF - VIEW_HALF), round(VIEW_HALF - fy * 2 * VIEW_HALF)];
  };
  const hit = (event: React.PointerEvent): number => {
    const rect = svgRef.current?.getBoundingClientRect();
    if (!rect) return -1;
    const scale = width / rect.width;
    const px = (event.clientX - rect.left) * scale;
    const py = (event.clientY - rect.top) * scale;
    return points.findIndex(([x, y]) => Math.hypot(toX(x) - px, toY(y) - py) <= HIT_RADIUS);
  };

  const setPoints = (next: PathPoint[]) => onChange({ ...path, points: next });

  const handleDown = (event: React.PointerEvent) => {
    event.currentTarget.setPointerCapture(event.pointerId);
    const existing = hit(event);
    if (existing >= 0) {
      setSelected(existing);
      setDragging(existing);
      return;
    }
    const at = toPath(event);
    // Alt inserts into the span under the cursor; a plain click extends the end.
    const index = event.altKey && points.length >= 2 ? nearestSegment(points, at) + 1 : points.length;
    setPoints([...points.slice(0, index), at, ...points.slice(index)]);
    setSelected(index);
    setDragging(index);
  };

  const handleMove = (event: React.PointerEvent) => {
    if (dragging === null || !points[dragging]) return;
    setPoints(points.map((p, i) => (i === dragging ? toPath(event) : p)));
  };

  const handleKey = (event: React.KeyboardEvent) => {
    if (selected === null || (event.key !== "Delete" && event.key !== "Backspace")) return;
    event.preventDefault();
    // A path needs two points; the table's ✕ follows the same rule.
    if (points.length <= 2) return;
    setPoints(points.filter((_, i) => i !== selected));
    setSelected(null);
  };

  const flip = (axis: 0 | 1) =>
    setPoints(points.map((p) => (axis === 0 ? [-p[0], p[1]] : [p[0], -p[1]]) as PathPoint));

  const line = flattenPath(points, path.smooth, aspect, path.x_param, path.y_param)
    .map(([x, y], i) => `${i === 0 ? "M" : "L"}${toX(x).toFixed(1)},${toY(y).toFixed(1)}`)
    .join(" ");
  const start = points[0];
  const end = points[points.length - 1];

  return (
    <div style={{ marginBottom: 10 }}>
      <svg
        ref={svgRef}
        width={width}
        height={height}
        viewBox={`0 0 ${width} ${height}`}
        tabIndex={0}
        onPointerDown={handleDown}
        onPointerMove={handleMove}
        onPointerUp={() => setDragging(null)}
        onPointerCancel={() => setDragging(null)}
        onKeyDown={handleKey}
        style={{
          maxWidth: "100%",
          height: "auto",
          display: "block",
          background: "var(--wc-bg-app)",
          border: "1px solid var(--wc-border-strong)",
          borderRadius: 4,
          cursor: "crosshair",
          outline: "none",
          touchAction: "none",
        }}
      >
        {/* The stage, and its centre axes. */}
        <rect
          x={toX(-0.5)} y={toY(0.5)} width={toX(0.5) - toX(-0.5)} height={toY(-0.5) - toY(0.5)}
          fill="var(--wc-bg-surface)" stroke="var(--wc-border-strong)" strokeWidth={1}
        />
        <line x1={toX(0)} y1={0} x2={toX(0)} y2={height} stroke="var(--wc-border)" strokeWidth={0.75} />
        <line x1={0} y1={toY(0)} x2={width} y2={toY(0)} stroke="var(--wc-border)" strokeWidth={0.75} />

        <path d={line} fill="none" stroke="var(--wc-accent)" strokeWidth={2} />

        {points.map(([x, y], i) => {
          const isSelected = i === selected;
          const cx = toX(x);
          const cy = toY(y);
          if (i === 0 && start) {
            // Start: a triangle, as in QLab.
            return (
              <polygon
                key={i}
                points={`${cx - 6},${cy + 5} ${cx + 6},${cy + 5} ${cx},${cy - 6}`}
                fill={isSelected ? "var(--wc-accent)" : "var(--wc-bg-surface)"}
                stroke="#3fb950" strokeWidth={2}
              />
            );
          }
          if (i === points.length - 1 && end) {
            return (
              <rect
                key={i} x={cx - 5} y={cy - 5} width={10} height={10}
                fill={isSelected ? "var(--wc-accent)" : "var(--wc-bg-surface)"}
                stroke="#f85149" strokeWidth={2}
              />
            );
          }
          return (
            <circle
              key={i} cx={cx} cy={cy} r={isSelected ? 5 : 4}
              fill={isSelected ? "var(--wc-accent)" : "var(--wc-bg-surface)"}
              stroke="var(--wc-accent)" strokeWidth={1.5}
            />
          );
        })}
      </svg>
      <div style={{ fontSize: 10, color: "var(--wc-text-faint)", margin: "3px 0 6px" }}>
        Click adds a point at the end · Alt-click inserts · drag moves · Delete removes
      </div>
      <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
        <button style={smallButton} title="Same points, travelled the other way" onClick={() => setPoints([...points].reverse())}>
          Reverse
        </button>
        <button style={smallButton} title="Mirror left ↔ right about the centre" onClick={() => flip(0)}>
          Flip ↔
        </button>
        <button style={smallButton} title="Mirror up ↔ down about the centre" onClick={() => flip(1)}>
          Flip ↕
        </button>
      </div>
    </div>
  );
}
