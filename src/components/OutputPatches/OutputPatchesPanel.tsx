// Output Patch management — named device+channel routes for Audio/Video cues.
// Shown in Preferences → Audio. Mirrors InputPatchesPanel; patches live in the
// workspace and are resolved by cues at GO time.

import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import type { DeviceInfo, OutputPatch } from "../../lib/types";
import {
  listOutputDevices,
  getOutputPatchTable,
  setOutputPatch,
  removeOutputPatch,
  setDefaultOutputPatch,
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

interface EditablePatch extends OutputPatch {
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

export function OutputPatchesPanel({ backend }: { backend?: string }) {
  const [patches, setPatches] = useState<EditablePatch[]>([]);
  const [defaultId, setDefaultId] = useState<string | null>(null);
  const [devices, setDevices] = useState<DeviceInfo[]>([]);
  const [channelText, setChannelText] = useState<Record<string, string>>({});

  const reload = () =>
    getOutputPatchTable()
      .then((t) => {
        setPatches(t.patches);
        setDefaultId(t.default_patch_id);
      })
      .catch(console.error);

  useEffect(() => {
    const refresh = () => {
      reload();
      listOutputDevices(backend).then(setDevices).catch(console.error);
    };
    refresh();
    // The preferences window persists hidden between opens, and the device
    // universe changes with the audio backend (ASIO vs shared) — refetch on
    // every focus AND when the backend/device is applied (device-changed).
    window.addEventListener("focus", refresh);
    const unlisten = listen("device-changed", refresh);
    return () => {
      window.removeEventListener("focus", refresh);
      void unlisten.then((u) => u());
    };
    // Re-runs when the Backend dropdown changes, so the patch device list
    // follows the selection instead of waiting for Apply.
  }, [backend]);

  const handleAdd = async () => {
    const device = devices[0]?.id ?? "";
    try {
      await setOutputPatch(null, "New Output", device, [0, 1]);
      await reload();
    } catch (e) { console.error(e); }
  };

  const handleChange = (id: string, field: keyof OutputPatch, value: string | number[]) => {
    setPatches((prev) =>
      prev.map((p) => (p.id === id ? { ...p, [field]: value, dirty: true } : p)),
    );
  };

  const commit = async (patch: EditablePatch) => {
    if (!patch.dirty) return;
    try {
      await setOutputPatch(patch.id, patch.name, patch.device_id, patch.channels);
      setPatches((prev) => prev.map((p) => (p.id === patch.id ? { ...p, dirty: false } : p)));
    } catch (e) { console.error(e); }
  };

  /** Set the device and persist immediately (the custom Select has no onBlur). */
  const setDevice = async (patch: EditablePatch, deviceId: string) => {
    try {
      await setOutputPatch(patch.id, patch.name, deviceId, patch.channels);
      setPatches((prev) =>
        prev.map((p) => (p.id === patch.id ? { ...p, device_id: deviceId, dirty: false } : p)),
      );
    } catch (e) { console.error(e); }
  };

  const handleRemove = async (id: string) => {
    try {
      await removeOutputPatch(id);
      await reload();
    } catch (e) { console.error(e); }
  };

  const handleSetDefault = async (id: string) => {
    try {
      await setDefaultOutputPatch(id);
      setDefaultId(id);
    } catch (e) { console.error(e); }
  };

  return (
    <div>
      <div style={{ display: "flex", alignItems: "center", marginBottom: 6 }}>
        <span style={{ fontSize: 11, fontWeight: 600, color: "var(--wc-text-muted)", textTransform: "uppercase", letterSpacing: "0.06em" }}>
          Output Patches
        </span>
        <button style={{ ...btnStyle, marginLeft: "auto" }} onClick={() => void handleAdd()}>
          + Add
        </button>
      </div>

      {patches.length === 0 && (
        <div style={{ fontSize: 11, color: "var(--wc-text-faint)", marginBottom: 4 }}>
          No patches — all audio plays on the main output device, channels 1-2.
        </div>
      )}

      {patches.map((patch) => {
        const deviceMissing = !devices.some((d) => d.id === patch.device_id);
        return (
        <div
          key={patch.id}
          style={{
            display: "grid",
            gridTemplateColumns: "18px 1fr 1.4fr 70px 24px",
            gap: 6, alignItems: "center", marginBottom: 4,
          }}
        >
          <button
            title={defaultId === patch.id ? "Default patch" : "Make default"}
            onClick={() => void handleSetDefault(patch.id)}
            style={{
              background: "transparent", border: "none", cursor: "pointer",
              color: defaultId === patch.id ? "var(--wc-accent)" : "var(--wc-text-faint)",
              fontSize: 13, padding: 0,
            }}
          >
            {defaultId === patch.id ? "★" : "☆"}
          </button>
          <input
            style={inputStyle}
            value={patch.name}
            onChange={(e) => handleChange(patch.id, "name", e.target.value)}
            onBlur={() => void commit(patch)}
          />
          <Select
            style={{
              ...inputStyle,
              cursor: "pointer",
              ...(deviceMissing ? { border: "1px solid #f59e0b", color: "#f59e0b" } : {}),
            }}
            value={patch.device_id}
            onChange={(e) => void setDevice(patch, e.target.value)}
          >
            {deviceMissing && (
              <option value={patch.device_id}>⚠ unavailable — {patch.device_id}</option>
            )}
            {devices.map((d) => (
              <option key={d.id} value={d.id}>{d.name}</option>
            ))}
          </Select>
          <input
            style={inputStyle}
            title="Output channels, 1-based (e.g. 1, 2)"
            value={channelText[patch.id] ?? channelsToText(patch.channels)}
            onChange={(e) => {
              setChannelText((prev) => ({ ...prev, [patch.id]: e.target.value }));
              handleChange(patch.id, "channels", textToChannels(e.target.value));
            }}
            onBlur={() => {
              setChannelText((prev) => {
                const { [patch.id]: _removed, ...rest } = prev;
                return rest;
              });
              void commit(patch);
            }}
          />
          <button
            style={{ ...btnStyle, padding: "4px 6px" }}
            title="Remove patch"
            onClick={() => void handleRemove(patch.id)}
          >
            ✕
          </button>
        </div>
        );
      })}

      <span style={{ fontSize: 10, color: "var(--wc-text-faint)" }}>
        ★ = default for cues with no explicit patch. Channels are 1-based on the
        selected device. Cues pick their patch in the inspector's Levels tab.
      </span>
    </div>
  );
}
