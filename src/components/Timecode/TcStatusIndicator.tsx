// Timecode status indicator shown in the Transport Bar.
// Shows the current received TC position and flashes on lock.

import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import type { TcPosition } from "../../lib/types";

function pad(n: number, w = 2) { return String(n).padStart(w, "0"); }

function formatTc(pos: TcPosition): string {
  const sep = pos.rate.endsWith("df") ? ";" : ":";
  return `${pad(pos.h)}:${pad(pos.m)}:${pad(pos.s)}${sep}${pad(pos.f)}`;
}

export function TcStatusIndicator() {
  const [pos, setPos] = useState<TcPosition | null>(null);
  const [running, setRunning] = useState(false);
  const [flash, setFlash] = useState(false);

  useEffect(() => {
    const unlisten = listen<TcPosition & { running?: boolean }>("timecode", (ev) => {
      setPos(ev.payload);
      setRunning(true);
      setFlash(true);
      setTimeout(() => setFlash(false), 80);
    });
    const unlistenStop = listen("timecode-stopped", () => {
      setRunning(false);
    });
    return () => {
      void unlisten.then((fn) => fn());
      void unlistenStop.then((fn) => fn());
    };
  }, []);

  if (!pos) return null;

  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 5,
        padding: "2px 8px",
        background: flash ? "var(--wc-accent-dim)" : "var(--wc-bg-app)",
        border: "1px solid var(--wc-border)",
        borderRadius: 4,
        transition: "background 80ms",
        userSelect: "none",
      }}
      title={`Timecode: ${formatTc(pos)} @ ${pos.rate}`}
    >
      <span
        style={{
          width: 7, height: 7,
          borderRadius: "50%",
          background: running ? "#22c55e" : "var(--wc-text-faint)",
          flexShrink: 0,
          transition: "background 0.3s",
        }}
      />
      <span style={{ fontFamily: "monospace", fontSize: 11, color: running ? "var(--wc-text)" : "var(--wc-text-muted)" }}>
        {formatTc(pos)}
      </span>
      <span style={{ fontSize: 10, color: "var(--wc-text-faint)" }}>{pos.rate}</span>
    </div>
  );
}
