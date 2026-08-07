// Graphical fade-curve editor — QLab's Curve tab.
//
// Two panels side by side: the envelope used when a value rises, and the one
// used when it falls. That split is the whole point — an envelope that sounds
// natural coming up is not the one that sounds natural going down. The lock
// between them mirrors the two, which is how every fade behaved before this
// existed, so it is the default.
//
// Click the curve to add a control point, drag to shape it, select and press
// Delete to remove it. The maths lives in `lib/curve.ts`, mirroring the Rust
// so what you draw is what the engine plays.

import { useCallback, useRef, useState } from "react";
import type { CurveKind, CurvePoint, CurveShape, FadeShapes } from "../../lib/types";
import { CURVE_KIND_LABELS, curveUsesPoints } from "../../lib/types";
import { resolvedPoints } from "../../lib/curve";
import { bendThrough, curvePath, curveY, sampleCurve, segmentAt } from "../../lib/curve";
import { Select } from "../common/Select";
import { DragNumber } from "../common/DragNumber";

const KINDS: CurveKind[] = ["s_curve", "linear", "parametric", "exponential"];

const inputStyle: React.CSSProperties = {
  background: "var(--wc-bg-app)",
  border: "1px solid var(--wc-border-strong)",
  borderRadius: 4,
  color: "var(--wc-text)",
  fontSize: 12,
  padding: "3px 6px",
};

/** Which point of `shape` is within grabbing distance of (t, v), if any. */
function hitPoint(shape: CurveShape, t: number, v: number, tolerance: number): number {
  return (shape.points ?? []).findIndex(
    (p) => Math.abs(p.t - t) < tolerance && Math.abs(p.v - v) < tolerance * 1.6,
  );
}

function CurvePanel({
  shape,
  size,
  label,
  disabled,
  descending = false,
  onChange,
}: {
  shape: CurveShape;
  size: number;
  label: string;
  disabled?: boolean;
  /** Draw the *value* falling away rather than progress climbing — how a
   *  fade-out actually reads, and how QLab draws its down shape. */
  descending?: boolean;
  onChange: (shape: CurveShape) => void;
}) {
  const svgRef = useRef<SVGSVGElement | null>(null);
  const [dragging, setDragging] = useState<number | null>(null);
  const [selected, setSelected] = useState<number | null>(null);
  /** Segment being bowed with Alt held, if any. */
  const [bowing, setBowing] = useState<number | null>(null);
  const editable = curveUsesPoints(shape.kind) && !disabled;
  const height = Math.round(size * 0.66);

  /** Pointer position as normalised (t, v), with v measured from the bottom. */
  const toCurveSpace = useCallback(
    (event: React.PointerEvent): CurvePoint => {
      const rect = svgRef.current?.getBoundingClientRect();
      if (!rect) return { t: 0, v: 0 };
      const fraction = (event.clientY - rect.top) / rect.height;
      // `v` is always stored as progress; only the drawing is flipped.
      const progress = descending ? fraction : 1 - fraction;
      return {
        t: Math.max(0, Math.min(1, (event.clientX - rect.left) / rect.width)),
        v: Math.max(0, Math.min(1, progress)),
      };
    },
    [descending],
  );

  /** Set the bow of `segment` so the curve passes through (t, v). */
  const bowTo = (segment: number, t: number, v: number) => {
    const points = resolvedPoints(shape);
    const left = points[segment];
    const right = points[segment + 1];
    if (!left || !right) return;
    const span = right.t - left.t;
    const height = right.v - left.v;
    // A flat segment has no chord to bow away from.
    if (span <= 0 || Math.abs(height) < 1e-6) return;
    const local = (t - left.t) / span;
    const target = (v - left.v) / height;
    const bends = [...(shape.bends ?? [])];
    while (bends.length < points.length - 1) bends.push(0);
    bends[segment] = bendThrough(local, target);
    onChange({ ...shape, bends });
  };

  const handleDown = (event: React.PointerEvent) => {
    if (!editable) return;
    const { t, v } = toCurveSpace(event);

    // Alt bends the segment under the cursor instead of adding a point —
    // shaping the curve between two points without cluttering it with a third.
    if (event.altKey) {
      const segment = segmentAt(shape, t);
      setBowing(segment);
      setSelected(null);
      bowTo(segment, t, v);
      event.currentTarget.setPointerCapture(event.pointerId);
      return;
    }

    const existing = hitPoint(shape, t, v, 0.05);
    if (existing >= 0) {
      setSelected(existing);
      setDragging(existing);
    } else {
      // Add where the operator clicked, then keep dragging it. The new point
      // splits a segment in two, so the bend list gains an entry there —
      // otherwise every later segment would inherit its neighbour's bow.
      const points = [...(shape.points ?? []), { t, v }].sort((a, b) => a.t - b.t);
      const index = points.findIndex((p) => p.t === t && p.v === v);
      const split = segmentAt(shape, t);
      const bends = [...(shape.bends ?? [])];
      while (bends.length < resolvedPoints(shape).length - 1) bends.push(0);
      bends.splice(split, 0, bends[split] ?? 0);
      onChange({ ...shape, points, bends });
      setSelected(index);
      setDragging(index);
    }
    event.currentTarget.setPointerCapture(event.pointerId);
  };

  const handleMove = (event: React.PointerEvent) => {
    if (bowing !== null) {
      const { t, v } = toCurveSpace(event);
      bowTo(bowing, t, v);
      return;
    }
    if (dragging === null) return;
    const { t, v } = toCurveSpace(event);
    const points = [...(shape.points ?? [])];
    if (!points[dragging]) return;
    points[dragging] = { t, v };
    // Re-sort so a point dragged past a neighbour keeps the list ordered, and
    // follow it so the drag does not jump to a different point.
    const moved = points[dragging];
    points.sort((a, b) => a.t - b.t);
    setDragging(points.indexOf(moved));
    setSelected(points.indexOf(moved));
    onChange({ ...shape, points });
  };

  const endDrag = () => {
    setDragging(null);
    setBowing(null);
  };

  const handleKey = (event: React.KeyboardEvent) => {
    if (selected === null || !editable) return;
    if (event.key !== "Delete" && event.key !== "Backspace") return;
    event.preventDefault();
    // Removing a point merges two segments; drop one of their bends with it.
    const bends = [...(shape.bends ?? [])];
    bends.splice(selected, 1);
    onChange({
      ...shape,
      points: (shape.points ?? []).filter((_, i) => i !== selected),
      bends,
    });
    setSelected(null);
  };

  const points = shape.points ?? [];

  return (
    <div style={{ flex: "0 1 auto", minWidth: 0 }}>
      <div style={{ fontSize: 10, color: "var(--wc-text-muted)", marginBottom: 3 }}>{label}</div>
      <svg
        ref={svgRef}
        width={size}
        height={height}
        viewBox={`0 0 ${size} ${height}`}
        tabIndex={editable ? 0 : -1}
        onPointerDown={handleDown}
        onPointerMove={handleMove}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
        onKeyDown={handleKey}
        style={{
          // Fixed size, never stretched: a curve is read by its shape, and a
          // panel the width of the window makes a 3-second fade look like a
          // landscape. maxWidth keeps it inside a narrow inspector column.
          maxWidth: "100%",
          height: "auto",
          display: "block",
          background: "var(--wc-bg-app)",
          border: "1px solid var(--wc-border-strong)",
          borderRadius: 4,
          cursor: editable ? "crosshair" : "default",
          opacity: disabled ? 0.45 : 1,
          outline: "none",
          touchAction: "none",
        }}
      >
        {/* Quarter grid — enough to read the shape, quiet enough to ignore. */}
        {[0.25, 0.5, 0.75].map((f) => (
          <g key={f} stroke="var(--wc-border)" strokeWidth={0.5}>
            <line x1={f * size} y1={0} x2={f * size} y2={height} />
            <line x1={0} y1={f * height} x2={size} y2={f * height} />
          </g>
        ))}
        {/* The straight reference, so a shaped curve reads at a glance. */}
        <line
          x1={0} y1={curveY(0, height, descending)}
          x2={size} y2={curveY(1, height, descending)}
          stroke="var(--wc-border-strong)" strokeWidth={0.75} strokeDasharray="3 3"
        />
        <path
          d={curvePath(shape, size, height, 96, descending)}
          fill="none"
          stroke="var(--wc-accent)"
          strokeWidth={2}
        />
        {editable &&
          points.map((point, index) => (
            <circle
              key={index}
              cx={point.t * size}
              cy={curveY(point.v, height, descending)}
              r={index === selected ? 5 : 4}
              fill={index === selected ? "var(--wc-accent)" : "var(--wc-bg-surface)"}
              stroke="var(--wc-accent)"
              strokeWidth={1.5}
            />
          ))}
      </svg>
      {editable && (
        <div style={{ fontSize: 10, color: "var(--wc-text-faint)", marginTop: 3 }}>
          {points.length === 0
            ? "Click to add a point · Alt-drag to bend"
            : `${points.length} point${points.length !== 1 ? "s" : ""} · Delete removes · Alt-drag bends`}
        </div>
      )}
    </div>
  );
}

export function CurveEditor({
  shapes,
  onChange,
  compact = false,
}: {
  shapes: FadeShapes;
  onChange: (shapes: FadeShapes) => void;
  /** Inspector-sized: one panel while mirrored, tighter controls. */
  compact?: boolean;
}) {
  const size = compact ? 132 : 300;

  const setKind = (kind: CurveKind) => {
    // Changing kind resets the points: they mean nothing to an analytic shape,
    // and silently keeping them would resurrect them on switching back.
    const keep = curveUsesPoints(kind);
    const apply = (shape: CurveShape): CurveShape => ({
      ...shape,
      kind,
      points: keep ? shape.points ?? [] : [],
      bends: keep ? shape.bends ?? [] : [],
    });
    onChange({ ...shapes, up: apply(shapes.up), down: apply(shapes.down) });
  };

  const setUp = (up: CurveShape) =>
    onChange({ ...shapes, up, down: shapes.mirrored ? up : shapes.down });
  const setDown = (down: CurveShape) =>
    onChange({ ...shapes, down, up: shapes.mirrored ? down : shapes.up });

  const toggleLock = () => {
    const mirrored = !shapes.mirrored;
    // Locking adopts the rising curve for both, which is what the operator
    // sees on the left and expects to win.
    onChange({ ...shapes, mirrored, down: mirrored ? shapes.up : shapes.down });
  };

  const reset = () =>
    onChange({
      up: { kind: "s_curve", intensity: 0, points: [], bends: [] },
      down: { kind: "s_curve", intensity: 0, points: [], bends: [] },
      mirrored: true,
    });

  const kind = shapes.up.kind;
  const midpoint = Math.round(sampleCurve(shapes.up, 0.5) * 100);

  return (
    <div>
      <div style={{ display: "flex", gap: 6, alignItems: "flex-end", marginBottom: 8, flexWrap: "wrap" }}>
        <div style={{ width: compact ? "100%" : 200, minWidth: 130 }}>
          <div style={{ fontSize: 10, color: "var(--wc-text-muted)", marginBottom: 2 }}>Shape</div>
          <Select
            style={{ ...inputStyle, cursor: "pointer", width: "100%" }}
            value={kind}
            onChange={(e) => setKind(e.target.value as CurveKind)}
          >
            {KINDS.map((k) => (
              <option key={k} value={k}>{CURVE_KIND_LABELS[k]}</option>
            ))}
          </Select>
        </div>
        {kind === "parametric" && (
          <div>
            <div style={{ fontSize: 10, color: "var(--wc-text-muted)", marginBottom: 2 }}>
              Intensity
            </div>
            <DragNumber
              style={{ ...inputStyle, width: 66 }}
              min={-10}
              max={10}
              step={0.25}
              value={shapes.up.intensity ?? 0}
              onChange={(e) => {
                const intensity = Math.max(-10, Math.min(10, parseFloat(e.target.value) || 0));
                onChange({
                  ...shapes,
                  up: { ...shapes.up, intensity },
                  down: { ...shapes.down, intensity },
                });
              }}
            />
          </div>
        )}
        <button
          onClick={toggleLock}
          title={
            shapes.mirrored
              ? "Rising and falling are locked together — click to shape them separately"
              : "Rising and falling are separate — click to lock them together"
          }
          style={{
            padding: "4px 10px",
            background: shapes.mirrored ? "var(--wc-accent)" : "var(--wc-bg-surface)",
            border: "1px solid var(--wc-border-strong)",
            borderRadius: 4,
            color: shapes.mirrored ? "var(--wc-accent-fg)" : "var(--wc-text)",
            fontSize: 12,
            cursor: "pointer",
          }}
        >
          {shapes.mirrored ? "🔒 Locked" : "🔓 Separate"}
        </button>
      </div>

      <div style={{ display: "flex", gap: 10 }}>
        <CurvePanel
          shape={shapes.up}
          size={size}
          label={shapes.mirrored ? "Both directions" : "Rising"}
          onChange={setUp}
        />
        {!shapes.mirrored && (
          <CurvePanel
            shape={shapes.down}
            size={size}
            label="Falling"
            descending
            onChange={setDown}
          />
        )}
      </div>

      <div style={{ display: "flex", alignItems: "center", gap: 10, marginTop: 8, flexWrap: "wrap" }}>
        <button
          onClick={reset}
          style={{
            padding: "3px 10px",
            background: "var(--wc-bg-surface)",
            border: "1px solid var(--wc-border-strong)",
            borderRadius: 4,
            color: "var(--wc-text-secondary)",
            fontSize: 11,
            cursor: "pointer",
          }}
        >
          Reset to Default Shape
        </button>
        <span style={{ fontSize: 11, color: "var(--wc-text-faint)" }}>
          {midpoint}% of the way at halfway
        </span>
      </div>
    </div>
  );
}
