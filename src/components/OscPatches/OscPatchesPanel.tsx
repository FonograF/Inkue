// OSC Patch management panel — shown in Workspace Settings.
// Allows the operator to add, edit, and remove named UDP send targets.

import { useEffect, useState } from "react";
import type { OscPatch } from "../../lib/types";
import { listOscPatches, addOscPatch, updateOscPatch, removeOscPatch } from "../../lib/commands";

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

interface EditablePatch extends OscPatch {
  dirty?: boolean;
}

export function OscPatchesPanel() {
  const [patches, setPatches] = useState<EditablePatch[]>([]);

  const reload = () => listOscPatches().then(setPatches).catch(console.error);

  useEffect(() => { reload(); }, []);

  const handleAdd = async () => {
    try {
      const patch = await addOscPatch("New Patch", "127.0.0.1", 53000);
      setPatches((prev) => [...prev, patch]);
    } catch (e) { console.error(e); }
  };

  const handleChange = (id: string, field: keyof OscPatch, value: string | number) => {
    setPatches((prev) =>
      prev.map((p) =>
        p.id === id ? { ...p, [field]: field === "port" ? Number(value) : value, dirty: true } : p
      )
    );
  };

  const handleBlur = async (patch: EditablePatch) => {
    if (!patch.dirty) return;
    try {
      await updateOscPatch({ id: patch.id, name: patch.name, ip: patch.ip, port: patch.port });
      setPatches((prev) => prev.map((p) => p.id === patch.id ? { ...p, dirty: false } : p));
    } catch (e) { console.error(e); }
  };

  const handleRemove = async (id: string) => {
    try {
      await removeOscPatch(id);
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
        OSC Patches
      </div>

      {patches.length === 0 && (
        <p style={{ color: "var(--wc-text-faint)", fontSize: 12, marginBottom: 8 }}>
          No patches defined.
        </p>
      )}

      {/* Header row */}
      {patches.length > 0 && (
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 80px 28px", gap: 6, marginBottom: 4 }}>
          <span style={{ fontSize: 11, color: "var(--wc-text-muted)" }}>Name</span>
          <span style={{ fontSize: 11, color: "var(--wc-text-muted)" }}>IP</span>
          <span style={{ fontSize: 11, color: "var(--wc-text-muted)" }}>Port</span>
          <span />
        </div>
      )}

      {patches.map((patch) => (
        <div
          key={patch.id}
          style={{ display: "grid", gridTemplateColumns: "1fr 1fr 80px 28px", gap: 6, marginBottom: 6 }}
        >
          <input
            style={inputStyle}
            value={patch.name}
            onChange={(e) => handleChange(patch.id, "name", e.target.value)}
            onBlur={() => handleBlur(patch)}
          />
          <input
            style={inputStyle}
            value={patch.ip}
            placeholder="127.0.0.1"
            onChange={(e) => handleChange(patch.id, "ip", e.target.value)}
            onBlur={() => handleBlur(patch)}
          />
          <input
            style={inputStyle}
            type="number"
            min={1}
            max={65535}
            value={patch.port}
            onChange={(e) => handleChange(patch.id, "port", e.target.value)}
            onBlur={() => handleBlur(patch)}
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
        + Add Patch
      </button>
    </div>
  );
}
