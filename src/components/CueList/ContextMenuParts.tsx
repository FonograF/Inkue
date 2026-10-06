// Building blocks of the cue-list context menu: items, hover flyouts,
// separators and a header line.

import { useLayoutEffect, useRef, useState, type ReactNode } from "react";

interface ItemProps {
  label: string;
  onClick: () => void;
  danger?: boolean;
  /** Swatch shown before the label (cue type, cue color). */
  color?: string;
  /** Check mark: the whole selection already has this value. */
  checked?: boolean;
  disabled?: boolean;
  /** Muted text on the right (a count, a shortcut). */
  hint?: string;
}

export function CtxItem({ label, onClick, danger, color, checked, disabled, hint }: ItemProps) {
  const [hovered, setHovered] = useState(false);
  const textColor = disabled ? "var(--wc-text-faint)" : danger ? "#ef4444" : "var(--wc-text)";
  return (
    <button
      disabled={disabled}
      style={{
        display: "flex", alignItems: "center", gap: 8, width: "100%", padding: "6px 16px 6px 10px",
        background: hovered && !disabled ? "var(--wc-bg-hover)" : "transparent", border: "none",
        textAlign: "left", color: textColor,
        fontSize: 13, cursor: disabled ? "default" : "pointer", whiteSpace: "nowrap",
      }}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onClick={onClick}
    >
      <span style={{ width: 12, flexShrink: 0, color: "var(--wc-accent)", fontSize: 11 }}>{checked ? "✓" : ""}</span>
      {color && <Swatch color={color} />}
      <span style={{ flex: 1 }}>{label}</span>
      {hint && <span style={{ color: "var(--wc-text-muted)", fontSize: 11 }}>{hint}</span>}
    </button>
  );
}

interface SubmenuProps {
  label: string;
  openLeft: boolean;
  children: ReactNode;
  /** Muted text before the arrow — e.g. "2/5" when only part of the selection is concerned. */
  hint?: string;
}

// A row that reveals a flyout of child items on hover. The flyout opens to the
// right by default, or to the left when the menu sits near the right edge of
// the window (`openLeft`) or when it would not fit on the right.
//
// The flyout is `position: fixed`, placed from the row's on-screen rect: a long
// flyout scrolls (overflow-y: auto), and CSS then clips overflow-x too — an
// absolutely positioned nested flyout would be trapped inside its parent's
// scroll box instead of opening beside it.
export function CtxSubmenu({ label, openLeft, children, hint }: SubmenuProps) {
  const [open, setOpen] = useState(false);
  const rowRef = useRef<HTMLDivElement>(null);
  const flyoutRef = useRef<HTMLDivElement>(null);
  const [placement, setPlacement] = useState<{ left: number; top: number } | null>(null);

  useLayoutEffect(() => {
    if (!open) {
      setPlacement(null);
      return;
    }
    const row = rowRef.current?.getBoundingClientRect();
    const flyout = flyoutRef.current?.getBoundingClientRect();
    if (!row || !flyout) return;
    setPlacement(placeFlyout(row, flyout, openLeft));
  }, [open, openLeft]);

  return (
    <div ref={rowRef} style={{ position: "relative" }} onMouseEnter={() => setOpen(true)} onMouseLeave={() => setOpen(false)}>
      <button
        style={{
          display: "flex", alignItems: "center", gap: 8,
          width: "100%", padding: "6px 16px 6px 10px",
          background: open ? "var(--wc-bg-hover)" : "transparent", border: "none",
          textAlign: "left", color: "var(--wc-text)", fontSize: 13, cursor: "default", whiteSpace: "nowrap",
        }}
      >
        <span style={{ width: 12, flexShrink: 0 }} />
        <span style={{ flex: 1 }}>{label}</span>
        {hint && <span style={{ color: "var(--wc-text-muted)", fontSize: 11 }}>{hint}</span>}
        <span style={{ color: "var(--wc-text-muted)" }}>{openLeft ? "‹" : "›"}</span>
      </button>
      {open && (
        <div
          ref={flyoutRef}
          style={{
            position: "fixed",
            left: placement?.left ?? 0,
            top: placement?.top ?? 0,
            visibility: placement ? "visible" : "hidden",
            zIndex: 10000,
            ...menuPanelStyle, minWidth: 180, maxHeight: FLYOUT_MAX_HEIGHT, overflowY: "auto",
          }}
        >
          {children}
        </div>
      )}
    </div>
  );
}

const FLYOUT_MAX_HEIGHT = 420;
const VIEWPORT_MARGIN = 4;
/** Lines the flyout's first item up with its row (the panel's top padding). */
const FLYOUT_TOP_OFFSET = 5;

interface Rect { left: number; right: number; top: number; width: number; height: number }

/** Where a flyout goes beside its row, kept inside the window. */
export function placeFlyout(
  row: Rect,
  flyout: Pick<Rect, "width" | "height">,
  preferLeft: boolean,
  viewport = { width: window.innerWidth, height: window.innerHeight },
): { left: number; top: number } {
  const fitsRight = row.right + flyout.width <= viewport.width - VIEWPORT_MARGIN;
  const fitsLeft = row.left - flyout.width >= VIEWPORT_MARGIN;
  const goLeft = fitsLeft && (preferLeft || !fitsRight);
  const left = goLeft
    ? row.left - flyout.width
    : Math.max(VIEWPORT_MARGIN, Math.min(row.right, viewport.width - flyout.width - VIEWPORT_MARGIN));
  const top = Math.max(
    VIEWPORT_MARGIN,
    Math.min(row.top - FLYOUT_TOP_OFFSET, viewport.height - flyout.height - VIEWPORT_MARGIN),
  );
  return { left, top };
}

export function CtxSeparator() {
  return <div style={{ height: 1, background: "var(--wc-border-strong)", margin: "4px 0" }} />;
}

export function CtxHeader({ children }: { children: ReactNode }) {
  return (
    <div
      style={{
        padding: "4px 16px 6px 30px", fontSize: 11, color: "var(--wc-text-muted)",
        textTransform: "uppercase", letterSpacing: "0.05em", whiteSpace: "nowrap",
      }}
    >
      {children}
    </div>
  );
}

function Swatch({ color }: { color: string }) {
  const empty = color === "transparent";
  return (
    <span
      style={{
        width: 10, height: 10, borderRadius: 2, flexShrink: 0,
        background: empty ? "transparent" : color,
        border: empty ? "1px solid var(--wc-text-faint)" : "none",
      }}
    />
  );
}

export const menuPanelStyle: React.CSSProperties = {
  background: "var(--wc-bg-surface)",
  border: "1px solid var(--wc-border-strong)",
  borderRadius: 6,
  padding: "4px 0",
  boxShadow: "0 4px 16px rgba(0,0,0,0.6)",
};
