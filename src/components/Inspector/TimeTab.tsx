import { useEffect, useState } from "react";
import type { AudioCueData, VideoCueData, CueSummary } from "../../lib/types";
import { Grid2, MiniField, NumberInput, Section, ToggleRow, inputStyle } from "./Field";
import { WaveformViewer } from "./WaveformViewer";
import { VideoTrimmer } from "./VideoTrimmer";
import { ScrubBar } from "./ScrubBar";
import { DragNumber } from "../common/DragNumber";

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
        <Section title="Duration">
          <Grid2>
            <MiniField label="Wait (s)">
              <NumberInput
                value={(cue.wait_duration_ms ?? 5000) / 1000}
                step={0.1}
                min={0}
                max={86400}
                onCommit={(v) => onSave({ wait_duration_ms: Math.round(v * 1000) } as never)}
              />
            </MiniField>
          </Grid2>
        </Section>
      )}

      {isFade && (
        <Section title="Duration">
          <Grid2>
            <MiniField label="Fade (s)">
              <NumberInput
                value={(cue.fade_duration_ms ?? 2000) / 1000}
                step={0.1}
                min={0.1}
                max={3600}
                onCommit={(v) => onSave({ fade_duration_ms: Math.round(v * 1000) } as never)}
              />
            </MiniField>
          </Grid2>
        </Section>
      )}

      {isImage && (
        <Section title="Display">
          <ToggleRow
            label="Limit display duration"
            checked={cue.display_duration_ms != null}
            onToggle={(v) => onSave({ display_duration_ms: v ? 5000 : null } as never)}
          >
            <Grid2>
              <MiniField label="Duration (s)">
                <NumberInput
                  value={(cue.display_duration_ms ?? 5000) / 1000}
                  step={0.1}
                  min={0.1}
                  max={86400}
                  onCommit={(v) => onSave({ display_duration_ms: Math.round(v * 1000) } as never)}
                />
              </MiniField>
            </Grid2>
          </ToggleRow>
          {cue.display_duration_ms == null && (
            <div style={{ fontSize: 11, color: "var(--wc-text-faint)", marginTop: -4, marginBottom: 6 }}>
              ∞ — holds until stopped or replaced.
            </div>
          )}
        </Section>
      )}

      <Section title="Waits">
        <Grid2>
          <MiniField label="Pre-Wait (s)">
            <NumberInput
              value={cue.pre_wait_ms / 1000}
              step={0.1}
              min={0}
              max={86400}
              onCommit={(v) => onSave({ pre_wait_ms: Math.round(v * 1000) })}
            />
          </MiniField>
          <MiniField label="Post-Wait (s)">
            <NumberInput
              value={cue.post_wait_ms / 1000}
              step={0.1}
              min={0}
              max={86400}
              onCommit={(v) => onSave({ post_wait_ms: Math.round(v * 1000) })}
            />
          </MiniField>
        </Grid2>
      </Section>

      {isAudio && cue.file_path && (
        <WaveformViewer cue={cue} onSave={onSave} onExpand={onOpenWaveform} />
      )}
      {isVideo && cue.file_path && (
        <VideoTrimmer
          cue={cue as VideoCueData}
          durationMs={fileDurationMs ?? liveDurationMs ?? 0}
          onSave={onSave as (p: Partial<VideoCueData>) => void}
          onExpand={onOpenWaveform}
        />
      )}

      {(isAudio || isVideo) && (
        <Section title="Clip">
          <Grid2>
            <MiniField label="Start Time (s)">
              <ClipTimeField
                ms={cue.start_time_ms}
                placeholder="0.000"
                onCommit={(start_time_ms) => onSave({ start_time_ms })}
              />
            </MiniField>
            <MiniField label="End Time (s)">
              <ClipTimeField
                ms={cue.end_time_ms}
                placeholder="end of file"
                onCommit={(end_time_ms) => onSave({ end_time_ms })}
              />
            </MiniField>
          </Grid2>

          <ToggleRow
            label="Loop"
            checked={cue.loop_count > 0}
            onToggle={(v) => onSave({ loop_count: v ? 1 : 0 })}
          >
            <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
              {cue.loop_count > 0 && cue.loop_count < LOOP_INFINITE && (
                <NumberInput
                  value={cue.loop_count}
                  step={1}
                  min={1}
                  max={LOOP_INFINITE - 1}
                  width={64}
                  onCommit={(v) => onSave({ loop_count: Math.max(1, Math.round(v)) })}
                />
              )}
              {cue.loop_count === LOOP_INFINITE && (
                <span style={{ fontSize: 16, lineHeight: 1 }}>∞</span>
              )}
              <button
                title={cue.loop_count === LOOP_INFINITE ? "Set finite loop count" : "Loop infinitely"}
                onClick={() =>
                  onSave({
                    loop_count: cue.loop_count === LOOP_INFINITE ? 1 : LOOP_INFINITE,
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
            </div>
          </ToggleRow>

          {isAudio && (
            <Grid2>
              <MiniField label="Rate (0.1 – 4×)">
                <NumberInput
                  value={cue.rate}
                  step={0.1}
                  min={0.1}
                  max={4}
                  onCommit={(v) => onSave({ rate: v })}
                />
              </MiniField>
            </Grid2>
          )}

          {isVideo && (
            <ToggleRow
              label="Hold last frame at end (no cut to black)"
              checked={cue.hold_last_frame === true}
              onToggle={(v) => onSave({ hold_last_frame: v } as never)}
            />
          )}
        </Section>
      )}
    </>
  );
}

/**
 * Clip start/end in seconds, stored in ms and clearable back to `null`
 * ("from the start" / "to the end of the file").
 *
 * Controlled rather than the `defaultValue` + remount-key pattern it replaced:
 * a drag wheel has to move the shown value on every pointer event, which an
 * uncontrolled field cannot do.
 */
function ClipTimeField({
  ms,
  placeholder,
  onCommit,
}: {
  ms: number | null;
  placeholder: string;
  onCommit: (ms: number | null) => void;
}) {
  const asText = (v: number | null) => (v != null ? (v / 1000).toFixed(3) : "");
  const [draft, setDraft] = useState(asText(ms));
  useEffect(() => { setDraft(asText(ms)); }, [ms]);

  return (
    <DragNumber
      style={inputStyle}
      step="0.001"
      min="0"
      value={draft}
      placeholder={placeholder}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={() => {
        const parsed = parseFloat(draft);
        onCommit(draft.trim() === "" || Number.isNaN(parsed) ? null : Math.round(parsed * 1000));
      }}
    />
  );
}
