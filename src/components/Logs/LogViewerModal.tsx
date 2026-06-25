import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getRecentLogs, clearLogs, openLogsFolder } from "../../lib/commands";
import type { LogLine } from "../../lib/types";

const LEVEL_COLOR: Record<string, string> = {
  ERROR: "#ef4444",
  WARN: "#fbbf24",
  INFO: "#86efac",
  DEBUG: "var(--wc-text-muted)",
  TRACE: "var(--wc-text-faint)",
};

type LevelFilter = "ALL" | "INFO" | "WARN" | "ERROR";
const FILTER_ORDER = ["TRACE", "DEBUG", "INFO", "WARN", "ERROR"];

/** In-app log viewer: live-tails the backend log buffer (event-driven, no poll). */
export function LogViewerModal({ onClose }: { onClose: () => void }) {
  const [lines, setLines] = useState<LogLine[]>([]);
  const [filter, setFilter] = useState<LevelFilter>("ALL");
  const [autoScroll, setAutoScroll] = useState(true);
  const scrollRef = useRef<HTMLDivElement>(null);

  const refresh = useCallback(async () => {
    try {
      setLines(await getRecentLogs(2000));
    } catch (e) {
      console.error("get logs failed", e);
    }
  }, []);

  // Initial fetch + live-tail on the backend's "logs-updated" event.
  useEffect(() => {
    void refresh();
    let unlisten: (() => void) | undefined;
    void listen("logs-updated", () => { void refresh(); }).then((u) => { unlisten = u; });
    return () => unlisten?.();
  }, [refresh]);

  const visible = lines.filter((l) => {
    if (filter === "ALL") return true;
    const min = FILTER_ORDER.indexOf(filter);
    return FILTER_ORDER.indexOf(l.level) >= min;
  });

  // Keep the view pinned to the newest line while auto-scroll is on.
  useEffect(() => {
    if (autoScroll && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [visible.length, autoScroll]);

  const handleCopy = () => {
    const text = visible.map((l) => `${l.ts} ${l.level} ${l.target} ${l.message}`).join("\n");
    void navigator.clipboard.writeText(text).catch(() => {});
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
          borderRadius: 12, padding: "18px 20px", width: 820, height: "78vh",
          display: "flex", flexDirection: "column",
          boxShadow: "0 16px 48px rgba(0,0,0,0.8)",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: 12, marginBottom: 12 }}>
          <span style={{ fontSize: 16, fontWeight: 700, color: "var(--wc-text-bright)" }}>Logs</span>
          <div style={{ flex: 1 }} />
          {(["ALL", "INFO", "WARN", "ERROR"] as LevelFilter[]).map((f) => (
            <button
              key={f}
              onClick={() => setFilter(f)}
              style={{
                background: filter === f ? "var(--wc-bg-hover)" : "transparent",
                border: "1px solid var(--wc-border)", borderRadius: 5,
                color: filter === f ? "var(--wc-text-bright)" : "var(--wc-text-secondary)",
                cursor: "pointer", fontSize: 11, padding: "3px 9px",
              }}
            >
              {f}
            </button>
          ))}
          <label style={{ display: "flex", alignItems: "center", gap: 5, fontSize: 11, color: "var(--wc-text-secondary)", marginLeft: 6 }}>
            <input type="checkbox" checked={autoScroll} onChange={(e) => setAutoScroll(e.target.checked)} />
            Suivre
          </label>
        </div>

        <div
          ref={scrollRef}
          style={{
            flex: 1, overflowY: "auto", background: "rgba(0,0,0,0.25)",
            border: "1px solid var(--wc-border)", borderRadius: 6, padding: "8px 10px",
            fontFamily: "monospace", fontSize: 11.5, lineHeight: 1.5,
          }}
        >
          {visible.length === 0 ? (
            <div style={{ color: "var(--wc-text-faint)" }}>Aucune entrée.</div>
          ) : (
            visible.map((l, i) => (
              <div key={i} style={{ display: "flex", gap: 8, whiteSpace: "pre-wrap", wordBreak: "break-word" }}>
                <span style={{ color: "var(--wc-text-faint)", flexShrink: 0 }}>{l.ts}</span>
                <span style={{ color: LEVEL_COLOR[l.level] ?? "var(--wc-text)", flexShrink: 0, width: 44 }}>{l.level}</span>
                <span style={{ color: "var(--wc-text)" }}>{l.message}</span>
              </div>
            ))
          )}
        </div>

        <div style={{ display: "flex", alignItems: "center", gap: 10, marginTop: 12 }}>
          <span style={{ fontSize: 11, color: "var(--wc-text-muted)" }}>{visible.length} ligne{visible.length > 1 ? "s" : ""}</span>
          <div style={{ flex: 1 }} />
          <button onClick={() => void openLogsFolder()} style={btn}>Ouvrir le dossier</button>
          <button onClick={handleCopy} style={btn}>Copier</button>
          <button onClick={() => { void clearLogs().then(refresh); }} style={btn}>Vider</button>
          <button onClick={onClose} style={{ ...btn, background: "var(--wc-bg-hover)", color: "var(--wc-text)" }}>Fermer</button>
        </div>
      </div>
    </div>
  );
}

const btn: React.CSSProperties = {
  background: "transparent", border: "1px solid var(--wc-border-strong)",
  borderRadius: 6, color: "var(--wc-text-secondary)", cursor: "pointer",
  fontSize: 12, padding: "5px 14px",
};
