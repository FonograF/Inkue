// A single row in the cue list table.

import { useState, useRef } from "react";
import { PlayheadIndicator } from "./PlayheadIndicator";
import type { ColumnDef } from "./columns";
import type { CueColorStyle, CueSummary } from "../../lib/types";
import { useTimingStore } from "../../stores/timingStore";
import { updateCue } from "../../lib/commands";

function parseSeconds(s: string): number | null {
  const colonMatch = /^(\d+):(\d{1,2})(?:\.(\d+))?$/.exec(s.trim());
  if (colonMatch) {
    const mins = parseInt(colonMatch[1], 10);
    const secs = parseInt(colonMatch[2], 10);
    const frac = colonMatch[3] ? parseFloat("0." + colonMatch[3]) : 0;
    return Math.round((mins * 60 + secs + frac) * 1000);
  }
  const n = parseFloat(s.trim());
  return isNaN(n) || n < 0 ? null : Math.round(n * 1000);
}

const INLINE_INPUT_STYLE: React.CSSProperties = {
  width: "100%",
  background: "var(--wc-bg-app)",
  border: "1px solid var(--wc-accent)",
  borderRadius: 3,
  color: "var(--wc-text)",
  fontSize: 12,
  textAlign: "right",
  padding: "0 4px",
  outline: "none",
  boxSizing: "border-box",
  height: "100%",
};

function RunningLed() {
  const delayRef = useRef<string | null>(null);
  if (!delayRef.current) {
    const phase = (Date.now() % 1800) / 1000;
    delayRef.current = `-${phase.toFixed(3)}s`;
  }
  return (
    <span
      style={{
        display: "inline-block",
        width: 8,
        height: 8,
        borderRadius: "50%",
        background: "#22c55e",
        flexShrink: 0,
        animation: `wc-led-pulse 1.8s ease-in-out ${delayRef.current} infinite`,
      }}
    />
  );
}

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
  light: "💡",
  mic: "🎤",
  timecode: "🕐",
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
  cyan:   "#06b6d4",
  blue:   "#3b82f6",
  purple: "#a855f7",
  pink:   "#ec4899",
  white:  "#f1f5f9",
  black:  "#334155",
};

/** `#rrggbb` -> `rgba(r, g, b, alpha)`, used to tint the whole row without
 *  drowning out the text in "full row" cue colour style. */
function hexToRgba(hex: string, alpha: number): string {
  const m = /^#([0-9a-f]{6})$/i.exec(hex);
  if (!m) return hex;
  const n = parseInt(m[1], 16);
  return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, ${alpha})`;
}

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
  onRefresh?: () => void;
  /** How the cue's colour tag is rendered — left-edge stripe, or the whole row tinted. */
  cueColorStyle?: CueColorStyle;
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
  onRefresh,
  cueColorStyle = "stripe",
}: Props) {
  const timing = useTimingStore((s) => s.timings[cue.id]);

  const [editingCell, setEditingCell] = useState<"pre_wait_ms" | "post_wait_ms" | "duration_ms" | "notes" | null>(null);
  const [editingValue, setEditingValue] = useState("");

  async function commitInlineEdit() {
    if (!editingCell) return;
    let field: Record<string, unknown>;
    if (editingCell === "notes") {
      field = { notes: editingValue };
    } else {
      const ms = parseSeconds(editingValue);
      if (ms === null) { setEditingCell(null); return; }
      if (editingCell === "pre_wait_ms") field = { pre_wait_ms: ms };
      else if (editingCell === "post_wait_ms") field = { post_wait_ms: ms };
      else {
        if (cue.cue_type === "wait") field = { wait_duration_ms: ms };
        else if (cue.cue_type === "fade") field = { fade_duration_ms: ms };
        else { setEditingCell(null); return; }
      }
    }
    await updateCue(cue.id, field).catch(console.error);
    onRefresh?.();
    setEditingCell(null);
  }

  function startEditMs(cell: "pre_wait_ms" | "post_wait_ms" | "duration_ms", currentMs: number) {
    setEditingCell(cell);
    setEditingValue((currentMs / 1000).toFixed(1));
  }

  function startEditNotes() {
    setEditingCell("notes");
    setEditingValue(cue.notes ?? "");
  }

  function inlineInput(align: "left" | "right" = "right") {
    return (
      <input
        autoFocus
        value={editingValue}
        onChange={(e) => setEditingValue(e.target.value)}
        onFocus={(e) => e.target.select()}
        onBlur={() => void commitInlineEdit()}
        onKeyDown={(e) => {
          if (e.key === "Enter") { e.preventDefault(); void commitInlineEdit(); }
          if (e.key === "Escape") { e.preventDefault(); setEditingCell(null); }
          e.stopPropagation();
        }}
        onClick={(e) => e.stopPropagation()}
        onMouseDown={(e) => e.stopPropagation()}
        style={{ ...INLINE_INPUT_STYLE, textAlign: align }}
      />
    );
  }

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

  const colorAccent = COLOR_SWATCHES[cue.color] ?? "transparent";
  const fullRowTint = cueColorStyle === "full_row" && colorAccent !== "transparent"
    ? hexToRgba(colorAccent, 0.28)
    : null;

  let bg = fullRowTint ?? "transparent";
  if (isDragOver)      bg = "var(--wc-bg-drag-over)";
  else if (isSelected) bg = "var(--wc-accent-dim)";
  else if (isRunning)  bg = "var(--wc-bg-running)";
  else if (isPaused)   bg = "var(--wc-bg-paused)";

  // Solid background for sticky-right cells (must be opaque to cover scrolled content).
  const stickyBg = isRunning  ? "var(--wc-bg-running)"
    : isPaused ? "var(--wc-bg-paused)"
    : isGroup  ? "var(--wc-bg-group)"
    : "var(--wc-bg-app)";

  const rowStyle: React.CSSProperties = {
    ...gridStyle,
    position: "relative",
    alignItems: "center",
    paddingTop: 2,
    paddingBottom: 2,
    paddingLeft: depth > 0 ? `${8 + depth * 20}px` : undefined,
    cursor: isDragSource ? "none" : "grab",
    userSelect: "none",
    background: isDragSource ? "transparent"
      : isGroup && !isSelected ? (bg === "transparent" ? "var(--wc-bg-group)" : bg) : bg,
    borderBottom: isDragSource ? "1px dashed var(--wc-border)"
      : isDragOver ? "1px solid var(--wc-accent)" : "1px solid var(--wc-border)",
    boxShadow: isGroupDropTarget
      ? "inset 0 0 0 2px #22d3ee"
      : isDragOver
      ? "inset 0 0 0 1px var(--wc-accent)"
      : "none",
    fontSize: 13,
    color: isDisabled ? "var(--wc-text-faint)" : "var(--wc-text)",
    minHeight: rowHeight,
    opacity: isDragSource ? 0.15 : isDisabled ? 0.55 : 1,
    transition: "opacity 0.15s",
  };

  const filename = cue.file_path
    ? cue.file_path.split(/[\\/]/).pop() ?? cue.file_path
    : "";

  const renderCell = (id: string) => {
    switch (id) {
      case "playhead":
        if (isGroup) {
          return (
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", width: "100%", paddingLeft: 6 }}>
              <PlayheadIndicator visible={isAtPlayhead} />
              <button
                style={{
                  background: "none", border: "none", cursor: "pointer",
                  color: "var(--wc-text-muted)", fontSize: 10, padding: "0 4px",
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
          <div style={{ display: "flex", justifyContent: "flex-start", alignItems: "center", width: "100%", paddingLeft: 6 }}>
            <PlayheadIndicator visible={isAtPlayhead} />
          </div>
        );

      case "led":
        return (
          <div style={{ display: "flex", alignItems: "center", justifyContent: "center", height: "100%" }}>
            {isRunning && <RunningLed />}
          </div>
        );

      case "number":
        return (
          <span style={{ fontFamily: "monospace", color: "var(--wc-text-secondary)" }}>
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
                <span title="Disabled" style={{ color: "var(--wc-text-faint)", flexShrink: 0, fontSize: 10 }}>off</span>
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
              color: "var(--wc-text-muted)",
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
        if (editingCell === "pre_wait_ms") return inlineInput();
        return (
          <div
            onDoubleClick={(e) => { e.stopPropagation(); startEditMs("pre_wait_ms", cue.pre_wait_ms ?? 0); }}
            style={{ display: "flex", alignItems: "center", justifyContent: "flex-end", height: "100%", paddingRight: 8, cursor: "text", color: "var(--wc-text-secondary)", fontSize: 12 }}
          >
            {cue.pre_wait_ms ? `${(cue.pre_wait_ms / 1000).toFixed(1)}s` : ""}
          </div>
        );

      case "duration": {
        const canEdit = cue.cue_type === "wait" || cue.cue_type === "fade";
        if (editingCell === "duration_ms") return inlineInput();
        return (
          <div
            onDoubleClick={canEdit ? (e) => { e.stopPropagation(); startEditMs("duration_ms", cue.duration_ms ?? 0); } : undefined}
            style={{ display: "flex", alignItems: "center", justifyContent: "flex-end", height: "100%", paddingRight: 8, cursor: canEdit ? "text" : "default", color: cue.is_loading ? "#f59e0b" : "var(--wc-text-secondary)", fontSize: 12 }}
          >
            {cue.is_loading
              ? "Loading…"
              : cue.duration_ms != null
                ? `${(cue.duration_ms / 1000).toFixed(1)}s`
                : ""}
          </div>
        );
      }

      case "post_wait":
        if (editingCell === "post_wait_ms") return inlineInput();
        return (
          <div
            onDoubleClick={(e) => { e.stopPropagation(); startEditMs("post_wait_ms", cue.post_wait_ms ?? 0); }}
            style={{ display: "flex", alignItems: "center", justifyContent: "flex-end", height: "100%", paddingRight: 8, cursor: "text", color: "var(--wc-text-secondary)", fontSize: 12 }}
          >
            {cue.post_wait_ms ? `${(cue.post_wait_ms / 1000).toFixed(1)}s` : ""}
          </div>
        );

      case "continue":
        return (
          <span style={{ display: "block", textAlign: "center", color: "var(--wc-text-muted)" }}>
            {CONTINUE_LABELS[cue.continue_mode] ?? ""}
          </span>
        );

      case "notes":
        if (editingCell === "notes") return inlineInput("left");
        return (
          <div
            title={cue.notes || undefined}
            onDoubleClick={(e) => { e.stopPropagation(); startEditNotes(); }}
            style={{
              display: "flex", alignItems: "center",
              height: "100%", paddingLeft: 5,
              overflow: "hidden", cursor: "text",
              color: "var(--wc-text-muted)", fontSize: 12,
            }}
          >
            <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
              {cue.notes || ""}
            </span>
          </div>
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
          z-index 0 keeps it below column content (playhead indicator, etc.).
          Always drawn — "full row" adds a background tint on top of this, it
          doesn't replace it. */}
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
        <div
          key={col.id}
          style={col.stickyRight ? {
            position: "sticky",
            right: 0,
            zIndex: 2,
            background: stickyBg,
            boxShadow: "-4px 0 8px rgba(0,0,0,0.18)",
            alignSelf: "stretch",
          } : {
            minWidth: 0,
            overflow: "hidden",
            position: "relative",
            zIndex: 1,
            alignSelf: "stretch",
          }}
        >
          {renderCell(col.id)}
        </div>
      ))}
    </div>
  );
}
