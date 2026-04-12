import type { AudioCueData } from "../../lib/types";
import { Field, inputStyle } from "./Field";
import { ColorPicker } from "./ColorPicker";

export function BasicsTab({
  cue,
  isAudio,
  onSave,
  onBrowse,
}: {
  cue: AudioCueData;
  isAudio: boolean;
  onSave: (p: Partial<AudioCueData>) => void;
  onBrowse: () => void;
}) {
  return (
    <>
      <Field label="Cue #">
        <input
          style={inputStyle}
          defaultValue={cue.number ?? ""}
          onBlur={(e) => onSave({ number: e.target.value || null })}
        />
      </Field>
      <Field label="Name">
        <input
          style={inputStyle}
          defaultValue={cue.name}
          onBlur={(e) => onSave({ name: e.target.value })}
        />
      </Field>
      <Field label="Notes">
        <textarea
          style={{ ...inputStyle, resize: "vertical", minHeight: 60 }}
          defaultValue={cue.notes}
          onBlur={(e) => onSave({ notes: e.target.value })}
        />
      </Field>
      {isAudio && (
        <Field label="File">
          <div style={{ display: "flex", gap: 4 }}>
            <input
              style={{ ...inputStyle, flex: 1 }}
              readOnly
              value={cue.file_path ? cue.file_path.split(/[\\/]/).pop() ?? cue.file_path : "(no file)"}
              title={cue.file_path ?? ""}
            />
            <button
              style={{
                padding: "3px 10px",
                background: "#334155",
                border: "none",
                borderRadius: 4,
                color: "#e2e8f0",
                cursor: "pointer",
                fontSize: 12,
                flexShrink: 0,
              }}
              onClick={onBrowse}
            >
              Browse…
            </button>
          </div>
        </Field>
      )}
      <Field label="Continue">
        <select
          style={inputStyle}
          value={cue.continue_mode}
          onChange={(e) =>
            onSave({
              continue_mode: e.target.value as AudioCueData["continue_mode"],
            })
          }
        >
          <option value="do_not_continue">Do Not Continue</option>
          <option value="auto_continue">Auto-Continue</option>
          <option value="auto_follow">Auto-Follow</option>
        </select>
      </Field>
      <Field label="Color">
        <ColorPicker
          value={cue.color}
          onChange={(c) => onSave({ color: c })}
        />
      </Field>
    </>
  );
}
