import type { AudioCueData, ImageCueData, VideoCueData } from "../../lib/types";
import { Field, inputStyle } from "./Field";
import { CurveSelect } from "../common/CurveSelect";

export function FadeTab({
  cue,
  onSave,
}: {
  cue: AudioCueData | VideoCueData | ImageCueData;
  onSave: (p: Partial<AudioCueData | VideoCueData | ImageCueData>) => void;
}) {
  return (
    <>
      {/* Fade In */}
      <div
        style={{
          marginBottom: 14,
          paddingBottom: 14,
          borderBottom: "1px solid #1e293b",
        }}
      >
        <div
          style={{
            fontSize: 11,
            color: "#64748b",
            marginBottom: 8,
            textTransform: "uppercase",
            letterSpacing: "0.05em",
          }}
        >
          Fade In
        </div>
        <Field label="Duration (s)">
          <input
            style={inputStyle}
            type="number"
            step="0.1"
            min="0"
            defaultValue={
              cue.fade_in_ms != null ? (cue.fade_in_ms / 1000).toFixed(2) : ""
            }
            placeholder="none"
            onBlur={(e) =>
              onSave({
                fade_in_ms: e.target.value
                  ? Math.round(parseFloat(e.target.value) * 1000)
                  : null,
              })
            }
          />
        </Field>
        <Field label="Curve">
          <CurveSelect
            value={cue.fade_in_curve ?? "s_curve"}
            onChange={(v) => onSave({ fade_in_curve: v })}
          />
        </Field>
      </div>

      {/* Fade Out */}
      <div>
        <div
          style={{
            fontSize: 11,
            color: "#64748b",
            marginBottom: 8,
            textTransform: "uppercase",
            letterSpacing: "0.05em",
          }}
        >
          Fade Out
        </div>
        <Field label="Duration (s)">
          <input
            style={inputStyle}
            type="number"
            step="0.1"
            min="0"
            defaultValue={
              cue.fade_out_ms != null
                ? (cue.fade_out_ms / 1000).toFixed(2)
                : ""
            }
            placeholder="none (0.5s on Stop)"
            onBlur={(e) =>
              onSave({
                fade_out_ms: e.target.value
                  ? Math.round(parseFloat(e.target.value) * 1000)
                  : null,
              })
            }
          />
        </Field>
        <Field label="Curve">
          <CurveSelect
            value={cue.fade_out_curve ?? "s_curve"}
            onChange={(v) => onSave({ fade_out_curve: v })}
          />
        </Field>
      </div>
    </>
  );
}
