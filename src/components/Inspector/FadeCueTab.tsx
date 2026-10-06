// Fade Cue main tab: targets, fade parameters, audio/visual goals, on-complete.
// Extracted from BasicsTab so Basics stays identity-only.

import type { CueSummary, FadeCueData, FadeShapes } from "../../lib/types";
import { CurveEditor } from "../Curve/CurveEditor";

/** Locked S-Curve — what a fade has always done, for a cue saved before
 *  shapes existed and re-read before the backend fills them in. */
const DEFAULT_FADE_SHAPES: FadeShapes = {
  up: { kind: "s_curve", intensity: 0, points: [], bends: [] },
  down: { kind: "s_curve", intensity: 0, points: [], bends: [] },
  mirrored: true,
};
import { useWorkspaceStore } from "../../stores/workspaceStore";
import { flattenCues, makesSound, showsPicture } from "../../lib/cueTree";
import { Grid2, MiniField, NumberInput, Section, SliderRow, ToggleRow, inputStyle } from "./Field";
import { CueTargetPicker } from "./CueTargetPicker";
import { Select } from "../common/Select";

const hintStyle = { fontSize: 11, color: "var(--wc-text-faint)", margin: "6px 0 10px", lineHeight: 1.5 } as const;

export function FadeCueTab({
  cue,
  onSave,
  onOpenCurveEditor,
}: {
  cue: FadeCueData;
  onSave: (p: Partial<FadeCueData>) => void;
  /** Opens the full-size curve editor in the dock under the cue list. */
  onOpenCurveEditor?: () => void;
}) {
  const allCues = useWorkspaceStore((s) => s.cues);

  const everyCue = flattenCues(allCues);
  const targetIds: string[] = cue.target_cue_ids ?? [];
  const targetCues = targetIds
    .map((id) => everyCue.find((c) => c.id === id))
    .filter((c): c is CueSummary => !!c);
  // Groups count through their children: a group holding a video is both.
  const hasSound = targetCues.some(makesSound);
  const hasPicture = targetCues.some(showsPicture);
  const noTarget = targetCues.length === 0;
  const crossfadeInto = cue.crossfade_into_id ?? null;
  // Show audio goals when targets make sound, or while no target is selected yet.
  const showVolume = hasSound || noTarget;
  // Show visual goals when targets show a picture, or no target yet.
  const showBrightness = hasPicture || noTarget;

  // Anything with a picture can be dissolved into, nested in a group or not.
  const pictureCues = everyCue.filter(
    (c) => c.id !== cue.id && (c.cue_type === "video" || c.cue_type === "image" || c.cue_type === "camera"),
  );
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

      {(hasPicture || noTarget || crossfadeInto !== null) && (
        <Section title="Crossfade">
          <Select
            style={inputStyle}
            value={crossfadeInto ?? ""}
            onChange={(e) => onSave({ crossfade_into_id: e.target.value || null })}
          >
            <option value="">None — plain fade</option>
            {pictureCues.map((c) => (
              <option key={c.id} value={c.id}>
                {c.number ? `${c.number} — ` : ""}{c.name || "(untitled)"}
              </option>
            ))}
            {crossfadeInto !== null && !pictureCues.some((c) => c.id === crossfadeInto) && (
              <option value={crossfadeInto}>(missing cue)</option>
            )}
          </Select>
          <div style={hintStyle}>
            {crossfadeInto
              ? "GO starts this cue. From its first frame, the targets' pictures dissolve into it over the fade time, along the curve — on another output they fade out on their own screen. Their sound fades out with them, and they stop when the dissolve ends."
              : "Pick a Video, Image or Camera cue to dissolve the targets into, without the dip to black of two crossing fades."}
          </div>
        </Section>
      )}

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
        </Grid2>
      </Section>

      <Section title="Curve">
        <CurveEditor
          compact
          shapes={cue.fade_shapes ?? DEFAULT_FADE_SHAPES}
          onChange={(fade_shapes) => onSave({ fade_shapes })}
        />
        {onOpenCurveEditor && (
          <button
            onClick={onOpenCurveEditor}
            title="Open the curve editor below the cue list"
            style={{
              marginTop: 8, padding: "3px 10px",
              background: "var(--wc-bg-surface)", border: "1px solid var(--wc-border-strong)",
              borderRadius: 4, color: "var(--wc-text-secondary)", fontSize: 11, cursor: "pointer",
            }}
          >
            ⤢ Open curve editor
          </button>
        )}
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
        <Section
          title="Visual"
          hint={crossfadeInto ? "Used only if there is nothing to dissolve into." : undefined}
        >
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

      <Section
        title="On Complete"
        hint={crossfadeInto ? "A crossfade always stops the targets it dissolved away." : undefined}
      >
        <ToggleRow
          label="Stop targets when the fade ends"
          checked={cue.stop_at_end ?? false}
          onToggle={(v) => onSave({ stop_at_end: v })}
        />
      </Section>
    </>
  );
}
