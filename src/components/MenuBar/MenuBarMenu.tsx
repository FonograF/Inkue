// Shared chrome for a title-bar menu: the button, the dropdown shell, and the
// item rendering (label, shortcut, checkmark, disabled, separator).
//
// File and View predate this component and still carry their own copy of the
// same markup in App.tsx; Edit and Action are built on it so a fourth and
// fifth copy never appeared. They can migrate here whenever they are touched.

import { useState } from "react";

export type MenuBarItem =
  | { type: "separator" }
  | {
      type: "item";
      label: string;
      onClick: () => void;
      shortcut?: string;
      /** Renders a ✓ gutter; omit entirely for menus that never check items. */
      checked?: boolean;
      disabled?: boolean;
    };

interface Props {
  label: string;
  title?: string;
  items: MenuBarItem[];
  /** Width of the dropdown; wider menus need more room for their shortcuts. */
  minWidth?: number;
  /**
   * Fired as the menu opens — the moment to refresh state the items depend on
   * (Undo availability, for instance) so nothing has to be polled while the
   * menu is closed.
   */
  onOpen?: () => void;
}

export function MenuBarMenu({ label, title, items, minWidth = 210, onOpen }: Props) {
  const [open, setOpen] = useState(false);
  const [hovered, setHovered] = useState<string | null>(null);

  const close = () => setOpen(false);
  const showsCheckGutter = items.some((i) => i.type === "item" && i.checked !== undefined);

  return (
    <div style={{ position: "relative", flexShrink: 0 }}>
      {open && <div style={{ position: "fixed", inset: 0, zIndex: 9990 }} onClick={close} />}
      <button
        onClick={(e) => {
          e.stopPropagation();
          setOpen((v) => {
            if (!v) onOpen?.();
            return !v;
          });
        }}
        title={title}
        style={{
          background: open ? "var(--wc-bg-surface)" : "transparent",
          border: "none", color: "var(--wc-text)", cursor: "pointer",
          fontSize: 12, padding: "3px 8px", borderRadius: 4, userSelect: "none",
        }}
      >
        {label}
      </button>
      {open && (
        <div
          style={{
            position: "absolute", left: 0, top: "100%", marginTop: 2,
            background: "var(--wc-bg-surface)", border: "1px solid var(--wc-border-strong)",
            borderRadius: 6, padding: "4px 0", minWidth,
            boxShadow: "0 8px 24px rgba(0,0,0,0.7)", zIndex: 9999,
          }}
        >
          {items.map((item, i) =>
            item.type === "separator" ? (
              <div key={`sep-${i}`} style={{ height: 1, background: "var(--wc-border-strong)", margin: "4px 0" }} />
            ) : (
              <button
                key={item.label}
                disabled={item.disabled}
                onMouseEnter={() => setHovered(item.label)}
                onMouseLeave={() => setHovered(null)}
                onClick={(e) => { e.stopPropagation(); close(); item.onClick(); }}
                style={{
                  display: "flex", alignItems: "center", gap: 8,
                  width: "100%", padding: "6px 14px",
                  background: hovered === item.label && !item.disabled ? "var(--wc-bg-hover)" : "transparent",
                  border: "none",
                  // Disabled entries stay visible on purpose: "Undo" greyed out
                  // tells the operator there is nothing to undo, which hiding
                  // it would not.
                  color: item.disabled ? "var(--wc-text-muted)" : "var(--wc-text)",
                  fontSize: 13,
                  cursor: item.disabled ? "default" : "pointer",
                  textAlign: "left",
                }}
              >
                {showsCheckGutter && (
                  <span style={{ width: 14, textAlign: "center", color: "var(--wc-text-secondary)" }}>
                    {item.checked ? "✓" : ""}
                  </span>
                )}
                <span style={{ flex: 1 }}>{item.label}</span>
                {item.shortcut && (
                  <span style={{ color: "var(--wc-text-muted)", fontSize: 11, flexShrink: 0 }}>
                    {item.shortcut}
                  </span>
                )}
              </button>
            )
          )}
        </div>
      )}
    </div>
  );
}
