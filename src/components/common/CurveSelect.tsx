// Shared curve selector component — shows a mini SVG preview of each fade curve.
// Curve math matches the Rust engine exactly (ring_command.rs).

import { useState } from "react";
import type { FadeCurve } from "../../lib/types";

export const FADE_CURVES: { value: FadeCurve; label: string }[] = [
  { value: "s_curve",     label: "S-Curve"     },
  { value: "linear",      label: "Linear"      },
  { value: "exponential", label: "Exponential" },
];

// Linear: t
// S-Curve (smooth-step): 3t² − 2t³
// Exponential: (e^(5t) − 1) / (e^5 − 1)
function applyCurve(curve: FadeCurve, t: number): number {
  switch (curve) {
    case "linear": return t;
    case "s_curve": return t * t * (3 - 2 * t);
    case "exponential": {
      const K = 5;
      return (Math.exp(K * t) - 1) / (Math.exp(K) - 1);
    }
  }
}

export function CurveSvg({ curve, w, h }: { curve: FadeCurve; w: number; h: number }) {
  const N = 40;
  const pad = 2;
  const iw = w - pad * 2;
  const ih = h - pad * 2;
  const points = Array.from({ length: N + 1 }, (_, i) => {
    const t = i / N;
    const gain = applyCurve(curve, t);
    return `${(pad + t * iw).toFixed(1)},${(pad + (1 - gain) * ih).toFixed(1)}`;
  }).join(" ");

  return (
    <svg width={w} height={h} viewBox={`0 0 ${w} ${h}`} style={{ display: "block", flexShrink: 0 }}>
      <line x1={pad} y1={pad} x2={pad} y2={h - pad} stroke="var(--wc-border-strong)" strokeWidth="1" />
      <line x1={pad} y1={h - pad} x2={w - pad} y2={h - pad} stroke="var(--wc-border-strong)" strokeWidth="1" />
      <polyline
        points={points}
        fill="none"
        stroke="var(--wc-accent)"
        strokeWidth="1.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

export function CurveSelect({
  value,
  onChange,
  baseStyle,
}: {
  value: FadeCurve;
  onChange: (v: FadeCurve) => void;
  baseStyle?: React.CSSProperties;
}) {
  const [open, setOpen] = useState(false);
  const current = FADE_CURVES.find((c) => c.value === value) ?? FADE_CURVES[0];

  return (
    <div style={{ position: "relative", width: "100%" }}>
      {open && (
        <div
          style={{ position: "fixed", inset: 0, zIndex: 100 }}
          onClick={() => setOpen(false)}
        />
      )}

      <button
        onClick={() => setOpen((v) => !v)}
        style={{
          ...baseStyle,
          display: "flex",
          alignItems: "center",
          gap: 8,
          cursor: "pointer",
          textAlign: "left",
          background: open ? "var(--wc-bg-hover)" : (baseStyle?.background ?? "var(--wc-bg-surface)"),
          width: "100%",
          boxSizing: "border-box",
        }}
      >
        <CurveSvg curve={current.value} w={44} h={26} />
        <span style={{ flex: 1 }}>{current.label}</span>
        <span style={{ color: "var(--wc-text-muted)", fontSize: 10 }}>▾</span>
      </button>

      {open && (
        <div
          style={{
            position: "absolute",
            top: "calc(100% + 3px)",
            left: 0,
            right: 0,
            zIndex: 101,
            background: "var(--wc-bg-surface)",
            border: "1px solid var(--wc-border-strong)",
            borderRadius: 5,
            overflow: "hidden",
            boxShadow: "0 8px 24px rgba(0,0,0,0.6)",
          }}
        >
          {FADE_CURVES.map((c) => (
            <button
              key={c.value}
              onClick={() => { onChange(c.value); setOpen(false); }}
              style={{
                display: "flex",
                alignItems: "center",
                gap: 10,
                width: "100%",
                padding: "6px 10px",
                background: c.value === value ? "var(--wc-bg-hover)" : "transparent",
                border: "none",
                cursor: "pointer",
                textAlign: "left",
              }}
            >
              <CurveSvg curve={c.value} w={56} h={32} />
              <span style={{ color: "var(--wc-text)", fontSize: 13 }}>{c.label}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
