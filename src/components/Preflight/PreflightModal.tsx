import { useEffect, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import { relinkMedia } from "../../lib/commands";
import type { CueValidation, Severity } from "../../lib/types";

const SEVERITY_COLOR: Record<Severity, string> = {
  error: "#ef4444",
  warning: "#fbbf24",
};

/** Modal listing every cue with unresolved dependencies, with inline relink. */
export function PreflightModal({ onClose }: { onClose: () => void }) {
  const validation = useWorkspaceStore((s) => s.validation);
  const refreshValidation = useWorkspaceStore((s) => s.refreshValidation);
  const refreshCues = useWorkspaceStore((s) => s.refreshCues);
  const [busy, setBusy] = useState(false);

  // Always re-run the check when the panel opens so it reflects the latest edits.
  useEffect(() => {
    void refreshValidation();
  }, [refreshValidation]);

  const errorCount = validation.reduce(
    (n, v) => n + v.issues.filter((i) => i.severity === "error").length,
    0,
  );
  const warnCount = validation.reduce(
    (n, v) => n + v.issues.filter((i) => i.severity === "warning").length,
    0,
  );

  const handleRelink = async (v: CueValidation) => {
    if (!v.missing_file) return;
    const picked = await openDialog({ multiple: false });
    if (typeof picked !== "string") return;
    setBusy(true);
    try {
      await relinkMedia(v.cue_id, picked);
      await refreshValidation();
      await refreshCues();
    } catch (e) {
      console.error("relink failed", e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      style={{
        position: "fixed", inset: 0, zIndex: 99999,
        background: "rgba(0,0,0,0.6)",
        display: "flex", alignItems: "center", justifyContent: "center",
      }}
      onClick={onClose}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        style={{
          background: "var(--wc-bg-surface)", border: "1px solid var(--wc-border-strong)",
          borderRadius: 12, padding: "24px 28px", width: 640, maxHeight: "80vh",
          display: "flex", flexDirection: "column",
          boxShadow: "0 16px 48px rgba(0,0,0,0.8)",
        }}
      >
        <div style={{ display: "flex", alignItems: "baseline", gap: 12, marginBottom: 4 }}>
          <span style={{ fontSize: 18, fontWeight: 700, color: "var(--wc-text-bright)" }}>
            Check Workspace
          </span>
        </div>

        <div style={{ fontSize: 12, color: "var(--wc-text-secondary)", marginBottom: 18 }}>
          {validation.length === 0 ? (
            <span style={{ color: "#86efac" }}>✓ No problems found — the show is ready.</span>
          ) : (
            <>
              <span style={{ color: SEVERITY_COLOR.error }}>{errorCount} error{errorCount > 1 ? "s" : ""}</span>
              {" · "}
              <span style={{ color: SEVERITY_COLOR.warning }}>{warnCount} warning{warnCount > 1 ? "s" : ""}</span>
              {" across "}{validation.length} cue{validation.length > 1 ? "s" : ""}
            </>
          )}
        </div>

        <div style={{ overflowY: "auto", display: "flex", flexDirection: "column", gap: 10 }}>
          {validation.map((v) => (
            <div
              key={v.cue_id}
              style={{
                border: "1px solid var(--wc-border)", borderRadius: 8,
                padding: "10px 12px", background: "rgba(255,255,255,0.02)",
              }}
            >
              <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 12 }}>
                <span style={{ fontSize: 13, color: "var(--wc-text-bright)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                  {v.cue_number ? `${v.cue_number} · ` : ""}{v.cue_name || "(untitled)"}
                </span>
                {v.missing_file && (
                  <button
                    disabled={busy}
                    onClick={() => void handleRelink(v)}
                    style={{
                      flexShrink: 0,
                      background: "var(--wc-bg-hover)", border: "1px solid var(--wc-border-strong)",
                      borderRadius: 6, color: "var(--wc-text)", cursor: busy ? "default" : "pointer",
                      fontSize: 12, padding: "4px 12px", opacity: busy ? 0.5 : 1,
                    }}
                  >
                    Locate…
                  </button>
                )}
              </div>
              <div style={{ marginTop: 6, display: "flex", flexDirection: "column", gap: 3 }}>
                {v.issues.map((issue, i) => (
                  <div key={i} style={{ fontSize: 12, color: "var(--wc-text-secondary)", display: "flex", gap: 8 }}>
                    <span style={{ color: SEVERITY_COLOR[issue.severity], flexShrink: 0 }}>
                      {issue.severity === "error" ? "✕" : "⚠"}
                    </span>
                    <span>{issue.message}</span>
                  </div>
                ))}
              </div>
            </div>
          ))}
        </div>

        <div style={{ display: "flex", justifyContent: "flex-end", gap: 10, marginTop: 20 }}>
          <button
            onClick={() => void refreshValidation()}
            style={{
              background: "transparent", border: "1px solid var(--wc-border-strong)",
              borderRadius: 6, color: "var(--wc-text-secondary)", cursor: "pointer",
              fontSize: 13, padding: "6px 16px",
            }}
          >
            Re-check
          </button>
          <button
            onClick={onClose}
            style={{
              background: "var(--wc-bg-hover)", border: "1px solid var(--wc-border-strong)",
              borderRadius: 6, color: "var(--wc-text)", cursor: "pointer",
              fontSize: 13, padding: "6px 18px",
            }}
          >
            Close
          </button>
        </div>
      </div>
    </div>
  );
}
