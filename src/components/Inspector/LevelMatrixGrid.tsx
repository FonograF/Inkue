// Crosspoint grid: how much of each input channel reaches each channel of the
// cue's Output Patch, in dB.
//
// While a cell is being edited the value goes straight to the engine
// (`setLiveCrosspoint`) so a playing cue changes under your hand; the whole
// matrix is persisted once on blur, which is also the only thing that lands in
// the undo stack.

import { useEffect, useState } from "react";

import { setLiveCrosspoint } from "../../lib/commands";
import type { CueId } from "../../lib/types";
import { DragNumber } from "../common/DragNumber";

/** dB floor — the engine treats anything at or below this as silence. */
const SILENT_DB = -60;

const INPUT_LABELS = ["L", "R"];

export function LevelMatrixGrid({
  cueId,
  matrix,
  patchName,
  deviceChannels,
  onSave,
}: {
  cueId: CueId;
  matrix: number[][] | null;
  /** Name of the patch the cue plays through, shown above the grid. */
  patchName: string;
  /** Device channel indices of that patch — one grid column each. */
  deviceChannels: number[];
  onSave: (matrix: number[][] | null) => void;
}) {
  // Always show at least a stereo pair, so a workspace with no patch defined
  // yet still gets a usable grid.
  const columns = Math.max(deviceChannels.length, 2);
  const [draft, setDraft] = useState<number[][]>(() => normalise(matrix, columns));

  useEffect(() => {
    setDraft(normalise(matrix, columns));
  }, [cueId, matrix, columns]);

  const enabled = matrix !== null;

  const editCell = (input: number, output: number, db: number) => {
    const next = draft.map((row) => [...row]);
    next[input][output] = db;
    setDraft(next);
    // Live: one cell, straight to the engine.
    void setLiveCrosspoint(cueId, input, output, db).catch(console.error);
  };

  return (
    <div style={{ marginTop: 4 }}>
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginBottom: 6 }}>
        <span style={{ fontSize: 12, color: "var(--wc-text-secondary)" }}>
          Level Matrix
        </span>
        <button
          onClick={() => onSave(enabled ? null : stereoDefault(columns))}
          style={{
            background: "var(--wc-bg-app)", border: "1px solid var(--wc-border-strong)",
            borderRadius: 4, color: "var(--wc-text-secondary)", fontSize: 11,
            padding: "3px 8px", cursor: "pointer",
          }}
        >
          {enabled ? "Use pan instead" : "Use a matrix"}
        </button>
      </div>

      {!enabled ? (
        <div style={{ fontSize: 11, color: "var(--wc-text-faint)" }}>
          This cue routes with the Pan control above. A matrix replaces pan and
          sends each input channel to any patch channel at its own level.
        </div>
      ) : (
        <div style={{ overflowX: "auto" }}>
          <table style={{ borderCollapse: "collapse", fontSize: 11 }}>
            <thead>
              <tr>
                <th style={{ ...cellStyle, color: "var(--wc-text-faint)", fontWeight: 400 }} />
                {Array.from({ length: columns }, (_, c) => (
                  <th
                    key={c}
                    style={{ ...cellStyle, color: "var(--wc-text-secondary)", fontWeight: 500 }}
                    title={
                      deviceChannels[c] !== undefined
                        ? `Patch output ${c + 1} → device channel ${deviceChannels[c] + 1}`
                        : `Patch output ${c + 1}`
                    }
                  >
                    {c + 1}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {draft.map((row, input) => (
                <tr key={input}>
                  <td style={{ ...cellStyle, color: "var(--wc-text-secondary)" }}>
                    {INPUT_LABELS[input] ?? input + 1}
                  </td>
                  {row.map((db, output) => (
                    <td key={output} style={cellStyle}>
                      <DragNumber
                        step="1"
                        min={SILENT_DB}
                        max="12"
                        value={Number.isFinite(db) ? db : SILENT_DB}
                        onChange={(e) => editCell(input, output, parseFloat(e.target.value))}
                        onBlur={() => onSave(draft)}
                        title={db <= SILENT_DB ? "Silent" : `${db} dB`}
                        style={{
                          width: 46,
                          background: db <= SILENT_DB ? "var(--wc-bg-app)" : "var(--wc-bg-input)",
                          border: "1px solid var(--wc-border)",
                          borderRadius: 3,
                          color: db <= SILENT_DB ? "var(--wc-text-faint)" : "var(--wc-text)",
                          fontSize: 11,
                          padding: "2px 3px",
                          textAlign: "center",
                        }}
                      />
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
          <div style={{ fontSize: 11, color: "var(--wc-text-faint)", marginTop: 6 }}>
            Columns are the outputs of <strong>{patchName}</strong> — hover one
            to see the device channel it lands on. {SILENT_DB} dB = silent.
            Values apply as you type; the cue’s Volume still scales the whole
            matrix.
          </div>
        </div>
      )}
    </div>
  );
}

/** Pad or trim a stored matrix to the current column count. */
function normalise(matrix: number[][] | null, columns: number): number[][] {
  const rows = matrix ?? stereoDefault(columns);
  return Array.from({ length: 2 }, (_, i) =>
    Array.from({ length: columns }, (_, c) => rows[i]?.[c] ?? SILENT_DB)
  );
}

/** L → channel 1, R → channel 2, everything else silent. */
function stereoDefault(columns: number): number[][] {
  return Array.from({ length: 2 }, (_, i) =>
    Array.from({ length: columns }, (_, c) => (c === i ? 0 : SILENT_DB))
  );
}

const cellStyle: React.CSSProperties = {
  padding: "2px 3px",
  textAlign: "center",
};
