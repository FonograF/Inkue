// Fixture patch (M3): name lighting instruments, place them at DMX addresses,
// identify them (test), and warn about address clashes. Backed by the workspace
// (`list_fixtures` / `add_fixture` / `update_fixture` / `remove_fixture`).

import { useCallback, useEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";
import {
  addFixture,
  dmxTestFixture,
  getFixtureConflicts,
  listBuiltinFixtureTypes,
  listFixtures,
  removeFixture,
  updateFixture,
} from "../../lib/commands";
import type { ChannelWidth, FixtureConflict, FixtureType, PatchedFixture } from "../../lib/types";
import { Select } from "../common/Select";

const widthChannels = (w: ChannelWidth) => (w === "Bit16" ? 2 : 1);

function footprint(t: FixtureType): number {
  return t.parameters.reduce((max, p) => Math.max(max, p.channel_offset + widthChannels(p.width)), 0);
}

export function FixturePatch() {
  const [fixtures, setFixtures] = useState<PatchedFixture[]>([]);
  const [types, setTypes] = useState<FixtureType[]>([]);
  const [conflicts, setConflicts] = useState<FixtureConflict[]>([]);
  const [identifying, setIdentifying] = useState<string | null>(null);
  const identifyingRef = useRef<string | null>(null);
  identifyingRef.current = identifying;

  const [newTypeIdx, setNewTypeIdx] = useState(0);
  const [newUniverse, setNewUniverse] = useState(1);
  const [newAddress, setNewAddress] = useState(1);

  const refresh = useCallback(async () => {
    setFixtures(await listFixtures());
    setConflicts(await getFixtureConflicts());
  }, []);

  useEffect(() => {
    listBuiltinFixtureTypes().then(setTypes).catch(console.error);
    refresh().catch(console.error);
    // Turn any active identify off when the panel closes.
    return () => {
      if (identifyingRef.current) dmxTestFixture(identifyingRef.current, false).catch(() => {});
    };
  }, [refresh]);

  const conflictIds = new Set(conflicts.flatMap((c) => [c.fixture_a, c.fixture_b]));

  const handleAdd = async () => {
    const t = types[newTypeIdx];
    if (!t) return;
    // Unique default label so two fixtures of the same type are distinguishable
    // (e.g. "RGB 1", "RGB 2"); the operator renames as needed.
    const base = t.name.split(" ")[0];
    const n = fixtures.filter((f) => f.label.startsWith(base)).length + 1;
    await addFixture(`${base} ${n}`, newUniverse, newAddress, t).catch(console.error);
    // Auto-advance the next start address by this fixture's footprint.
    setNewAddress(Math.min(512, newAddress + footprint(t)));
    await refresh();
  };

  const commit = async (fixture: PatchedFixture, patch: Partial<PatchedFixture>) => {
    await updateFixture({ ...fixture, ...patch }).catch(console.error);
    await refresh();
  };

  const handleRemove = async (id: string) => {
    if (identifying === id) {
      await dmxTestFixture(id, false).catch(() => {});
      setIdentifying(null);
    }
    await removeFixture(id).catch(console.error);
    await refresh();
  };

  const toggleIdentify = async (id: string) => {
    if (identifying === id) {
      await dmxTestFixture(id, false).catch(console.error);
      setIdentifying(null);
      return;
    }
    if (identifying) await dmxTestFixture(identifying, false).catch(() => {});
    await dmxTestFixture(id, true).catch(console.error);
    setIdentifying(id);
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
      {fixtures.length === 0 && (
        <div style={{ color: "var(--wc-text-faint)", fontSize: 12 }}>No fixtures patched yet — add one below.</div>
      )}

      {fixtures.map((f) => {
        const fp = footprint(f.fixture_type);
        const last = f.base_address + fp - 1;
        const conflict = conflictIds.has(f.id);
        return (
          <div
            key={f.id}
            style={{ ...rowStyle, borderColor: conflict ? "#b45309" : "var(--wc-border)" }}
          >
            <input
              style={{ ...txtStyle, flex: 1, minWidth: 0 }}
              value={f.label}
              onChange={(e) => commit(f, { label: e.target.value })}
              title="Fixture label"
            />
            <span style={{ color: "var(--wc-text-muted)", fontSize: 10, flexShrink: 0 }}>{f.fixture_type.name}</span>
            <label style={lblStyle}>U</label>
            <input
              style={{ ...numStyle, width: 42 }}
              type="number" min={1} max={63999} value={f.universe}
              onChange={(e) => commit(f, { universe: Number(e.target.value) })}
              title="Universe"
            />
            <label style={lblStyle}>@</label>
            <input
              style={{ ...numStyle, width: 48 }}
              type="number" min={1} max={512} value={f.base_address}
              onChange={(e) => commit(f, { base_address: Number(e.target.value) })}
              title="DMX start address"
            />
            <span style={{ color: "var(--wc-text-faint)", fontSize: 10, width: 52, flexShrink: 0 }}>
              →{last} ({fp}ch)
            </span>
            <button
              style={identifying === f.id ? identifyOnBtn : smallBtn}
              onClick={() => toggleIdentify(f.id)}
              title="Identify — drive this fixture to a visible level"
            >
              {identifying === f.id ? "◉" : "○"}
            </button>
            <button style={smallBtn} onClick={() => handleRemove(f.id)} title="Remove">✕</button>
          </div>
        );
      })}

      {conflicts.length > 0 && (
        <div style={{ display: "flex", flexDirection: "column", gap: 2, marginTop: 2 }}>
          {conflicts.map((c, i) => (
            <div key={i} style={{ color: "#f59e0b", fontSize: 11 }}>⚠ {c.message}</div>
          ))}
        </div>
      )}

      {/* Add fixture */}
      <div style={{ ...rowStyle, marginTop: 4, background: "var(--wc-bg-app)" }}>
        <Select
          style={{ ...selStyle, flex: 1, minWidth: 0 }}
          value={newTypeIdx}
          onChange={(e) => setNewTypeIdx(Number(e.target.value))}
        >
          {types.map((t, i) => (
            <option key={i} value={i}>{t.name}</option>
          ))}
        </Select>
        <label style={lblStyle}>U</label>
        <input
          style={{ ...numStyle, width: 42 }}
          type="number" min={1} max={63999} value={newUniverse}
          onChange={(e) => setNewUniverse(Number(e.target.value))}
        />
        <label style={lblStyle}>@</label>
        <input
          style={{ ...numStyle, width: 48 }}
          type="number" min={1} max={512} value={newAddress}
          onChange={(e) => setNewAddress(Number(e.target.value))}
        />
        <button style={{ ...smallBtn, color: "#a855f7" }} onClick={handleAdd}>+ add</button>
      </div>
    </div>
  );
}

const rowStyle: CSSProperties = {
  display: "flex", gap: 6, alignItems: "center",
  border: "1px solid var(--wc-border)", borderRadius: 4, padding: "3px 5px",
};
const lblStyle: CSSProperties = { color: "var(--wc-text-muted)", fontSize: 11, flexShrink: 0 };
const numStyle: CSSProperties = {
  background: "var(--wc-bg-app)", border: "1px solid var(--wc-border-strong)", borderRadius: 4, color: "var(--wc-text)", padding: "2px 4px", fontSize: 12,
};
const txtStyle: CSSProperties = { ...numStyle };
const selStyle: CSSProperties = { ...numStyle, cursor: "pointer" };
const smallBtn: CSSProperties = {
  background: "none", border: "1px solid var(--wc-border-strong)", borderRadius: 4, color: "var(--wc-text-muted)", fontSize: 11, padding: "1px 6px", cursor: "pointer", flexShrink: 0,
};
const identifyOnBtn: CSSProperties = {
  ...smallBtn, color: "#fde047", borderColor: "#a16207", background: "#422006",
};
