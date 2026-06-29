// Inspector tab shown for every cue: sets an optional timecode trigger.

import { useEffect, useState } from "react";
import type { CueSummary, TcRate, TcTrigger } from "../../lib/types";
import { getCueTcTrigger, setCueTcTrigger } from "../../lib/commands";
import { Select } from "../common/Select";

interface Props {
  cue: CueSummary;
  onSave?: () => void;
}

const inputStyle: React.CSSProperties = {
  background: "var(--wc-bg-app)",
  border: "1px solid var(--wc-border-strong)",
  borderRadius: 4,
  color: "var(--wc-text)",
  fontSize: 12,
  padding: "3px 6px",
};

const TC_RATES: TcRate[] = ["24", "25", "29.97", "29.97df", "30"];
const TC_RATE_LABELS: Record<TcRate, string> = {
  "24": "24 fps",
  "25": "25 fps (PAL)",
  "29.97": "29.97 fps",
  "29.97df": "29.97df (NTSC DF)",
  "30": "30 fps",
};

export function TriggersTab({ cue, onSave }: Props) {
  const [trigger, setTrigger] = useState<TcTrigger | null>(null);
  const [enabled, setEnabled] = useState(false);
  const [loading, setLoading] = useState(true);
  const [posInput, setPosInput] = useState("");
  const [rate, setRate] = useState<TcRate>("29.97df");
  const [realTime, setRealTime] = useState(false);

  useEffect(() => {
    setLoading(true);
    getCueTcTrigger(cue.id)
      .then((t) => {
        setTrigger(t);
        setEnabled(t != null);
        if (t) {
          setPosInput(t.position);
          setRate(t.rate);
          setRealTime(t.real_time);
        } else {
          setPosInput("");
          setRate("29.97df");
          setRealTime(false);
        }
      })
      .catch(console.error)
      .finally(() => setLoading(false));
  }, [cue.id]);

  const handleEnable = async (enable: boolean) => {
    setEnabled(enable);
    if (!enable) {
      await setCueTcTrigger(cue.id, null, null, false).catch(console.error);
      setTrigger(null);
      setPosInput("");
      onSave?.();
    }
  };

  const handleApply = async () => {
    if (!posInput.trim()) return;
    try {
      await setCueTcTrigger(cue.id, posInput.trim(), rate, realTime);
      const updated = await getCueTcTrigger(cue.id);
      setTrigger(updated);
      onSave?.();
    } catch (e) { console.error(e); }
  };

  if (loading) return <div style={{ color: "var(--wc-text-faint)", fontSize: 12 }}>Loading…</div>;

  return (
    <div>
      <div style={{ marginBottom: 14 }}>
        <label style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 13 }}>
          <input
            type="checkbox"
            checked={enabled}
            onChange={(e) => handleEnable(e.target.checked)}
            style={{ accentColor: "var(--wc-accent)", width: 14, height: 14 }}
          />
          <span style={{ color: enabled ? "var(--wc-text)" : "var(--wc-text-muted)" }}>Timecode trigger</span>
        </label>
      </div>

      {enabled && (
        <div style={{ paddingLeft: 4 }}>
          <div style={{ marginBottom: 10 }}>
            <label style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 12, color: "var(--wc-text-secondary)", marginBottom: 6 }}>
              <input
                type="checkbox"
                checked={realTime}
                onChange={(e) => setRealTime(e.target.checked)}
                style={{ accentColor: "var(--wc-accent)" }}
              />
              Real Time (milliseconds)
            </label>
          </div>

          <div style={{ display: "flex", gap: 8, marginBottom: 10, alignItems: "flex-end" }}>
            <div style={{ flex: 1 }}>
              <div style={{ fontSize: 10, color: "var(--wc-text-muted)", marginBottom: 2 }}>
                {realTime ? "Offset (ms)" : "Timecode (HH:MM:SS:FF)"}
              </div>
              <input
                style={{ ...inputStyle, width: "100%", fontFamily: "monospace" }}
                value={posInput}
                placeholder={realTime ? "e.g. 90000" : "e.g. 00:01:30:00"}
                onChange={(e) => setPosInput(e.target.value)}
                onKeyDown={(e) => { if (e.key === "Enter") void handleApply(); }}
              />
            </div>
            {!realTime && (
              <div>
                <div style={{ fontSize: 10, color: "var(--wc-text-muted)", marginBottom: 2 }}>Rate</div>
                <Select
                  style={{ ...inputStyle, cursor: "pointer" }}
                  value={rate}
                  onChange={(e) => setRate(e.target.value as TcRate)}
                >
                  {TC_RATES.map((r) => (
                    <option key={r} value={r}>{TC_RATE_LABELS[r]}</option>
                  ))}
                </Select>
              </div>
            )}
          </div>

          {trigger && (
            <div style={{ fontSize: 11, color: "#22c55e", fontFamily: "monospace", marginBottom: 8 }}>
              ✓ {trigger.real_time ? `${trigger.position} ms` : trigger.position}
            </div>
          )}

          <button
            style={{
              padding: "4px 12px",
              background: "var(--wc-accent-dim)",
              border: "1px solid var(--wc-accent)",
              borderRadius: 4,
              color: "var(--wc-text)",
              fontSize: 12,
              cursor: "pointer",
            }}
            onClick={() => void handleApply()}
          >
            Apply Trigger
          </button>
        </div>
      )}

      {!enabled && (
        <p style={{ fontSize: 12, color: "var(--wc-text-faint)", marginTop: 4 }}>
          Enable to fire this cue at a specific timecode position when TC sync is active for this Cue List.
        </p>
      )}
    </div>
  );
}
