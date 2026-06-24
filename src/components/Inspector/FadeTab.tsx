import type { AudioCueData, FadeCurve, ImageCueData, VideoCueData } from "../../lib/types";
import { Field, inputStyle } from "./Field";
import { CurveSelect } from "../common/CurveSelect";

function FadeSection({
  label,
  durationMs,
  curve,
  idPrefix,
  onChange,
}: {
  label: string;
  durationMs: number | null;
  curve: FadeCurve | null;
  idPrefix: string;
  onChange: (patch: Record<string, unknown>) => void;
}) {
  return (
    <div
      style={{
        marginBottom: 14,
        paddingBottom: 14,
        borderBottom: "1px solid var(--wc-border)",
      }}
    >
      <div
        style={{
          fontSize: 11,
          color: "var(--wc-text-muted)",
          marginBottom: 8,
          textTransform: "uppercase",
          letterSpacing: "0.05em",
        }}
      >
        {label}
      </div>
      <Field label="Duration (s)">
        <input
          key={`${idPrefix}-dur`}
          style={inputStyle}
          type="number"
          step="0.1"
          min="0"
          defaultValue={durationMs != null ? (durationMs / 1000).toFixed(2) : ""}
          placeholder="none"
          onBlur={(e) =>
            onChange({
              [`${idPrefix}_ms`]: e.target.value
                ? Math.round(parseFloat(e.target.value) * 1000)
                : null,
            })
          }
        />
      </Field>
      <Field label="Curve">
        <CurveSelect
          value={curve ?? "s_curve"}
          onChange={(v) => onChange({ [`${idPrefix}_curve`]: v })}
        />
      </Field>
    </div>
  );
}

export function FadeTab({
  cue,
  onSave,
}: {
  cue: AudioCueData | VideoCueData | ImageCueData;
  onSave: (p: Partial<AudioCueData | VideoCueData | ImageCueData>) => void;
}) {
  const isVideo = "video_fade_in_ms" in cue;
  const vc = isVideo ? (cue as VideoCueData) : null;

  return (
    <>
      {isVideo && (
        <>
          <FadeSection
            label="Image Fade In"
            durationMs={vc!.video_fade_in_ms}
            curve={vc!.video_fade_in_curve}
            idPrefix="video_fade_in"
            onChange={(p) => onSave(p as Partial<VideoCueData>)}
          />
          <FadeSection
            label="Image Fade Out"
            durationMs={vc!.video_fade_out_ms}
            curve={vc!.video_fade_out_curve}
            idPrefix="video_fade_out"
            onChange={(p) => onSave(p as Partial<VideoCueData>)}
          />
          <FadeSection
            label="Audio Fade In"
            durationMs={cue.fade_in_ms}
            curve={cue.fade_in_curve}
            idPrefix="fade_in"
            onChange={(p) => onSave(p)}
          />
          <FadeSection
            label="Audio Fade Out"
            durationMs={cue.fade_out_ms}
            curve={cue.fade_out_curve}
            idPrefix="fade_out"
            onChange={(p) => onSave(p)}
          />
        </>
      )}

      {!isVideo && (
        <>
          <FadeSection
            label="Fade In"
            durationMs={cue.fade_in_ms}
            curve={cue.fade_in_curve}
            idPrefix="fade_in"
            onChange={(p) => onSave(p)}
          />
          <FadeSection
            label="Fade Out"
            durationMs={cue.fade_out_ms}
            curve={cue.fade_out_curve}
            idPrefix="fade_out"
            onChange={(p) => onSave(p)}
          />
        </>
      )}
    </>
  );
}
