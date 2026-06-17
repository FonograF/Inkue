import type { AudioCueData, CueSummary } from "../../lib/types";
import { Field, inputStyle } from "./Field";
import { ColorPicker } from "./ColorPicker";
import { setGroupMode } from "../../lib/commands";
import { useWorkspaceStore } from "../../stores/workspaceStore";

const listStyle: React.CSSProperties = {
  maxHeight: 110,
  overflowY: "auto",
  border: "1px solid #334155",
  borderRadius: 4,
  padding: "2px 0",
};

function CueCheckboxList({
  allCues,
  selfId,
  selectedIds,
  onChange,
}: {
  allCues: CueSummary[];
  selfId: string;
  selectedIds: string[];
  onChange: (ids: string[]) => void;
}) {
  const candidates = allCues.filter((c) => c.id !== selfId);
  if (candidates.length === 0) {
    return <span style={{ color: "#64748b", fontSize: 12 }}>No other cues</span>;
  }
  return (
    <div style={listStyle}>
      {candidates.map((c) => (
        <label
          key={c.id}
          style={{ display: "flex", alignItems: "center", gap: 6, padding: "2px 8px", cursor: "pointer" }}
        >
          <input
            type="checkbox"
            checked={selectedIds.includes(c.id)}
            onChange={(e) => {
              const next = e.target.checked
                ? [...selectedIds, c.id]
                : selectedIds.filter((id) => id !== c.id);
              onChange(next);
            }}
          />
          <span style={{ fontSize: 12, color: "#e2e8f0", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            {c.number ? `${c.number} — ` : ""}{c.name}
          </span>
        </label>
      ))}
    </div>
  );
}

export function BasicsTab({
  cue,
  isAudio,
  isVideo,
  isImage,
  isGroup,
  isFade,
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
  isFade?: boolean;
  isStop?: boolean;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  onSave: (p: Partial<any>) => void;
  onBrowse: () => void;
  onBrowseVideo?: () => void;
  onBrowseImage?: () => void;
  onRefresh?: () => void;
}) {
  const allCues = useWorkspaceStore((s) => s.cues);

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
      {isFade && (() => {
        const targetIds: string[] = cue.target_cue_ids ?? [];
        const targetCues = targetIds.map((id: string) => allCues.find((c) => c.id === id)).filter(Boolean);
        const hasAudio  = targetCues.some((c) => c!.cue_type === "audio");
        const hasVideo  = targetCues.some((c) => c!.cue_type === "video");
        const hasImage  = targetCues.some((c) => c!.cue_type === "image");
        const hasVisual = hasVideo || hasImage;
        // Show audio volume when: targets include audio or video (video has audio track),
        // or no target selected yet (default / unknown).
        const showVolume = hasAudio || hasVideo || (!hasVisual && !hasAudio);
        // Show brightness when: targets include image or video, or no target selected.
        const showBrightness = hasVisual || (!hasAudio && !hasVisual);
        // target_volume_db maps to brightness: -60 dB = 0% (black), 0 dB = 100% (visible).
        const volDb: number = cue.target_volume_db ?? -60;
        const brightnessPercent = Math.round(((volDb + 60) / 60) * 100);
        return (
          <>
            <Field label="Targets">
              <CueCheckboxList
                allCues={allCues}
                selfId={cue.id}
                selectedIds={targetIds}
                onChange={(ids) => {
                  const nums = ids
                    .map((id: string) => allCues.find((c) => c.id === id)?.number)
                    .filter((n): n is string => n != null);
                  onSave({ target_cue_ids: ids, target_cue_numbers: nums });
                }}
              />
            </Field>
            {showVolume && (
              <Field label="Target Volume (dB)">
                <input
                  style={inputStyle}
                  type="number"
                  step="0.5"
                  min="-60"
                  max="12"
                  key={`fade-vol-${volDb}`}
                  defaultValue={volDb}
                  onBlur={(e) => onSave({ target_volume_db: parseFloat(e.target.value) })}
                />
              </Field>
            )}
            {showBrightness && (
              <Field label="Target Brightness (%)">
                <input
                  style={inputStyle}
                  type="number"
                  step="1"
                  min="0"
                  max="100"
                  key={`fade-bright-${brightnessPercent}`}
                  defaultValue={brightnessPercent}
                  onBlur={(e) => {
                    const pct = Math.max(0, Math.min(100, parseInt(e.target.value, 10) || 0));
                    // Map [0, 100]% → [-60, 0] dB
                    const db = (pct / 100) * 60 - 60;
                    onSave({ target_volume_db: Math.round(db * 10) / 10 });
                  }}
                />
              </Field>
            )}
            <Field label="Curve">
              <select
                style={inputStyle}
                value={cue.fade_curve ?? "s_curve"}
                onChange={(e) => onSave({ fade_curve: e.target.value })}
              >
                <option value="linear">Linear</option>
                <option value="s_curve">S-Curve</option>
                <option value="exponential">Exponential</option>
              </select>
            </Field>
            <Field label="Stop at End">
              <input
                type="checkbox"
                checked={cue.stop_at_end ?? false}
                onChange={(e) => onSave({ stop_at_end: e.target.checked })}
                style={{ width: 16, height: 16, cursor: "pointer" }}
              />
            </Field>
          </>
        );
      })()}
      {isStop && (
        <>
          <Field label="Target">
            <div style={{ marginBottom: 4 }}>
              <label style={{ display: "flex", alignItems: "center", gap: 6, cursor: "pointer", fontSize: 12, color: "#e2e8f0" }}>
                <input
                  type="radio"
                  checked={(cue.target_cue_ids ?? []).length === 0}
                  onChange={() => onSave({ target_cue_ids: [], target_cue_numbers: [] })}
                />
                All Cues
              </label>
            </div>
            <CueCheckboxList
              allCues={allCues}
              selfId={cue.id}
              selectedIds={cue.target_cue_ids ?? []}
              onChange={(ids) => {
                const nums = ids
                  .map((id: string) => allCues.find((c) => c.id === id)?.number)
                  .filter((n): n is string => n != null);
                onSave({ target_cue_ids: ids, target_cue_numbers: nums });
              }}
            />
          </Field>
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
      <Field label="Disabled">
        <input
          type="checkbox"
          checked={cue.is_disabled ?? false}
          onChange={(e) => onSave({ is_disabled: e.target.checked })}
          style={{ width: 16, height: 16, cursor: "pointer" }}
        />
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
