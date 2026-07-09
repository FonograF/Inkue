// Projector Tools shown in Preferences → Display: a visual corner-pin /
// alignment editor (re-frame or warp the whole picture inside the projector)
// and calibration test patterns (alignment grid, colour bars, colorimetry
// image, …).  Everything applies live to the output window.

import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import type { OutputTransform, TestPatternKind } from "../../lib/types";
import { DEFAULT_OUTPUT_TRANSFORM } from "../../lib/types";
import {
  clearTestPattern,
  getOutputTransform,
  setOutputTransform,
  showTestPattern,
} from "../../lib/commands";
import { IMAGE_EXTENSIONS } from "../../lib/mediaTypes";
import { WarpEditor } from "./WarpEditor";

const sectionLabelStyle: React.CSSProperties = {
  fontSize: 10, fontWeight: 600, color: "var(--wc-text-muted)",
  textTransform: "uppercase", letterSpacing: "0.07em",
  marginBottom: 10, paddingBottom: 5,
  borderBottom: "1px solid var(--wc-border)",
};

const buttonStyle = (active: boolean): React.CSSProperties => ({
  padding: "5px 10px",
  fontSize: 12,
  borderRadius: 4,
  cursor: "pointer",
  whiteSpace: "nowrap",
  border: active ? "1px solid var(--wc-accent)" : "1px solid var(--wc-border-strong)",
  background: active ? "var(--wc-accent)" : "var(--wc-bg-surface)",
  color: active ? "var(--wc-accent-fg)" : "var(--wc-text)",
});

const PATTERNS: { kind: TestPatternKind; label: string; hint: string }[] = [
  { kind: "grid", label: "Grid", hint: "Alignment grid: cells + centre cross + border" },
  { kind: "smpte_bars", label: "SMPTE Bars", hint: "SMPTE HD colour bars" },
  { kind: "rgb_test", label: "RGB", hint: "RGB test chart" },
  { kind: "test_card", label: "Test Card", hint: "Fine detail — useful for focus" },
  { kind: "white", label: "White", hint: "Full white — light output / uniformity" },
  { kind: "gray", label: "Gray 50%", hint: "50% grey" },
  { kind: "black", label: "Black", hint: "Full black — black level / ambient light" },
];

function TransformField({
  label,
  value,
  step,
  min,
  max,
  suffix,
  onCommit,
}: {
  label: string;
  value: number;
  step: number;
  min: number;
  max: number;
  suffix: string;
  onCommit: (v: number) => void;
}) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 8 }}>
      <span style={{ width: 90, fontSize: 12, color: "var(--wc-text-secondary)", flexShrink: 0 }}>
        {label}
      </span>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onCommit(parseFloat(e.target.value))}
        style={{ flex: 1, cursor: "pointer" }}
      />
      <input
        type="number"
        min={min}
        max={max}
        step={step}
        key={`${label}-${value}`}
        defaultValue={value}
        onBlur={(e) => {
          const parsed = parseFloat(e.target.value);
          if (Number.isNaN(parsed)) return;
          onCommit(Math.min(max, Math.max(min, parsed)));
        }}
        style={{
          width: 64,
          background: "var(--wc-bg-app)",
          border: "1px solid var(--wc-border-strong)",
          borderRadius: 4,
          color: "var(--wc-text)",
          fontSize: 12,
          padding: "3px 6px",
        }}
      />
      <span style={{ width: 58, fontSize: 11, color: "var(--wc-text-muted)", flexShrink: 0 }}>
        {suffix}
      </span>
    </div>
  );
}

export function ProjectorToolsSection() {
  const [transform, setTransformState] = useState<OutputTransform>(DEFAULT_OUTPUT_TRANSFORM);
  const [activePattern, setActivePatternState] = useState<TestPatternKind | null>(null);
  const [customImagePath, setCustomImagePath] = useState<string | null>(null);
  // Debounce slider drags: one backend call per animation-ish frame is plenty.
  const pendingRef = useRef<number | null>(null);
  // Mirror of activePattern readable from the unmount cleanup (setState is
  // unreliable there).
  const activePatternRef = useRef<TestPatternKind | null>(null);

  const setActivePattern = (kind: TestPatternKind | null) => {
    activePatternRef.current = kind;
    setActivePatternState(kind);
  };

  useEffect(() => {
    getOutputTransform().then(setTransformState).catch(console.error);
    // Leaving Preferences (unmount) clears any test pattern still showing —
    // a calibration grid must never survive into the show.
    return () => {
      if (pendingRef.current !== null) window.clearTimeout(pendingRef.current);
      if (activePatternRef.current !== null) {
        activePatternRef.current = null;
        void clearTestPattern().catch(console.error);
      }
    };
  }, []);

  const applyTransform = (partial: Partial<OutputTransform>) => {
    const next = { ...transform, ...partial };
    setTransformState(next);
    if (pendingRef.current !== null) window.clearTimeout(pendingRef.current);
    pendingRef.current = window.setTimeout(() => {
      pendingRef.current = null;
      void setOutputTransform(next).catch(console.error);
    }, 40);
  };

  const cornersPinned = transform.corners.some(([x, y]) => x !== 0 || y !== 0);
  const isIdentity =
    transform.pan_x === 0 && transform.pan_y === 0 &&
    transform.scale === 1 && transform.rotation === 0 &&
    !cornersPinned;

  const togglePattern = async (kind: TestPatternKind, path?: string) => {
    if (activePattern === kind && kind !== "custom_image") {
      setActivePattern(null);
      await clearTestPattern().catch(console.error);
      return;
    }
    setActivePattern(kind);
    await showTestPattern(kind === "custom_image" ? { kind, path } : { kind }).catch(console.error);
  };

  const pickCustomImage = async () => {
    const result = await open({
      multiple: false,
      filters: [{ name: "Image Files", extensions: [...IMAGE_EXTENSIONS] }],
    });
    if (typeof result === "string") {
      setCustomImagePath(result);
      await togglePattern("custom_image", result);
    }
  };

  return (
    <>
      <div style={{ marginBottom: 24 }}>
        <div style={sectionLabelStyle}>Projector Alignment</div>
        <div style={{ fontSize: 11, color: "var(--wc-text-faint)", marginBottom: 10 }}>
          Warps everything on the output window (all Video and Image cues, and the test
          patterns below) inside the projector. <b>Drag a corner</b> to pin it
          (perspective warp), <b>drag the centre cross</b> to move the whole picture.
          Applies live and is saved in the workspace — show the Grid pattern below
          while aligning.
        </div>
        <div style={{ marginBottom: 10 }}>
          <WarpEditor transform={transform} onChange={(t) => applyTransform(t)} />
        </div>
        <TransformField
          label="Rotation"
          value={transform.rotation}
          step={0.1}
          min={-180}
          max={180}
          suffix="° cw"
          onCommit={(v) => applyTransform({ rotation: Math.round(v * 10) / 10 })}
        />
        <TransformField
          label="Scale"
          value={transform.scale}
          step={0.01}
          min={0.1}
          max={2}
          suffix="×"
          onCommit={(v) => applyTransform({ scale: v })}
        />
        <div style={{ display: "flex", gap: 6 }}>
          <button
            disabled={!cornersPinned}
            onClick={() => applyTransform({ corners: [[0, 0], [0, 0], [0, 0], [0, 0]] })}
            style={{
              ...buttonStyle(false),
              color: !cornersPinned ? "var(--wc-text-faint)" : "var(--wc-text)",
              cursor: !cornersPinned ? "default" : "pointer",
            }}
          >
            Reset corners
          </button>
          <button
            disabled={isIdentity}
            onClick={() => applyTransform({ ...DEFAULT_OUTPUT_TRANSFORM })}
            style={{
              ...buttonStyle(false),
              color: isIdentity ? "var(--wc-text-faint)" : "var(--wc-text)",
              cursor: isIdentity ? "default" : "pointer",
            }}
          >
            Reset all
          </button>
        </div>
      </div>

      <div style={{ marginBottom: 24 }}>
        <div style={sectionLabelStyle}>Test Patterns</div>
        <div style={{ fontSize: 11, color: "var(--wc-text-faint)", marginBottom: 10 }}>
          Shown fullscreen on the configured output (replaces any playing visual cue).
          The alignment above applies, so you calibrate exactly what the audience sees.
        </div>
        <div style={{ display: "flex", flexWrap: "wrap", gap: 6, marginBottom: 8 }}>
          {PATTERNS.map((p) => (
            <button
              key={p.kind}
              title={p.hint}
              onClick={() => void togglePattern(p.kind)}
              style={buttonStyle(activePattern === p.kind)}
            >
              {p.label}
            </button>
          ))}
          <button
            title={customImagePath ?? "Show a colorimetry reference image of your choice"}
            onClick={() => void pickCustomImage()}
            style={buttonStyle(activePattern === "custom_image")}
          >
            Image…
          </button>
        </div>
        <button
          disabled={activePattern === null}
          onClick={() => {
            setActivePattern(null);
            void clearTestPattern().catch(console.error);
          }}
          style={{
            ...buttonStyle(false),
            color: activePattern === null ? "var(--wc-text-faint)" : "var(--wc-text)",
            cursor: activePattern === null ? "default" : "pointer",
          }}
        >
          Clear pattern
        </button>
      </div>
    </>
  );
}
