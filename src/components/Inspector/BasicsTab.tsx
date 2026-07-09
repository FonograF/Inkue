import { useMemo, useState } from "react";
import type { AudioCueData, CueSummary } from "../../lib/types";
import { Field, inputStyle } from "./Field";
import { ColorPicker } from "./ColorPicker";
import { setGroupMode, setPlaylistLoop } from "../../lib/commands";
import type { CueType, GroupMode } from "../../lib/types";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import { Select } from "../common/Select";

const listStyle: React.CSSProperties = {
  maxHeight: 150,
  overflowY: "auto",
  border: "1px solid var(--wc-border-strong)",
  borderRadius: 4,
  padding: "2px 0",
};

const chipStyle: React.CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  gap: 4,
  padding: "1px 4px 1px 7px",
  borderRadius: 10,
  background: "var(--wc-accent)",
  color: "var(--wc-accent-fg)",
  fontSize: 11,
  maxWidth: "100%",
};

const cueLabel = (c: CueSummary) => `${c.number ? `${c.number} — ` : ""}${c.name || "(untitled)"}`;

/// Searchable multi-select of cues. Selected cues show as removable chips above
/// a filterable list, so picking targets stays fast even with hundreds of cues.
function CueCheckboxList({
  allCues,
  selfId,
  selectedIds,
  onChange,
  filterTypes,
}: {
  allCues: CueSummary[];
  selfId: string;
  selectedIds: string[];
  onChange: (ids: string[]) => void;
  /** When set, only cues of these types are offered (e.g. fade-able types). */
  filterTypes?: CueType[];
}) {
  const [query, setQuery] = useState("");

  const candidates = useMemo(
    () =>
      allCues.filter(
        (c) => c.id !== selfId && (!filterTypes || filterTypes.includes(c.cue_type)),
      ),
    [allCues, selfId, filterTypes],
  );

  const selectedCues = useMemo(
    () => selectedIds.map((id) => allCues.find((c) => c.id === id)).filter((c): c is CueSummary => !!c),
    [selectedIds, allCues],
  );

  const q = query.trim().toLowerCase();
  const visible = q
    ? candidates.filter((c) => cueLabel(c).toLowerCase().includes(q))
    : candidates;

  if (candidates.length === 0) {
    return <span style={{ color: "var(--wc-text-muted)", fontSize: 12 }}>No eligible cues</span>;
  }

  const toggle = (id: string, on: boolean) =>
    onChange(on ? [...selectedIds, id] : selectedIds.filter((x) => x !== id));

  return (
    <div style={{ width: "100%" }}>
      {/* Selected chips */}
      {selectedCues.length > 0 && (
        <div style={{ display: "flex", flexWrap: "wrap", gap: 4, marginBottom: 6 }}>
          {selectedCues.map((c) => (
            <span key={c.id} style={chipStyle} title={cueLabel(c)}>
              <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", maxWidth: 140 }}>
                {cueLabel(c)}
              </span>
              <button
                onClick={() => toggle(c.id, false)}
                style={{ background: "none", border: "none", color: "inherit", cursor: "pointer", fontSize: 13, lineHeight: 1, padding: "0 2px" }}
                title="Remove"
              >
                ×
              </button>
            </span>
          ))}
          <button
            onClick={() => onChange([])}
            style={{ background: "none", border: "none", color: "var(--wc-text-muted)", cursor: "pointer", fontSize: 11 }}
          >
            Clear all
          </button>
        </div>
      )}

      {/* Search box */}
      <input
        type="text"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder={`Search ${candidates.length} cue${candidates.length > 1 ? "s" : ""}…`}
        style={{ ...inputStyle, marginBottom: 4, fontSize: 12 }}
      />

      {/* Filtered list */}
      <div style={listStyle}>
        {visible.length === 0 ? (
          <div style={{ padding: "4px 8px", fontSize: 12, color: "var(--wc-text-muted)" }}>No match</div>
        ) : (
          visible.map((c) => {
            const checked = selectedIds.includes(c.id);
            return (
              <label
                key={c.id}
                style={{
                  display: "flex", alignItems: "center", gap: 6, padding: "2px 8px", cursor: "pointer",
                  background: checked ? "var(--wc-bg-surface)" : undefined,
                }}
              >
                <input type="checkbox" checked={checked} onChange={(e) => toggle(c.id, e.target.checked)} />
                <span style={{ fontSize: 12, color: "var(--wc-text)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                  {cueLabel(c)}
                </span>
              </label>
            );
          })
        )}
      </div>
    </div>
  );
}

/// Small section subheader used to group the Fade inspector controls.
function SubHeader({ children }: { children: React.ReactNode }) {
  return (
    <div style={{
      fontSize: 10, fontWeight: 700, letterSpacing: "0.06em", textTransform: "uppercase",
      color: "var(--wc-text-muted)", margin: "12px 0 6px", paddingBottom: 3,
      borderBottom: "1px solid var(--wc-border)",
    }}>
      {children}
    </div>
  );
}

/// A row whose control is gated by a leading checkbox (e.g. "Fade Volume").
function ToggleRow({
  label, checked, onToggle, children,
}: {
  label: string; checked: boolean; onToggle: (v: boolean) => void; children?: React.ReactNode;
}) {
  return (
    <div style={{ marginBottom: 8 }}>
      <label style={{ display: "flex", alignItems: "center", gap: 8, cursor: "pointer" }}>
        <input type="checkbox" checked={checked} onChange={(e) => onToggle(e.target.checked)}
          style={{ width: 15, height: 15, cursor: "pointer" }} />
        <span style={{ fontSize: 13, color: "var(--wc-text)" }}>{label}</span>
      </label>
      {checked && children && <div style={{ marginTop: 6, paddingLeft: 23 }}>{children}</div>}
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
                background: "var(--wc-bg-hover)",
                border: "none",
                borderRadius: 4,
                color: "var(--wc-text)",
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
        <Select
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
        </Select>
      </Field>
      {isGroup && (
        <>
          <Field label="Mode">
            <Select
              style={inputStyle}
              value={cue.group_mode ?? "simultaneous"}
              onChange={async (e) => {
                await setGroupMode(cue.id, e.target.value as GroupMode).catch(console.error);
                onRefresh?.();
              }}
            >
              <option value="simultaneous">Simultaneous</option>
              <option value="sequential">Sequential</option>
              <option value="playlist">Playlist</option>
              <option value="start_random">Start Random</option>
            </Select>
          </Field>
          {cue.group_mode === "playlist" && (
            <Field label="Loop">
              <input
                type="checkbox"
                checked={cue.playlist_loop ?? false}
                onChange={async (e) => {
                  await setPlaylistLoop(cue.id, e.target.checked).catch(console.error);
                  onRefresh?.();
                }}
                style={{ width: 16, height: 16, cursor: "pointer" }}
              />
            </Field>
          )}
        </>
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
        const volDb: number = cue.target_volume_db ?? -60;
        const brightnessPercent: number = cue.target_brightness_pct ?? 0;
        const fadeVolume: boolean = cue.fade_volume ?? true;
        const panEnabled: boolean = cue.target_pan != null;
        const panValue: number = cue.target_pan ?? 0;
        const fadeSecs = ((cue.fade_duration_ms ?? 2000) / 1000).toFixed(1);
        return (
          <>
            <SubHeader>Targets</SubHeader>
            <CueCheckboxList
              allCues={allCues}
              selfId={cue.id}
              selectedIds={targetIds}
              filterTypes={["audio", "video", "image", "group"]}
              onChange={(ids) => {
                const nums = ids
                  .map((id: string) => allCues.find((c) => c.id === id)?.number)
                  .filter((n): n is string => n != null);
                onSave({ target_cue_ids: ids, target_cue_numbers: nums });
              }}
            />

            <SubHeader>Fade</SubHeader>
            <Field label="Time (s)">
              <input
                style={{ ...inputStyle, width: 90 }}
                type="number"
                step="0.1"
                min="0"
                key={`fade-dur-${cue.fade_duration_ms}`}
                defaultValue={fadeSecs}
                onBlur={(e) => onSave({ fade_duration_ms: Math.round(parseFloat(e.target.value) * 1000) })}
              />
            </Field>
            <Field label="Curve">
              <Select
                style={inputStyle}
                value={cue.fade_curve ?? "s_curve"}
                onChange={(e) => onSave({ fade_curve: e.target.value })}
              >
                <option value="linear">Linear</option>
                <option value="s_curve">S-Curve</option>
                <option value="exponential">Exponential</option>
              </Select>
            </Field>

            {showVolume && (
              <>
                <SubHeader>Audio</SubHeader>
                <ToggleRow
                  label="Fade volume"
                  checked={fadeVolume}
                  onToggle={(v) => onSave({ fade_volume: v })}
                >
                  <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                    <span style={{ fontSize: 12, color: "var(--wc-text-secondary)", width: 56 }}>To (dB)</span>
                    <input
                      style={{ ...inputStyle, width: 90 }}
                      type="number" step="0.5" min="-60" max="12"
                      key={`fade-vol-${volDb}`}
                      defaultValue={volDb}
                      onBlur={(e) => onSave({ target_volume_db: parseFloat(e.target.value) })}
                    />
                  </div>
                </ToggleRow>
                <ToggleRow
                  label="Fade pan"
                  checked={panEnabled}
                  onToggle={(v) => onSave({ target_pan: v ? panValue : null })}
                >
                  <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                    <span style={{ fontSize: 12, color: "var(--wc-text-secondary)", width: 56 }}>To</span>
                    <input
                      type="range" min="-1" max="1" step="0.01"
                      value={panValue}
                      onChange={(e) => onSave({ target_pan: parseFloat(e.target.value) })}
                      style={{ flex: 1 }}
                    />
                    <span style={{ width: 44, textAlign: "right", fontFamily: "monospace", fontSize: 12 }}>
                      {panValue === 0 ? "C" : `${panValue > 0 ? "R" : "L"}${Math.round(Math.abs(panValue) * 100)}`}
                    </span>
                  </div>
                </ToggleRow>
              </>
            )}

            {showBrightness && (
              <>
                <SubHeader>Visual</SubHeader>
                <Field label="Brightness (%)">
                  <input
                    style={{ ...inputStyle, width: 90 }}
                    type="number" step="1" min="0" max="100"
                    key={`fade-bright-${brightnessPercent}`}
                    defaultValue={brightnessPercent}
                    onBlur={(e) => {
                      const pct = Math.max(0, Math.min(100, parseInt(e.target.value, 10) || 0));
                      onSave({ target_brightness_pct: pct });
                    }}
                  />
                </Field>
              </>
            )}

            <SubHeader>On Complete</SubHeader>
            <label style={{ display: "flex", alignItems: "center", gap: 8, cursor: "pointer" }}>
              <input
                type="checkbox"
                checked={cue.stop_at_end ?? false}
                onChange={(e) => onSave({ stop_at_end: e.target.checked })}
                style={{ width: 15, height: 15, cursor: "pointer" }}
              />
              <span style={{ fontSize: 13, color: "var(--wc-text)" }}>Stop targets when the fade ends</span>
            </label>
          </>
        );
      })()}
      {isStop && (
        <>
          <Field label="Target">
            <div style={{ marginBottom: 4 }}>
              <label style={{ display: "flex", alignItems: "center", gap: 6, cursor: "pointer", fontSize: 12, color: "var(--wc-text)" }}>
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
            <Select
              style={inputStyle}
              value={cue.hard_stop_mode ? "hard" : "soft"}
              onChange={(e) => onSave({ hard_stop_mode: e.target.value === "hard" })}
            >
              <option value="soft">Soft (fade out)</option>
              <option value="hard">Hard (immediate cut)</option>
            </Select>
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
