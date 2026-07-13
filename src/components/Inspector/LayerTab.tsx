// Layer tab for visual cues (Video / Image / Camera): how this cue composites
// with the other layers on the output — stacking order, opacity, blend mode.

import type { BlendMode, CameraCueData, ImageCueData, LayerStyle, VideoCueData } from "../../lib/types";
import { DEFAULT_LAYER_STYLE } from "../../lib/types";
import { NumberInput, Section, SliderRow, ToggleRow, inputStyle } from "./Field";
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

export function LayerTab({
  cue,
  onSave,
}: {
  cue: VideoCueData | ImageCueData | CameraCueData;
  onSave: (p: Partial<VideoCueData | ImageCueData | CameraCueData>) => void;
}) {
  const layerStyle: LayerStyle = cue.layer_style ?? DEFAULT_LAYER_STYLE;
  const patch = (partial: Partial<LayerStyle>) =>
    onSave({ layer_style: { ...layerStyle, ...partial } });

  return (
    <Section
      title="Compositing"
      hint="Visual cues stack as layers on the output; changes apply live."
    >
      <ToggleRow
        label="Automatic layer order (newest on top)"
        checked={layerStyle.layer === null}
        onToggle={(v) => patch({ layer: v ? null : 500 })}
      />
      {layerStyle.layer !== null && (
        <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 10, paddingLeft: 23 }}>
          <span style={{ fontSize: 12, color: "var(--wc-text-secondary)" }}>Layer (1–1000, higher = in front)</span>
          <NumberInput
            value={layerStyle.layer}
            step={1}
            min={1}
            max={1000}
            width={80}
            onCommit={(v) => patch({ layer: Math.round(v) })}
          />
        </div>
      )}
      <SliderRow
        label="Opacity"
        value={layerStyle.opacity}
        min={0}
        max={1}
        step={0.01}
        format={(v) => `${Math.round(v * 100)}%`}
        onChange={(v) => patch({ opacity: v })}
      />
      <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 10 }}>
        <label style={{ width: 100, color: "var(--wc-text-secondary)", flexShrink: 0, fontSize: 12 }}>
          Blend Mode
        </label>
        <Select
          style={{ ...inputStyle, cursor: "pointer" }}
          value={layerStyle.blend_mode}
          onChange={(e) => patch({ blend_mode: e.target.value as BlendMode })}
        >
          {BLEND_MODES.map((m) => (
            <option key={m.value} value={m.value}>{m.label}</option>
          ))}
        </Select>
      </div>
    </Section>
  );
}
