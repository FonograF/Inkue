// Command Cue main tab: the cues this one acts on.
//
// The action is fixed by the cue type (a Goto Cue always gotos), so there is
// nothing to choose but the targets — and, for Goto, exactly one of them.

import { COMMAND_CUE_TYPES, type CommandCueType, type StopCueData } from "../../lib/types";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import { Section } from "./Field";
import { CueTargetPicker } from "./CueTargetPicker";

export function CommandTab({
  cue,
  onSave,
}: {
  cue: StopCueData;
  onSave: (p: Partial<StopCueData>) => void;
}) {
  const allCues = useWorkspaceStore((s) => s.cues);
  const targetIds: string[] = cue.target_cue_ids ?? [];
  const meta = COMMAND_CUE_TYPES.find((c) => c.type === (cue.cue_type as CommandCueType));
  const singleTarget = cue.cue_type === "goto";

  return (
    <Section title={singleTarget ? "Target" : "Targets"}>
      <div style={{ fontSize: 12, color: "var(--wc-text-secondary)", marginBottom: 10 }}>
        {meta?.hint ?? "Acts on the cues below."}
        {singleTarget && " — only the first target is used."}
      </div>

      <CueTargetPicker
        allCues={allCues}
        selfId={cue.id}
        selectedIds={targetIds}
        onChange={(ids) => {
          const kept = singleTarget ? ids.slice(-1) : ids;
          const nums = kept
            .map((id) => allCues.find((c) => c.id === id)?.number)
            .filter((n): n is string => n != null);
          onSave({ target_cue_ids: kept, target_cue_numbers: nums });
        }}
      />

      {targetIds.length === 0 && (
        <div style={{ fontSize: 11, color: "var(--wc-text-faint)", marginTop: 8 }}>
          No target — this cue will do nothing.
        </div>
      )}
    </Section>
  );
}
