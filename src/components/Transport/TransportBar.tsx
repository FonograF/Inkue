// Bottom transport bar: GO / STOP + running cue info + horizontal VU-meter + volume slider.

import { useEffect, useState } from "react";
import { go, stopAll, setMasterVolume, getPreferences } from "../../lib/commands";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import { useTransportStore } from "../../stores/transportStore";

interface Props {
  onRefresh: () => void;
}

// ---------------------------------------------------------------------------
// dB helpers
// ---------------------------------------------------------------------------

const MIN_DB = -60;
const MAX_DB = 0;

function linearToDb(linear: number): number {
  return linear > 0 ? 20 * Math.log10(linear) : -Infinity;
}

function dbToRatio(db: number): number {
  return Math.max(0, Math.min(1, (db - MIN_DB) / (MAX_DB - MIN_DB)));
}

const DB_TICKS = [0, -6, -12, -18, -24, -36];

// ---------------------------------------------------------------------------
// Meter + slider section — all rows share the same layout grid
// ---------------------------------------------------------------------------

const BAR_W = 220;
const BAR_H = 9;
const LABEL_W = 10; // width of "L" / "R" / "" label column
const GAP = 5;      // gap between label and bar

const METER_GRADIENT =
  "linear-gradient(to right, #4ade80 0%, #84cc16 55%, #facc15 72%, #f97316 85%, #ef4444 100%)";

function MeterRow({ label, fillPct }: { label: string; fillPct: number }) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: GAP }}>
      <span style={{ width: LABEL_W, fontSize: 9, color: "#475569", textAlign: "right", flexShrink: 0 }}>
        {label}
      </span>
      <div
        style={{
          width: BAR_W,
          height: BAR_H,
          background: "#0f172a",
          borderRadius: 2,
          border: "1px solid #1e293b",
          position: "relative",
          overflow: "hidden",
          flexShrink: 0,
        }}
      >
        <div
          style={{
            position: "absolute",
            inset: 0,
            right: `${100 - fillPct}%`,
            background: METER_GRADIENT,
            backgroundSize: `${BAR_W}px ${BAR_H}px`,
            transition: "right 40ms ease-out",
          }}
        />
      </div>
    </div>
  );
}

function TickRow() {
  return (
    <div style={{ display: "flex", alignItems: "flex-end", gap: GAP }}>
      {/* Spacer matching the label column */}
      <div style={{ width: LABEL_W, flexShrink: 0 }} />
      {/* Tick scale */}
      <div style={{ width: BAR_W, position: "relative", height: 14, flexShrink: 0 }}>
        {DB_TICKS.map((db) => (
          <div
            key={db}
            style={{
              position: "absolute",
              left: `${dbToRatio(db) * 100}%`,
              transform: "translateX(-50%)",
              display: "flex",
              flexDirection: "column",
              alignItems: "center",
              gap: 1,
            }}
          >
            <span style={{ fontSize: 9, color: "#475569", lineHeight: 1 }}>{db}</span>
            <div style={{ width: 1, height: 3, background: "#334155" }} />
          </div>
        ))}
      </div>
    </div>
  );
}

function VolumeRow({
  valueDb,
  onChange,
}: {
  valueDb: number;
  onChange: (db: number) => void;
}) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: GAP }}>
      {/* Spacer matching label column */}
      <div style={{ width: LABEL_W, flexShrink: 0 }} />

      {/* Native range input — same width as the meter bars */}
      <input
        type="range"
        min={MIN_DB}
        max={MAX_DB}
        step={0.5}
        value={valueDb}
        onChange={(e) => onChange(Number(e.target.value))}
        style={{
          width: BAR_W,
          margin: 0,
          padding: 0,
          flexShrink: 0,
          cursor: "pointer",
          accentColor: "#475569",
        }}
      />

      {/* Value readout — fixed width to prevent layout shift */}
      <span
        style={{
          fontSize: 10,
          color: "#64748b",
          fontFamily: "monospace",
          width: 58,
          flexShrink: 0,
          display: "inline-block",
        }}
      >
        {valueDb >= 0 ? `+${valueDb.toFixed(1)}` : valueDb.toFixed(1)} dB
      </span>
    </div>
  );
}

// ---------------------------------------------------------------------------
// TransportBar
// ---------------------------------------------------------------------------

export function TransportBar({ onRefresh }: Props) {
  const { cues } = useWorkspaceStore();
  const { masterPeakL, masterPeakR } = useTransportStore();

  const [volumeDb, setVolumeDb] = useState(0);

  useEffect(() => {
    getPreferences()
      .then((prefs) => {
        const db = prefs.audio.default_volume_db;
        setVolumeDb(db);
        void setMasterVolume(db).catch(console.error);
      })
      .catch(console.error);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const handleVolumeChange = (db: number) => {
    setVolumeDb(db);
    void setMasterVolume(db).catch(console.error);
  };

  const runningCues = cues.filter(
    (c) => c.state === "running" || c.state === "paused"
  );

  const fillL = dbToRatio(linearToDb(masterPeakL)) * 100;
  const fillR = dbToRatio(linearToDb(masterPeakR)) * 100;

  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 12,
        padding: "0 16px",
        background: "#020617",
        borderTop: "2px solid #1e293b",
        height: 96,
        flexShrink: 0,
      }}
    >
      {/* GO */}
      <button
        onClick={async () => { await go().catch(console.error); onRefresh(); }}
        title="GO (Space)"
        style={{
          padding: "14px 36px",
          fontSize: 22,
          fontWeight: 700,
          background: "#16a34a",
          color: "white",
          border: "none",
          borderRadius: 8,
          cursor: "pointer",
          letterSpacing: "0.12em",
          boxShadow: "0 2px 12px #16a34a99",
          minWidth: 100,
          flexShrink: 0,
        }}
      >
        GO
      </button>

      {/* STOP — click to stop all; mousedown-drag into cue list to insert a Stop cue */}
      <button
        onMouseDown={(e) => {
          if (e.button !== 0) return;
          document.dispatchEvent(
            new CustomEvent("wincue:cue-drag-start", {
              detail: { cueType: "stop", startX: e.clientX, startY: e.clientY },
            }),
          );
        }}
        onClick={async () => { await stopAll().catch(console.error); onRefresh(); }}
        title="Stop All (Escape) · Drag into cue list to insert a Stop cue"
        style={{
          padding: "14px 22px",
          fontSize: 18,
          fontWeight: 600,
          background: "#991b1b",
          color: "white",
          border: "none",
          borderRadius: 8,
          cursor: "grab",
          flexShrink: 0,
          userSelect: "none",
        }}
      >
        ■ STOP
      </button>

      {/* Running cue info */}
      <div style={{ flex: 1, overflow: "hidden" }}>
        {runningCues.length === 0 ? (
          <span style={{ color: "#334155", fontSize: 18, fontWeight: 600 }}>Idle</span>
        ) : (
          runningCues.slice(0, 3).map((c) => (
            <div
              key={c.id}
              style={{
                fontSize: 16,
                fontWeight: 600,
                color: c.state === "paused" ? "#f97316" : "#4ade80",
                whiteSpace: "nowrap",
                overflow: "hidden",
                textOverflow: "ellipsis",
              }}
            >
              {c.state === "paused" ? "⏸" : "▶"}{" "}
              {c.number ? `[${c.number}] ` : ""}
              {c.name}
            </div>
          ))
        )}
      </div>

      {/* Meter + slider block */}
      <div style={{ display: "flex", flexDirection: "column", justifyContent: "center", gap: 3 }}>
        <TickRow />
        <MeterRow label="L" fillPct={fillL} />
        <MeterRow label="R" fillPct={fillR} />
        <VolumeRow valueDb={volumeDb} onChange={handleVolumeChange} />
      </div>
    </div>
  );
}
