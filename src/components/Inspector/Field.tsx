// Shared label/field wrapper and input style used across all Inspector tabs.

export const inputStyle: React.CSSProperties = {
  background: "var(--wc-bg-surface)",
  border: "1px solid var(--wc-border-strong)",
  borderRadius: 4,
  color: "var(--wc-text)",
  padding: "3px 8px",
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
      <label style={{ width: 100, color: "var(--wc-text-secondary)", flexShrink: 0 }}>
        {label}
      </label>
      <div style={{ flex: 1 }}>{children}</div>
    </div>
  );
}
