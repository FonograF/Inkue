// MIDI trigger editor, shown on every cue's Triggers tab.
//
// Learn is the primary path: press the key or pad you want and the trigger is
// filled in from what actually arrived, which is far more reliable than asking
// an operator to know their controller's note numbers. The fields stay
// editable afterwards for the cases Learn cannot guess — chiefly "any channel"
// and requiring a specific value.

import { useEffect, useRef, useState } from "react";
import type { MidiTrigger, MidiTriggerType } from "../../lib/types";
import {
  clearMidiLearn,
  getCueMidiTrigger,
  getMidiTriggerConfig,
  learnMidiTrigger,
  setCueMidiTrigger,
} from "../../lib/commands";
import { Select } from "../common/Select";
import { DragNumber } from "../common/DragNumber";

const inputStyle: React.CSSProperties = {
  background: "var(--wc-bg-app)",
  border: "1px solid var(--wc-border-strong)",
  borderRadius: 4,
  color: "var(--wc-text)",
  fontSize: 12,
  padding: "3px 6px",
};

const TYPE_LABELS: Record<MidiTriggerType, string> = {
  note_on: "Note On",
  note_off: "Note Off",
  control_change: "Control Change",
  program_change: "Program Change",
};

const DEFAULT_TRIGGER: MidiTrigger = {
  message_type: "note_on",
  channel: 1,
  data1: 60,
  data2: null,
};

function data1Label(type: MidiTriggerType): string {
  if (type === "control_change") return "CC#";
  if (type === "program_change") return "Program";
  return "Note";
}

/** Program Change carries no second data byte, so there is no value to require. */
function hasValue(type: MidiTriggerType): boolean {
  return type !== "program_change";
}

export function MidiTriggerSection({ cueId, onSave }: { cueId: string; onSave?: () => void }) {
  const [trigger, setTrigger] = useState<MidiTrigger | null>(null);
  const [listening, setListening] = useState(false);
  const [inputReady, setInputReady] = useState(true);
  const pollRef = useRef<number | null>(null);

  useEffect(() => {
    getCueMidiTrigger(cueId).then(setTrigger).catch(console.error);
    getMidiTriggerConfig()
      .then((c) => setInputReady(c.enabled))
      .catch(console.error);
    return stopLearning;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cueId]);

  function stopLearning() {
    if (pollRef.current !== null) {
      window.clearInterval(pollRef.current);
      pollRef.current = null;
    }
    setListening(false);
  }

  const commit = async (next: MidiTrigger | null) => {
    setTrigger(next);
    await setCueMidiTrigger(cueId, next).catch(console.error);
    onSave?.();
  };

  const startLearning = async () => {
    // Drop whatever arrived earlier, so Learn waits for a genuinely new press.
    await clearMidiLearn().catch(console.error);
    setListening(true);
    pollRef.current = window.setInterval(async () => {
      const learned = await learnMidiTrigger().catch(() => null);
      if (learned) {
        stopLearning();
        void commit(learned);
      }
    }, 120);
  };

  const update = (patch: Partial<MidiTrigger>) => {
    void commit({ ...(trigger ?? DEFAULT_TRIGGER), ...patch });
  };

  return (
    <div style={{ marginTop: 18, borderTop: "1px solid var(--wc-border)", paddingTop: 14 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 10 }}>
        <label style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 13 }}>
          <input
            type="checkbox"
            checked={trigger != null}
            onChange={(e) => void commit(e.target.checked ? DEFAULT_TRIGGER : null)}
            style={{ accentColor: "var(--wc-accent)", width: 14, height: 14 }}
          />
          <span style={{ color: trigger ? "var(--wc-text)" : "var(--wc-text-muted)" }}>
            MIDI trigger
          </span>
        </label>
        {trigger && (
          <button
            style={{
              marginLeft: "auto",
              padding: "3px 10px",
              background: listening ? "var(--wc-accent)" : "var(--wc-bg-surface)",
              border: "1px solid var(--wc-border-strong)",
              borderRadius: 4,
              color: listening ? "var(--wc-accent-fg)" : "var(--wc-text)",
              fontSize: 11,
              cursor: "pointer",
            }}
            onClick={() => (listening ? stopLearning() : void startLearning())}
          >
            {listening ? "Listening… (cancel)" : "Learn"}
          </button>
        )}
      </div>

      {trigger && (
        <div style={{ paddingLeft: 4 }}>
          {!inputReady && (
            <div style={{ fontSize: 11, color: "#fbbf24", marginBottom: 8 }}>
              MIDI triggers are off for this machine — turn them on in
              Preferences → Network to arm this cue.
            </div>
          )}

          <div style={{ display: "flex", gap: 6, alignItems: "flex-end", flexWrap: "wrap" }}>
            <div style={{ minWidth: 130 }}>
              <div style={{ fontSize: 10, color: "var(--wc-text-muted)", marginBottom: 2 }}>Type</div>
              <Select
                style={{ ...inputStyle, cursor: "pointer", width: "100%" }}
                value={trigger.message_type}
                onChange={(e) => update({ message_type: e.target.value as MidiTriggerType })}
              >
                {(Object.keys(TYPE_LABELS) as MidiTriggerType[]).map((t) => (
                  <option key={t} value={t}>{TYPE_LABELS[t]}</option>
                ))}
              </Select>
            </div>
            <div>
              <div style={{ fontSize: 10, color: "var(--wc-text-muted)", marginBottom: 2 }}>Ch</div>
              <DragNumber
                style={{ ...inputStyle, width: 50 }}
                min={0}
                max={16}
                value={trigger.channel}
                title="0 = any channel"
                onChange={(e) =>
                  update({ channel: Math.max(0, Math.min(16, parseInt(e.target.value, 10) || 0)) })
                }
              />
            </div>
            <div>
              <div style={{ fontSize: 10, color: "var(--wc-text-muted)", marginBottom: 2 }}>
                {data1Label(trigger.message_type)}
              </div>
              <DragNumber
                style={{ ...inputStyle, width: 56 }}
                min={0}
                max={127}
                value={trigger.data1}
                onChange={(e) =>
                  update({ data1: Math.max(0, Math.min(127, parseInt(e.target.value, 10) || 0)) })
                }
              />
            </div>
            {hasValue(trigger.message_type) && (
              <div>
                <div style={{ fontSize: 10, color: "var(--wc-text-muted)", marginBottom: 2 }}>
                  Value
                </div>
                <div style={{ display: "flex", gap: 4, alignItems: "center" }}>
                  <Select
                    style={{ ...inputStyle, cursor: "pointer" }}
                    value={trigger.data2 === null ? "any" : "exact"}
                    onChange={(e) =>
                      update({ data2: e.target.value === "any" ? null : 127 })
                    }
                  >
                    <option value="any">Any</option>
                    <option value="exact">Exactly</option>
                  </Select>
                  {trigger.data2 !== null && (
                    <DragNumber
                      style={{ ...inputStyle, width: 56 }}
                      min={0}
                      max={127}
                      value={trigger.data2}
                      onChange={(e) =>
                        update({
                          data2: Math.max(0, Math.min(127, parseInt(e.target.value, 10) || 0)),
                        })
                      }
                    />
                  )}
                </div>
              </div>
            )}
          </div>

          <div style={{ fontSize: 11, color: "var(--wc-text-faint)", marginTop: 8 }}>
            Channel 0 matches any channel. A Note On with velocity 0 counts as a
            release, so it fires a Note Off trigger, not a Note On — set Value to
            <em> Exactly 127</em> for a footswitch that also sends 0 when let go.
          </div>
        </div>
      )}

      {!trigger && (
        <p style={{ fontSize: 12, color: "var(--wc-text-faint)", marginTop: 4 }}>
          Fire this cue when a MIDI message arrives — a key, pad, footswitch or
          program change from any controller on the trigger input.
        </p>
      )}
    </div>
  );
}
