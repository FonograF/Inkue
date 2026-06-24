// Input Patch management — named live-audio capture sources for Mic Cues.
// Shown in Preferences → Audio. Mirrors OscPatchesPanel.

import { useEffect, useState } from "react";
import type { DeviceInfo, InputPatch } from "../../lib/types";
import {
  listInputDevices,
  listInputPatches,
  addInputPatch,
  updateInputPatch,
  removeInputPatch,
} from "../../lib/commands";
import { Select } from "../common/Select";

const inputStyle: React.CSSProperties = {
  background: "var(--wc-bg-app)",
  border: "1px solid var(--wc-border-strong)",
  borderRadius: 4,
  color: "var(--wc-text)",
  fontSize: 12,
  padding: "4px 8px",
  width: "100%",
};

const btnStyle: React.CSSProperties = {
  padding: "4px 10px",
  background: "var(--wc-bg-surface)",
  border: "1px solid var(--wc-border-strong)",
  borderRadius: 4,
  color: "var(--wc-text-secondary)",
  fontSize: 12,
  cursor: "pointer",
};

interface EditablePatch extends InputPatch {
  dirty?: boolean;
}

/** 0-based channel array → "1, 2" (1-based for display). */
function channelsToText(channels: number[]): string {
  return channels.map((c) => c + 1).join(", ");
}

/** "1, 2" (1-based) → 0-based channel array, ignoring junk. */
function textToChannels(text: string): number[] {
  return text
    .split(",")
    .map((s) => parseInt(s.trim(), 10))
    .filter((n) => Number.isFinite(n) && n >= 1)
    .map((n) => n - 1);
}

export function InputPatchesPanel() {
  const [patches, setPatches] = useState<EditablePatch[]>([]);
  const [devices, setDevices] = useState<DeviceInfo[]>([]);
  const [channelText, setChannelText] = useState<Record<string, string>>({});

  const reload = () => listInputPatches().then(setPatches).catch(console.error);

  useEffect(() => {
    reload();
    listInputDevices().then(setDevices).catch(console.error);
  }, []);

  const handleAdd = async () => {
    const device = devices[0]?.id ?? "";
    try {
      const patch = await addInputPatch("New Input", device, [0, 1]);
      setPatches((prev) => [...prev, patch]);
    } catch (e) { console.error(e); }
  };

  const handleChange = (id: string, field: keyof InputPatch, value: string | number[]) => {
    setPatches((prev) =>
      prev.map((p) => (p.id === id ? { ...p, [field]: value, dirty: true } : p)),
    );
  };

  const commit = async (patch: EditablePatch) => {
    if (!patch.dirty) return;
    try {
      await updateInputPatch({ id: patch.id, name: patch.name, device_id: patch.device_id, channels: patch.channels });
      setPatches((prev) => prev.map((p) => (p.id === patch.id ? { ...p, dirty: false } : p)));
    } catch (e) { console.error(e); }
  };

  /** Set the device and persist immediately (the custom Select has no onBlur). */
  const setDevice = async (patch: EditablePatch, deviceId: string) => {
    try {
      await updateInputPatch({ id: patch.id, name: patch.name, device_id: deviceId, channels: patch.channels });
      setPatches((prev) => prev.map((p) => (p.id === patch.id ? { ...p, device_id: deviceId, dirty: false } : p)));
    } catch (e) { console.error(e); }
  };

  const handleRemove = async (id: string) => {
    try {
      await removeInputPatch(id);
      setPatches((prev) => prev.filter((p) => p.id !== id));
    } catch (e) { console.error(e); }
  };

  return (
    <div>
      <div style={{
        fontSize: 11, fontWeight: 600, color: "var(--wc-text-muted)",
        textTransform: "uppercase", letterSpacing: "0.07em",
        marginBottom: 10, paddingBottom: 5,
        borderBottom: "1px solid var(--wc-border)",
      }}>
        Input Patches (Mic Cues)
      </div>

      {patches.length === 0 && (
        <p style={{ color: "var(--wc-text-faint)", fontSize: 12, marginBottom: 8 }}>
          No input patches. Add one to route a live mic / line into a Mic Cue.
        </p>
      )}

      {patches.map((patch) => (
        <div
          key={patch.id}
          style={{ display: "grid", gridTemplateColumns: "1fr 1fr 90px 28px", gap: 6, marginBottom: 6, alignItems: "center" }}
        >
          <input
            style={inputStyle}
            value={patch.name}
            placeholder="Name"
            onChange={(e) => handleChange(patch.id, "name", e.target.value)}
            onBlur={() => commit(patch)}
          />
          <Select
            style={{ ...inputStyle, cursor: "pointer" }}
            value={patch.device_id}
            onChange={(e) => { void setDevice(patch, e.target.value); }}
          >
            <option value="">— default input —</option>
            {devices.map((d) => (
              <option key={d.id} value={d.id}>{d.name}</option>
            ))}
            {patch.device_id && !devices.some((d) => d.id === patch.device_id) && (
              <option value={patch.device_id}>{patch.device_id} (not found)</option>
            )}
          </Select>
          <input
            style={inputStyle}
            value={channelText[patch.id] ?? channelsToText(patch.channels)}
            placeholder="1, 2"
            title="Device channels (1-based), comma-separated"
            onChange={(e) => {
              setChannelText((t) => ({ ...t, [patch.id]: e.target.value }));
              handleChange(patch.id, "channels", textToChannels(e.target.value));
            }}
            onBlur={() => {
              setChannelText((t) => { const next = { ...t }; delete next[patch.id]; return next; });
              commit(patch);
            }}
          />
          <button
            style={{ ...btnStyle, color: "#ef4444", padding: "4px 6px" }}
            onClick={() => handleRemove(patch.id)}
            title="Remove patch"
          >
            ✕
          </button>
        </div>
      ))}

      <button style={{ ...btnStyle, marginTop: 4 }} onClick={handleAdd}>
        + Add Input Patch
      </button>
    </div>
  );
}
