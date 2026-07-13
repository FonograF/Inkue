import type { AudioCueData, CameraCueData, FadeCurve, ImageCueData, VideoCueData } from "../../lib/types";
import { Grid2, MiniField, Section, inputStyle } from "./Field";
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
    <Section title={label}>
      <Grid2>
        <MiniField label="Duration (s)">
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
        </MiniField>
        <MiniField label="Curve">
          <CurveSelect
            value={curve ?? "s_curve"}
            onChange={(v) => onChange({ [`${idPrefix}_curve`]: v })}
          />
        </MiniField>
      </Grid2>
    </Section>
  );
}

export function FadeTab({
  cue,
  onSave,
}: {
  cue: AudioCueData | VideoCueData | ImageCueData | CameraCueData;
  onSave: (p: Partial<AudioCueData | VideoCueData | ImageCueData | CameraCueData>) => void;
}) {
  // Camera: visual fades only (a live feed has no decoded audio voice).
  const isCamera = cue.cue_type === "camera";
  const isVideo = !isCamera && "video_fade_in_ms" in cue;
  const vc = isVideo ? (cue as VideoCueData) : null;

  return (
    <>
      {isCamera && (
        <>
          <FadeSection
            label="Video Fade In"
            durationMs={(cue as CameraCueData).video_fade_in_ms}
            curve={(cue as CameraCueData).video_fade_in_curve}
            idPrefix="video_fade_in"
            onChange={(p) => onSave(p as Partial<CameraCueData>)}
          />
          <FadeSection
            label="Video Fade Out"
            durationMs={(cue as CameraCueData).video_fade_out_ms}
            curve={(cue as CameraCueData).video_fade_out_curve}
            idPrefix="video_fade_out"
            onChange={(p) => onSave(p as Partial<CameraCueData>)}
          />
        </>
      )}

      {isVideo && (
        <>
          <FadeSection
            label="Video Fade In"
            durationMs={vc!.video_fade_in_ms}
            curve={vc!.video_fade_in_curve}
            idPrefix="video_fade_in"
            onChange={(p) => onSave(p as Partial<VideoCueData>)}
          />
          <FadeSection
            label="Video Fade Out"
            durationMs={vc!.video_fade_out_ms}
            curve={vc!.video_fade_out_curve}
            idPrefix="video_fade_out"
            onChange={(p) => onSave(p as Partial<VideoCueData>)}
          />
          <FadeSection
            label="Audio Fade In"
            durationMs={vc!.fade_in_ms}
            curve={vc!.fade_in_curve}
            idPrefix="fade_in"
            onChange={(p) => onSave(p)}
          />
          <FadeSection
            label="Audio Fade Out"
            durationMs={vc!.fade_out_ms}
            curve={vc!.fade_out_curve}
            idPrefix="fade_out"
            onChange={(p) => onSave(p)}
          />
        </>
      )}

      {!isVideo && !isCamera && (
        <>
          <FadeSection
            label="Fade In"
            durationMs={(cue as AudioCueData | ImageCueData).fade_in_ms}
            curve={(cue as AudioCueData | ImageCueData).fade_in_curve}
            idPrefix="fade_in"
            onChange={(p) => onSave(p)}
          />
          <FadeSection
            label="Fade Out"
            durationMs={(cue as AudioCueData | ImageCueData).fade_out_ms}
            curve={(cue as AudioCueData | ImageCueData).fade_out_curve}
            idPrefix="fade_out"
            onChange={(p) => onSave(p)}
          />
        </>
      )}
    </>
  );
}
