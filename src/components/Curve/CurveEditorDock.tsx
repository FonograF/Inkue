// Curve editor dock: the full-size fade curve editor under the cue list,
// opened by the ⤢ button in the inspector's Curve section.
//
// Same idea as the clip editor dock — the inspector column is too narrow to
// place control points precisely, so the real editing happens down here where
// there is room for both curves side by side.

import { useEffect, useState } from "react";
import type { FadeCueData, FadeShapes } from "../../lib/types";
import { getCue, updateCue } from "../../lib/commands";
import { CurveEditor } from "./CurveEditor";

const DEFAULT_SHAPES: FadeShapes = {
  up: { kind: "s_curve", intensity: 0, points: [], bends: [] },
  down: { kind: "s_curve", intensity: 0, points: [], bends: [] },
  mirrored: true,
};

export function CurveEditorDock({
  cueId,
  onClose,
  onSaved,
  reloadToken,
}: {
  cueId: string;
  onClose: () => void;
  /** Called after every save so the cue list + inspector refresh. */
  onSaved: () => void;
  /** Bump to re-fetch the cue (the inspector edited it). */
  reloadToken?: number;
}) {
  const [cue, setCue] = useState<FadeCueData | null>(null);

  useEffect(() => {
    getCue(cueId)
      .then((data) => setCue(data as unknown as FadeCueData))
      .catch(console.error);
  }, [cueId, reloadToken]);

  const save = async (fade_shapes: FadeShapes) => {
    setCue((prev) => (prev ? { ...prev, fade_shapes } : prev));
    await updateCue(cueId, { fade_shapes }).catch(console.error);
    onSaved();
  };

  return (
    <div
      style={{
        borderTop: "1px solid var(--wc-border-strong)",
        background: "var(--wc-bg-deepest)",
        padding: "10px 14px 14px",
        flexShrink: 0,
      }}
    >
      <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 10 }}>
        <span style={{ fontSize: 12, fontWeight: 600, color: "var(--wc-text)" }}>
          Fade Curve
        </span>
        <span
          style={{
            fontSize: 12, color: "var(--wc-text-muted)", overflow: "hidden",
            textOverflow: "ellipsis", whiteSpace: "nowrap",
          }}
        >
          {cue?.number ? `${cue.number} · ` : ""}{cue?.name ?? ""}
        </span>
        <button
          onClick={onClose}
          style={{
            marginLeft: "auto", padding: "2px 10px",
            background: "var(--wc-bg-surface)", border: "1px solid var(--wc-border-strong)",
            borderRadius: 4, color: "var(--wc-text-secondary)", fontSize: 11, cursor: "pointer",
          }}
        >
          Close
        </button>
      </div>

      {cue ? (
        <CurveEditor shapes={cue.fade_shapes ?? DEFAULT_SHAPES} onChange={(s) => void save(s)} />
      ) : (
        <div style={{ fontSize: 12, color: "var(--wc-text-faint)" }}>Loading…</div>
      )}
    </div>
  );
}
