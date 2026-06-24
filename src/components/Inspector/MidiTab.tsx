// Inspector tab for MIDI cues — manages the list of MIDI messages to send on GO.

import { useEffect, useState } from "react";
import type { MidiCueData, MidiMessage, MidiMessageType } from "../../lib/types";
import { listMidiOutputPorts, sendMidiTest } from "../../lib/commands";
import { Select } from "../common/Select";

interface Props {
  cue: MidiCueData;
  onSave: (partial: Partial<MidiCueData>) => void;
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

const MESSAGE_TYPE_LABELS: Record<MidiMessageType, string> = {
  note_on:        "Note On",
  note_off:       "Note Off",
  control_change: "Control Change",
  program_change: "Program Change",
};

function hasData2(type: MidiMessageType): boolean {
  return type !== "program_change";
}

function data1Label(type: MidiMessageType): string {
  if (type === "control_change") return "CC#";
  if (type === "program_change") return "Prog";
  return "Note";
}

function defaultMessage(ports: string[]): MidiMessage {
  return {
    port_name: ports[0] ?? "",
    message_type: "note_on",
    channel: 1,
    data1: 60,
    data2: 100,
  };
}

export function MidiTab({ cue, onSave }: Props) {
  const [ports, setPorts] = useState<string[]>([]);
  const [messages, setMessages] = useState<MidiMessage[]>(cue.messages ?? []);
  const [testResults, setTestResults] = useState<Record<number, string>>({});

  useEffect(() => {
    listMidiOutputPorts().then(setPorts).catch(console.error);
  }, []);

  // Sync local state when cue changes from outside (e.g. undo)
  useEffect(() => {
    setMessages(cue.messages ?? []);
  }, [cue.id]);

  const commit = (updated: MidiMessage[]) => {
    setMessages(updated);
    onSave({ messages: updated });
  };

  const addMessage = () => commit([...messages, defaultMessage(ports)]);

  const removeMessage = (idx: number) =>
    commit(messages.filter((_, i) => i !== idx));

  const updateMessage = (idx: number, patch: Partial<MidiMessage>) => {
    const updated = messages.map((m, i) => (i === idx ? { ...m, ...patch } : m));
    commit(updated);
  };

  const testSend = async (idx: number) => {
    const msg = messages[idx];
    if (!msg) return;
    try {
      await sendMidiTest(msg.port_name, msg.message_type, msg.channel, msg.data1, msg.data2);
      setTestResults((r) => ({ ...r, [idx]: "✓" }));
    } catch (e) {
      setTestResults((r) => ({ ...r, [idx]: "✗ " + String(e) }));
    }
    setTimeout(() => setTestResults((r) => { const next = { ...r }; delete next[idx]; return next; }), 2000);
  };

  return (
    <div>
      {messages.length === 0 && (
        <div style={{ color: "var(--wc-text-faint)", fontSize: 12, marginBottom: 12 }}>
          No messages. Click + Add to configure.
        </div>
      )}

      {messages.map((msg, idx) => (
        <div
          key={idx}
          style={{
            border: "1px solid var(--wc-border)",
            borderRadius: 6,
            padding: 10,
            marginBottom: 8,
            background: "var(--wc-bg-app)",
          }}
        >
          {/* Row 1: Port + Type */}
          <div style={{ display: "flex", gap: 6, marginBottom: 6 }}>
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontSize: 10, color: "var(--wc-text-muted)", marginBottom: 2 }}>Port</div>
              {ports.length > 0 ? (
                <Select
                  style={{ ...selectStyle, width: "100%" }}
                  value={msg.port_name}
                  onChange={(e) => updateMessage(idx, { port_name: e.target.value })}
                >
                  {ports.map((p) => (
                    <option key={p} value={p}>{p}</option>
                  ))}
                  {!ports.includes(msg.port_name) && msg.port_name && (
                    <option value={msg.port_name}>{msg.port_name} (not found)</option>
                  )}
                </Select>
              ) : (
                <input
                  style={{ ...inputStyle, width: "100%" }}
                  placeholder="Port name"
                  value={msg.port_name}
                  onChange={(e) => updateMessage(idx, { port_name: e.target.value })}
                />
              )}
            </div>
            <div>
              <div style={{ fontSize: 10, color: "var(--wc-text-muted)", marginBottom: 2 }}>Type</div>
              <Select
                style={selectStyle}
                value={msg.message_type}
                onChange={(e) => updateMessage(idx, { message_type: e.target.value as MidiMessageType })}
              >
                {(Object.keys(MESSAGE_TYPE_LABELS) as MidiMessageType[]).map((t) => (
                  <option key={t} value={t}>{MESSAGE_TYPE_LABELS[t]}</option>
                ))}
              </Select>
            </div>
          </div>

          {/* Row 2: Channel + Data1 + Data2 */}
          <div style={{ display: "flex", gap: 6, alignItems: "flex-end", marginBottom: 6 }}>
            <div>
              <div style={{ fontSize: 10, color: "var(--wc-text-muted)", marginBottom: 2 }}>Ch</div>
              <input
                style={{ ...inputStyle, width: 44 }}
                type="number"
                min={1}
                max={16}
                value={msg.channel}
                onChange={(e) => updateMessage(idx, { channel: Math.max(1, Math.min(16, parseInt(e.target.value, 10) || 1)) })}
              />
            </div>
            <div>
              <div style={{ fontSize: 10, color: "var(--wc-text-muted)", marginBottom: 2 }}>{data1Label(msg.message_type)}</div>
              <input
                style={{ ...inputStyle, width: 52 }}
                type="number"
                min={0}
                max={127}
                value={msg.data1}
                onChange={(e) => updateMessage(idx, { data1: Math.max(0, Math.min(127, parseInt(e.target.value, 10) || 0)) })}
              />
            </div>
            {hasData2(msg.message_type) && (
              <div>
                <div style={{ fontSize: 10, color: "var(--wc-text-muted)", marginBottom: 2 }}>
                  {msg.message_type === "control_change" ? "Value" : "Vel"}
                </div>
                <input
                  style={{ ...inputStyle, width: 52 }}
                  type="number"
                  min={0}
                  max={127}
                  value={msg.data2}
                  onChange={(e) => updateMessage(idx, { data2: Math.max(0, Math.min(127, parseInt(e.target.value, 10) || 0)) })}
                />
              </div>
            )}
            <div style={{ marginLeft: "auto", display: "flex", gap: 4, alignItems: "center" }}>
              {testResults[idx] && (
                <span style={{ fontSize: 11, color: testResults[idx].startsWith("✓") ? "#4ade80" : "#f87171" }}>
                  {testResults[idx]}
                </span>
              )}
              <button style={btnStyle} onClick={() => testSend(idx)}>▶ Test</button>
              <button
                style={{ ...btnStyle, color: "#ef4444" }}
                onClick={() => removeMessage(idx)}
              >
                ✕
              </button>
            </div>
          </div>
        </div>
      ))}

      <button
        style={{ ...btnStyle, color: "#86efac", marginTop: 4 }}
        onClick={addMessage}
      >
        + Add Message
      </button>

      {ports.length === 0 && (
        <div style={{ marginTop: 10, fontSize: 11, color: "var(--wc-text-muted)" }}>
          No MIDI output ports detected. Connect a MIDI device or install a virtual MIDI driver.
        </div>
      )}
    </div>
  );
}
