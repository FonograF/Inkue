// Floating Output Mixer window (label "mixer" in tauri.conf.json).
// Mini-DAW layout: one vertical strip per Output Patch — vertical fader
// (gain_db, persisted + hot-applied) next to a segmented stereo VU with
// proper meter ballistics (instant attack, smooth decay, peak hold),
// driven by the backend "patch-levels" event.

import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";
import type { OutputPatch } from "../lib/types";
import { getOutputPatchTable, setOutputPatchGain } from "../lib/commands";

const FADER_MIN = -60;
const FADER_MAX = 12;

/// Meter ballistics — tuned against flicker: the backend emits peaks ~30×/s,
/// so the bar tracks a slowly-decaying envelope through smoothing instead of
/// jumping to every packet.
/** Envelope decay per frame (0.955^60 ≈ −24 dB/s release). */
const ENV_DECAY_PER_FRAME = 0.955;
/** Fill smoothing toward the envelope (fraction per frame, both directions). */
const FILL_SMOOTHING = 0.30;
/** Peak-hold duration before the needle starts to fall (ms). */
const PEAK_HOLD_MS = 700;
/** Peak-needle decay per frame once the hold expired. */
const HOLD_DECAY_PER_FRAME = 0.006;

interface PatchLevel {
  slot: number;
  peak_l: number;
  peak_r: number;
}

/** Linear peak → fill fraction on a -60..0 dBFS scale. */
function peakToFraction(peak: number): number {
  if (peak <= 0) return 0;
  const db = 20 * Math.log10(peak);
  return Math.min(Math.max((db + 60) / 60, 0), 1);
}

/**
 * One meter channel: a continuous colour ladder (green → amber → red mapped
 * to fixed dB positions) revealed by the fill level, finely striated by a
 * repeating gradient so it reads as a real LED ladder — no stacked boxes.
 */
function MeterBar({ fill, hold }: { fill: number; hold: number }) {
  return (
    <div
      style={{
        width: 6, height: "100%", position: "relative",
        borderRadius: 3, overflow: "hidden",
        background: "var(--wc-bg-deepest)",
      }}
    >
      {/* Full-scale colour ladder (always present, dimmed when unlit) */}
      <div
        style={{
          position: "absolute", inset: 0,
          background:
            "linear-gradient(to top, #15803d 0%, #22c55e 55%, #a3e635 70%, #eab308 78%, #f59e0b 88%, #ef4444 94%, #ef4444 100%)",
        }}
      />
      {/* Unlit mask: covers everything above the fill; lets 10% ghost through */}
      <div
        style={{
          position: "absolute", top: 0, left: 0, right: 0,
          height: `${(1 - fill) * 100}%`,
          background: "var(--wc-bg-deepest)",
          opacity: 0.9,
        }}
      />
      {/* Fine LED pitch (2px on / 1px off) */}
      <div
        style={{
          position: "absolute", inset: 0, pointerEvents: "none",
          background:
            "repeating-linear-gradient(to top, transparent 0px, transparent 2px, rgba(0,0,0,0.5) 2px, rgba(0,0,0,0.5) 3px)",
        }}
      />
      {/* Peak-hold needle */}
      {hold > 0.02 && (
        <div
          style={{
            position: "absolute", left: 0, right: 0,
            bottom: `${hold * 100}%`,
            height: 2, background: "#f8fafc",
            opacity: 0.95,
          }}
        />
      )}
    </div>
  );
}

/**
 * Stereo VU with meter ballistics, one rAF loop per strip:
 * instant attack on each backend level event, smooth per-frame decay,
 * peak-hold needle that sits [`PEAK_HOLD_MS`] then falls.
 */
function StereoVu({ level }: { level: PatchLevel | undefined }) {
  const [display, setDisplay] = useState({ fl: 0, fr: 0, hl: 0, hr: 0 });
  // Envelope: bumped by backend peaks, decays exponentially in the rAF loop.
  const envL = useRef(0);
  const envR = useRef(0);
  // Displayed fill: eased toward the envelope — never jumps, never flickers.
  const fillL = useRef(0);
  const fillR = useRef(0);
  const holdL = useRef(0);
  const holdR = useRef(0);
  const holdExpiryL = useRef(0);
  const holdExpiryR = useRef(0);
  const rafId = useRef(0);

  // New backend level → raise the envelope; the peak needle catches the raw
  // value immediately (a peak meter must not miss transients).
  useEffect(() => {
    if (!level) return;
    const fl = peakToFraction(level.peak_l);
    const fr = peakToFraction(level.peak_r);
    if (fl > envL.current) envL.current = fl;
    if (fr > envR.current) envR.current = fr;
    const now = performance.now();
    if (fl >= holdL.current) { holdL.current = fl; holdExpiryL.current = now + PEAK_HOLD_MS; }
    if (fr >= holdR.current) { holdR.current = fr; holdExpiryR.current = now + PEAK_HOLD_MS; }
  }, [level]);

  useEffect(() => {
    const frame = () => {
      const now = performance.now();
      envL.current *= ENV_DECAY_PER_FRAME;
      envR.current *= ENV_DECAY_PER_FRAME;
      fillL.current += (envL.current - fillL.current) * FILL_SMOOTHING;
      fillR.current += (envR.current - fillR.current) * FILL_SMOOTHING;
      if (now >= holdExpiryL.current) holdL.current = Math.max(0, holdL.current - HOLD_DECAY_PER_FRAME);
      if (now >= holdExpiryR.current) holdR.current = Math.max(0, holdR.current - HOLD_DECAY_PER_FRAME);
      setDisplay({ fl: fillL.current, fr: fillR.current, hl: holdL.current, hr: holdR.current });
      rafId.current = requestAnimationFrame(frame);
    };
    rafId.current = requestAnimationFrame(frame);
    return () => cancelAnimationFrame(rafId.current);
  }, []);

  return (
    <div
      style={{
        display: "flex", gap: 3, height: "100%",
        padding: 3, borderRadius: 5,
        background: "var(--wc-bg-deepest)",
        border: "1px solid var(--wc-border)",
        boxShadow: "inset 0 2px 6px rgba(0,0,0,0.55)",
      }}
    >
      <MeterBar fill={display.fl} hold={display.hl} />
      <MeterBar fill={display.fr} hold={display.hr} />
    </div>
  );
}

/** Faint dB reference marks drawn across the meter area. */
function DbScale() {
  const marks = [
    { db: 0, label: "0" },
    { db: -12, label: "12" },
    { db: -24, label: "24" },
    { db: -40, label: "40" },
  ];
  return (
    <div style={{ position: "relative", width: 14, height: "100%", flexShrink: 0 }}>
      {marks.map(({ db, label }) => {
        const bottom = ((db + 60) / 60) * 100;
        return (
          <div
            key={db}
            style={{
              position: "absolute", left: 0, right: 0,
              bottom: `calc(${bottom}% - 4px)`,
              display: "flex", alignItems: "center", gap: 2,
            }}
          >
            <div style={{ width: 3, height: 1, background: "var(--wc-text-faint)", opacity: 0.7 }} />
            <span style={{ fontSize: 7, color: "var(--wc-text-faint)", lineHeight: 1 }}>{label}</span>
          </div>
        );
      })}
    </div>
  );
}

/** Custom vertical fader — pointer-driven, identical on all three OS. */
function VerticalFader({
  value,
  onChange,
  onReset,
}: {
  value: number;
  onChange: (v: number) => void;
  onReset: () => void;
}) {
  const trackRef = useRef<HTMLDivElement>(null);

  const valueFromPointer = useCallback((clientY: number) => {
    const track = trackRef.current;
    if (!track) return value;
    const rect = track.getBoundingClientRect();
    const t = 1 - Math.min(Math.max((clientY - rect.top) / rect.height, 0), 1);
    return Math.round((FADER_MIN + t * (FADER_MAX - FADER_MIN)) * 2) / 2;
  }, [value]);

  const handlePointerDown = (e: React.PointerEvent) => {
    e.currentTarget.setPointerCapture(e.pointerId);
    onChange(valueFromPointer(e.clientY));
  };
  const handlePointerMove = (e: React.PointerEvent) => {
    if (e.buttons & 1) onChange(valueFromPointer(e.clientY));
  };

  const fraction = (value - FADER_MIN) / (FADER_MAX - FADER_MIN);
  const unityFraction = (0 - FADER_MIN) / (FADER_MAX - FADER_MIN);

  return (
    <div
      ref={trackRef}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onDoubleClick={onReset}
      title="Drag to adjust — double-click resets to 0 dB"
      style={{
        width: 26, height: "100%", position: "relative",
        cursor: "ns-resize", touchAction: "none",
      }}
    >
      {/* Track groove */}
      <div
        style={{
          position: "absolute", left: "50%", top: 0, bottom: 0,
          width: 4, transform: "translateX(-50%)", borderRadius: 2,
          background: "var(--wc-bg-deepest)", border: "1px solid var(--wc-border)",
        }}
      />
      {/* Unity (0 dB) marker */}
      <div
        style={{
          position: "absolute", left: 2, right: 2,
          bottom: `${unityFraction * 100}%`,
          height: 1, background: "var(--wc-text-faint)", opacity: 0.6,
        }}
      />
      {/* Thumb */}
      <div
        style={{
          position: "absolute", left: "50%",
          bottom: `calc(${fraction * 100}% - 7px)`,
          transform: "translateX(-50%)",
          width: 22, height: 14, borderRadius: 3,
          background: "linear-gradient(to bottom, var(--wc-bg-hover), var(--wc-bg-surface))",
          border: "1px solid var(--wc-border-strong)",
          boxShadow: "0 2px 5px rgba(0,0,0,0.5)",
        }}
      >
        <div style={{ marginTop: 6, height: 1, background: "var(--wc-accent)" }} />
      </div>
    </div>
  );
}

function MixerStrip({
  patch,
  gain,
  level,
  onGain,
}: {
  patch: OutputPatch;
  gain: number;
  level: PatchLevel | undefined;
  onGain: (v: number) => void;
}) {
  return (
    <div
      style={{
        display: "flex", flexDirection: "column", alignItems: "center",
        width: 96, flexShrink: 0, padding: "10px 8px 8px",
        borderRight: "1px solid var(--wc-border)",
        background: "var(--wc-bg-surface)",
      }}
    >
      <span
        title={patch.name}
        style={{
          fontSize: 11, fontWeight: 600, color: "var(--wc-text)",
          maxWidth: "100%", overflow: "hidden", textOverflow: "ellipsis",
          whiteSpace: "nowrap", marginBottom: 2,
        }}
      >
        {patch.name}
      </span>
      <span style={{ fontSize: 9, color: "var(--wc-text-faint)", marginBottom: 8 }}>
        ch {patch.channels.map((c) => c + 1).join("-")}
      </span>

      <div style={{ display: "flex", gap: 6, flex: 1, minHeight: 0, alignItems: "stretch" }}>
        <VerticalFader value={gain} onChange={onGain} onReset={() => onGain(0)} />
        <StereoVu level={level} />
        <DbScale />
      </div>

      <span
        style={{
          marginTop: 8, fontSize: 10, fontFamily: "monospace",
          color: gain === 0 ? "var(--wc-text-muted)" : "var(--wc-accent)",
        }}
      >
        {gain > 0 ? "+" : ""}{gain.toFixed(1)} dB
      </span>
    </div>
  );
}

export function MixerWindow() {
  const [patches, setPatches] = useState<OutputPatch[]>([]);
  const [gains, setGains] = useState<Record<string, number>>({});
  const [levels, setLevels] = useState<Record<number, PatchLevel>>({});

  // Follow the main window's theme (persisted by App in localStorage).
  useEffect(() => {
    const theme = localStorage.getItem("wc_theme") ?? "dark";
    document.documentElement.setAttribute("data-theme", theme);
  }, []);

  const reload = useCallback(() => {
    getOutputPatchTable()
      .then((t) => {
        setPatches(t.patches);
        setGains((prev) => {
          const next: Record<string, number> = {};
          for (const p of t.patches) next[p.id] = prev[p.id] ?? p.gain_db ?? 0;
          return next;
        });
      })
      .catch(console.error);
  }, []);

  useEffect(() => {
    reload();
    // Fresh object per event so StereoVu's attack effect fires every update.
    const unlistenLevels = listen<{ levels: PatchLevel[] }>("patch-levels", (e) => {
      setLevels((prev) => {
        const next = { ...prev };
        for (const l of e.payload.levels) next[l.slot] = { ...l };
        return next;
      });
    });
    // Patch table edits in Preferences arrive as workspace-modified.
    const unlistenWs = listen("workspace-modified", reload);
    return () => {
      void unlistenLevels.then((u) => u());
      void unlistenWs.then((u) => u());
    };
  }, [reload]);

  const hide = () => void getCurrentWindow().hide();

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") hide();
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);

  const commitGain = (patchId: string, gainDb: number) => {
    setGains((prev) => ({ ...prev, [patchId]: gainDb }));
    setOutputPatchGain(patchId, gainDb).catch(console.error);
  };

  return (
    <div
      style={{
        position: "fixed", inset: 0, display: "flex", flexDirection: "column",
        background: "var(--wc-bg-app)", border: "1px solid var(--wc-border-strong)",
        borderRadius: 8, overflow: "hidden", userSelect: "none",
      }}
    >
      {/* Draggable title bar */}
      <div
        onMouseDown={(e) => {
          if ((e.target as HTMLElement).closest("button")) return;
          void getCurrentWindow().startDragging();
        }}
        style={{
          height: 30, display: "flex", alignItems: "center", flexShrink: 0,
          padding: "0 10px", cursor: "grab",
          background: "var(--wc-bg-deepest)", borderBottom: "1px solid var(--wc-border)",
        }}
      >
        <span style={{ fontSize: 11, fontWeight: 700, letterSpacing: 0.5, color: "var(--wc-text-bright)" }}>
          OUTPUT MIXER
        </span>
        <button
          onClick={hide}
          style={{
            marginLeft: "auto", background: "transparent", border: "none",
            color: "var(--wc-text-muted)", cursor: "pointer", fontSize: 14,
            lineHeight: 1, padding: 4,
          }}
        >
          ✕
        </button>
      </div>

      {/* Strips */}
      {patches.length === 0 ? (
        <div
          style={{
            flex: 1, display: "flex", alignItems: "center", justifyContent: "center",
            fontSize: 12, color: "var(--wc-text-muted)", padding: 16, textAlign: "center",
          }}
        >
          No Output Patches.
          Create them in Preferences → Audio → Output Patches.
        </div>
      ) : (
        <div style={{ flex: 1, display: "flex", overflowX: "auto", minHeight: 0 }}>
          {patches.map((patch, index) => (
            <MixerStrip
              key={patch.id}
              patch={patch}
              gain={gains[patch.id] ?? 0}
              level={levels[index]}
              onGain={(v) => commitGain(patch.id, v)}
            />
          ))}
        </div>
      )}
    </div>
  );
}
