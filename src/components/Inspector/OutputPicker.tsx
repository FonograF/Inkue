// Output selector for visual cues (Video / Image / Camera / Text): which
// window the cue plays on.  Hidden while the show has only the main output —
// the choice does not exist yet, so it should not clutter the Inspector.

import { MAIN_OUTPUT_ID } from "../../lib/types";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import { Select } from "../common/Select";
import { Field, inputStyle } from "./Field";

export function OutputPicker({
  value,
  onChange,
}: {
  /** The cue's output id; null = the main output. */
  value: string | null;
  onChange: (outputId: string | null) => void;
}) {
  const outputs = useWorkspaceStore((s) => s.videoOutputs);
  if (outputs.length <= 1 && value === null) return null;

  const known = value === null || outputs.some((o) => o.id === value);

  return (
    <Field label="Output">
      <Select
        style={{ ...inputStyle, cursor: "pointer" }}
        value={value ?? MAIN_OUTPUT_ID}
        onChange={(e) => onChange(e.target.value === MAIN_OUTPUT_ID ? null : e.target.value)}
      >
        {outputs.map((o) => (
          <option key={o.id} value={o.id}>
            {o.is_main ? "Main" : o.name}
          </option>
        ))}
        {!known && value !== null && <option value={value}>(deleted output)</option>}
      </Select>
    </Field>
  );
}
