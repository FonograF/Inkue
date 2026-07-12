// Geometry tab for Video and Image cues: fit mode, position, scale,
// rotation and crop. Edits apply live when the cue is on the output window.

import type { BlendMode, CameraCueData, FitMode, ImageCueData, LayerStyle, VideoCueData, VideoGeometry } from "../../lib/types";
import { DEFAULT_GEOMETRY, DEFAULT_LAYER_STYLE } from "../../lib/types";
import { Field, inputStyle } from "./Field";
import { Select } from "../common/Select";

const BLEND_MODES: { value: BlendMode; label: string }[] = [
  { value: "normal", label: "Normal" },
  { value: "add", label: "Add" },
  { value: "multiply", label: "Multiply" },
  { value: "screen", label: "Screen" },
  { value: "overlay", label: "Overlay" },
  { value: "soft_light", label: "Soft Light" },
  { value: "hard_light", label: "Hard Light" },
  { value: "darken", label: "Darken" },
  { value: "lighten", label: "Lighten" },
  { value: "color_dodge", label: "Color Dodge" },
  { value: "color_burn", label: "Color Burn" },
  { value: "difference", label: "Difference" },
  { value: "exclusion", label: "Exclusion" },
  { value: "subtract", label: "Subtract" },
];

const FIT_MODES: { value: FitMode; label: string; hint: string }[] = [
  { value: "fit", label: "Fit", hint: "keep aspect, letterbox" },
  { value: "fill", label: "Fill", hint: "keep aspect, crop overflow" },
  { value: "stretch", label: "Stretch", hint: "ignore aspect ratio" },
];

const sectionTitleStyle: React.CSSProperties = {
  fontSize: 11,
  color: "var(--wc-text-muted)",
  margin: "14px 0 8px",
  textTransform: "uppercase",
  letterSpacing: "0.05em",
};

function NumberField({
  label,
  value,
  step,
  min,
  max,
  suffix,
  onCommit,
}: {
  label: string;
  value: number;
  step: number;
  min: number;
  max: number;
  suffix?: string;
  onCommit: (v: number) => void;
}) {
  return (
    <Field label={label}>
      <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
        <input
          style={{ ...inputStyle, width: 90 }}
          type="number"
          step={step}
          min={min}
          max={max}
          key={`${label}-${value}`}
          defaultValue={value}
          onBlur={(e) => {
            const parsed = parseFloat(e.target.value);
            if (Number.isNaN(parsed)) return;
            onCommit(Math.min(max, Math.max(min, parsed)));
          }}
        />
        {suffix && (
          <span style={{ color: "var(--wc-text-muted)", fontSize: 12 }}>{suffix}</span>
        )}
      </div>
    </Field>
  );
}

export function GeometryTab({
  cue,
  onSave,
}: {
  cue: VideoCueData | ImageCueData | CameraCueData;
  onSave: (p: Partial<VideoCueData | ImageCueData | CameraCueData>) => void;
}) {
  const geometry: VideoGeometry = cue.geometry ?? DEFAULT_GEOMETRY;
  const patch = (partial: Partial<VideoGeometry>) =>
    onSave({ geometry: { ...geometry, ...partial } });

  const layerStyle: LayerStyle = cue.layer_style ?? DEFAULT_LAYER_STYLE;
  const patchLayer = (partial: Partial<LayerStyle>) =>
    onSave({ layer_style: { ...layerStyle, ...partial } });

  const isDefault =
    geometry.fit_mode === "fit" &&
    geometry.pan_x === 0 &&
    geometry.pan_y === 0 &&
    geometry.scale === 1 &&
    geometry.rotation === 0 &&
    geometry.crop_left === 0 &&
    geometry.crop_right === 0 &&
    geometry.crop_top === 0 &&
    geometry.crop_bottom === 0;

  return (
    <>
      <div style={{ ...sectionTitleStyle, marginTop: 0 }}>Compositing</div>
      <Field label="Layer">
        <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
          <input
            type="checkbox"
            checked={layerStyle.layer === null}
            onChange={(e) => patchLayer({ layer: e.target.checked ? null : 500 })}
          />
          {layerStyle.layer === null ? (
            <span style={{ color: "var(--wc-text-muted)", fontSize: 12 }}>
              automatic (newest on top)
            </span>
          ) : (
            <input
              style={{ ...inputStyle, width: 80 }}
              type="number"
              min={1}
              max={1000}
              step={1}
              key={`layer-${layerStyle.layer}`}
              defaultValue={layerStyle.layer}
              onBlur={(e) => {
                const parsed = parseInt(e.target.value, 10);
                if (Number.isNaN(parsed)) return;
                patchLayer({ layer: Math.min(1000, Math.max(1, parsed)) });
              }}
            />
          )}
        </div>
      </Field>
      <Field label="Opacity">
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <input
            type="range"
            min={0}
            max={1}
            step={0.01}
            value={layerStyle.opacity}
            onChange={(e) => patchLayer({ opacity: parseFloat(e.target.value) })}
            style={{ flex: 1, cursor: "pointer" }}
          />
          <span style={{ width: 40, textAlign: "right", fontSize: 12, color: "var(--wc-text-secondary)" }}>
            {Math.round(layerStyle.opacity * 100)}%
          </span>
        </div>
      </Field>
      <Field label="Blend Mode">
        <Select
          style={{ ...inputStyle, cursor: "pointer" }}
          value={layerStyle.blend_mode}
          onChange={(e) => patchLayer({ blend_mode: e.target.value as BlendMode })}
        >
          {BLEND_MODES.map((m) => (
            <option key={m.value} value={m.value}>{m.label}</option>
          ))}
        </Select>
      </Field>
      <div style={sectionTitleStyle}>Fit</div>
      <div style={{ display: "flex", gap: 6, marginBottom: 4 }}>
        {FIT_MODES.map((m) => (
          <button
            key={m.value}
            title={m.hint}
            onClick={() => patch({ fit_mode: m.value })}
            style={{
              flex: 1,
              padding: "5px 0",
              fontSize: 12,
              borderRadius: 4,
              cursor: "pointer",
              border:
                geometry.fit_mode === m.value
                  ? "1px solid var(--wc-accent)"
                  : "1px solid var(--wc-border-strong)",
              background:
                geometry.fit_mode === m.value ? "var(--wc-accent)" : "var(--wc-bg-surface)",
              color:
                geometry.fit_mode === m.value ? "var(--wc-accent-fg)" : "var(--wc-text)",
            }}
          >
            {m.label}
          </button>
        ))}
      </div>
      <div style={{ fontSize: 11, color: "var(--wc-text-faint)", marginBottom: 6 }}>
        {FIT_MODES.find((m) => m.value === geometry.fit_mode)?.hint}
      </div>

      <div style={sectionTitleStyle}>Position &amp; Scale</div>
      <NumberField
        label="Position X"
        value={geometry.pan_x}
        step={0.01}
        min={-1}
        max={1}
        suffix="of width"
        onCommit={(v) => patch({ pan_x: v })}
      />
      <NumberField
        label="Position Y"
        value={geometry.pan_y}
        step={0.01}
        min={-1}
        max={1}
        suffix="of height"
        onCommit={(v) => patch({ pan_y: v })}
      />
      <NumberField
        label="Scale"
        value={geometry.scale}
        step={0.05}
        min={0.05}
        max={8}
        suffix="× (1 = 100%)"
        onCommit={(v) => patch({ scale: v })}
      />
      <NumberField
        label="Rotation"
        value={geometry.rotation}
        step={90}
        min={0}
        max={359}
        suffix="° clockwise"
        onCommit={(v) => patch({ rotation: Math.round(v) })}
      />

      <div style={sectionTitleStyle}>Crop (fraction of each edge)</div>
      <NumberField
        label="Left"
        value={geometry.crop_left}
        step={0.01}
        min={0}
        max={0.45}
        onCommit={(v) => patch({ crop_left: v })}
      />
      <NumberField
        label="Right"
        value={geometry.crop_right}
        step={0.01}
        min={0}
        max={0.45}
        onCommit={(v) => patch({ crop_right: v })}
      />
      <NumberField
        label="Top"
        value={geometry.crop_top}
        step={0.01}
        min={0}
        max={0.45}
        onCommit={(v) => patch({ crop_top: v })}
      />
      <NumberField
        label="Bottom"
        value={geometry.crop_bottom}
        step={0.01}
        min={0}
        max={0.45}
        onCommit={(v) => patch({ crop_bottom: v })}
      />

      <button
        disabled={isDefault}
        onClick={() => onSave({ geometry: { ...DEFAULT_GEOMETRY } })}
        style={{
          marginTop: 10,
          padding: "5px 14px",
          fontSize: 12,
          borderRadius: 4,
          border: "1px solid var(--wc-border-strong)",
          background: "var(--wc-bg-surface)",
          color: isDefault ? "var(--wc-text-faint)" : "var(--wc-text)",
          cursor: isDefault ? "default" : "pointer",
        }}
      >
        Reset to defaults
      </button>
      <div style={{ fontSize: 11, color: "var(--wc-text-faint)", marginTop: 8 }}>
        Changes apply live while this cue is on the output window.
      </div>
    </>
  );
}
