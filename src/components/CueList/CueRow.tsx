// A single row in the cue list table.

import { PlayheadIndicator } from "./PlayheadIndicator";
import type { ColumnDef } from "./columns";
import type { CueSummary } from "../../lib/types";
import { useTimingStore } from "../../stores/timingStore";

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const CUE_TYPE_ICONS: Record<string, string> = {
  audio: "🔊",
  memo: "📝",
  wait: "⏱",
  group: "📁",
  fade: "📉",
};

const CONTINUE_LABELS: Record<string, string> = {
  do_not_continue: "—",
  auto_continue: "↓",
  auto_follow: "→",
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
  isDragOver?: boolean;
  /** True while this cue is being dragged (dims the row). */
  isDragSource?: boolean;
  /** Called on mousedown to start a cue drag operation. */
  onCueDragStart: (e: React.MouseEvent) => void;
  onClick: () => void;
  onDoubleClick: () => void;
  onContextMenu: (e: React.MouseEvent) => void;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function CueRow({
  cue,
  cueIndex,
  gridStyle,
  visibleDefs,
  isSelected,
  isAtPlayhead,
  isDragOver,
  isDragSource,
  onCueDragStart,
  onClick,
  onDoubleClick,
  onContextMenu,
}: Props) {
  const timing = useTimingStore((s) => s.timings[cue.id]);

  const isRunning = cue.state === "running";
  const isPaused  = cue.state === "paused";

  const progressPct =
    timing && cue.duration_ms && cue.duration_ms > 0
      ? Math.min(100, (timing.action_elapsed_ms / cue.duration_ms) * 100)
      : null;

  let bg = "transparent";
  if (isDragOver)   bg = "#1e3a5f";
  else if (isSelected) bg = "#1d4ed8";
  else if (isRunning)  bg = "#14532d";
  else if (isPaused)   bg = "#78350f";

  const rowStyle: React.CSSProperties = {
    ...gridStyle,
    alignItems: "center",
    paddingTop: 2,
    paddingBottom: 2,
    cursor: isDragSource ? "grabbing" : "grab",
    userSelect: "none",
    background: bg,
    borderBottom: isDragOver ? "1px solid #3b82f6" : "1px solid #1e293b",
    outline: isDragOver ? "1px solid #3b82f6" : "none",
    fontSize: 13,
    color: "#e2e8f0",
    minHeight: 26,
    opacity: isDragSource ? 0.4 : 1,
    transition: "opacity 0.1s",
  };

  const filename = cue.file_path
    ? cue.file_path.split(/[\\/]/).pop() ?? cue.file_path
    : "";

  const renderCell = (id: string) => {
    switch (id) {
      case "playhead":
        return <PlayheadIndicator visible={isAtPlayhead} />;

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
                display: "block",
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
                paddingLeft: 5,
              }}
            >
              {cue.name}
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

      default:
        return null;
    }
  };

  return (
    <div
      style={rowStyle}
      data-cue-id={cue.id}
      data-cue-index={cueIndex}
      onMouseDown={onCueDragStart}
      onClick={onClick}
      onDoubleClick={onDoubleClick}
      onContextMenu={onContextMenu}
    >
      {visibleDefs.map((col) => (
        <div key={col.id} style={{ minWidth: 0, overflow: "hidden" }}>
          {renderCell(col.id)}
        </div>
      ))}
    </div>
  );
}
