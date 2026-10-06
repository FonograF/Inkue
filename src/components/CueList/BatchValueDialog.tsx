// One-number prompt for a batch edit from the cue context menu ("Set
// Pre-Wait…", "Adjust Volume…"). Works in display units — seconds, dB, % —
// the caller converts to what the backend stores.

import { useState } from "react";
import { DragNumber } from "../common/DragNumber";
import { DialogRow, DialogShell, dialogInputStyle } from "../common/DialogShell";

export interface BatchValuePrompt {
  title: string;
  label: string;
  unit: string;
  /** Starting value, when every affected cue shares one. */
  initial?: number;
  min?: number;
  max?: number;
  step?: number;
  /** Whole numbers only (play counts). */
  integer?: boolean;
  onConfirm: (value: number) => void;
}

interface Props {
  prompt: BatchValuePrompt;
  /** How many cues the edit will touch, out of the selection. */
  affected: number;
  selected: number;
  onCancel: () => void;
}

export function BatchValueDialog({ prompt, affected, selected, onCancel }: Props) {
  const [text, setText] = useState(prompt.initial !== undefined ? formatValue(prompt.initial) : "");
  const value = parseFloat(text);
  const valid =
    Number.isFinite(value) &&
    (prompt.min === undefined || value >= prompt.min) &&
    (prompt.max === undefined || value <= prompt.max) &&
    (!prompt.integer || Number.isInteger(value));

  const subtitle = affected === selected
    ? `Applies to ${cueCount(affected)}.`
    : `Applies to ${affected} of ${cueCount(selected)} — the others have no such setting.`;

  return (
    <DialogShell
      title={prompt.title}
      subtitle={subtitle}
      confirmLabel="Apply"
      canConfirm={valid}
      onCancel={onCancel}
      onConfirm={() => prompt.onConfirm(value)}
    >
      <DialogRow label={prompt.label}>
        <DragNumber
          autoFocus
          step={prompt.step ?? "any"}
          min={prompt.min}
          max={prompt.max}
          value={text}
          onChange={(e) => setText(e.target.value)}
          style={dialogInputStyle}
        />
        <span style={{ fontSize: 12, color: "var(--wc-text-muted)", width: 28 }}>{prompt.unit}</span>
      </DialogRow>
      <div style={{ height: 8 }} />
    </DialogShell>
  );
}

function cueCount(n: number): string {
  return `${n} cue${n === 1 ? "" : "s"}`;
}

function formatValue(value: number): string {
  return String(parseFloat(value.toFixed(3)));
}
