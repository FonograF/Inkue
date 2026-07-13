// Searchable multi-select of cues. Selected cues show as removable chips above
// a filterable list, so picking targets stays fast even with hundreds of cues.
// Shared by the Fade and Stop cue tabs.

import { useMemo, useState } from "react";
import type { CueSummary, CueType } from "../../lib/types";
import { inputStyle } from "./Field";

const listStyle: React.CSSProperties = {
  maxHeight: 180,
  overflowY: "auto",
  border: "1px solid var(--wc-border-strong)",
  borderRadius: 4,
  padding: "2px 0",
  background: "var(--wc-bg-surface)",
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

export function CueTargetPicker({
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
              <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", maxWidth: 160 }}>
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
                  display: "flex", alignItems: "center", gap: 6, padding: "3px 8px", cursor: "pointer",
                  background: checked ? "var(--wc-bg-hover)" : undefined,
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
