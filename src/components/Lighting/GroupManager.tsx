// Fixture groups: name a set of fixtures so one Light Cue control (colour /
// intensity) drives them all. Members are picked by toggling fixture chips.

import { useCallback, useEffect, useState } from "react";
import type { CSSProperties } from "react";
import {
  addFixtureGroup,
  listFixtureGroups,
  listFixtures,
  removeFixtureGroup,
  updateFixtureGroup,
} from "../../lib/commands";
import type { FixtureGroup, PatchedFixture } from "../../lib/types";

function Chip({ label, on, onClick }: { label: string; on: boolean; onClick: () => void }) {
  return (
    <button onClick={onClick} style={on ? chipOn : chipOff} title={on ? "Member — click to remove" : "Click to add"}>
      {label}
    </button>
  );
}

export function GroupManager() {
  const [groups, setGroups] = useState<FixtureGroup[]>([]);
  const [fixtures, setFixtures] = useState<PatchedFixture[]>([]);
  const [newLabel, setNewLabel] = useState("");
  const [newMembers, setNewMembers] = useState<string[]>([]);

  const refresh = useCallback(async () => {
    setGroups(await listFixtureGroups());
    setFixtures(await listFixtures());
  }, []);

  useEffect(() => {
    refresh().catch(console.error);
  }, [refresh]);

  const toggleNew = (id: string) =>
    setNewMembers((m) => (m.includes(id) ? m.filter((x) => x !== id) : [...m, id]));

  const create = async () => {
    if (newMembers.length === 0) return;
    const label = newLabel.trim() || `Group ${groups.length + 1}`;
    await addFixtureGroup(label, newMembers).catch(console.error);
    setNewLabel("");
    setNewMembers([]);
    await refresh();
  };

  const commit = async (group: FixtureGroup, patch: Partial<FixtureGroup>) => {
    await updateFixtureGroup({ ...group, ...patch }).catch(console.error);
    await refresh();
  };

  const toggleMember = (group: FixtureGroup, id: string) => {
    const next = group.fixture_ids.includes(id)
      ? group.fixture_ids.filter((x) => x !== id)
      : [...group.fixture_ids, id];
    commit(group, { fixture_ids: next });
  };

  if (fixtures.length === 0) {
    return <div style={{ color: "var(--wc-text-faint)", fontSize: 12 }}>Patch fixtures first, then group them here.</div>;
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
      {groups.map((g) => (
        <div key={g.id} style={cardStyle}>
          <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
            <input
              style={{ ...txtStyle, flex: 1, minWidth: 0 }}
              value={g.label}
              onChange={(e) => commit(g, { label: e.target.value })}
            />
            <span style={{ color: "var(--wc-text-muted)", fontSize: 10 }}>{g.fixture_ids.length} fix.</span>
            <button style={smallBtn} onClick={async () => { await removeFixtureGroup(g.id).catch(console.error); await refresh(); }} title="Delete group">✕</button>
          </div>
          <div style={chipWrap}>
            {fixtures.map((f) => (
              <Chip key={f.id} label={f.label} on={g.fixture_ids.includes(f.id)} onClick={() => toggleMember(g, f.id)} />
            ))}
          </div>
        </div>
      ))}

      {/* New group */}
      <div style={{ ...cardStyle, background: "var(--wc-bg-app)" }}>
        <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
          <input
            style={{ ...txtStyle, flex: 1, minWidth: 0 }}
            placeholder="New group name"
            value={newLabel}
            onChange={(e) => setNewLabel(e.target.value)}
          />
          <button style={{ ...smallBtn, color: "#a855f7" }} disabled={newMembers.length === 0} onClick={create}>+ create</button>
        </div>
        <div style={chipWrap}>
          {fixtures.map((f) => (
            <Chip key={f.id} label={f.label} on={newMembers.includes(f.id)} onClick={() => toggleNew(f.id)} />
          ))}
        </div>
      </div>
    </div>
  );
}

const cardStyle: CSSProperties = {
  display: "flex", flexDirection: "column", gap: 6,
  border: "1px solid var(--wc-border)", borderRadius: 4, padding: "5px 7px",
};
const chipWrap: CSSProperties = { display: "flex", flexWrap: "wrap", gap: 4 };
const txtStyle: CSSProperties = {
  background: "var(--wc-bg-app)", border: "1px solid var(--wc-border-strong)", borderRadius: 4, color: "var(--wc-text)", padding: "2px 4px", fontSize: 12,
};
const smallBtn: CSSProperties = {
  background: "none", border: "1px solid var(--wc-border-strong)", borderRadius: 4, color: "var(--wc-text-muted)", fontSize: 11, padding: "1px 6px", cursor: "pointer",
};
const chipOff: CSSProperties = {
  background: "var(--wc-bg-app)", border: "1px solid var(--wc-border-strong)", borderRadius: 10, color: "var(--wc-text-secondary)", fontSize: 10, padding: "1px 8px", cursor: "pointer",
};
const chipOn: CSSProperties = {
  background: "var(--wc-accent-dim)", border: "1px solid var(--wc-accent)", borderRadius: 10, color: "var(--wc-text)", fontSize: 10, padding: "1px 8px", cursor: "pointer",
};
