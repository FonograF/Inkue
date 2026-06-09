// Floating OSC message monitor — shows all raw incoming packets in real time.

import { useEffect, useRef } from "react";
import { useTransportStore } from "../../stores/transportStore";

const KNOWN_ADDRS = new Set([
  "/wincue/go", "/wincue/stop", "/wincue/hardstop",
  "/wincue/pause", "/wincue/resume",
  "/wincue/select/next", "/wincue/select/previous",
]);

function isKnown(addr: string): boolean {
  if (KNOWN_ADDRS.has(addr)) return true;
  return /^\/wincue\/cue\/.+\/(go|select|stop)$/.test(addr);
}

export function OscMonitor({ onClose }: { onClose: () => void }) {
  const { oscLog, clearOscLog } = useTransportStore();
  const bottomRef = useRef<HTMLDivElement>(null);

  // Auto-scroll to bottom on new entries.
  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [oscLog.length]);

  return (
    <div
      style={{
        position: "fixed",
        bottom: 104,
        right: 16,
        width: 460,
        maxHeight: 320,
        background: "#020617",
        border: "1px solid #334155",
        borderRadius: 8,
        boxShadow: "0 8px 32px rgba(0,0,0,0.7)",
        zIndex: 9999,
        display: "flex",
        flexDirection: "column",
        fontFamily: "monospace",
        fontSize: 12,
      }}
      onClick={(e) => e.stopPropagation()}
    >
      {/* Header */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          padding: "6px 10px",
          borderBottom: "1px solid #1e293b",
          gap: 8,
          flexShrink: 0,
        }}
      >
        <span style={{ color: "#4ade80", fontSize: 10 }}>●</span>
        <span style={{ color: "#94a3b8", fontWeight: 600, fontSize: 12, flex: 1 }}>
          OSC Monitor
        </span>
        <button
          onClick={clearOscLog}
          style={{
            background: "none", border: "1px solid #334155", borderRadius: 4,
            color: "#64748b", fontSize: 11, padding: "1px 8px", cursor: "pointer",
          }}
        >
          Clear
        </button>
        <button
          onClick={onClose}
          style={{
            background: "none", border: "none",
            color: "#64748b", fontSize: 16, cursor: "pointer", lineHeight: 1, padding: "0 2px",
          }}
        >
          ✕
        </button>
      </div>

      {/* Log */}
      <div style={{ overflowY: "auto", flex: 1, padding: "4px 0" }}>
        {oscLog.length === 0 ? (
          <div style={{ color: "#334155", padding: "12px 14px", fontSize: 12 }}>
            Waiting for OSC packets…
          </div>
        ) : (
          oscLog.map((entry) => {
            const known = isKnown(entry.addr);
            return (
              <div
                key={entry.id}
                style={{
                  display: "grid",
                  gridTemplateColumns: "88px 1fr",
                  gap: 8,
                  padding: "2px 12px",
                  borderBottom: "1px solid #0f172a",
                }}
              >
                {/* Timestamp */}
                <span style={{ color: "#475569", fontSize: 11, paddingTop: 1 }}>
                  {entry.ts}
                </span>
                {/* Address + args */}
                <div>
                  <span
                    style={{
                      color: known ? "#4ade80" : "#f97316",
                      fontWeight: 600,
                    }}
                  >
                    {entry.addr}
                  </span>
                  {entry.args.length > 0 && (
                    <span style={{ color: "#64748b", marginLeft: 8 }}>
                      {entry.args.join("  ")}
                    </span>
                  )}
                  {!known && (
                    <span style={{ color: "#ef4444", marginLeft: 8, fontSize: 10 }}>
                      ← unknown
                    </span>
                  )}
                </div>
              </div>
            );
          })
        )}
        <div ref={bottomRef} />
      </div>

      {/* Footer hint */}
      <div style={{
        padding: "4px 12px",
        borderTop: "1px solid #1e293b",
        fontSize: 10,
        color: "#334155",
        flexShrink: 0,
      }}>
        <span style={{ color: "#4ade80" }}>■</span> matched &nbsp;
        <span style={{ color: "#f97316" }}>■</span> unknown &nbsp;·&nbsp; max 100 entries
      </div>
    </div>
  );
}
