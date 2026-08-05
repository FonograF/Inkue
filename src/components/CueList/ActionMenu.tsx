// "Action" menu-bar dropdown — list-wide operations that aren't cue creation.
// Sits in the title bar next to File and View, and matches their look and
// dismissal behaviour (backdrop click-catcher, same z-index layering).
// Currently: Renumber All Cues (resequence numbers on demand, since reordering
// no longer renumbers automatically by default).

import { useState } from "react";
import { renumberCues } from "../../lib/commands";

interface Props {
  onDone: () => void;
}

export function ActionMenu({ onDone }: Props) {
  const [open, setOpen] = useState(false);
  const [hovered, setHovered] = useState<string | null>(null);

  const close = () => setOpen(false);

  const runRenumber = async () => {
    close();
    await renumberCues().catch(console.error);
    onDone();
  };

  return (
    <div style={{ position: "relative", flexShrink: 0 }}>
      {open && (
        <div style={{ position: "fixed", inset: 0, zIndex: 9990 }} onClick={close} />
      )}
      <button
        onClick={(e) => { e.stopPropagation(); setOpen((v) => !v); }}
        title="List-wide actions"
        style={{
          background: open ? "var(--wc-bg-surface)" : "transparent",
          border: "none", color: "var(--wc-text)", cursor: "pointer",
          fontSize: 12, padding: "3px 8px", borderRadius: 4, userSelect: "none",
        }}
      >
        Action
      </button>
      {open && (
        <div
          style={{
            position: "absolute", left: 0, top: "100%", marginTop: 2,
            background: "var(--wc-bg-surface)", border: "1px solid var(--wc-border-strong)", borderRadius: 6,
            padding: "4px 0", minWidth: 200,
            boxShadow: "0 8px 24px rgba(0,0,0,0.7)", zIndex: 9999,
          }}
        >
          <button
            onMouseEnter={() => setHovered("renumber")}
            onMouseLeave={() => setHovered(null)}
            onClick={(e) => { e.stopPropagation(); void runRenumber(); }}
            style={{
              display: "flex", alignItems: "center",
              width: "100%", padding: "6px 14px",
              background: hovered === "renumber" ? "var(--wc-bg-hover)" : "transparent",
              border: "none", color: "var(--wc-text)", fontSize: 13,
              cursor: "pointer", textAlign: "left",
            }}
          >
            Renumber All Cues
          </button>
        </div>
      )}
    </div>
  );
}
