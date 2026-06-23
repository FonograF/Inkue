// Live lighting Dashboard (QLab-style): one row per patched fixture with an
// intensity slider + a colour picker (for RGB fixtures) + sliders for any other
// parameter. Editing drives the DMX engine live; the Light Cue inspector then
// captures this state with "Capture live state".
//
// The Dashboard owns its working values (seeded from the live engine snapshot),
// so dragging a slider never fights the ~20 fps monitor feed. "↻ Live" reseeds
// from the engine (e.g. after a cue GO), "Clear" zeroes every fixture.

import { useEffect, useState } from "react";
import type { CSSProperties } from "react";
import { dmxClearFixtures, dmxSetFixtureParam, listFixtures } from "../../lib/commands";
import type { DmxUniverseSnapshot, FixtureParam, PatchedFixture } from "../../lib/types";
import { hexToRgb, paramIndexOfKind, rgbToHex } from "../../lib/fixtureColor";

/** Read a parameter's current normalised value (0–1) from a universe snapshot. */
function valueFromSnapshot(fixture: PatchedFixture, param: FixtureParam, snap: DmxUniverseSnapshot[]): number {
  const u = snap.find((s) => s.universe === fixture.universe);
  if (!u) return 0;
  const addr0 = fixture.base_address - 1 + param.channel_offset;
  if (param.width === "Bit16") {
    const hi = u.channels[addr0] ?? 0;
    const lo = u.channels[addr0 + 1] ?? 0;
    return ((hi << 8) | lo) / 65535;
  }
  return (u.channels[addr0] ?? 0) / 255;
}

export function FixtureDashboard({ snapshot }: { snapshot: DmxUniverseSnapshot[] }) {
  const [fixtures, setFixtures] = useState<PatchedFixture[]>([]);
  // fixtureId -> value per parameter index.
  const [values, setValues] = useState<Record<string, number[]>>({});

  const seed = (fxs: PatchedFixture[], snap: DmxUniverseSnapshot[]) => {
    const next: Record<string, number[]> = {};
    for (const f of fxs) next[f.id] = f.fixture_type.parameters.map((p) => valueFromSnapshot(f, p, snap));
    setValues(next);
  };

  useEffect(() => {
    listFixtures()
      .then((fxs) => {
        setFixtures(fxs);
        seed(fxs, snapshot);
      })
      .catch(console.error);
    // Seed once on mount; the user reseeds explicitly via "↻ Live".
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const setParam = (fixtureId: string, paramIndex: number, value: number) => {
    setValues((v) => {
      const row = v[fixtureId] ? [...v[fixtureId]] : [];
      row[paramIndex] = value;
      return { ...v, [fixtureId]: row };
    });
    dmxSetFixtureParam(fixtureId, paramIndex, value).catch(console.error);
  };

  const clearAll = async () => {
    await dmxClearFixtures().catch(console.error);
    setValues(() => {
      const next: Record<string, number[]> = {};
      for (const f of fixtures) next[f.id] = f.fixture_type.parameters.map(() => 0);
      return next;
    });
  };

  if (fixtures.length === 0) {
    return <div style={{ color: "#334155", fontSize: 12 }}>Patch a fixture to sculpt it here.</div>;
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
      <div style={{ display: "flex", gap: 6, marginBottom: 2 }}>
        <button style={smallBtn} onClick={() => seed(fixtures, snapshot)} title="Reseed from the live engine state">↻ Live</button>
        <button style={smallBtn} onClick={clearAll} title="Set every fixture to zero">Clear</button>
      </div>

      {fixtures.map((f) => {
        const vals = values[f.id] ?? f.fixture_type.parameters.map(() => 0);
        const params = f.fixture_type.parameters;
        const intIdx = paramIndexOfKind(params, "intensity");
        const rIdx = paramIndexOfKind(params, "red");
        const gIdx = paramIndexOfKind(params, "green");
        const bIdx = paramIndexOfKind(params, "blue");
        const hasRgb = rIdx >= 0 && gIdx >= 0 && bIdx >= 0;
        const colorIdx = new Set([rIdx, gIdx, bIdx].filter((i) => i >= 0));

        return (
          <div key={f.id} style={rowStyle}>
            <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <span style={{ color: "#e2e8f0", flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {f.label}
              </span>
              {hasRgb && (
                <input
                  type="color"
                  value={rgbToHex(vals[rIdx] ?? 0, vals[gIdx] ?? 0, vals[bIdx] ?? 0)}
                  onChange={(e) => {
                    const [r, g, b] = hexToRgb(e.target.value);
                    setParam(f.id, rIdx, r);
                    setParam(f.id, gIdx, g);
                    setParam(f.id, bIdx, b);
                  }}
                  style={{ width: 28, height: 22, padding: 0, border: "1px solid #334155", borderRadius: 4, background: "none", cursor: "pointer" }}
                  title="Colour"
                />
              )}
            </div>

            {intIdx >= 0 && (
              <Slider
                label="Dimmer"
                pct={Math.round((vals[intIdx] ?? 0) * 100)}
                onPct={(p) => setParam(f.id, intIdx, p / 100)}
              />
            )}

            {/* Any parameter that is not the dimmer or part of the RGB picker. */}
            {f.fixture_type.parameters.map((p, i) =>
              i === intIdx || colorIdx.has(i) ? null : (
                <Slider
                  key={i}
                  label={p.name}
                  pct={Math.round((vals[i] ?? 0) * 100)}
                  onPct={(pp) => setParam(f.id, i, pp / 100)}
                />
              ),
            )}
          </div>
        );
      })}
    </div>
  );
}

function Slider({ label, pct, onPct }: { label: string; pct: number; onPct: (p: number) => void }) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
      <span style={{ color: "#64748b", fontSize: 10, width: 52, flexShrink: 0 }}>{label}</span>
      <input type="range" min={0} max={100} value={pct} style={{ flex: 1 }} onChange={(e) => onPct(Number(e.target.value))} />
      <span style={{ color: "#cbd5e1", width: 34, textAlign: "right", fontSize: 11 }}>{pct}%</span>
    </div>
  );
}

const rowStyle: CSSProperties = {
  display: "flex", flexDirection: "column", gap: 4,
  border: "1px solid #1e293b", borderRadius: 4, padding: "5px 7px",
};
const smallBtn: CSSProperties = {
  background: "none", border: "1px solid #334155", borderRadius: 4, color: "#94a3b8", fontSize: 11, padding: "2px 8px", cursor: "pointer",
};
