// Fade Cue main tab: targets, fade parameters, audio/visual goals, on-complete.
// Extracted from BasicsTab so Basics stays identity-only.

import type { CueSummary, FadeCueData } from "../../lib/types";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import { Grid2, MiniField, NumberInput, Section, SliderRow, ToggleRow } from "./Field";
import { CueTargetPicker } from "./CueTargetPicker";
import { CurveSelect } from "../common/CurveSelect";

export function FadeCueTab({
  cue,
  onSave,
}: {
  cue: FadeCueData;
  onSave: (p: Partial<FadeCueData>) => void;
}) {
  const allCues = useWorkspaceStore((s) => s.cues);

  const targetIds: string[] = cue.target_cue_ids ?? [];
  const targetCues = targetIds
    .map((id) => allCues.find((c) => c.id === id))
    .filter((c): c is CueSummary => !!c);
  const hasAudio = targetCues.some((c) => c.cue_type === "audio");
  const hasVideo = targetCues.some((c) => c.cue_type === "video");
  const hasVisual = hasVideo || targetCues.some(
    (c) => c.cue_type === "image" || c.cue_type === "camera",
  );
  // Show audio goals when targets include audio or video (video has an audio
  // track), or while no target is selected yet (default / unknown).
  const showVolume = hasAudio || hasVideo || (!hasVisual && !hasAudio);
  // Show visual goals when targets include image or video, or no target yet.
  const showBrightness = hasVisual || (!hasAudio && !hasVisual);

  const volDb: number = cue.target_volume_db ?? -60;
  const brightnessPercent: number = cue.target_brightness_pct ?? 0;
  const fadeVolume: boolean = cue.fade_volume ?? true;
  const panEnabled: boolean = cue.target_pan != null;
  const panValue: number = cue.target_pan ?? 0;

  return (
    <>
      <Section title="Targets">
        <CueTargetPicker
          allCues={allCues}
          selfId={cue.id}
          selectedIds={targetIds}
          filterTypes={["audio", "video", "image", "camera", "group"]}
          onChange={(ids) => {
            const nums = ids
              .map((id) => allCues.find((c) => c.id === id)?.number)
              .filter((n): n is string => n != null);
            onSave({ target_cue_ids: ids, target_cue_numbers: nums });
          }}
        />
        <div style={{ height: 8 }} />
      </Section>

      <Section title="Fade">
        <Grid2>
          <MiniField label="Time (s)">
            <NumberInput
              value={(cue.fade_duration_ms ?? 2000) / 1000}
              step={0.1}
              min={0}
              max={3600}
              onCommit={(v) => onSave({ fade_duration_ms: Math.round(v * 1000) })}
            />
          </MiniField>
          <MiniField label="Curve">
            <CurveSelect
              value={cue.fade_curve ?? "s_curve"}
              onChange={(v) => onSave({ fade_curve: v })}
            />
          </MiniField>
        </Grid2>
      </Section>

      {showVolume && (
        <Section title="Audio">
          <ToggleRow
            label="Fade volume"
            checked={fadeVolume}
            onToggle={(v) => onSave({ fade_volume: v })}
          >
            <Grid2>
              <MiniField label="To (dB)">
                <NumberInput
                  value={volDb}
                  step={0.5}
                  min={-60}
                  max={12}
                  onCommit={(v) => onSave({ target_volume_db: v })}
                />
              </MiniField>
            </Grid2>
          </ToggleRow>
          <ToggleRow
            label="Fade pan"
            checked={panEnabled}
            onToggle={(v) => onSave({ target_pan: v ? panValue : null })}
          >
            <SliderRow
              label="To"
              value={panValue}
              min={-1}
              max={1}
              step={0.01}
              format={(v) => (v === 0 ? "C" : `${v > 0 ? "R" : "L"}${Math.round(Math.abs(v) * 100)}`)}
              onChange={(v) => onSave({ target_pan: v })}
            />
          </ToggleRow>
        </Section>
      )}

      {showBrightness && (
        <Section title="Visual">
          <SliderRow
            label="Brightness"
            value={brightnessPercent}
            min={0}
            max={100}
            step={1}
            format={(v) => `${Math.round(v)}%`}
            onChange={(v) => onSave({ target_brightness_pct: Math.round(v) })}
          />
        </Section>
      )}

      <Section title="On Complete">
        <ToggleRow
          label="Stop targets when the fade ends"
          checked={cue.stop_at_end ?? false}
          onToggle={(v) => onSave({ stop_at_end: v })}
        />
      </Section>
    </>
  );
}
