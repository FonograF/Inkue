import type { AudioCueData } from "../../lib/types";
import { Field, inputStyle } from "./Field";
import { WaveformViewer } from "./WaveformViewer";

export function TimeTab({
  cue,
  isAudio,
  isVideo,
  onSave,
  onOpenWaveform,
}: {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  cue: any;
  isAudio: boolean;
  isVideo?: boolean;
  onSave: (p: Partial<AudioCueData>) => void;
  onOpenWaveform: () => void;
}) {
  return (
    <>
      <Field label="Pre-Wait (s)">
        <input
          style={inputStyle}
          type="number"
          step="0.1"
          min="0"
          defaultValue={(cue.pre_wait_ms / 1000).toFixed(1)}
          onBlur={(e) =>
            onSave({
              pre_wait_ms: Math.round(parseFloat(e.target.value) * 1000),
            })
          }
        />
      </Field>
      <Field label="Post-Wait (s)">
        <input
          style={inputStyle}
          type="number"
          step="0.1"
          min="0"
          defaultValue={(cue.post_wait_ms / 1000).toFixed(1)}
          onBlur={(e) =>
            onSave({
              post_wait_ms: Math.round(parseFloat(e.target.value) * 1000),
            })
          }
        />
      </Field>
      {isAudio && cue.file_path && (
        <WaveformViewer cue={cue} onSave={onSave} onExpand={onOpenWaveform} />
      )}
      {(isAudio || isVideo) && (
        <>
          <Field label="Start Time (s)">
            <input
              style={inputStyle}
              type="number"
              step="0.001"
              min="0"
              key={`start-${cue.start_time_ms}`}
              defaultValue={
                cue.start_time_ms != null
                  ? (cue.start_time_ms / 1000).toFixed(3)
                  : ""
              }
              placeholder="0.000"
              onBlur={(e) =>
                onSave({
                  start_time_ms: e.target.value
                    ? Math.round(parseFloat(e.target.value) * 1000)
                    : null,
                })
              }
            />
          </Field>
          <Field label="End Time (s)">
            <input
              style={inputStyle}
              type="number"
              step="0.001"
              min="0"
              key={`end-${cue.end_time_ms}`}
              defaultValue={
                cue.end_time_ms != null
                  ? (cue.end_time_ms / 1000).toFixed(3)
                  : ""
              }
              placeholder="end of file"
              onBlur={(e) =>
                onSave({
                  end_time_ms: e.target.value
                    ? Math.round(parseFloat(e.target.value) * 1000)
                    : null,
                })
              }
            />
          </Field>
          <Field label="Loop Count">
            <input
              style={inputStyle}
              type="number"
              min="0"
              defaultValue={cue.loop_count}
              onBlur={(e) =>
                onSave({ loop_count: parseInt(e.target.value, 10) })
              }
            />
          </Field>
          {isAudio && (
            <Field label="Rate">
              <input
                style={inputStyle}
                type="number"
                step="0.1"
                min="0.1"
                max="4.0"
                defaultValue={cue.rate}
                onBlur={(e) => onSave({ rate: parseFloat(e.target.value) })}
              />
            </Field>
          )}
        </>
      )}
    </>
  );
}
