// Small modal frame shared by the cue-list prompts (Renumber, batch values):
// title, subtitle, body, Cancel / confirm buttons, Enter / Escape keys.

import type { ReactNode } from "react";

interface Props {
  title: string;
  subtitle?: ReactNode;
  confirmLabel: string;
  canConfirm: boolean;
  onCancel: () => void;
  onConfirm: () => void;
  children: ReactNode;
}

export function DialogShell({ title, subtitle, confirmLabel, canConfirm, onCancel, onConfirm, children }: Props) {
  const submit = () => { if (canConfirm) onConfirm(); };

  return (
    <div
      style={{
        position: "fixed", inset: 0, zIndex: 10000,
        background: "rgba(0,0,0,0.5)", display: "flex", alignItems: "center", justifyContent: "center",
      }}
      onClick={onCancel}
      onContextMenu={(e) => e.preventDefault()}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key === "Enter") submit();
          if (e.key === "Escape") onCancel();
        }}
        style={{
          background: "var(--wc-bg-surface)", border: "1px solid var(--wc-border-strong)",
          borderRadius: 8, padding: 20, minWidth: 320,
          boxShadow: "0 16px 48px rgba(0,0,0,0.6)",
        }}
      >
        <div style={{ fontSize: 14, fontWeight: 600, color: "var(--wc-text-bright)", marginBottom: 4 }}>
          {title}
        </div>
        {subtitle && (
          <div style={{ fontSize: 12, color: "var(--wc-text-muted)", marginBottom: 16 }}>{subtitle}</div>
        )}

        {children}

        <div style={{ display: "flex", justifyContent: "flex-end", gap: 8 }}>
          <button onClick={onCancel} style={buttonStyle}>Cancel</button>
          <button
            onClick={submit}
            disabled={!canConfirm}
            style={{
              ...buttonStyle,
              background: canConfirm ? "var(--wc-accent)" : "var(--wc-bg-hover)",
              color: canConfirm ? "var(--wc-accent-fg)" : "var(--wc-text-muted)",
              cursor: canConfirm ? "pointer" : "default",
            }}
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}

export function DialogRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 12, marginBottom: 10 }}>
      <span style={{ fontSize: 12, color: "var(--wc-text-secondary)", width: 80 }}>{label}</span>
      {children}
    </div>
  );
}

export const dialogInputStyle: React.CSSProperties = {
  flex: 1, background: "var(--wc-bg-input)", border: "1px solid var(--wc-border)",
  borderRadius: 4, color: "var(--wc-text)", fontSize: 13, padding: "5px 8px",
};

const buttonStyle: React.CSSProperties = {
  background: "var(--wc-bg-hover)", border: "1px solid var(--wc-border-strong)",
  borderRadius: 5, color: "var(--wc-text)", fontSize: 12, padding: "5px 14px", cursor: "pointer",
};
