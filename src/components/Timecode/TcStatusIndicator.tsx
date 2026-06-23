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
        background: flash ? "#1e3a5f" : "#0f172a",
        border: "1px solid #1e293b",
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
          background: running ? "#22c55e" : "#475569",
          flexShrink: 0,
          transition: "background 0.3s",
        }}
      />
      <span style={{ fontFamily: "monospace", fontSize: 11, color: running ? "#e2e8f0" : "#64748b" }}>
        {formatTc(pos)}
      </span>
      <span style={{ fontSize: 10, color: "#475569" }}>{pos.rate}</span>
    </div>
  );
}
