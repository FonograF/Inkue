// Numeric field with a drag wheel instead of the native spinner buttons.
//
// Clicking a 12-pixel arrow repeatedly is the wrong gesture for values you
// tune by feel — level, wait, rotation. Here the grip on the right is dragged
// vertically: up raises, down lowers, and holding Shift makes it ten times
// finer for the last decimal.
//
// It is a drop-in replacement for a native numeric input: same `value` /
// `onChange` /
// `onBlur` / `step` / `min` / `max` / `style` props, and `onChange` still hands
// back an object with `target.value` as a string, so existing call sites keep
// their `parseFloat(e.target.value)` untouched.
//
// The field itself is `type="text"` with a numeric input mode: that removes
// the native spinner across all three OS without fighting browser-specific
// pseudo-elements, and arrow-key stepping is implemented here instead.

import { useRef, useState } from "react";

/** Pixels of travel per step. Small enough to feel direct, large enough to aim. */
const PIXELS_PER_STEP = 4;
/** Shift divides the step by this, for fine tuning. */
const FINE_DIVISOR = 10;

interface Props {
  /** Controlled value. Omit and pass `defaultValue` for uncontrolled use. */
  value?: string | number;
  /** Uncontrolled seed — the field then owns its own value, exactly like a
   *  native input, and callers read it from `onBlur`'s `target.value`. */
  defaultValue?: string | number;
  onChange?: (e: { target: { value: string } }) => void;
  /** Receives an event-like object, so call sites reading `e.target.value`
   *  keep working — a real focus event satisfies the same shape. */
  onBlur?: (e: { target: { value: string } }) => void;
  step?: string | number;
  min?: string | number;
  max?: string | number;
  style?: React.CSSProperties;
  disabled?: boolean;
  title?: string;
  placeholder?: string;
  autoFocus?: boolean;
}

export function DragNumber({
  value,
  defaultValue,
  onChange,
  onBlur,
  step = 1,
  min,
  max,
  style,
  disabled,
  title,
  placeholder,
  autoFocus,
}: Props) {
  const [dragging, setDragging] = useState(false);
  const [hover, setHover] = useState(false);
  // Drag state accumulates *incrementally*: the last pointer position and an
  // unquantised running value. Recomputing from the drag's origin instead
  // would re-scale all the travel already made the moment Shift changes the
  // step, making the value jump.  `raw` is unquantised so movements finer than
  // one step still add up instead of being rounded away each frame.
  const drag = useRef<{ lastY: number; raw: number } | null>(null);
  // Uncontrolled mode keeps its own value; call sites using `defaultValue` +
  // a remount key still work, and dragging can still move what is displayed.
  const controlled = value !== undefined;
  const [internal, setInternal] = useState<string>(String(defaultValue ?? ""));
  const shown = controlled ? value : internal;

  const stepNum = Number(step) || 1;
  const minNum = min === undefined ? -Infinity : Number(min);
  const maxNum = max === undefined ? Infinity : Number(max);

  /** Round to the step's precision so dragging cannot produce 0.30000000004. */
  const quantise = (v: number, effectiveStep: number) => {
    const decimals = decimalsOf(effectiveStep);
    return parseFloat((Math.round(v / effectiveStep) * effectiveStep).toFixed(decimals));
  };

  const emit = (v: number) => {
    const clamped = Math.min(maxNum, Math.max(minNum, v));
    if (!controlled) setInternal(String(clamped));
    onChange?.({ target: { value: String(clamped) } });
  };

  const nudge = (steps: number, fine: boolean) => {
    const effective = fine ? stepNum / FINE_DIVISOR : stepNum;
    const current = Number(shown);
    const base = Number.isFinite(current) ? current : 0;
    emit(quantise(base + steps * effective, effective));
  };

  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (disabled || e.button !== 0) return;
    e.preventDefault();
    const current = Number(shown);
    drag.current = { lastY: e.clientY, raw: Number.isFinite(current) ? current : 0 };
    setDragging(true);
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  };

  const onPointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d) return;
    const effective = e.shiftKey ? stepNum / FINE_DIVISOR : stepNum;
    // Up is positive: screen Y grows downwards. Only this frame's travel is
    // scaled by the current step, so pressing or releasing Shift changes what
    // happens next without disturbing what came before.
    const travelled = (d.lastY - e.clientY) / PIXELS_PER_STEP;
    d.lastY = e.clientY;
    // Clamp the accumulator too, so dragging past a bound and coming back
    // responds immediately instead of unwinding an invisible overshoot.
    d.raw = Math.min(maxNum, Math.max(minNum, d.raw + travelled * effective));
    emit(quantise(d.raw, effective));
  };

  const endDrag = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!drag.current) return;
    drag.current = null;
    setDragging(false);
    (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
    // Commit exactly like leaving the field, so callers keep one save path.
    onBlur?.({ target: { value: String(shown) } });
  };

  const active = dragging || hover;

  return (
    <div style={{ position: "relative", display: "inline-flex", alignItems: "stretch", ...style }}>
      <input
        type="text"
        inputMode="decimal"
        value={shown}
        disabled={disabled}
        title={title}
        placeholder={placeholder}
        autoFocus={autoFocus}
        onChange={(e) => { if (!controlled) setInternal(e.target.value); onChange?.(e); }}
        onBlur={onBlur}
        onKeyDown={(e) => {
          if (e.key === "ArrowUp") { e.preventDefault(); nudge(1, e.shiftKey); }
          if (e.key === "ArrowDown") { e.preventDefault(); nudge(-1, e.shiftKey); }
        }}
        style={{
          width: "100%",
          background: "transparent",
          border: "none",
          color: "inherit",
          font: "inherit",
          padding: 0,
          margin: 0,
          outline: "none",
          // Room for the grip so long values never slide under it.
          paddingRight: 14,
          minWidth: 0,
        }}
      />
      <div
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
        onMouseEnter={() => setHover(true)}
        onMouseLeave={() => setHover(false)}
        title={disabled ? undefined : "Drag up or down to change · hold Shift for fine steps"}
        style={{
          position: "absolute",
          top: 0,
          right: 0,
          bottom: 0,
          width: 14,
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          justifyContent: "center",
          gap: 2,
          cursor: disabled ? "default" : "ns-resize",
          opacity: disabled ? 0.25 : active ? 1 : 0.45,
          transition: "opacity 0.12s",
          userSelect: "none",
          touchAction: "none",
        }}
      >
        <Chevron up active={active} />
        <Chevron active={active} />
      </div>
    </div>
  );
}

function Chevron({ up, active }: { up?: boolean; active: boolean }) {
  return (
    <svg width="7" height="4" viewBox="0 0 7 4" aria-hidden="true">
      <path
        d={up ? "M0.5 3.5L3.5 0.5L6.5 3.5" : "M0.5 0.5L3.5 3.5L6.5 0.5"}
        fill="none"
        stroke={active ? "var(--wc-text)" : "var(--wc-text-secondary)"}
        strokeWidth="1.2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

/** Decimal places implied by a step, so 0.05 quantises to two places. */
function decimalsOf(step: number): number {
  const s = String(step);
  const dot = s.indexOf(".");
  return dot === -1 ? 0 : Math.min(s.length - dot - 1, 6);
}
