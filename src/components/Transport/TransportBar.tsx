// Bottom transport bar: GO / STOP + running cue info + horizontal VU-meter + volume slider.

import { useEffect, useRef, useState } from "react";
import { go, stopAll, pauseCue, resumeCue, setMasterVolume, getPreferences } from "../../lib/commands";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import { useTransportStore } from "../../stores/transportStore";
import { OscMonitor } from "../Osc/OscMonitor";

interface Props {
  onRefresh: () => void;
}

// ---------------------------------------------------------------------------
// dB helpers
// ---------------------------------------------------------------------------

const MIN_DB = -60;
const MAX_DB = 0;
const DB_RANGE = MAX_DB - MIN_DB; // 60

function linearToDb(linear: number): number {
  return linear > 0 ? 20 * Math.log10(linear) : -Infinity;
}

/** Map a dB value to a 0–1 fill ratio for the meter bar. */
function dbToRatio(db: number): number {
  return Math.max(0, Math.min(1, (db - MIN_DB) / DB_RANGE));
}

/** Convert a linear peak (0.0–1.0) to a meter fill percentage (0–100). */
function peakToFillPct(linear: number): number {
  return dbToRatio(linearToDb(linear)) * 100;
}

// Decay / hold constants
/** Bar falls at this many dB per second. */
const BAR_DECAY_DB_PER_SEC = 20;
/** Converted to fill-% per animation frame (assuming ~60 fps). */
const BAR_DECAY_PCT_PER_FRAME = (BAR_DECAY_DB_PER_SEC / DB_RANGE) * 100 / 60;

/** Peak-hold needle stays pinned for this long (ms). */
const PEAK_HOLD_MS = 1500;
/** After hold expires, needle falls at this many dB per second. */
const HOLD_DECAY_DB_PER_SEC = 8;
const HOLD_DECAY_PCT_PER_FRAME = (HOLD_DECAY_DB_PER_SEC / DB_RANGE) * 100 / 60;

const DB_TICKS = [0, -6, -12, -18, -24, -36];

// ---------------------------------------------------------------------------
// Meter + slider section — all rows share the same layout grid
// ---------------------------------------------------------------------------

const BAR_W = 220;
const BAR_H = 9;
const LABEL_W = 10;
const GAP = 5;

const METER_GRADIENT =
  "linear-gradient(to right, #4ade80 0%, #84cc16 55%, #facc15 72%, #f97316 85%, #ef4444 100%)";

interface MeterRowProps {
  label: string;
  /** 0–100 fill percentage for the bar. */
  fillPct: number;
  /** 0–100 position of the peak-hold needle (0 = hidden). */
  holdPct: number;
}

function MeterRow({ label, fillPct, holdPct }: MeterRowProps) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: GAP }}>
      <span
        style={{
          width: LABEL_W,
          fontSize: 9,
          color: "#475569",
          textAlign: "right",
          flexShrink: 0,
        }}
      >
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
        {/* Decaying fill bar — driven by rAF, no CSS transition */}
        <div
          style={{
            position: "absolute",
            inset: 0,
            right: `${100 - fillPct}%`,
            background: METER_GRADIENT,
            backgroundSize: `${BAR_W}px ${BAR_H}px`,
          }}
        />
        {/* Peak-hold needle */}
        {holdPct > 0 && (
          <div
            style={{
              position: "absolute",
              top: 0,
              bottom: 0,
              left: `${holdPct}%`,
              width: 2,
              // Red needle when near clipping (>90% ≈ above -6 dB)
              background: holdPct > 90 ? "#ef4444" : "#facc15",
            }}
          />
        )}
      </div>
    </div>
  );
}

function TickRow() {
  return (
    <div style={{ display: "flex", alignItems: "flex-end", gap: GAP }}>
      <div style={{ width: LABEL_W, flexShrink: 0 }} />
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
      <div style={{ width: LABEL_W, flexShrink: 0 }} />
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

const OSC_ACTIVITY_MS = 300;

export function TransportBar({ onRefresh }: Props) {
  const { cues } = useWorkspaceStore();
  const { masterPeakL, masterPeakR, oscActivityAt } = useTransportStore();

  const [volumeDb, setVolumeDb] = useState(0);
  const [oscActive, setOscActive] = useState(false);
  const [oscMonitorOpen, setOscMonitorOpen] = useState(false);

  // ---- VU meter animation state (rendered via rAF) ----
  const [meterL, setMeterL] = useState({ fill: 0, hold: 0 });
  const [meterR, setMeterR] = useState({ fill: 0, hold: 0 });

  // Mutable refs shared between the Zustand effect and the rAF loop.
  // Using refs avoids stale closure issues inside requestAnimationFrame.
  const fillL = useRef(0);
  const fillR = useRef(0);
  const holdL = useRef(0);
  const holdR = useRef(0);
  const holdExpiryL = useRef(0); // performance.now() timestamp when hold expires
  const holdExpiryR = useRef(0);
  const rafId = useRef(0);

  // When the backend emits a new peak, update fill and hold refs immediately.
  useEffect(() => {
    const fL = peakToFillPct(masterPeakL);
    const fR = peakToFillPct(masterPeakR);

    // Bar can only jump UP instantly; it decays in the rAF loop.
    if (fL > fillL.current) fillL.current = fL;
    if (fR > fillR.current) fillR.current = fR;

    // Peak hold: bump the needle if the new peak exceeds the current hold.
    const now = performance.now();
    if (fL >= holdL.current) {
      holdL.current = fL;
      holdExpiryL.current = now + PEAK_HOLD_MS;
    }
    if (fR >= holdR.current) {
      holdR.current = fR;
      holdExpiryR.current = now + PEAK_HOLD_MS;
    }
  }, [masterPeakL, masterPeakR]);

  // rAF decay loop — runs independently of Tauri events.
  useEffect(() => {
    const frame = () => {
      const now = performance.now();

      // Decay the fill bars downward each frame.
      fillL.current = Math.max(0, fillL.current - BAR_DECAY_PCT_PER_FRAME);
      fillR.current = Math.max(0, fillR.current - BAR_DECAY_PCT_PER_FRAME);

      // Decay the hold needles after their hold period expires.
      if (now >= holdExpiryL.current) {
        holdL.current = Math.max(0, holdL.current - HOLD_DECAY_PCT_PER_FRAME);
      }
      if (now >= holdExpiryR.current) {
        holdR.current = Math.max(0, holdR.current - HOLD_DECAY_PCT_PER_FRAME);
      }

      setMeterL({ fill: fillL.current, hold: holdL.current });
      setMeterR({ fill: fillR.current, hold: holdR.current });

      rafId.current = requestAnimationFrame(frame);
    };

    rafId.current = requestAnimationFrame(frame);
    return () => cancelAnimationFrame(rafId.current);
  }, []); // runs once on mount

  // ---- OSC activity flash ----
  useEffect(() => {
    if (oscActivityAt == null) return;
    setOscActive(true);
    const timer = setTimeout(() => setOscActive(false), OSC_ACTIVITY_MS);
    return () => clearTimeout(timer);
  }, [oscActivityAt]);

  // ---- Volume preference ----
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

  // Pause toggle: pause every running cue, or (if none running) resume every
  // paused cue — same semantics as the OSC `/wincue/pause_toggle` handler.
  const hasRunning = cues.some((c) => c.state === "running");
  const hasPaused = cues.some((c) => c.state === "paused");
  const pauseDisabled = !hasRunning && !hasPaused;
  const pauseLabel = hasRunning ? "⏸ PAUSE" : hasPaused ? "▶ RESUME" : "⏸ PAUSE";

  const handlePauseToggle = async () => {
    try {
      if (hasRunning) {
        for (const c of cues.filter((c) => c.state === "running")) await pauseCue(c.id);
      } else if (hasPaused) {
        for (const c of cues.filter((c) => c.state === "paused")) await resumeCue(c.id);
      }
    } catch (err) {
      console.error(err);
    }
    onRefresh();
  };

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

      {/* STOP */}
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

      {/* PAUSE / RESUME (toggle) */}
      <button
        onClick={handlePauseToggle}
        disabled={pauseDisabled}
        title="Pause all running cues / resume all paused cues"
        style={{
          padding: "14px 22px",
          fontSize: 18,
          fontWeight: 600,
          background: pauseDisabled ? "#1e293b" : "#38bdf8",
          color: pauseDisabled ? "#475569" : "white",
          border: "none",
          borderRadius: 8,
          cursor: pauseDisabled ? "default" : "pointer",
          boxShadow: pauseDisabled ? "none" : "0 2px 12px #38bdf899",
          flexShrink: 0,
          userSelect: "none",
        }}
      >
        {pauseLabel}
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

      {/* OSC activity indicator — click to open/close monitor */}
      <button
        title="OSC activity — click to open monitor"
        onClick={() => setOscMonitorOpen((v) => !v)}
        style={{
          width: 14,
          height: 14,
          borderRadius: "50%",
          background: oscMonitorOpen ? "#3b82f6" : oscActive ? "#4ade80" : "#1e293b",
          border: `1px solid ${oscMonitorOpen ? "#3b82f6" : "#334155"}`,
          flexShrink: 0,
          transition: "background 0.1s",
          cursor: "pointer",
          padding: 0,
        }}
      />
      {oscMonitorOpen && <OscMonitor onClose={() => setOscMonitorOpen(false)} />}

      {/* Meter + slider block */}
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          justifyContent: "center",
          gap: 3,
        }}
      >
        <TickRow />
        <MeterRow label="L" fillPct={meterL.fill} holdPct={meterL.hold} />
        <MeterRow label="R" fillPct={meterR.fill} holdPct={meterR.hold} />
        <VolumeRow valueDb={volumeDb} onChange={handleVolumeChange} />
      </div>
    </div>
  );
}
