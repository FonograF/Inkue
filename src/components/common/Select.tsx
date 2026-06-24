// Custom dropdown that replaces the native <select>.
//
// WebKitGTK (Linux) renders the native <select> popup as a GTK widget that
// ignores the page's CSS background — so a dark theme with light text turns
// invisible (white-on-white). Building it ourselves keeps the exact same
// look on Windows/macOS/Linux. Same outside-click pattern as CurveSelect.

import { useState, Children, isValidElement, type ReactNode, type CSSProperties, type ReactElement } from "react";

interface OptionProps {
  value: string | number;
  disabled?: boolean;
  children?: ReactNode;
}

interface SelectProps {
  value: string | number;
  onChange: (e: { target: { value: string } }) => void;
  style?: CSSProperties;
  disabled?: boolean;
  children: ReactNode; // <option> elements
}

export function Select({ value, onChange, style, disabled, children }: SelectProps) {
  const [open, setOpen] = useState(false);

  const options = Children.toArray(children)
    .filter((c): c is ReactElement<OptionProps> => isValidElement(c))
    .map((el) => ({
      value: String(el.props.value),
      label: el.props.children,
      disabled: el.props.disabled,
    }));

  const current = options.find((o) => o.value === String(value));

  return (
    <div style={{ position: "relative", width: style?.width ?? "100%" }}>
      {open && (
        <div
          style={{ position: "fixed", inset: 0, zIndex: 100 }}
          onClick={() => setOpen(false)}
        />
      )}

      <button
        type="button"
        onClick={() => !disabled && setOpen((o) => !o)}
        disabled={disabled}
        style={{
          ...style,
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          gap: 6,
          width: "100%",
          boxSizing: "border-box",
          textAlign: "left",
          cursor: disabled ? "default" : "pointer",
          opacity: disabled ? (style?.opacity ?? 0.4) : (style?.opacity ?? 1),
        }}
      >
        <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          {current?.label ?? String(value)}
        </span>
        <span style={{ color: "var(--wc-text-muted)", fontSize: 10, flexShrink: 0 }}>▾</span>
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
            overflow: "auto",
            maxHeight: 240,
            boxShadow: "0 8px 24px rgba(0,0,0,0.6)",
          }}
        >
          {options.map((o) => (
            <button
              type="button"
              key={o.value}
              disabled={o.disabled}
              onClick={() => {
                if (o.disabled) return;
                onChange({ target: { value: o.value } });
                setOpen(false);
              }}
              style={{
                display: "block",
                width: "100%",
                padding: "5px 8px",
                fontSize: 12,
                textAlign: "left",
                border: "none",
                background: o.value === String(value) ? "var(--wc-bg-hover)" : "transparent",
                color: o.disabled ? "var(--wc-text-muted)" : "var(--wc-text)",
                cursor: o.disabled ? "default" : "pointer",
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              {o.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
