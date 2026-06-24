import type { AudioCueData, CueSummary } from "../../lib/types";
import { Field, inputStyle } from "./Field";
import { WaveformViewer } from "./WaveformViewer";
import { ScrubBar } from "./ScrubBar";

const LOOP_INFINITE = 4294967295; // u32::MAX

export function TimeTab({
  cue,
  selectedCue,
  isAudio,
  isVideo,
  isImage,
  isWait,
  isFade,
  onSave,
  onOpenWaveform,
}: {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  cue: any;
  selectedCue: CueSummary | null;
  isAudio: boolean;
  isVideo?: boolean;
  isImage?: boolean;
  isWait?: boolean;
  isFade?: boolean;
  onSave: (p: Partial<AudioCueData>) => void;
  onOpenWaveform: () => void;
}) {
  const liveState = selectedCue?.state ?? "standby";
  const liveDurationMs = selectedCue?.duration_ms ?? cue.duration_ms ?? null;
  // file_duration_ms = duration of one loop iteration (no loop multiplier).
  const fileDurationMs: number | null = selectedCue?.file_duration_ms ?? cue.file_duration_ms ?? null;
  // Detect looping: either infinite (duration null but file known) or finite multi-loop.
  const isLooping = fileDurationMs != null && (liveDurationMs == null || fileDurationMs < liveDurationMs);
  // Duration to use for the scrub bar: single iteration when looping, total otherwise.
  const scrubDurationMs = isLooping ? fileDurationMs : liveDurationMs;
  const showScrubber =
    (isAudio || isVideo) &&
    scrubDurationMs != null &&
    scrubDurationMs > 0 &&
    (liveState === "running" || liveState === "paused");

  return (
    <>
      {showScrubber && (
        <ScrubBar
          cueId={cue.id}
          durationMs={scrubDurationMs!}
          cueState={liveState}
          loopDurationMs={isLooping ? fileDurationMs! : undefined}
        />
      )}
      {isWait && (
        <Field label="Duration (s)">
          <input
            style={inputStyle}
            type="number"
            step="0.1"
            min="0"
            key={`wait-${cue.wait_duration_ms}`}
            defaultValue={((cue.wait_duration_ms ?? 5000) / 1000).toFixed(1)}
            onBlur={(e) =>
              onSave({
                wait_duration_ms: Math.round(parseFloat(e.target.value) * 1000),
              } as never)
            }
          />
        </Field>
      )}
      {isFade && (
        <Field label="Duration (s)">
          <input
            style={inputStyle}
            type="number"
            step="0.1"
            min="0.1"
            key={`fade-dur-${cue.fade_duration_ms}`}
            defaultValue={((cue.fade_duration_ms ?? 2000) / 1000).toFixed(1)}
            onBlur={(e) =>
              onSave({
                fade_duration_ms: Math.round(parseFloat(e.target.value) * 1000),
              } as never)
            }
          />
        </Field>
      )}
      {isImage && (
        <Field label="Display Duration">
          <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
            <input
              type="checkbox"
              id="img-dur-enabled"
              checked={cue.display_duration_ms != null}
              onChange={(e) =>
                onSave({
                  display_duration_ms: e.target.checked ? 5000 : null,
                } as never)
              }
            />
            {cue.display_duration_ms != null ? (
              <input
                style={{ ...inputStyle, width: 80 }}
                type="number"
                step="0.1"
                min="0.1"
                key={`img-dur-${cue.display_duration_ms}`}
                defaultValue={(cue.display_duration_ms / 1000).toFixed(1)}
                onBlur={(e) =>
                  onSave({
                    display_duration_ms: Math.round(parseFloat(e.target.value) * 1000),
                  } as never)
                }
              />
            ) : (
              <span style={{ color: "var(--wc-text-muted)", fontSize: 12 }}>∞ hold</span>
            )}
            {cue.display_duration_ms != null && (
              <span style={{ color: "var(--wc-text-muted)", fontSize: 12 }}>s</span>
            )}
          </div>
        </Field>
      )}
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
          <Field label="Loop">
            <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
              <input
                type="checkbox"
                checked={cue.loop_count > 0}
                onChange={(e) =>
                  onSave({ loop_count: e.target.checked ? 1 : 0 })
                }
              />
              {cue.loop_count > 0 && cue.loop_count < LOOP_INFINITE && (
                <input
                  style={{ ...inputStyle, width: 56 }}
                  type="number"
                  min="1"
                  key={`loop-${cue.loop_count}`}
                  defaultValue={cue.loop_count}
                  onBlur={(e) => {
                    const v = parseInt(e.target.value, 10);
                    onSave({ loop_count: v >= 1 ? v : 1 });
                  }}
                />
              )}
              {cue.loop_count === LOOP_INFINITE && (
                <span style={{ fontSize: 16, lineHeight: 1 }}>∞</span>
              )}
              {cue.loop_count > 0 && (
                <button
                  title={cue.loop_count === LOOP_INFINITE ? "Set finite loop count" : "Loop infinitely"}
                  onClick={() =>
                    onSave({
                      loop_count:
                        cue.loop_count === LOOP_INFINITE ? 1 : LOOP_INFINITE,
                    })
                  }
                  style={{
                    background: cue.loop_count === LOOP_INFINITE ? "var(--wc-accent)" : "transparent",
                    border: "1px solid var(--wc-accent)",
                    borderRadius: 4,
                    color: cue.loop_count === LOOP_INFINITE ? "var(--wc-accent-fg)" : "var(--wc-accent)",
                    cursor: "pointer",
                    fontSize: 13,
                    padding: "1px 6px",
                    lineHeight: 1.4,
                  }}
                >
                  ∞
                </button>
              )}
            </div>
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
