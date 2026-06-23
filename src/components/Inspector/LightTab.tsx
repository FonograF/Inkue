// Inspector tab for Light cues. Targets are shown as cards:
//  - one card per fixture group (colour / intensity → all members), and
//  - one card per individual fixture (colour picker + intensity + extra params).
// "Capture live state" freezes the live engine state (sculpted in the DMX panel
// Dashboard) into this cue as per-fixture targets.

import { useEffect, useState } from "react";
import type { FixtureGroup, LightCueData, ParamTarget, PatchedFixture } from "../../lib/types";
import { captureLiveTargets, listFixtureGroups, listFixtures } from "../../lib/commands";
import { hexToRgb, paramIndexOfKind, rgbToHex } from "../../lib/fixtureColor";
import { Field, inputStyle } from "./Field";
import { CurveSelect } from "../common/CurveSelect";
import { Select } from "../common/Select";

interface Props {
  cue: LightCueData;
  onSave: (partial: Partial<LightCueData>) => void;
}

const btnStyle: React.CSSProperties = {
  padding: "3px 8px", background: "#1e293b", border: "1px solid #334155",
  borderRadius: 4, color: "#94a3b8", fontSize: 11, cursor: "pointer",
};
const cardStyle: React.CSSProperties = {
  background: "#0f172a", border: "1px solid #1e293b", borderRadius: 6,
  padding: 8, marginBottom: 8, display: "flex", flexDirection: "column", gap: 4,
};
const swatchStyle: React.CSSProperties = {
  width: 28, height: 22, padding: 0, border: "1px solid #334155", borderRadius: 4, background: "none", cursor: "pointer",
};

function Slider({ label, pct, onPct }: { label: string; pct: number; onPct: (p: number) => void }) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
      <span style={{ color: "#64748b", fontSize: 10, width: 52, flexShrink: 0 }}>{label}</span>
      <input type="range" min={0} max={100} value={pct} style={{ flex: 1 }} onChange={(e) => onPct(Number(e.target.value))} />
      <span style={{ color: "#cbd5e1", width: 34, textAlign: "right", fontSize: 11 }}>{pct}%</span>
    </div>
  );
}

/** The parameter kinds present across a group's member fixtures. */
function groupKinds(group: FixtureGroup, fixtures: PatchedFixture[]): Set<string> {
  const kinds = new Set<string>();
  for (const fid of group.fixture_ids) {
    const f = fixtures.find((x) => x.id === fid);
    if (f) for (const p of f.fixture_type.parameters) kinds.add(p.kind);
  }
  return kinds;
}

function FixtureCard({
  fixture, valueOf, setValues, onRemove,
}: {
  fixture: PatchedFixture;
  valueOf: (paramIndex: number) => number;
  setValues: (changes: { paramIndex: number; value: number }[]) => void;
  onRemove: () => void;
}) {
  const params = fixture.fixture_type.parameters;
  const intIdx = paramIndexOfKind(params, "intensity");
  const rIdx = paramIndexOfKind(params, "red");
  const gIdx = paramIndexOfKind(params, "green");
  const bIdx = paramIndexOfKind(params, "blue");
  const hasRgb = rIdx >= 0 && gIdx >= 0 && bIdx >= 0;
  const colorIdx = new Set([rIdx, gIdx, bIdx].filter((i) => i >= 0));

  return (
    <div style={cardStyle}>
      <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 4 }}>
        <span style={{ color: "#e2e8f0", flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{fixture.label}</span>
        {hasRgb && (
          <input
            type="color"
            value={rgbToHex(valueOf(rIdx), valueOf(gIdx), valueOf(bIdx))}
            onChange={(e) => {
              const [r, g, b] = hexToRgb(e.target.value);
              setValues([{ paramIndex: rIdx, value: r }, { paramIndex: gIdx, value: g }, { paramIndex: bIdx, value: b }]);
            }}
            style={swatchStyle}
            title="Colour"
          />
        )}
        <button style={{ ...btnStyle, color: "#ef4444", padding: "1px 6px" }} onClick={onRemove} title="Remove from cue">✕</button>
      </div>
      {intIdx >= 0 && <Slider label="Dimmer" pct={Math.round(valueOf(intIdx) * 100)} onPct={(p) => setValues([{ paramIndex: intIdx, value: p / 100 }])} />}
      {params.map((p, i) =>
        i === intIdx || colorIdx.has(i) ? null : (
          <Slider key={i} label={p.name} pct={Math.round(valueOf(i) * 100)} onPct={(pp) => setValues([{ paramIndex: i, value: pp / 100 }])} />
        ),
      )}
    </div>
  );
}

function GroupCard({
  group, kinds, valueOf, setValues, onRemove,
}: {
  group: FixtureGroup;
  kinds: Set<string>;
  valueOf: (kind: string) => number;
  setValues: (changes: { param_kind: string; value: number }[]) => void;
  onRemove: () => void;
}) {
  const hasInt = kinds.has("intensity");
  const hasRgb = kinds.has("red") && kinds.has("green") && kinds.has("blue");

  return (
    <div style={{ ...cardStyle, borderColor: "#3b82f6" }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 4 }}>
        <span style={{ color: "#bfdbfe", flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          👥 {group.label} <span style={{ color: "#64748b", fontSize: 10 }}>({group.fixture_ids.length})</span>
        </span>
        {hasRgb && (
          <input
            type="color"
            value={rgbToHex(valueOf("red"), valueOf("green"), valueOf("blue"))}
            onChange={(e) => {
              const [r, g, b] = hexToRgb(e.target.value);
              setValues([{ param_kind: "red", value: r }, { param_kind: "green", value: g }, { param_kind: "blue", value: b }]);
            }}
            style={swatchStyle}
            title="Colour (all members)"
          />
        )}
        <button style={{ ...btnStyle, color: "#ef4444", padding: "1px 6px" }} onClick={onRemove} title="Remove from cue">✕</button>
      </div>
      {hasInt && <Slider label="Dimmer" pct={Math.round(valueOf("intensity") * 100)} onPct={(p) => setValues([{ param_kind: "intensity", value: p / 100 }])} />}
    </div>
  );
}

export function LightTab({ cue, onSave }: Props) {
  const [fixtures, setFixtures] = useState<PatchedFixture[]>([]);
  const [groups, setGroups] = useState<FixtureGroup[]>([]);
  const [addFixId, setAddFixId] = useState("");
  const [addGrpId, setAddGrpId] = useState("");
  const targets = cue.targets ?? [];

  useEffect(() => {
    listFixtures().then(setFixtures).catch(console.error);
    listFixtureGroups().then(setGroups).catch(console.error);
  }, []);

  const saveTargets = (updated: ParamTarget[]) => onSave({ targets: updated } as Partial<LightCueData>);

  const valueOfFixture = (fixtureId: string, paramIndex: number) =>
    targets.find((t) => t.kind === "fixture" && t.fixture_id === fixtureId && t.param_index === paramIndex)?.value ?? 0;

  const valueOfGroup = (groupId: string, kind: string) =>
    targets.find((t) => t.kind === "group" && t.group_id === groupId && t.param_kind === kind)?.value ?? 0;

  const setFixtureValues = (fixtureId: string, changes: { paramIndex: number; value: number }[]) => {
    const next = [...targets];
    for (const c of changes) {
      const i = next.findIndex((t) => t.kind === "fixture" && t.fixture_id === fixtureId && t.param_index === c.paramIndex);
      if (i >= 0) next[i] = { ...next[i], value: c.value } as ParamTarget;
      else next.push({ kind: "fixture", fixture_id: fixtureId, param_index: c.paramIndex, value: c.value });
    }
    saveTargets(next);
  };

  const setGroupValues = (groupId: string, changes: { param_kind: string; value: number }[]) => {
    const next = [...targets];
    for (const c of changes) {
      const i = next.findIndex((t) => t.kind === "group" && t.group_id === groupId && t.param_kind === c.param_kind);
      if (i >= 0) next[i] = { ...next[i], value: c.value } as ParamTarget;
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      else next.push({ kind: "group", group_id: groupId, param_kind: c.param_kind as any, value: c.value });
    }
    saveTargets(next);
  };

  const removeFixture = (fixtureId: string) => saveTargets(targets.filter((t) => !(t.kind === "fixture" && t.fixture_id === fixtureId)));
  const removeGroup = (groupId: string) => saveTargets(targets.filter((t) => !(t.kind === "group" && t.group_id === groupId)));

  const addFixture = (fixtureId: string) => {
    const f = fixtures.find((x) => x.id === fixtureId);
    if (!f) return;
    const added: ParamTarget[] = f.fixture_type.parameters.map((p, i) => ({ kind: "fixture", fixture_id: fixtureId, param_index: i, value: p.default }));
    saveTargets([...targets, ...added]);
    setAddFixId("");
  };

  const addGroup = (groupId: string) => {
    const g = groups.find((x) => x.id === groupId);
    if (!g) return;
    const kinds = groupKinds(g, fixtures);
    const added: ParamTarget[] = [];
    if (kinds.has("intensity")) added.push({ kind: "group", group_id: groupId, param_kind: "intensity", value: 0 });
    if (kinds.has("red") && kinds.has("green") && kinds.has("blue")) {
      for (const k of ["red", "green", "blue"] as const) added.push({ kind: "group", group_id: groupId, param_kind: k, value: 0 });
    }
    saveTargets([...targets, ...added]);
    setAddGrpId("");
  };

  const handleCapture = async () => {
    try {
      saveTargets(await captureLiveTargets());
    } catch (e) {
      console.error(e);
    }
  };

  const presentGroupIds = Array.from(new Set(targets.filter((t) => t.kind === "group").map((t) => (t.kind === "group" ? t.group_id : ""))));
  const presentFixtureIds = Array.from(new Set(targets.filter((t) => t.kind === "fixture").map((t) => (t.kind === "fixture" ? t.fixture_id : ""))));
  const availableGroups = groups.filter((g) => !presentGroupIds.includes(g.id));
  const availableFixtures = fixtures.filter((f) => !presentFixtureIds.includes(f.id));
  const fade = cue.fade ?? { duration_ms: 0, curve: "s_curve" };
  const isEmpty = presentGroupIds.length === 0 && presentFixtureIds.length === 0;

  return (
    <div>
      <Field label="Fade time (s)">
        <input
          key={`fade-${cue.id}`}
          style={inputStyle}
          type="number"
          step="0.1"
          min="0"
          defaultValue={(fade.duration_ms / 1000).toFixed(2)}
          onBlur={(e) => onSave({ fade: { ...fade, duration_ms: e.target.value ? Math.round(parseFloat(e.target.value) * 1000) : 0 } } as Partial<LightCueData>)}
        />
      </Field>
      <Field label="Curve">
        <CurveSelect value={fade.curve} onChange={(v) => onSave({ fade: { ...fade, curve: v } } as Partial<LightCueData>)} />
      </Field>

      <div style={{ display: "flex", alignItems: "center", margin: "14px 0 8px" }}>
        <div style={{ flex: 1, fontSize: 11, color: "#64748b", textTransform: "uppercase", letterSpacing: "0.05em" }}>Targets</div>
        <button
          style={{ ...btnStyle, color: "#fbbf24", borderColor: "#a16207" }}
          onClick={handleCapture}
          title="Record the current live state of every patched fixture (DMX panel → Dashboard) into this cue"
        >
          ⏺ Capture live state
        </button>
      </div>

      {isEmpty && (
        <p style={{ color: "#475569", fontSize: 12, marginBottom: 8 }}>
          No targets — add a group or fixture below, or sculpt them in the DMX Dashboard and Capture.
        </p>
      )}

      {presentGroupIds.map((gid) => {
        const g = groups.find((x) => x.id === gid);
        if (!g) {
          return (
            <div key={gid} style={cardStyle}>
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <span style={{ color: "#f59e0b", fontSize: 11, flex: 1 }}>⚠ group no longer exists</span>
                <button style={{ ...btnStyle, color: "#ef4444", padding: "1px 6px" }} onClick={() => removeGroup(gid)}>✕</button>
              </div>
            </div>
          );
        }
        return (
          <GroupCard
            key={gid}
            group={g}
            kinds={groupKinds(g, fixtures)}
            valueOf={(k) => valueOfGroup(gid, k)}
            setValues={(changes) => setGroupValues(gid, changes)}
            onRemove={() => removeGroup(gid)}
          />
        );
      })}

      {presentFixtureIds.map((fid) => {
        const f = fixtures.find((x) => x.id === fid);
        if (!f) {
          return (
            <div key={fid} style={cardStyle}>
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <span style={{ color: "#f59e0b", fontSize: 11, flex: 1 }}>⚠ fixture no longer patched</span>
                <button style={{ ...btnStyle, color: "#ef4444", padding: "1px 6px" }} onClick={() => removeFixture(fid)}>✕</button>
              </div>
            </div>
          );
        }
        return (
          <FixtureCard
            key={fid}
            fixture={f}
            valueOf={(pi) => valueOfFixture(fid, pi)}
            setValues={(changes) => setFixtureValues(fid, changes)}
            onRemove={() => removeFixture(fid)}
          />
        );
      })}

      <div style={{ display: "flex", flexDirection: "column", gap: 6, marginTop: 6 }}>
        {availableGroups.length > 0 && (
          <div style={{ display: "flex", gap: 6 }}>
            <Select style={{ ...inputStyle, flex: 1, cursor: "pointer" }} value={addGrpId} onChange={(e) => setAddGrpId(e.target.value)}>
              <option value="">— Add group —</option>
              {availableGroups.map((g) => (<option key={g.id} value={g.id}>👥 {g.label}</option>))}
            </Select>
            <button style={btnStyle} disabled={!addGrpId} onClick={() => addGrpId && addGroup(addGrpId)}>+ Add</button>
          </div>
        )}
        {availableFixtures.length > 0 && (
          <div style={{ display: "flex", gap: 6 }}>
            <Select style={{ ...inputStyle, flex: 1, cursor: "pointer" }} value={addFixId} onChange={(e) => setAddFixId(e.target.value)}>
              <option value="">— Add fixture —</option>
              {availableFixtures.map((f) => (<option key={f.id} value={f.id}>{f.label}</option>))}
            </Select>
            <button style={btnStyle} disabled={!addFixId} onClick={() => addFixId && addFixture(addFixId)}>+ Add</button>
          </div>
        )}
      </div>

      {fixtures.length === 0 && (
        <p style={{ color: "#475569", fontSize: 11, marginTop: 8 }}>No fixtures patched. Add them in the DMX panel → Fixtures.</p>
      )}
    </div>
  );
}
