// Start number + increment prompt for "Renumber Selected Cues…".
//
// Cue numbers are strings, so the increment is free to be fractional: 1 / 0.5
// gives 1, 1.5, 2 — the usual way of slipping cues into an existing sequence
// without disturbing what follows.

import { useState } from "react";
import { DragNumber } from "../common/DragNumber";
import { DialogRow, DialogShell, dialogInputStyle } from "../common/DialogShell";

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

  return (
    <DialogShell
      title="Renumber Selected Cues"
      subtitle={`${cueCount} cue${cueCount === 1 ? "" : "s"} selected — other cues keep their numbers.`}
      confirmLabel="Renumber"
      canConfirm={valid}
      onCancel={onCancel}
      onConfirm={() => onConfirm(startValue, incrementValue)}
    >
      <DialogRow label="Start at">
        <DragNumber
          autoFocus
          step="any"
          value={start}
          onChange={(e) => setStart(e.target.value)}
          style={dialogInputStyle}
        />
      </DialogRow>
      <DialogRow label="Increment">
        <DragNumber
          step="any"
          value={increment}
          onChange={(e) => setIncrement(e.target.value)}
          style={dialogInputStyle}
        />
      </DialogRow>

      <div style={{ fontSize: 12, color: "var(--wc-text-secondary)", margin: "12px 0 18px" }}>
        Result: <span style={{ color: "var(--wc-text)" }}>{preview}</span>
      </div>
    </DialogShell>
  );
}

/** Mirrors the backend's number formatting so the preview cannot lie. */
function formatPreview(value: number): string {
  if (Number.isInteger(value)) return String(value);
  return String(parseFloat(value.toFixed(6)));
}
