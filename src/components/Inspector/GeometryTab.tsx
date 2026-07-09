// Geometry tab for Video and Image cues: fit mode, position, scale,
// rotation and crop. Edits apply live when the cue is on the output window.

import type { CameraCueData, FitMode, ImageCueData, VideoCueData, VideoGeometry } from "../../lib/types";
import { DEFAULT_GEOMETRY } from "../../lib/types";
import { Field, inputStyle } from "./Field";

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
      <div style={{ ...sectionTitleStyle, marginTop: 0 }}>Fit</div>
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
