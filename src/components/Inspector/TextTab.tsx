import { useState, useEffect } from "react";
import type { TextCueData, TextPosition } from "../../lib/types";
import { listSystemFonts } from "../../lib/commands";
import { Field, inputStyle } from "./Field";

const POSITION_GRID: { value: TextPosition; label: string }[] = [
  { value: "top_left",      label: "↖" },
  { value: "top_center",    label: "↑" },
  { value: "top_right",     label: "↗" },
  { value: "middle_left",   label: "←" },
  { value: "center",        label: "●" },
  { value: "middle_right",  label: "→" },
  { value: "bottom_left",   label: "↙" },
  { value: "bottom_center", label: "↓" },
  { value: "bottom_right",  label: "↘" },
];

export function TextTab({
  cue,
  onSave,
}: {
  cue: TextCueData;
  onSave: (p: Partial<TextCueData>) => Promise<void>;
}) {
  const [fonts, setFonts] = useState<string[]>([]);

  useEffect(() => {
    listSystemFonts().then(setFonts).catch(console.error);
  }, []);

  return (
    <div style={{ display: "flex", flexDirection: "column" }}>
      <Field label="Text">
        <textarea
          value={cue.text}
          onChange={(e) => void onSave({ text: e.target.value })}
          rows={5}
          style={{
            ...inputStyle,
            resize: "vertical",
            fontFamily: "inherit",
            lineHeight: 1.5,
            padding: "6px 8px",
          }}
          placeholder="Enter text to display…"
        />
      </Field>

      <Field label="Font">
        <select
          value={cue.font}
          onChange={(e) => void onSave({ font: e.target.value })}
          style={inputStyle}
        >
          {(fonts.length > 0 ? fonts : [cue.font]).map((f) => (
            <option key={f} value={f}>{f}</option>
          ))}
        </select>
      </Field>

      <Field label="Size">
        <input
          type="number"
          min={8}
          max={500}
          value={cue.font_size}
          onChange={(e) => void onSave({ font_size: Math.max(8, Number(e.target.value) || 60) })}
          style={inputStyle}
        />
      </Field>

      <Field label="Color">
        <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
          <input
            type="color"
            value={cue.text_color}
            onChange={(e) => void onSave({ text_color: e.target.value })}
            style={{
              width: 36, height: 28, padding: 2,
              border: "1px solid var(--wc-border-strong)",
              borderRadius: 4, cursor: "pointer", background: "transparent",
              flexShrink: 0,
            }}
          />
          <input
            type="text"
            value={cue.text_color}
            maxLength={7}
            onChange={(e) => {
              const v = e.target.value;
              if (/^#[0-9A-Fa-f]{6}$/.test(v)) void onSave({ text_color: v });
            }}
            style={{ ...inputStyle, fontFamily: "monospace" }}
            placeholder="#FFFFFF"
          />
        </div>
      </Field>

      <Field label="Position">
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "1fr 1fr 1fr",
            gap: 3,
            width: 120,
          }}
        >
          {POSITION_GRID.map((p) => (
            <button
              key={p.value}
              title={p.value.replace(/_/g, " ")}
              onClick={() => void onSave({ position: p.value })}
              style={{
                padding: "6px 0",
                fontSize: 16,
                cursor: "pointer",
                background:
                  cue.position === p.value
                    ? "var(--wc-accent)"
                    : "var(--wc-bg-hover)",
                color:
                  cue.position === p.value
                    ? "var(--wc-accent-fg)"
                    : "var(--wc-text-secondary)",
                border: "1px solid var(--wc-border-strong)",
                borderRadius: 4,
                lineHeight: 1,
              }}
            >
              {p.label}
            </button>
          ))}
        </div>
      </Field>

      <Field label="Duration">
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <input
            type="checkbox"
            id="text-auto-complete"
            checked={cue.display_duration_ms !== null}
            onChange={(e) =>
              void onSave({ display_duration_ms: e.target.checked ? 5000 : null })
            }
          />
          <label
            htmlFor="text-auto-complete"
            style={{ fontSize: 12, color: "var(--wc-text-secondary)", cursor: "pointer" }}
          >
            Auto-complete after
          </label>
          {cue.display_duration_ms !== null && (
            <>
              <input
                type="number"
                min={100}
                step={100}
                value={cue.display_duration_ms ?? 5000}
                onChange={(e) =>
                  void onSave({
                    display_duration_ms: Math.max(100, Number(e.target.value) || 5000),
                  })
                }
                style={{ ...inputStyle, width: 80 }}
              />
              <span style={{ fontSize: 12, color: "var(--wc-text-faint)" }}>ms</span>
            </>
          )}
        </div>
      </Field>
    </div>
  );
}
