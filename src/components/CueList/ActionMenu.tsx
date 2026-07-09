// "Action" toolbar dropdown — list-wide operations that aren't cue creation.
// Currently: Renumber All Cues (resequence numbers on demand, since reordering
// no longer renumbers automatically by default).

import { useEffect, useRef, useState } from "react";
import { renumberCues } from "../../lib/commands";

interface Props {
  buttonStyle: React.CSSProperties;
  onDone: () => void;
}

export function ActionMenu({ buttonStyle, onDone }: Props) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDocMouseDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDocMouseDown);
    return () => document.removeEventListener("mousedown", onDocMouseDown);
  }, [open]);

  const runRenumber = async () => {
    setOpen(false);
    await renumberCues().catch(console.error);
    onDone();
  };

  return (
    <div ref={ref} style={{ position: "relative" }}>
      <button style={buttonStyle} onClick={() => setOpen((v) => !v)} title="List-wide actions">
        Action ▾
      </button>
      {open && (
        <div
          style={{
            position: "absolute", top: "100%", left: 0, marginTop: 4, zIndex: 1000,
            background: "var(--wc-bg-surface)", border: "1px solid var(--wc-border-strong)",
            borderRadius: 6, boxShadow: "0 8px 24px rgba(0,0,0,0.5)", minWidth: 200, padding: 4,
          }}
        >
          <button
            onClick={() => void runRenumber()}
            style={{
              display: "block", width: "100%", textAlign: "left", padding: "6px 10px",
              background: "transparent", border: "none", color: "var(--wc-text)",
              fontSize: 12, cursor: "pointer", borderRadius: 4,
            }}
            onMouseEnter={(e) => (e.currentTarget.style.background = "var(--wc-accent)")}
            onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
          >
            Renumber All Cues
          </button>
        </div>
      )}
    </div>
  );
}
