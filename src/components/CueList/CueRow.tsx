// A single row in the cue list table.

import { useState } from "react";
import { PlayheadIndicator } from "./PlayheadIndicator";
import type { ColumnDef } from "./columns";
import type { CueSummary } from "../../lib/types";
import { useTimingStore } from "../../stores/timingStore";

function StopButton({ onStop }: { onStop: () => void }) {
  const [hovered, setHovered] = useState(false);
  return (
    <button
      title="Stop"
      onMouseDown={(e) => e.stopPropagation()}
      onClick={(e) => { e.stopPropagation(); onStop(); }}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      style={{
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        background: hovered ? "rgba(239,68,68,0.25)" : "rgba(239,68,68,0.12)",
        border: `1px solid ${hovered ? "#ef4444" : "rgba(239,68,68,0.45)"}`,
        borderRadius: 4,
        cursor: "pointer",
        padding: 0,
        width: 22,
        height: 22,
        flexShrink: 0,
      }}
    >
      <div
        style={{
          width: 8,
          height: 8,
          background: hovered ? "#fca5a5" : "#ef4444",
          borderRadius: 1,
        }}
      />
    </button>
  );
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const CUE_TYPE_ICONS: Record<string, string> = {
  audio: "🔊",
  memo: "📝",
  wait: "⏱",
  group: "📁",
  fade: "📉",
  stop: "⬛",
  video: "🎬",
  image: "🖼",
  osc: "📡",
  midi: "🎹",
};

const CONTINUE_LABELS: Record<string, string> = {
  do_not_continue: "—",
  auto_continue: "↓",
  auto_follow: "→",
};

const COLOR_SWATCHES: Record<string, string> = {
  none:   "transparent",
  red:    "#ef4444",
  orange: "#f97316",
  yellow: "#eab308",
  green:  "#22c55e",
  blue:   "#3b82f6",
  purple: "#a855f7",
  pink:   "#ec4899",
  white:  "#f1f5f9",
  black:  "#334155",
};

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

interface Props {
  cue: CueSummary;
  /** Zero-based position of this row in the visible cue list. */
  cueIndex: number;
  /** Pre-built grid style (display, gridTemplateColumns, gap, padding, minWidth). */
  gridStyle: React.CSSProperties;
  visibleDefs: ColumnDef[];
  isSelected: boolean;
  isAtPlayhead: boolean;
  rowHeight?: number;
  isDragOver?: boolean;
  /** True while this cue is being dragged (dims the row). */
  isDragSource?: boolean;
  /** Nesting depth — 0 = top-level, 1 = inside a group, etc. */
  depth?: number;
  /** True if this is a Group cue. */
  isGroup?: boolean;
  /** Whether the group is currently expanded. */
  isGroupExpanded?: boolean;
  /** Toggle the group's expand/collapse state. */
  onToggleExpand?: () => void;
  /** True when a cue is being dragged over the middle of this group row (drop-into-group). */
  isGroupDropTarget?: boolean;
  /** ID of the parent group, if this cue is a child. Used for within-group insert detection. */
  parentGroupId?: string | null;
  /** Called on mousedown to start a cue drag operation. */
  onCueDragStart: (e: React.MouseEvent) => void;
  onClick: (e: React.MouseEvent) => void;
  onDoubleClick: () => void;
  onContextMenu: (e: React.MouseEvent) => void;
  onStop?: (cueId: string) => void;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function CueRow({
  cue,
  cueIndex,
  gridStyle,
  visibleDefs,
  rowHeight = 26,
  isSelected,
  isAtPlayhead,
  isDragOver,
  isDragSource,
  depth = 0,
  isGroup = false,
  isGroupExpanded = false,
  onToggleExpand,
  isGroupDropTarget = false,
  parentGroupId = null,
  onCueDragStart,
  onClick,
  onDoubleClick,
  onContextMenu,
  onStop,
}: Props) {
  const timing = useTimingStore((s) => s.timings[cue.id]);

  const isRunning  = cue.state === "running";
  const isPaused   = cue.state === "paused";
  const isDisabled = cue.is_disabled ?? false;
  const isBroken   = cue.is_broken ?? false;
  const isWarning  = cue.is_warning ?? false;

  // Use file_duration_ms (single loop period) so the bar resets at each loop
  // iteration. Falls back to total duration_ms for non-looping cues.
  const loopPeriodMs = cue.file_duration_ms ?? cue.duration_ms;
  const progressPct =
    isRunning && timing && loopPeriodMs && loopPeriodMs > 0
      ? Math.min(100, ((timing.action_elapsed_ms % loopPeriodMs) / loopPeriodMs) * 100)
      : null;

  const accentColor = getComputedStyle(document.documentElement).getPropertyValue("--wc-accent").trim() || "#3b82f6";
  let bg = "transparent";
  if (isDragOver)      bg = "#1e3a5f";
  else if (isSelected) bg = accentColor;
  else if (isRunning)  bg = "#14532d";
  else if (isPaused)   bg = "#78350f";

  const colorAccent = COLOR_SWATCHES[cue.color] ?? "transparent";

  const rowStyle: React.CSSProperties = {
    ...gridStyle,
    position: "relative",
    alignItems: "center",
    paddingTop: 2,
    paddingBottom: 2,
    paddingLeft: depth > 0 ? `${8 + depth * 20}px` : undefined,
    cursor: isDragSource ? "grabbing" : "grab",
    userSelect: "none",
    background: isGroup && !isSelected ? (bg === "transparent" ? "#0d1b2a" : bg) : bg,
    borderBottom: isDragOver ? "1px solid #3b82f6" : "1px solid #1e293b",
    boxShadow: isGroupDropTarget
      ? "inset 0 0 0 2px #22d3ee"
      : isDragOver
      ? "inset 0 0 0 1px #3b82f6"
      : "none",
    fontSize: 13,
    color: isDisabled ? "#475569" : "#e2e8f0",
    minHeight: rowHeight,
    opacity: isDragSource ? 0.4 : isDisabled ? 0.55 : 1,
    transition: "opacity 0.1s",
  };

  const filename = cue.file_path
    ? cue.file_path.split(/[\\/]/).pop() ?? cue.file_path
    : "";

  const renderCell = (id: string) => {
    switch (id) {
      case "playhead":
        if (isGroup) {
          return (
            <div style={{ display: "flex", justifyContent: "flex-end", alignItems: "center", width: "100%", gap: 4 }}>
              <PlayheadIndicator visible={isAtPlayhead} />
              <button
                style={{
                  background: "none", border: "none", cursor: "pointer",
                  color: "#64748b", fontSize: 10, padding: "0 4px",
                  lineHeight: 1, display: "flex", alignItems: "center",
                  flexShrink: 0,
                }}
                onMouseDown={(e) => e.stopPropagation()}
                onClick={(e) => { e.stopPropagation(); onToggleExpand?.(); }}
              >
                {isGroupExpanded ? "▼" : "▶"}
              </button>
            </div>
          );
        }
        return (
          <div style={{ display: "flex", justifyContent: "flex-end", alignItems: "center", width: "100%" }}>
            <PlayheadIndicator visible={isAtPlayhead} />
          </div>
        );

      case "number":
        return (
          <span style={{ fontFamily: "monospace", color: "#94a3b8" }}>
            {cue.number ?? ""}
          </span>
        );

      case "name":
        return (
          <div style={{ position: "relative", overflow: "hidden", minWidth: 0 }}>
            {progressPct !== null && (
              <div
                style={{
                  position: "absolute",
                  inset: 0,
                  width: `${progressPct}%`,
                  background: "rgba(74, 222, 128, 0.28)",
                  transition: "width 0.05s linear",
                  pointerEvents: "none",
                  zIndex: 0,
                }}
              />
            )}
            <span
              style={{
                position: "relative",
                zIndex: 1,
                display: "flex",
                alignItems: "center",
                gap: 5,
                overflow: "hidden",
                paddingLeft: 5,
              }}
            >
              {isBroken && (
                <span title="Media file missing" style={{ color: "#ef4444", flexShrink: 0, fontSize: 11, fontWeight: 700 }}>!</span>
              )}
              {isWarning && !isBroken && (
                <span title={cue.warning_message ?? "Warning"} style={{ color: "#eab308", flexShrink: 0, fontSize: 11, fontWeight: 700 }}>⚠</span>
              )}
              {isDisabled && (
                <span title="Disabled" style={{ color: "#475569", flexShrink: 0, fontSize: 10 }}>off</span>
              )}
              <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", textDecoration: isDisabled ? "line-through" : "none" }}>
                {cue.name}
              </span>
            </span>
          </div>
        );

      case "target":
        return (
          <span
            style={{
              display: "block",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
              color: "#64748b",
              fontSize: 12,
              paddingLeft: 5,
            }}
          >
            {filename}
          </span>
        );

      case "type":
        return (
          <span style={{ display: "block", textAlign: "center" }}>
            {CUE_TYPE_ICONS[cue.cue_type] ?? "?"}
          </span>
        );

      case "pre_wait":
        return (
          <span style={{ display: "block", textAlign: "right", color: "#94a3b8", fontSize: 12, paddingRight: 8 }}>
            {cue.pre_wait_ms ? `${(cue.pre_wait_ms / 1000).toFixed(1)}s` : ""}
          </span>
        );

      case "duration":
        return (
          <span style={{ display: "block", textAlign: "right", color: cue.is_loading ? "#f59e0b" : "#94a3b8", fontSize: 12, paddingRight: 8 }}>
            {cue.is_loading
              ? "Loading…"
              : cue.duration_ms != null
                ? `${(cue.duration_ms / 1000).toFixed(1)}s`
                : ""}
          </span>
        );

      case "post_wait":
        return (
          <span style={{ display: "block", textAlign: "right", color: "#94a3b8", fontSize: 12, paddingRight: 8 }}>
            {cue.post_wait_ms ? `${(cue.post_wait_ms / 1000).toFixed(1)}s` : ""}
          </span>
        );

      case "continue":
        return (
          <span style={{ display: "block", textAlign: "center", color: "#64748b" }}>
            {CONTINUE_LABELS[cue.continue_mode] ?? ""}
          </span>
        );

      case "notes":
        return (
          <span
            title={cue.notes || undefined}
            style={{
              display: "block",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
              color: "#64748b",
              fontSize: 12,
              fontStyle: cue.notes ? "normal" : "italic",
              paddingLeft: 5,
            }}
          >
            {cue.notes || ""}
          </span>
        );

      case "stop_btn":
        return (
          <div style={{ display: "flex", alignItems: "center", justifyContent: "center", height: "100%" }}>
            {(isRunning || isPaused) && (
              <StopButton onStop={() => onStop?.(cue.id)} />
            )}
          </div>
        );

      default:
        return null;
    }
  };

  return (
    <div
      style={rowStyle}
      data-cue-id={cue.id}
      data-cue-index={cueIndex}
      data-is-group={isGroup ? "true" : undefined}
      data-cue-depth={depth}
      data-parent-group-id={parentGroupId ?? undefined}
      onMouseDown={onCueDragStart}
      onClick={onClick}
      onDoubleClick={onDoubleClick}
      onContextMenu={onContextMenu}
    >
      {/* Color indicator strip — shifts right with nesting depth (4 px per level).
          z-index 0 keeps it below column content (playhead indicator, etc.). */}
      <div
        style={{
          position: "absolute",
          left: depth * 4,
          top: 0,
          bottom: 0,
          width: 4,
          background: colorAccent,
          pointerEvents: "none",
          zIndex: 0,
        }}
      />
      {visibleDefs.map((col) => (
        <div key={col.id} style={{ minWidth: 0, overflow: "hidden", position: "relative", zIndex: 1 }}>
          {renderCell(col.id)}
        </div>
      ))}
    </div>
  );
}
