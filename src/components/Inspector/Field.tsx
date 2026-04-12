// Shared label/field wrapper and input style used across all Inspector tabs.

export const inputStyle: React.CSSProperties = {
  background: "#1e293b",
  border: "1px solid #334155",
  borderRadius: 4,
  color: "#e2e8f0",
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
      <label style={{ width: 100, color: "#94a3b8", flexShrink: 0 }}>
        {label}
      </label>
      <div style={{ flex: 1 }}>{children}</div>
    </div>
  );
}
