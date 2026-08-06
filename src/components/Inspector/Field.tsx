import { useEffect, useState } from "react";
import { DragNumber } from "../common/DragNumber";
// Shared layout primitives and input styles used across all Inspector tabs.
//
// The inspector's visual language: content is grouped into `Section` cards,
// fields are label+control `Field` rows, related numerics sit side-by-side in
// a `Grid2`, continuous values use `SliderRow`, and short exclusive choices
// use `Segmented` buttons.

export const inputStyle: React.CSSProperties = {
  background: "var(--wc-bg-surface)",
  border: "1px solid var(--wc-border-strong)",
  borderRadius: 4,
  color: "var(--wc-text)",
  padding: "4px 8px",
  fontSize: 13,
  width: "100%",
  boxSizing: "border-box",
};

export function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        marginBottom: 10,
        gap: 8,
      }}
    >
      <label style={{ width: 100, color: "var(--wc-text-secondary)", flexShrink: 0, fontSize: 12 }}>
        {label}
      </label>
      <div style={{ flex: 1, minWidth: 0 }}>{children}</div>
    </div>
  );
}

/** Card grouping related fields under an uppercase micro-title. */
export function Section({
  title,
  children,
  hint,
}: {
  title: string;
  children: React.ReactNode;
  /** Optional caption below the title (e.g. live-apply note). */
  hint?: string;
}) {
  return (
    <div
      style={{
        background: "var(--wc-bg-deepest)",
        border: "1px solid var(--wc-border)",
        borderRadius: 6,
        padding: "10px 12px 6px",
        marginBottom: 10,
      }}
    >
      <div
        style={{
          fontSize: 10,
          fontWeight: 700,
          letterSpacing: "0.07em",
          textTransform: "uppercase",
          color: "var(--wc-text-muted)",
          marginBottom: hint ? 2 : 8,
        }}
      >
        {title}
      </div>
      {hint && (
        <div style={{ fontSize: 11, color: "var(--wc-text-faint)", marginBottom: 8 }}>{hint}</div>
      )}
      {children}
    </div>
  );
}

/** Two-column grid for compact side-by-side fields. */
export function Grid2({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: "1fr 1fr",
        columnGap: 10,
        rowGap: 8,
        marginBottom: 10,
      }}
    >
      {children}
    </div>
  );
}

/** Small stacked label + control, for use inside `Grid2` cells. */
export function MiniField({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div style={{ minWidth: 0 }}>
      <div style={{ fontSize: 11, color: "var(--wc-text-secondary)", marginBottom: 3 }}>
        {label}
      </div>
      {children}
    </div>
  );
}

/** Numeric input that clamps and commits on blur (or Enter). */
export function NumberInput({
  value,
  step,
  min,
  max,
  width,
  placeholder,
  onCommit,
}: {
  value: number | null;
  step: number;
  min: number;
  max: number;
  width?: number;
  placeholder?: string;
  onCommit: (v: number) => void;
}) {
  // Controlled draft: the field was uncontrolled (defaultValue + a remount
  // key), which cannot work with a drag wheel — dragging has to move the
  // displayed value on every pointer event.
  const [draft, setDraft] = useState<string>(value?.toString() ?? "");
  useEffect(() => { setDraft(value?.toString() ?? ""); }, [value]);

  const commit = () => {
    const parsed = parseFloat(draft);
    if (Number.isNaN(parsed)) return;
    onCommit(Math.min(max, Math.max(min, parsed)));
  };

  return (
    <DragNumber
      style={{ ...inputStyle, width: width ?? "100%" }}
      step={step}
      min={min}
      max={max}
      value={draft}
      placeholder={placeholder}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={commit}
    />
  );
}

/** Label + range slider + numeric value, committing on drag and on blur. */
export function SliderRow({
  label,
  value,
  min,
  max,
  step,
  format,
  onChange,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  /** Renders the numeric readout (e.g. percents, degrees). */
  format: (v: number) => string;
  onChange: (v: number) => void;
}) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 10 }}>
      <label style={{ width: 100, color: "var(--wc-text-secondary)", flexShrink: 0, fontSize: 12 }}>
        {label}
      </label>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onChange(parseFloat(e.target.value))}
        style={{ flex: 1, cursor: "pointer", minWidth: 0 }}
      />
      <span
        style={{
          width: 52,
          textAlign: "right",
          fontSize: 12,
          fontVariantNumeric: "tabular-nums",
          color: "var(--wc-text-secondary)",
          flexShrink: 0,
        }}
      >
        {format(value)}
      </span>
    </div>
  );
}

/** Row of mutually-exclusive segment buttons (e.g. Fit / Fill / Stretch). */
export function Segmented<T extends string>({
  options,
  value,
  onChange,
}: {
  options: { value: T; label: string; hint?: string }[];
  value: T;
  onChange: (v: T) => void;
}) {
  return (
    <div style={{ display: "flex", gap: 0, marginBottom: 10, borderRadius: 5, overflow: "hidden", border: "1px solid var(--wc-border-strong)" }}>
      {options.map((o, i) => {
        const active = value === o.value;
        return (
          <button
            key={o.value}
            title={o.hint}
            onClick={() => onChange(o.value)}
            style={{
              flex: 1,
              padding: "5px 0",
              fontSize: 12,
              cursor: "pointer",
              border: "none",
              borderLeft: i > 0 ? "1px solid var(--wc-border-strong)" : "none",
              background: active ? "var(--wc-accent)" : "var(--wc-bg-surface)",
              color: active ? "var(--wc-accent-fg)" : "var(--wc-text)",
              fontWeight: active ? 600 : 400,
            }}
          >
            {o.label}
          </button>
        );
      })}
    </div>
  );
}

/** A row whose nested controls are gated by a leading checkbox. */
export function ToggleRow({
  label, checked, onToggle, children,
}: {
  label: string; checked: boolean; onToggle: (v: boolean) => void; children?: React.ReactNode;
}) {
  return (
    <div style={{ marginBottom: 10 }}>
      <label style={{ display: "flex", alignItems: "center", gap: 8, cursor: "pointer" }}>
        <input type="checkbox" checked={checked} onChange={(e) => onToggle(e.target.checked)}
          style={{ width: 15, height: 15, cursor: "pointer" }} />
        <span style={{ fontSize: 13, color: "var(--wc-text)" }}>{label}</span>
      </label>
      {checked && children && <div style={{ marginTop: 8, paddingLeft: 23 }}>{children}</div>}
    </div>
  );
}
