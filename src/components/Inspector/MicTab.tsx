// Inspector tab for Mic cues — routes a live audio input to an Output Patch.

import { useEffect, useState } from "react";
import type { MicCueData, InputPatch, OutputPatch, FadeCurve } from "../../lib/types";
import { listInputPatches, getOutputPatches } from "../../lib/commands";
import { Select } from "../common/Select";

interface Props {
  cue: MicCueData;
  onSave: (partial: Partial<MicCueData>) => void;
}

const inputStyle: React.CSSProperties = {
  background: "#0f172a",
  border: "1px solid #334155",
  borderRadius: 4,
  color: "#e2e8f0",
  fontSize: 12,
  padding: "3px 6px",
};

const selectStyle: React.CSSProperties = { ...inputStyle, cursor: "pointer", width: "100%" };
const labelStyle: React.CSSProperties = { fontSize: 10, color: "#64748b", marginBottom: 2 };
const fieldStyle: React.CSSProperties = { marginBottom: 12 };

const CURVES: FadeCurve[] = ["linear", "s_curve", "exponential"];
const CURVE_LABELS: Record<FadeCurve, string> = {
  linear: "Linear",
  s_curve: "S-Curve",
  exponential: "Exponential",
};

export function MicTab({ cue, onSave }: Props) {
  const [inputPatches, setInputPatches] = useState<InputPatch[]>([]);
  const [outputPatches, setOutputPatches] = useState<OutputPatch[]>([]);

  useEffect(() => {
    listInputPatches().then(setInputPatches).catch(console.error);
    getOutputPatches().then(setOutputPatches).catch(console.error);
  }, []);

  const selectedInput = inputPatches.find((p) => p.id === cue.input_patch_id) ?? null;
  const inputChannelCount = cue.input_channels.length || (selectedInput?.channels.length ?? 0);

  return (
    <div>
      {/* Input Patch */}
      <div style={fieldStyle}>
        <div style={labelStyle}>Input Patch</div>
        <Select
          style={selectStyle}
          value={cue.input_patch_id ?? ""}
          onChange={(e) => onSave({ input_patch_id: e.target.value || null })}
        >
          <option value="">— none —</option>
          {inputPatches.map((p) => (
            <option key={p.id} value={p.id}>{p.name}</option>
          ))}
          {cue.input_patch_id && !selectedInput && (
            <option value={cue.input_patch_id}>(missing patch)</option>
          )}
        </Select>
        {inputPatches.length === 0 && (
          <div style={{ marginTop: 4, fontSize: 11, color: "#64748b" }}>
            No Input Patches yet — add one in the Audio Inputs panel.
          </div>
        )}
      </div>

      {/* Channel mode: mono vs stereo from the patch's channels */}
      <div style={fieldStyle}>
        <div style={labelStyle}>Source channels</div>
        <Select
          style={selectStyle}
          value={String(inputChannelCount === 1 ? 1 : 2)}
          onChange={(e) => {
            const n = parseInt(e.target.value, 10);
            const patchChans = selectedInput?.channels ?? [0, 1];
            const chans = n === 1 ? patchChans.slice(0, 1) : patchChans.slice(0, 2);
            onSave({ input_channels: chans });
          }}
        >
          <option value="1">Mono (1 ch)</option>
          <option value="2">Stereo (2 ch)</option>
        </Select>
      </div>

      {/* Output Patch */}
      <div style={fieldStyle}>
        <div style={labelStyle}>Output Patch</div>
        <Select
          style={selectStyle}
          value={cue.output_patch_id ?? ""}
          onChange={(e) => onSave({ output_patch_id: e.target.value || null })}
        >
          <option value="">— default —</option>
          {outputPatches.map((p) => (
            <option key={p.id} value={p.id}>{p.name}</option>
          ))}
        </Select>
      </div>

      {/* Level + Pan */}
      <div style={{ display: "flex", gap: 12, ...fieldStyle }}>
        <div style={{ flex: 1 }}>
          <div style={labelStyle}>Volume (dB)</div>
          <input
            style={{ ...inputStyle, width: "100%" }}
            type="number"
            min={-60}
            max={12}
            step={0.5}
            value={cue.volume_db}
            onChange={(e) =>
              onSave({ volume_db: Math.max(-60, Math.min(12, parseFloat(e.target.value) || 0)) })
            }
          />
        </div>
        <div style={{ flex: 1 }}>
          <div style={labelStyle}>Pan ({cue.pan.toFixed(2)})</div>
          <input
            style={{ width: "100%" }}
            type="range"
            min={-1}
            max={1}
            step={0.01}
            value={cue.pan}
            onChange={(e) => onSave({ pan: parseFloat(e.target.value) })}
          />
        </div>
      </div>

      {/* Fades */}
      <FadeRow
        label="Fade In"
        ms={cue.fade_in_ms}
        curve={cue.fade_in_curve}
        onChange={(ms, curve) => onSave({ fade_in_ms: ms, fade_in_curve: curve })}
      />
      <FadeRow
        label="Fade Out (also on stop)"
        ms={cue.fade_out_ms}
        curve={cue.fade_out_curve}
        onChange={(ms, curve) => onSave({ fade_out_ms: ms, fade_out_curve: curve })}
      />

      <div style={{ marginTop: 10, fontSize: 11, color: "#64748b" }}>
        Live input runs until the cue is stopped; the capture device is released when it stops.
      </div>
    </div>
  );
}

function FadeRow({
  label,
  ms,
  curve,
  onChange,
}: {
  label: string;
  ms: number | null;
  curve: FadeCurve | null;
  onChange: (ms: number | null, curve: FadeCurve | null) => void;
}) {
  const enabled = ms != null;
  return (
    <div style={fieldStyle}>
      <label style={{ display: "flex", alignItems: "center", gap: 6, ...labelStyle }}>
        <input
          type="checkbox"
          checked={enabled}
          onChange={(e) => onChange(e.target.checked ? 500 : null, e.target.checked ? (curve ?? "s_curve") : null)}
        />
        {label}
      </label>
      {enabled && (
        <div style={{ display: "flex", gap: 6, marginTop: 4 }}>
          <input
            style={{ ...inputStyle, width: 80 }}
            type="number"
            min={0}
            step={50}
            value={ms ?? 0}
            onChange={(e) => onChange(Math.max(0, parseInt(e.target.value, 10) || 0), curve ?? "s_curve")}
          />
          <Select
            style={{ ...selectStyle, width: "auto", flex: 1 }}
            value={curve ?? "s_curve"}
            onChange={(e) => onChange(ms ?? 500, e.target.value as FadeCurve)}
          >
            {CURVES.map((c) => (
              <option key={c} value={c}>{CURVE_LABELS[c]}</option>
            ))}
          </Select>
        </div>
      )}
    </div>
  );
}
