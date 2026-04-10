// Bottom transport bar: GO / STOP / PAUSE + running cue display.

import { go, stopAll } from "../../lib/commands";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import { useTransportStore } from "../../stores/transportStore";

interface Props {
  onRefresh: () => void;
}

export function TransportBar({ onRefresh }: Props) {
  const { cues } = useWorkspaceStore();
  const { masterPeakL, masterPeakR } = useTransportStore();

  const runningCues = cues.filter(
    (c) => c.state === "running" || c.state === "paused"
  );

  const handleGo = async () => {
    await go().catch(console.error);
    onRefresh();
  };

  const handleStop = async () => {
    await stopAll().catch(console.error);
    onRefresh();
  };

  const barStyle: React.CSSProperties = {
    display: "flex",
    alignItems: "center",
    gap: 12,
    padding: "6px 16px",
    background: "#020617",
    borderTop: "2px solid #1e293b",
    height: 52,
    flexShrink: 0,
  };

  const levelBarStyle = (_peak: number): React.CSSProperties => ({
    width: 8,
    height: 32,
    background: "#1e293b",
    borderRadius: 2,
    position: "relative",
    overflow: "hidden",
  });

  const levelFillStyle = (peak: number): React.CSSProperties => ({
    position: "absolute",
    bottom: 0,
    left: 0,
    right: 0,
    height: `${Math.min(peak * 100, 100)}%`,
    background:
      peak > 0.9 ? "#ef4444" : peak > 0.7 ? "#f97316" : "#4ade80",
    transition: "height 50ms ease-out",
  });

  return (
    <div style={barStyle}>
      {/* GO button */}
      <button
        style={{
          padding: "8px 28px",
          fontSize: 16,
          fontWeight: 700,
          background: "#16a34a",
          color: "white",
          border: "none",
          borderRadius: 6,
          cursor: "pointer",
          letterSpacing: "0.1em",
          boxShadow: "0 2px 8px #16a34a88",
          minWidth: 80,
        }}
        onClick={handleGo}
        title="GO (Space)"
      >
        GO
      </button>

      {/* STOP button */}
      <button
        style={{
          padding: "8px 16px",
          fontSize: 14,
          fontWeight: 600,
          background: "#991b1b",
          color: "white",
          border: "none",
          borderRadius: 6,
          cursor: "pointer",
        }}
        onClick={handleStop}
        title="Stop All (Escape)"
      >
        ■ STOP
      </button>

      {/* Running cue info */}
      <div style={{ flex: 1, overflow: "hidden" }}>
        {runningCues.length === 0 ? (
          <span style={{ color: "#475569", fontSize: 13 }}>Idle</span>
        ) : (
          runningCues.slice(0, 3).map((c) => (
            <div
              key={c.id}
              style={{
                fontSize: 12,
                color: c.state === "paused" ? "#f97316" : "#4ade80",
                whiteSpace: "nowrap",
                overflow: "hidden",
                textOverflow: "ellipsis",
              }}
            >
              {c.state === "paused" ? "⏸" : "▶"} {c.number ? `[${c.number}]` : ""}{" "}
              {c.name}
            </div>
          ))
        )}
      </div>

      {/* Master level meters */}
      <div style={{ display: "flex", gap: 3, alignItems: "flex-end" }}>
        <div style={levelBarStyle(masterPeakL)}>
          <div style={levelFillStyle(masterPeakL)} />
        </div>
        <div style={levelBarStyle(masterPeakR)}>
          <div style={levelFillStyle(masterPeakR)} />
        </div>
      </div>
    </div>
  );
}
