// Start number + increment prompt for "Renumber Selected Cues…".
//
// Cue numbers are strings, so the increment is free to be fractional: 1 / 0.5
// gives 1, 1.5, 2 — the usual way of slipping cues into an existing sequence
// without disturbing what follows.

import { useState } from "react";

interface Props {
  cueCount: number;
  onCancel: () => void;
  onConfirm: (start: number, increment: number) => void;
}

export function RenumberDialog({ cueCount, onCancel, onConfirm }: Props) {
  const [start, setStart] = useState("1");
  const [increment, setIncrement] = useState("1");

  const startValue = parseFloat(start);
  const incrementValue = parseFloat(increment);
  const valid = Number.isFinite(startValue) && Number.isFinite(incrementValue) && incrementValue !== 0;

  const preview = valid
    ? Array.from({ length: Math.min(cueCount, 3) }, (_, i) => formatPreview(startValue + i * incrementValue))
        .join(", ") + (cueCount > 3 ? ", …" : "")
    : "—";

  const submit = () => { if (valid) onConfirm(startValue, incrementValue); };

  return (
    <div
      style={{
        position: "fixed", inset: 0, zIndex: 10000,
        background: "rgba(0,0,0,0.5)", display: "flex", alignItems: "center", justifyContent: "center",
      }}
      onClick={onCancel}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key === "Enter") submit();
          if (e.key === "Escape") onCancel();
        }}
        style={{
          background: "var(--wc-bg-surface)", border: "1px solid var(--wc-border-strong)",
          borderRadius: 8, padding: 20, minWidth: 320,
          boxShadow: "0 16px 48px rgba(0,0,0,0.6)",
        }}
      >
        <div style={{ fontSize: 14, fontWeight: 600, color: "var(--wc-text-bright)", marginBottom: 4 }}>
          Renumber Selected Cues
        </div>
        <div style={{ fontSize: 12, color: "var(--wc-text-muted)", marginBottom: 16 }}>
          {cueCount} cue{cueCount === 1 ? "" : "s"} selected — other cues keep their numbers.
        </div>

        <Row label="Start at">
          <input
            autoFocus
            type="number"
            step="any"
            value={start}
            onChange={(e) => setStart(e.target.value)}
            style={inputStyle}
          />
        </Row>
        <Row label="Increment">
          <input
            type="number"
            step="any"
            value={increment}
            onChange={(e) => setIncrement(e.target.value)}
            style={inputStyle}
          />
        </Row>

        <div style={{ fontSize: 12, color: "var(--wc-text-secondary)", margin: "12px 0 18px" }}>
          Result: <span style={{ color: "var(--wc-text)" }}>{preview}</span>
        </div>

        <div style={{ display: "flex", justifyContent: "flex-end", gap: 8 }}>
          <button onClick={onCancel} style={buttonStyle}>Cancel</button>
          <button
            onClick={submit}
            disabled={!valid}
            style={{
              ...buttonStyle,
              background: valid ? "var(--wc-accent)" : "var(--wc-bg-hover)",
              color: valid ? "var(--wc-accent-fg)" : "var(--wc-text-muted)",
              cursor: valid ? "pointer" : "default",
            }}
          >
            Renumber
          </button>
        </div>
      </div>
    </div>
  );
}

/** Mirrors the backend's number formatting so the preview cannot lie. */
function formatPreview(value: number): string {
  if (Number.isInteger(value)) return String(value);
  return String(parseFloat(value.toFixed(6)));
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 12, marginBottom: 10 }}>
      <span style={{ fontSize: 12, color: "var(--wc-text-secondary)", width: 80 }}>{label}</span>
      {children}
    </div>
  );
}

const inputStyle: React.CSSProperties = {
  flex: 1, background: "var(--wc-bg-input)", border: "1px solid var(--wc-border)",
  borderRadius: 4, color: "var(--wc-text)", fontSize: 13, padding: "5px 8px",
};

const buttonStyle: React.CSSProperties = {
  background: "var(--wc-bg-hover)", border: "1px solid var(--wc-border-strong)",
  borderRadius: 5, color: "var(--wc-text)", fontSize: 12, padding: "5px 14px", cursor: "pointer",
};
