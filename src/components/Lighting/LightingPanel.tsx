// Floating DMX lighting panel (M1 dev surface): configure universe outputs
// (sACN / Art-Net), poke a channel, toggle blackout, and watch the live values
// streamed from the backend `dmx-monitor` event.

import { useEffect, useState } from "react";
import type { CSSProperties, ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  dmxGetBlackout,
  dmxSetBlackout,
  dmxSetChannel,
  dmxSetOutputs,
} from "../../lib/commands";
import type { DmxUniverseSnapshot, OutputProtocol, UniverseOutput } from "../../lib/types";

const LS_OUTPUTS_KEY = "wincue.dmx.outputs";

function loadOutputs(): UniverseOutput[] {
  try {
    const raw = localStorage.getItem(LS_OUTPUTS_KEY);
    if (raw) return JSON.parse(raw) as UniverseOutput[];
  } catch {
    /* ignore corrupt state */
  }
  return [{ universe: 1, protocol: "Sacn", destination: null, enabled: true }];
}

export function LightingPanel({ onClose }: { onClose: () => void }) {
  const [outputs, setOutputs] = useState<UniverseOutput[]>(loadOutputs);
  const [blackout, setBlackout] = useState(false);
  const [snapshot, setSnapshot] = useState<DmxUniverseSnapshot[]>([]);
  const [testUniverse, setTestUniverse] = useState(1);
  const [testAddress, setTestAddress] = useState(1);
  const [testValue, setTestValue] = useState(0);

  // Push the persisted outputs to the engine on mount (covers an app reload),
  // and pull the current blackout state.
  useEffect(() => {
    dmxSetOutputs(outputs).catch(console.error);
    dmxGetBlackout().then(setBlackout).catch(console.error);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Live monitor feed.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<DmxUniverseSnapshot[]>("dmx-monitor", (e) => setSnapshot(e.payload))
      .then((u) => { unlisten = u; })
      .catch(console.error);
    return () => unlisten?.();
  }, []);

  const pushOutputs = (next: UniverseOutput[]) => {
    setOutputs(next);
    localStorage.setItem(LS_OUTPUTS_KEY, JSON.stringify(next));
    dmxSetOutputs(next).catch(console.error);
  };

  const updateOutput = (i: number, patch: Partial<UniverseOutput>) =>
    pushOutputs(outputs.map((o, j) => (j === i ? { ...o, ...patch } : o)));

  const addOutput = () =>
    pushOutputs([...outputs, { universe: outputs.length + 1, protocol: "Sacn", destination: null, enabled: true }]);

  const removeOutput = (i: number) => pushOutputs(outputs.filter((_, j) => j !== i));

  const toggleBlackout = () => {
    const next = !blackout;
    setBlackout(next);
    dmxSetBlackout(next).catch(console.error);
  };

  const poke = (universe: number, address: number, value: number) => {
    setTestUniverse(universe);
    setTestAddress(address);
    setTestValue(value);
    dmxSetChannel(universe, address, value).catch(console.error);
  };

  return (
    <div style={panelStyle} onClick={(e) => e.stopPropagation()}>
      {/* Header */}
      <div style={headerStyle}>
        <span style={{ color: blackout ? "#ef4444" : "#a855f7", fontSize: 10 }}>●</span>
        <span style={{ color: "#94a3b8", fontWeight: 600, fontSize: 12, flex: 1 }}>Lighting (DMX)</span>
        <button onClick={toggleBlackout} style={blackout ? blackoutOnBtn : blackoutOffBtn}>
          {blackout ? "BLACKOUT ON" : "Blackout"}
        </button>
        <button onClick={onClose} style={closeBtn}>✕</button>
      </div>

      <div style={{ overflowY: "auto", flex: 1, padding: "8px 10px", display: "flex", flexDirection: "column", gap: 12 }}>
        {/* Outputs */}
        <Section title="Universe outputs">
          {outputs.map((o, i) => (
            <div key={i} style={{ display: "flex", gap: 6, alignItems: "center" }}>
              <input
                type="checkbox"
                checked={o.enabled}
                onChange={(e) => updateOutput(i, { enabled: e.target.checked })}
                title="Enabled"
              />
              <label style={lblStyle}>U</label>
              <input
                type="number" min={1} max={63999} value={o.universe}
                onChange={(e) => updateOutput(i, { universe: Number(e.target.value) })}
                style={{ ...numStyle, width: 52 }}
              />
              <select
                value={o.protocol}
                onChange={(e) => updateOutput(i, { protocol: e.target.value as OutputProtocol })}
                style={selStyle}
              >
                <option value="Sacn">sACN</option>
                <option value="ArtNet">Art-Net</option>
              </select>
              <input
                type="text"
                placeholder={o.protocol === "Sacn" ? "multicast" : "dest IP (required)"}
                value={o.destination ?? ""}
                onChange={(e) => updateOutput(i, { destination: e.target.value.trim() || null })}
                style={{ ...txtStyle, flex: 1 }}
              />
              <button onClick={() => removeOutput(i)} style={smallBtn} title="Remove">✕</button>
            </div>
          ))}
          <button onClick={addOutput} style={addBtn}>+ universe</button>
        </Section>

        {/* Test channel */}
        <Section title="Test channel (no fade)">
          <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
            <label style={lblStyle}>U</label>
            <input type="number" min={1} value={testUniverse}
              onChange={(e) => poke(Number(e.target.value), testAddress, testValue)}
              style={{ ...numStyle, width: 48 }} />
            <label style={lblStyle}>ch</label>
            <input type="number" min={1} max={512} value={testAddress}
              onChange={(e) => poke(testUniverse, Number(e.target.value), testValue)}
              style={{ ...numStyle, width: 56 }} />
            <input type="range" min={0} max={255} value={testValue}
              onChange={(e) => poke(testUniverse, testAddress, Number(e.target.value))}
              style={{ flex: 1 }} />
            <span style={{ color: "#cbd5e1", width: 30, textAlign: "right" }}>{testValue}</span>
          </div>
        </Section>

        {/* Monitor */}
        <Section title="Live output">
          {snapshot.length === 0 ? (
            <div style={{ color: "#334155", fontSize: 12 }}>No active universe yet — set a channel.</div>
          ) : (
            snapshot.map((u) => {
              const active = u.channels
                .map((v, idx) => ({ addr: idx + 1, v }))
                .filter((c) => c.v > 0);
              return (
                <div key={u.universe} style={{ marginBottom: 4 }}>
                  <span style={{ color: "#a855f7", fontWeight: 600 }}>U{u.universe}</span>
                  {active.length === 0 ? (
                    <span style={{ color: "#475569", marginLeft: 8 }}>all zero</span>
                  ) : (
                    <span style={{ marginLeft: 8, display: "inline-flex", flexWrap: "wrap", gap: 6 }}>
                      {active.map((c) => (
                        <span key={c.addr} style={chipStyle}>
                          <span style={{ color: "#64748b" }}>{c.addr}</span>:
                          <span style={{ color: "#e2e8f0" }}>{c.v}</span>
                        </span>
                      ))}
                    </span>
                  )}
                </div>
              );
            })
          )}
        </Section>
      </div>

      <div style={footerStyle}>
        Loopback test: point QLC+ / sACNView / OLA at this universe. sACN → multicast 239.255.0.U.
      </div>
    </div>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
      <div style={{ color: "#64748b", fontSize: 10, textTransform: "uppercase", letterSpacing: 0.5 }}>{title}</div>
      {children}
    </div>
  );
}

const panelStyle: CSSProperties = {
  position: "fixed", bottom: 104, right: 16, width: 480, maxHeight: 460,
  background: "#020617", border: "1px solid #334155", borderRadius: 8,
  boxShadow: "0 8px 32px rgba(0,0,0,0.7)", zIndex: 9999,
  display: "flex", flexDirection: "column", fontFamily: "monospace", fontSize: 12,
};
const headerStyle: CSSProperties = {
  display: "flex", alignItems: "center", padding: "6px 10px",
  borderBottom: "1px solid #1e293b", gap: 8, flexShrink: 0,
};
const footerStyle: CSSProperties = {
  padding: "4px 12px", borderTop: "1px solid #1e293b", fontSize: 10, color: "#334155", flexShrink: 0,
};
const lblStyle: CSSProperties = { color: "#64748b", fontSize: 11 };
const numStyle: CSSProperties = {
  background: "#0f172a", border: "1px solid #334155", borderRadius: 4, color: "#e2e8f0", padding: "2px 4px", fontSize: 12,
};
const txtStyle: CSSProperties = { ...numStyle };
const selStyle: CSSProperties = { ...numStyle, cursor: "pointer" };
const chipStyle: CSSProperties = {
  background: "#0f172a", border: "1px solid #1e293b", borderRadius: 4, padding: "0 5px", fontSize: 11,
};
const smallBtn: CSSProperties = {
  background: "none", border: "1px solid #334155", borderRadius: 4, color: "#64748b", fontSize: 11, padding: "1px 6px", cursor: "pointer",
};
const addBtn: CSSProperties = { ...smallBtn, alignSelf: "flex-start", color: "#a855f7" };
const closeBtn: CSSProperties = {
  background: "none", border: "none", color: "#64748b", fontSize: 16, cursor: "pointer", lineHeight: 1, padding: "0 2px",
};
const blackoutOffBtn: CSSProperties = {
  background: "none", border: "1px solid #334155", borderRadius: 4, color: "#94a3b8", fontSize: 11, padding: "1px 8px", cursor: "pointer",
};
const blackoutOnBtn: CSSProperties = {
  background: "#7f1d1d", border: "1px solid #ef4444", borderRadius: 4, color: "#fecaca", fontSize: 11, padding: "1px 8px", cursor: "pointer", fontWeight: 700,
};
