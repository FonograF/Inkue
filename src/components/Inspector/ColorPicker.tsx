import type { CueColor } from "../../lib/types";

const COLOR_OPTIONS: { value: CueColor; hex: string; label: string }[] = [
  { value: "none",   hex: "transparent", label: "None"   },
  { value: "red",    hex: "#ef4444",     label: "Red"    },
  { value: "orange", hex: "#f97316",     label: "Orange" },
  { value: "yellow", hex: "#eab308",     label: "Yellow" },
  { value: "green",  hex: "#22c55e",     label: "Green"  },
  { value: "blue",   hex: "#3b82f6",     label: "Blue"   },
  { value: "purple", hex: "#a855f7",     label: "Purple" },
  { value: "pink",   hex: "#ec4899",     label: "Pink"   },
  { value: "white",  hex: "#f1f5f9",     label: "White"  },
  { value: "black",  hex: "#334155",     label: "Black"  },
];

export function ColorPicker({
  value,
  onChange,
}: {
  value: CueColor;
  onChange: (c: CueColor) => void;
}) {
  return (
    <div style={{ display: "flex", gap: 5, flexWrap: "wrap" }}>
      {COLOR_OPTIONS.map(({ value: v, hex, label }) => (
        <button
          key={v}
          title={label}
          onClick={() => onChange(v)}
          style={{
            width: 20,
            height: 20,
            borderRadius: 4,
            border: v === value ? "2px solid #f1f5f9" : "2px solid #475569",
            background: hex === "transparent" ? "#1e293b" : hex,
            cursor: "pointer",
            padding: 0,
            flexShrink: 0,
            outline: v === value ? "1px solid #94a3b8" : "none",
            outlineOffset: 1,
          }}
        />
      ))}
    </div>
  );
}
