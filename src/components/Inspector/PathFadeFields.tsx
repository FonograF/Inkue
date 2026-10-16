// Path-mode controls of the Fade Cue's Geometry section: the path's points as
// a table, plus smooth / merge / loop and the axis mapping. The drawn path
// editor (phase 5 of the path-fade plan) will sit alongside the table.

import type { PathGeometry, PathParam } from "../../lib/types";
import { Grid2, MiniField, NumberInput, ToggleRow, inputStyle } from "./Field";
import { Select } from "../common/Select";

/** A new path: a short horizontal move through the centre. */
export const DEFAULT_PATH: PathGeometry = {
  points: [[-0.25, 0], [0.25, 0]],
  smooth: false,
  merge: 0,
  loop_path: false,
  x_param: "translation_x",
  y_param: "translation_y",
};

const PARAMS: { value: PathParam; label: string }[] = [
  { value: "translation_x", label: "Position X" },
  { value: "translation_y", label: "Position Y" },
];

const smallButton = {
  padding: "2px 8px",
  background: "var(--wc-bg-surface)",
  border: "1px solid var(--wc-border-strong)",
  borderRadius: 4,
  color: "var(--wc-text-secondary)",
  fontSize: 11,
  cursor: "pointer",
} as const;

export function PathFadeFields({
  path,
  onChange,
}: {
  path: PathGeometry;
  onChange: (next: PathGeometry) => void;
}) {
  const patch = (partial: Partial<PathGeometry>) => onChange({ ...path, ...partial });
  const setPoint = (index: number, point: [number, number]) =>
    patch({ points: path.points.map((p, i) => (i === index ? point : p)) });
  const removePoint = (index: number) => patch({ points: path.points.filter((_, i) => i !== index) });
  const addPoint = () => {
    const last = path.points[path.points.length - 1] ?? [0, 0];
    patch({ points: [...path.points, [last[0] + 0.1, last[1]]] });
  };

  return (
    <>
      <div style={{ fontSize: 11, color: "var(--wc-text-secondary)", marginBottom: 4 }}>
        Points (± of width / height, + = right / up)
      </div>
      {path.points.map(([x, y], i) => (
        <div key={i} style={{ display: "flex", alignItems: "center", gap: 6, marginBottom: 4 }}>
          <span style={{ width: 34, fontSize: 11, color: "var(--wc-text-faint)" }}>
            {i === 0 ? "Start" : i === path.points.length - 1 ? "End" : i + 1}
          </span>
          <NumberInput value={x} step={0.01} min={-2} max={2} onCommit={(v) => setPoint(i, [v, y])} />
          <NumberInput value={y} step={0.01} min={-2} max={2} onCommit={(v) => setPoint(i, [x, v])} />
          <button
            style={smallButton}
            title="Remove this point"
            disabled={path.points.length <= 2}
            onClick={() => removePoint(i)}
          >
            ✕
          </button>
        </div>
      ))}
      <div style={{ display: "flex", gap: 6, margin: "6px 0 10px" }}>
        <button style={smallButton} onClick={addPoint}>+ Add point</button>
        <button
          style={smallButton}
          title="Same points, travelled the other way"
          onClick={() => patch({ points: [...path.points].reverse() })}
        >
          Reverse
        </button>
      </div>

      <ToggleRow label="Smooth path" checked={path.smooth} onToggle={(v) => patch({ smooth: v })} />
      <ToggleRow label="Loop until stopped (Time = one pass)" checked={path.loop_path} onToggle={(v) => patch({ loop_path: v })} />
      <Grid2>
        <MiniField label="Merge (%)">
          <NumberInput
            value={Math.round(path.merge * 100)}
            step={1}
            min={0}
            max={100}
            onCommit={(v) => patch({ merge: v / 100 })}
          />
        </MiniField>
        <div />
        <MiniField label="Horizontal axis">
          <Select
            style={inputStyle}
            value={path.x_param}
            onChange={(e) => patch({ x_param: e.target.value as PathParam })}
          >
            {PARAMS.map((p) => <option key={p.value} value={p.value}>{p.label}</option>)}
          </Select>
        </MiniField>
        <MiniField label="Vertical axis">
          <Select
            style={inputStyle}
            value={path.y_param}
            onChange={(e) => patch({ y_param: e.target.value as PathParam })}
          >
            {PARAMS.map((p) => <option key={p.value} value={p.value}>{p.label}</option>)}
          </Select>
        </MiniField>
      </Grid2>
    </>
  );
}
