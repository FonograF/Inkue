// Update dialog — shown when a new version is available (startup check or
// manual check from the About dialog).

import { useUpdateStore } from "../../stores/updateStore";

/** Render `**bold**` spans of a markdown line as JSX (no dependency). */
function inlineBold(text: string): React.ReactNode {
  const parts = text.split(/\*\*([^*]+)\*\*/g);
  if (parts.length === 1) return text;
  return parts.map((part, i) =>
    i % 2 === 1 ? <strong key={i} style={{ color: "var(--wc-text)" }}>{part}</strong> : part,
  );
}

/**
 * Minimal markdown rendering for release notes (headers, bullets, bold,
 * separators).  The notes come from the GitHub release body via latest.json;
 * a full markdown library is overkill for this one read-only box.
 */
function ReleaseNotes({ notes }: { notes: string }) {
  const lines = notes.replace(/\r\n/g, "\n").split("\n");
  return (
    <>
      {lines.map((line, i) => {
        const h = line.match(/^(#{1,3})\s+(.*)$/);
        if (h) {
          const level = h[1].length;
          return (
            <div
              key={i}
              style={{
                fontWeight: 700,
                fontSize: level === 1 ? 13 : 12,
                color: "var(--wc-text-bright)",
                margin: i === 0 ? "0 0 4px" : "10px 0 4px",
              }}
            >
              {inlineBold(h[2])}
            </div>
          );
        }
        if (/^\s*---+\s*$/.test(line)) {
          return <hr key={i} style={{ border: "none", borderTop: "1px solid var(--wc-border)", margin: "8px 0" }} />;
        }
        const bullet = line.match(/^\s*[-*]\s+(.*)$/);
        if (bullet) {
          return (
            <div key={i} style={{ display: "flex", gap: 6, paddingLeft: 4 }}>
              <span style={{ flexShrink: 0 }}>•</span>
              <span>{inlineBold(bullet[1])}</span>
            </div>
          );
        }
        if (line.trim() === "") {
          return <div key={i} style={{ height: 6 }} />;
        }
        return <div key={i}>{inlineBold(line)}</div>;
      })}
    </>
  );
}

export function UpdateDialog() {
  const { status, version, notes, progress, error, dismissed, downloadAndInstall, dismiss } =
    useUpdateStore();

  const visible =
    !dismissed &&
    (status === "available" || status === "downloading" || status === "installing" ||
      (status === "error" && version !== null));
  if (!visible) return null;

  const busy = status === "downloading" || status === "installing";

  return (
    <div
      style={{
        position: "fixed", inset: 0, zIndex: 99998,
        background: "rgba(0,0,0,0.6)",
        display: "flex", alignItems: "center", justifyContent: "center",
      }}
      onClick={busy ? undefined : dismiss}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        style={{
          background: "var(--wc-bg-surface)", border: "1px solid var(--wc-border-strong)",
          borderRadius: 12, padding: "24px 28px", width: 440,
          boxShadow: "0 16px 48px rgba(0,0,0,0.8)",
        }}
      >
        <div style={{ fontSize: 16, fontWeight: 700, color: "var(--wc-text-bright)", marginBottom: 4 }}>
          Update available — Inkue v{version}
        </div>
        <div style={{ fontSize: 12, color: "var(--wc-text-muted)", marginBottom: 16 }}>
          The update is downloaded and installed in place; Inkue restarts when it is done.
        </div>

        {notes && (
          <div
            style={{
              background: "rgba(255,255,255,0.04)", border: "1px solid var(--wc-border)",
              borderRadius: 6, padding: "10px 12px", fontSize: 12,
              color: "var(--wc-text-secondary)", marginBottom: 16,
              maxHeight: 200, overflowY: "auto", lineHeight: 1.5,
            }}
          >
            <ReleaseNotes notes={notes} />
          </div>
        )}

        {busy && (
          <div style={{ marginBottom: 16 }}>
            <div
              style={{
                height: 6, borderRadius: 3, overflow: "hidden",
                background: "var(--wc-bg-hover)", border: "1px solid var(--wc-border)",
              }}
            >
              <div
                style={{
                  height: "100%", background: "var(--wc-accent)",
                  width: progress === null ? "100%" : `${Math.round(progress * 100)}%`,
                  opacity: progress === null ? 0.4 : 1,
                  transition: "width 0.2s ease",
                }}
              />
            </div>
            <div style={{ fontSize: 11, color: "var(--wc-text-muted)", marginTop: 6 }}>
              {status === "installing"
                ? "Installing… Inkue will restart."
                : progress === null
                  ? "Downloading…"
                  : `Downloading… ${Math.round(progress * 100)}%`}
            </div>
          </div>
        )}

        {status === "error" && (
          <div style={{ fontSize: 12, color: "#ef4444", marginBottom: 16 }}>{error}</div>
        )}

        <div style={{ display: "flex", justifyContent: "flex-end", gap: 8 }}>
          <button
            onClick={dismiss}
            disabled={busy}
            style={{
              background: "transparent", border: "1px solid var(--wc-border-strong)",
              borderRadius: 6, color: "var(--wc-text)", cursor: busy ? "default" : "pointer",
              fontSize: 13, padding: "6px 16px", opacity: busy ? 0.5 : 1,
            }}
          >
            Later
          </button>
          <button
            onClick={() => void downloadAndInstall()}
            disabled={busy}
            style={{
              background: "var(--wc-accent)", border: "1px solid var(--wc-accent-hover)",
              borderRadius: 6, color: "var(--wc-accent-fg)", cursor: busy ? "default" : "pointer",
              fontSize: 13, fontWeight: 600, padding: "6px 16px", opacity: busy ? 0.5 : 1,
            }}
          >
            {status === "error" ? "Retry" : "Install & Restart"}
          </button>
        </div>
      </div>
    </div>
  );
}
