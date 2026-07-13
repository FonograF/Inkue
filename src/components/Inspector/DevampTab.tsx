// Devamp Cue main tab: which vamping cues to release, and whether the target
// stops at the end of its current slice or continues into the next one.

import type { DevampCueData } from "../../lib/types";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import { Section, Segmented } from "./Field";
import { CueTargetPicker } from "./CueTargetPicker";

export function DevampTab({
  cue,
  onSave,
}: {
  cue: DevampCueData;
  onSave: (p: Partial<DevampCueData>) => void;
}) {
  const allCues = useWorkspaceStore((s) => s.cues);
  const targetIds: string[] = cue.target_cue_ids ?? [];

  return (
    <>
      <Section
        title="Targets"
        hint="Cues whose current slice loop this GO releases (set slices in the clip editor)."
      >
        <CueTargetPicker
          allCues={allCues}
          selfId={cue.id}
          selectedIds={targetIds}
          filterTypes={["audio", "video", "group"]}
          onChange={(ids) => {
            const nums = ids
              .map((id) => allCues.find((c) => c.id === id)?.number)
              .filter((n): n is string => n != null);
            onSave({ target_cue_ids: ids, target_cue_numbers: nums });
          }}
        />
        <div style={{ height: 8 }} />
      </Section>

      <Section title="After the current pass">
        <Segmented
          options={[
            { value: "continue", label: "Continue", hint: "Play on into the next slice" },
            { value: "stop", label: "Stop", hint: "Stop at the end of the current slice" },
          ]}
          value={cue.stop_at_end ? "stop" : "continue"}
          onChange={(v) => onSave({ stop_at_end: v === "stop" })}
        />
        <div style={{ fontSize: 11, color: "var(--wc-text-faint)", marginTop: -4, marginBottom: 6 }}>
          {cue.stop_at_end
            ? "The target finishes its current pass, then stops at the slice boundary."
            : "The target finishes its current pass, then continues into the next slice."}
        </div>
      </Section>
    </>
  );
}
