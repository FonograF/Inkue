// Panel showing all currently running or paused cues, always visible while active.

import { useWorkspaceStore } from "../../stores/workspaceStore";
import { useTimingStore } from "../../stores/timingStore";
import type { CueSummary } from "../../lib/types";
import { stopCue } from "../../lib/commands";

const CUE_TYPE_ICONS: Record<string, string> = {
  audio: "🔊", memo: "📝", wait: "⏱", group: "📁",
  fade: "📉", stop: "⬛", video: "🎬", image: "🖼",
  osc: "📡", midi: "🎹", light: "💡", mic: "🎤", timecode: "🕐", text: "🔤",
};

const COLOR_SWATCHES: Record<string, string> = {
  none: "transparent", red: "#ef4444", orange: "#f97316", yellow: "#eab308",
  green: "#22c55e", cyan: "#06b6d4", blue: "#3b82f6",
  purple: "#a855f7", pink: "#ec4899", white: "#f1f5f9", black: "#334155",
};

function flattenActive(cues: CueSummary[]): CueSummary[] {
  const result: CueSummary[] = [];
  for (const cue of cues) {
    if (cue.state === "running" || cue.state === "paused") result.push(cue);
    if (cue.children?.length) result.push(...flattenActive(cue.children));
  }
  return result;
}

function formatTime(ms: number): string {
  const s = Math.floor(ms / 1000);
  const m = Math.floor(s / 60);
  return `${m}:${String(s % 60).padStart(2, "0")}`;
}

function ActiveCueRow({ cue }: { cue: CueSummary }) {
  const timing = useTimingStore((s) => s.timings[cue.id]);
  const isPaused = cue.state === "paused";
  const colorAccent = COLOR_SWATCHES[cue.color] ?? "transparent";

  const loopPeriod = cue.file_duration_ms ?? cue.duration_ms;
  const elapsed = timing?.action_elapsed_ms ?? 0;
  const remaining = timing?.remaining_ms ?? (cue.duration_ms != null ? Math.max(0, cue.duration_ms - elapsed) : null);
  const progressPct = loopPeriod && loopPeriod > 0
    ? Math.min(100, ((elapsed % loopPeriod) / loopPeriod) * 100)
    : null;

  return (
    <div
      style={{
        position: "relative",
        display: "flex",
        alignItems: "center",
        height: 26,
        gap: 6,
        padding: "0 8px 0 12px",
        borderBottom: "1px solid var(--wc-border)",
        background: isPaused ? "var(--wc-bg-paused)" : "var(--wc-bg-running)",
        fontSize: 12,
        flexShrink: 0,
      }}
    >
      {colorAccent !== "transparent" && (
        <div style={{
          position: "absolute", left: 0, top: 0, bottom: 0, width: 4,
          background: colorAccent, pointerEvents: "none",
        }} />
      )}
      {progressPct !== null && (
        <div style={{
          position: "absolute", bottom: 0, left: 0,
          height: 2, width: `${progressPct}%`,
          background: isPaused ? "#fb923c" : "#22c55e",
          transition: "width 0.05s linear",
          pointerEvents: "none",
        }} />
      )}

      <span style={{ flexShrink: 0, fontSize: 11 }}>
        {CUE_TYPE_ICONS[cue.cue_type] ?? "?"}
      </span>

      {cue.number && (
        <span style={{ flexShrink: 0, color: "var(--wc-text-secondary)", fontFamily: "monospace", fontSize: 11, minWidth: 28, textAlign: "right" }}>
          {cue.number}
        </span>
      )}

      <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", color: "var(--wc-text)" }}>
        {cue.name || "(untitled)"}
      </span>

      <span style={{
        flexShrink: 0, fontSize: 10, fontWeight: 700,
        color: isPaused ? "#fb923c" : "#4ade80",
        letterSpacing: "0.04em",
      }}>
        {isPaused ? "PAUSED" : "RUNNING"}
      </span>

      <span style={{ flexShrink: 0, fontFamily: "monospace", fontSize: 11, color: "var(--wc-text-secondary)", minWidth: 36, textAlign: "right" }}>
        {remaining !== null
          ? formatTime(remaining)
          : elapsed > 0
          ? `+${formatTime(elapsed)}`
          : ""}
      </span>

      <button
        onClick={() => stopCue(cue.id).catch(console.error)}
        title="Stop"
        style={{
          flexShrink: 0,
          display: "flex", alignItems: "center", justifyContent: "center",
          width: 18, height: 18,
          background: "rgba(239,68,68,0.15)",
          border: "1px solid rgba(239,68,68,0.4)",
          borderRadius: 3,
          cursor: "pointer", padding: 0,
        }}
      >
        <div style={{ width: 6, height: 6, background: "#ef4444", borderRadius: 1 }} />
      </button>
    </div>
  );
}

export function ActiveCuesView() {
  const cues = useWorkspaceStore((s) => s.cues);
  const activeCues = flattenActive(cues);

  if (activeCues.length === 0) return null;

  return (
    <div style={{
      flexShrink: 0,
      maxHeight: 180,
      overflowY: "auto",
      background: "var(--wc-bg-surface)",
      borderBottom: "1px solid var(--wc-border-strong)",
    }}>
      <div style={{
        display: "flex", alignItems: "center", gap: 6,
        padding: "3px 10px",
        borderBottom: "1px solid var(--wc-border)",
        position: "sticky", top: 0, zIndex: 1,
        background: "var(--wc-bg-surface)",
        flexShrink: 0,
      }}>
        <span style={{ fontSize: 10, fontWeight: 700, letterSpacing: "0.08em", color: "var(--wc-text-muted)", textTransform: "uppercase" }}>
          Active
        </span>
        <span style={{
          fontSize: 9, fontWeight: 700,
          background: "#22c55e", color: "#052e16",
          borderRadius: 8, padding: "0 5px", lineHeight: "14px",
        }}>
          {activeCues.length}
        </span>
      </div>
      {activeCues.map((cue) => (
        <ActiveCueRow key={cue.id} cue={cue} />
      ))}
    </div>
  );
}
