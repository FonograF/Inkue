import type { AudioCueData } from "../../lib/types";
import { Field, inputStyle } from "./Field";
import { ColorPicker } from "./ColorPicker";
import { setGroupMode } from "../../lib/commands";

export function BasicsTab({
  cue,
  isAudio,
  isVideo,
  isImage,
  isGroup,
  isStop,
  onSave,
  onBrowse,
  onBrowseVideo,
  onBrowseImage,
  onRefresh,
}: {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  cue: any;
  isAudio: boolean;
  isVideo?: boolean;
  isImage?: boolean;
  isGroup?: boolean;
  isStop?: boolean;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  onSave: (p: Partial<any>) => void;
  onBrowse: () => void;
  onBrowseVideo?: () => void;
  onBrowseImage?: () => void;
  onRefresh?: () => void;
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
          defaultValue={cue.notes ?? ""}
          onBlur={(e) => onSave({ notes: e.target.value })}
        />
      </Field>
      {(isAudio || isVideo || isImage) && (
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
              onClick={isVideo ? onBrowseVideo : isImage ? onBrowseImage : onBrowse}
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
      {isGroup && (
        <Field label="Mode">
          <select
            style={inputStyle}
            value={cue.group_mode ?? "simultaneous"}
            onChange={async (e) => {
              await setGroupMode(cue.id, e.target.value as "simultaneous" | "sequential")
                .catch(console.error);
              onRefresh?.();
            }}
          >
            <option value="simultaneous">Simultaneous</option>
            <option value="sequential">Sequential</option>
          </select>
        </Field>
      )}
      {isStop && (
        <>
          <Field label="Target">
            <select
              style={inputStyle}
              value={cue.target_cue_number == null ? "__all__" : "__specific__"}
              onChange={(e) => {
                if (e.target.value === "__all__") {
                  onSave({ target_cue_number: null });
                } else {
                  onSave({ target_cue_number: "" });
                }
              }}
            >
              <option value="__all__">All Cues</option>
              <option value="__specific__">Specific Cue…</option>
            </select>
          </Field>
          {cue.target_cue_number != null && (
            <Field label="Cue #">
              <input
                style={inputStyle}
                placeholder="Cue number (e.g. 5, 1.5, Intro)"
                defaultValue={cue.target_cue_number ?? ""}
                onBlur={(e) => onSave({ target_cue_number: e.target.value || null })}
              />
            </Field>
          )}
          <Field label="Stop Mode">
            <select
              style={inputStyle}
              value={cue.hard_stop_mode ? "hard" : "soft"}
              onChange={(e) => onSave({ hard_stop_mode: e.target.value === "hard" })}
            >
              <option value="soft">Soft (fade out)</option>
              <option value="hard">Hard (immediate cut)</option>
            </select>
          </Field>
        </>
      )}
      <Field label="Color">
        <ColorPicker
          value={cue.color}
          onChange={(c) => onSave({ color: c })}
        />
      </Field>
    </>
  );
}
