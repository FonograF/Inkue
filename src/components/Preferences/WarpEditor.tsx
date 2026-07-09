// Visual corner-pin editor for the global output transform.
// An SVG stage shows the output frame; drag the four corner handles to pin
// corners (perspective warp), drag the centre to pan. Mirrors the Rust warp
// math (warp.rs) in y-down normalized space so the preview matches the render.

import { useRef } from "react";
import type { OutputTransform } from "../../lib/types";

const W = 320;
const H = 180;
const PAD = 42;

type Pt = [number, number];

/** Warped quad corners in stage px, order TL, TR, BR, BL (mirrors warp.rs). */
function quadPoints(t: OutputTransform, withOffsets: boolean): Pt[] {
  const base: Pt[] = [[0, 0], [1, 0], [1, 1], [0, 1]]; // TL TR BR BL, y-down
  // Storage order TL, TR, BL, BR → quad order TL, TR, BR, BL.
  const offsets = [t.corners[0], t.corners[1], t.corners[3], t.corners[2]];
  const rad = (t.rotation * Math.PI) / 180;
  const sin = Math.sin(rad);
  const cos = Math.cos(rad);
  const scale = Math.max(t.scale, 0.01);

  return base.map((p, i) => {
    const x = (p[0] - 0.5) * scale;
    const y = (p[1] - 0.5) * scale;
    const xr = x * cos - y * sin;
    const yr = x * sin + y * cos;
    const fx = xr + 0.5 + t.pan_x + (withOffsets ? offsets[i][0] : 0);
    const fy = yr + 0.5 + t.pan_y + (withOffsets ? offsets[i][1] : 0);
    return [fx * W, fy * H];
  });
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

export function WarpEditor({
  transform,
  onChange,
}: {
  transform: OutputTransform;
  onChange: (t: OutputTransform) => void;
}) {
  const svgRef = useRef<SVGSVGElement>(null);
  // Snapshot taken at drag start so the whole drag is relative to it.
  const dragRef = useRef<{ kind: "corner" | "center"; index: number } | null>(null);

  const stagePoint = (e: React.PointerEvent): Pt => {
    const svg = svgRef.current;
    if (!svg) return [0, 0];
    const ctm = svg.getScreenCTM();
    if (!ctm) return [0, 0];
    const p = new DOMPoint(e.clientX, e.clientY).matrixTransform(ctm.inverse());
    return [p.x, p.y];
  };

  const quad = quadPoints(transform, true);
  const quadBase = quadPoints(transform, false);
  // Quad → storage index (quad TL,TR,BR,BL ↔ storage TL,TR,BL,BR).
  const storageIndex = [0, 1, 3, 2];

  const handlePointerMove = (e: React.PointerEvent) => {
    const drag = dragRef.current;
    if (!drag) return;
    const [px, py] = stagePoint(e);
    if (drag.kind === "center") {
      onChange({
        ...transform,
        pan_x: clamp(px / W - 0.5, -0.9, 0.9),
        pan_y: clamp(py / H - 0.5, -0.9, 0.9),
      });
      return;
    }
    const base = quadPoints(transform, false)[drag.index];
    const corners = transform.corners.map((c) => [...c]) as OutputTransform["corners"];
    corners[storageIndex[drag.index]] = [
      clamp(px / W - base[0] / W, -0.75, 0.75),
      clamp(py / H - base[1] / H, -0.75, 0.75),
    ];
    onChange({ ...transform, corners });
  };

  const endDrag = (e: React.PointerEvent) => {
    if (dragRef.current) {
      dragRef.current = null;
      (e.currentTarget as Element).releasePointerCapture?.(e.pointerId);
    }
  };

  const startDrag = (kind: "corner" | "center", index: number) => (e: React.PointerEvent) => {
    e.preventDefault();
    dragRef.current = { kind, index };
    (e.currentTarget as Element).setPointerCapture?.(e.pointerId);
  };

  const center: Pt = [(0.5 + transform.pan_x) * W, (0.5 + transform.pan_y) * H];
  const cornerLabels = ["TL", "TR", "BR", "BL"];

  return (
    <svg
      ref={svgRef}
      viewBox={`${-PAD} ${-PAD} ${W + 2 * PAD} ${H + 2 * PAD}`}
      style={{
        width: "100%",
        maxWidth: 460,
        display: "block",
        background: "var(--wc-bg-deepest)",
        border: "1px solid var(--wc-border-strong)",
        borderRadius: 6,
        touchAction: "none",
        userSelect: "none",
      }}
      onPointerMove={handlePointerMove}
      onPointerUp={endDrag}
      onPointerCancel={endDrag}
    >
      {/* Screen frame */}
      <rect x={0} y={0} width={W} height={H} fill="var(--wc-bg-app)" stroke="var(--wc-border)" />
      {/* Reference grid */}
      {[1, 2, 3].map((i) => (
        <line key={`v${i}`} x1={(W / 4) * i} y1={0} x2={(W / 4) * i} y2={H}
          stroke="var(--wc-border)" strokeWidth={0.5} />
      ))}
      {[1, 2, 3].map((i) => (
        <line key={`h${i}`} x1={0} y1={(H / 4) * i} x2={W} y2={(H / 4) * i}
          stroke="var(--wc-border)" strokeWidth={0.5} />
      ))}

      {/* Base (un-pinned) quad for reference while corners are offset */}
      <polygon
        points={quadBase.map((p) => p.join(",")).join(" ")}
        fill="none"
        stroke="var(--wc-text-faint)"
        strokeWidth={0.8}
        strokeDasharray="4 4"
      />

      {/* Warped output quad */}
      <polygon
        points={quad.map((p) => p.join(",")).join(" ")}
        fill="var(--wc-accent)"
        fillOpacity={0.14}
        stroke="var(--wc-accent)"
        strokeWidth={1.5}
      />

      {/* Centre pan handle */}
      <g onPointerDown={startDrag("center", 0)} style={{ cursor: "move" }}>
        <circle cx={center[0]} cy={center[1]} r={12} fill="transparent" />
        <line x1={center[0] - 7} y1={center[1]} x2={center[0] + 7} y2={center[1]}
          stroke="var(--wc-accent)" strokeWidth={1.5} />
        <line x1={center[0]} y1={center[1] - 7} x2={center[0]} y2={center[1] + 7}
          stroke="var(--wc-accent)" strokeWidth={1.5} />
      </g>

      {/* Corner handles */}
      {quad.map((p, i) => {
        const pinned =
          transform.corners[storageIndex[i]][0] !== 0 ||
          transform.corners[storageIndex[i]][1] !== 0;
        return (
          <g key={i} onPointerDown={startDrag("corner", i)} style={{ cursor: "grab" }}>
            <circle cx={p[0]} cy={p[1]} r={13} fill="transparent" />
            <circle
              cx={p[0]} cy={p[1]} r={6}
              fill={pinned ? "var(--wc-accent)" : "var(--wc-bg-surface)"}
              stroke="var(--wc-accent)"
              strokeWidth={1.5}
            />
            <text
              x={p[0]} y={p[1] - 11}
              textAnchor="middle"
              fontSize={9}
              fill="var(--wc-text-muted)"
              style={{ pointerEvents: "none" }}
            >
              {cornerLabels[i]}
            </text>
          </g>
        );
      })}
    </svg>
  );
}
