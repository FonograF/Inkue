// Report shown after importing a QLab workspace.
//
// The point is not to celebrate a success — it is to tell the operator exactly
// what needs their hands before the show can run: the cues QLab could describe
// and Inkue cannot, and the media that did not resolve. Everything else is
// summarised in one line and stays out of the way.

import type { ImportReport } from "../../lib/types";

const overlay: React.CSSProperties = {
  position: "fixed", inset: 0, zIndex: 99999,
  background: "rgba(0,0,0,0.6)", display: "flex",
  alignItems: "center", justifyContent: "center",
};

const panel: React.CSSProperties = {
  background: "var(--wc-bg-surface)",
  border: "1px solid var(--wc-border-strong)",
  borderRadius: 10,
  padding: "24px 28px",
  maxWidth: 620,
  width: "90%",
  maxHeight: "80vh",
  display: "flex",
  flexDirection: "column",
  boxShadow: "0 16px 48px rgba(0,0,0,0.8)",
};

export function QlabImportDialog({
  report,
  onClose,
}: {
  report: ImportReport;
  onClose: () => void;
}) {
  const attention = report.cues.filter((c) => c.note);
  const missing = report.media_missing;

  return (
    <div style={overlay} onClick={onClose}>
      <div style={panel} onClick={(e) => e.stopPropagation()}>
        <h3 style={{ margin: "0 0 4px", fontSize: 15, color: "var(--wc-text)" }}>
          Imported “{report.workspace_name}”
          <span
            title="QLab import is in beta: cue types are covered, some properties are not yet mapped."
            style={{
              marginLeft: 8, padding: "1px 6px", borderRadius: 4, fontSize: 10,
              letterSpacing: 0.5, verticalAlign: "middle",
              background: "rgba(251,191,36,0.16)", border: "1px solid rgba(251,191,36,0.55)",
              color: "#fcd34d",
            }}
          >
            BETA
          </span>
        </h3>
        <div style={{ fontSize: 12, color: "var(--wc-text-muted)", marginBottom: 16 }}>
          {report.cue_count} cue{report.cue_count !== 1 ? "s" : ""} in{" "}
          {report.cue_list_count} list{report.cue_list_count !== 1 ? "s" : ""} ·{" "}
          {report.media_found} media file{report.media_found !== 1 ? "s" : ""} found
        </div>

        <div style={{ overflowY: "auto", flex: 1, minHeight: 0 }}>
          {attention.length > 0 && (
            <>
              <div style={{ fontSize: 12, color: "var(--wc-text)", marginBottom: 8 }}>
                <strong>{attention.length}</strong> cue
                {attention.length !== 1 ? "s need" : " needs"} your attention — nothing
                was lost, but these could not be carried over as they were:
              </div>
              <div style={{ marginBottom: 16 }}>
                {attention.map((cue, index) => (
                  <div
                    key={index}
                    style={{
                      display: "flex", gap: 8, fontSize: 12, padding: "4px 0",
                      borderBottom: "1px solid var(--wc-border)",
                    }}
                  >
                    <span style={{ color: "var(--wc-text-muted)", minWidth: 44, flexShrink: 0 }}>
                      {cue.cue_number ?? "—"}
                    </span>
                    <span style={{ color: "var(--wc-text)", minWidth: 110, flexShrink: 0 }}>
                      {cue.qlab_class}
                    </span>
                    <span style={{ color: "var(--wc-text-secondary)" }}>{cue.note}</span>
                  </div>
                ))}
              </div>
            </>
          )}

          {missing.length > 0 && (
            <>
              <div style={{ fontSize: 12, color: "#fbbf24", marginBottom: 6 }}>
                {missing.length} media file{missing.length !== 1 ? "s" : ""} could not be
                found next to the QLab workspace. Use Check Workspace… to relink them.
              </div>
              <div
                style={{
                  fontSize: 11, fontFamily: "monospace", color: "var(--wc-text-muted)",
                  marginBottom: 16, wordBreak: "break-all",
                }}
              >
                {missing.slice(0, 12).map((path) => (
                  <div key={path}>{path}</div>
                ))}
                {missing.length > 12 && <div>…and {missing.length - 12} more</div>}
              </div>
            </>
          )}

          {attention.length === 0 && missing.length === 0 && (
            <div style={{ fontSize: 13, color: "#4ade80", marginBottom: 16 }}>
              Everything came across cleanly.
            </div>
          )}
        </div>

        <div style={{ fontSize: 11, color: "var(--wc-text-faint)", margin: "12px 0 16px" }}>
          This workspace has not been saved yet: its media still points at the QLab
          project folder. <strong>Save As</strong> to keep it, then{" "}
          <strong>Collect and Save</strong> to make it self-contained.
        </div>

        <button
          onClick={onClose}
          style={{
            alignSelf: "flex-end",
            padding: "6px 18px",
            background: "var(--wc-accent)",
            border: "none",
            borderRadius: 5,
            color: "var(--wc-accent-fg)",
            fontSize: 13,
            cursor: "pointer",
          }}
        >
          Close
        </button>
      </div>
    </div>
  );
}
