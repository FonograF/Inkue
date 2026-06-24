// Inspector tab for OSC cues — manages the list of messages to send on GO.

import { useEffect, useState } from "react";
import type { OscArg, OscCueData, OscMessage, OscPatch } from "../../lib/types";
import { listOscPatches, sendOscTest } from "../../lib/commands";
import { Select } from "../common/Select";

interface Props {
  cue: OscCueData;
  onSave: (partial: Partial<OscCueData>) => void;
}

const inputStyle: React.CSSProperties = {
  background: "var(--wc-bg-app)",
  border: "1px solid var(--wc-border-strong)",
  borderRadius: 4,
  color: "var(--wc-text)",
  fontSize: 12,
  padding: "3px 6px",
};

const selectStyle: React.CSSProperties = { ...inputStyle, cursor: "pointer" };

const btnStyle: React.CSSProperties = {
  padding: "2px 8px",
  background: "var(--wc-bg-surface)",
  border: "1px solid var(--wc-border-strong)",
  borderRadius: 4,
  color: "var(--wc-text-secondary)",
  fontSize: 11,
  cursor: "pointer",
};

function ArgRow({
  arg,
  onChange,
  onRemove,
}: {
  arg: OscArg;
  onChange: (arg: OscArg) => void;
  onRemove: () => void;
}) {
  return (
    <div style={{ display: "flex", gap: 4, alignItems: "center", marginBottom: 4 }}>
      <Select
        style={{ ...selectStyle, width: 60 }}
        value={arg.type}
        onChange={(e) => {
          const type = e.target.value as OscArg["type"];
          const value = type === "int" ? 0 : type === "float" ? 0.0 : type === "bool" ? false : "";
          // eslint-disable-next-line @typescript-eslint/no-explicit-any
          onChange({ type, value } as any);
        }}
      >
        <option value="int">int</option>
        <option value="float">float</option>
        <option value="str">str</option>
        <option value="bool">bool</option>
      </Select>

      {arg.type === "bool" ? (
        <Select
          style={{ ...selectStyle, flex: 1 }}
          value={String(arg.value)}
          onChange={(e) => onChange({ type: "bool", value: e.target.value === "true" })}
        >
          <option value="true">true</option>
          <option value="false">false</option>
        </Select>
      ) : (
        <input
          style={{ ...inputStyle, flex: 1 }}
          type={arg.type === "str" ? "text" : "number"}
          step={arg.type === "float" ? "any" : "1"}
          value={String(arg.value)}
          onChange={(e) => {
            const raw = e.target.value;
            if (arg.type === "int") onChange({ type: "int", value: parseInt(raw, 10) || 0 });
            else if (arg.type === "float") onChange({ type: "float", value: parseFloat(raw) || 0 });
            else onChange({ type: "str", value: raw });
          }}
        />
      )}

      <button style={btnStyle} onClick={onRemove}>✕</button>
    </div>
  );
}

function MessageRow({
  msg,
  patches,
  onChange,
  onRemove,
}: {
  msg: OscMessage;
  patches: OscPatch[];
  onChange: (msg: OscMessage) => void;
  onRemove: () => void;
}) {
  const [testResult, setTestResult] = useState<string | null>(null);

  const handleTest = async () => {
    setTestResult("Sending…");
    try {
      const result = await sendOscTest(msg.patch_id, msg);
      setTestResult(result);
    } catch (e) {
      setTestResult(`Error: ${e}`);
    }
  };

  return (
    <div
      style={{
        background: "var(--wc-bg-app)",
        border: "1px solid var(--wc-border)",
        borderRadius: 6,
        padding: 8,
        marginBottom: 8,
      }}
    >
      {/* Row 1: patch + address */}
      <div style={{ display: "flex", gap: 6, alignItems: "center", marginBottom: 4 }}>
        <Select
          style={{ ...selectStyle, flex: "0 0 110px" }}
          value={msg.patch_id}
          onChange={(e) => onChange({ ...msg, patch_id: e.target.value })}
        >
          <option value="">— Patch —</option>
          {patches.map((p) => (
            <option key={p.id} value={p.id}>{p.name}</option>
          ))}
        </Select>
        <input
          style={{ ...inputStyle, flex: 1, minWidth: 0 }}
          placeholder="/address"
          value={msg.address}
          onChange={(e) => onChange({ ...msg, address: e.target.value })}
        />
      </div>

      {/* Row 2: actions + test result */}
      <div style={{ display: "flex", gap: 6, alignItems: "center", marginBottom: 4 }}>
        <button
          style={{ ...btnStyle, color: "#67e8f9", flex: 1 }}
          onClick={handleTest}
          title="Send this message immediately to test connectivity"
        >
          ▶ Test send
        </button>
        <button style={{ ...btnStyle, color: "#ef4444" }} onClick={onRemove}>Remove</button>
      </div>

      {testResult && (
        <div style={{
          fontSize: 11,
          fontFamily: "monospace",
          marginBottom: 4,
          padding: "3px 6px",
          borderRadius: 4,
          background: testResult.startsWith("OK") ? "#052e16" : "#450a0a",
          color: testResult.startsWith("OK") ? "#4ade80" : "#f87171",
          wordBreak: "break-all",
        }}>
          {testResult}
        </div>
      )}

      {msg.args.map((arg, i) => (
        <ArgRow
          key={i}
          arg={arg}
          onChange={(newArg) => {
            const args = [...msg.args];
            args[i] = newArg;
            onChange({ ...msg, args });
          }}
          onRemove={() => {
            const args = msg.args.filter((_, idx) => idx !== i);
            onChange({ ...msg, args });
          }}
        />
      ))}

      <button
        style={{ ...btnStyle, marginTop: 2 }}
        onClick={() => onChange({ ...msg, args: [...msg.args, { type: "int", value: 0 }] })}
      >
        + Arg
      </button>
    </div>
  );
}

export function OscTab({ cue, onSave }: Props) {
  const [patches, setPatches] = useState<OscPatch[]>([]);
  const messages = cue.messages ?? [];

  useEffect(() => {
    listOscPatches().then(setPatches).catch(console.error);
  }, []);

  const save = (updated: typeof messages) => {
    onSave({ messages: updated } as Partial<OscCueData>);
  };

  return (
    <div>
      {messages.length === 0 && (
        <p style={{ color: "var(--wc-text-faint)", fontSize: 12, marginBottom: 8 }}>
          No messages — add one below.
        </p>
      )}

      {messages.map((msg, i) => (
        <MessageRow
          key={i}
          msg={msg}
          patches={patches}
          onChange={(updated) => {
            const next = [...messages];
            next[i] = updated;
            save(next);
          }}
          onRemove={() => save(messages.filter((_, idx) => idx !== i))}
        />
      ))}

      <button
        style={{ ...btnStyle, width: "100%", padding: "6px 0", marginTop: 4 }}
        onClick={() =>
          save([
            ...messages,
            { patch_id: patches[0]?.id ?? "", address: "/", args: [] },
          ])
        }
      >
        + Add Message
      </button>

      {patches.length === 0 && (
        <p style={{ color: "var(--wc-text-faint)", fontSize: 11, marginTop: 8 }}>
          No OSC patches defined. Add them in Preferences → Network → OSC Patches.
        </p>
      )}
    </div>
  );
}
