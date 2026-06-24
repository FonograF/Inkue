// Cart Mode view — cues as a responsive grid of trigger tiles.
// Click fires, drag reorders with live tile reflowing around the drop slot.

import { useState, useRef, useEffect, useMemo } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import { useTimingStore } from "../../stores/timingStore";
import type { CueSummary, CueType } from "../../lib/types";
import {
  addCue, goCue, moveCue, stopCue,
  setAudioFile, setVideoFile, setImageFile, updateCue,
} from "../../lib/commands";

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const CUE_TYPE_ICONS: Record<string, string> = {
  audio: "🔊", memo: "📝", wait: "⏱", group: "📁",
  fade: "📉", stop: "⬛", video: "🎬", image: "🖼",
  osc: "📡", midi: "🎹", light: "💡", mic: "🎤", timecode: "🕐",
};

const COLOR_SWATCHES: Record<string, string> = {
  none:   "transparent",
  red:    "#ef4444", orange: "#f97316", yellow: "#eab308",
  green:  "#22c55e", cyan:   "#06b6d4", blue:   "#3b82f6",
  purple: "#a855f7", pink:   "#ec4899", white:  "#f1f5f9", black: "#334155",
};

function hexToRgba(hex: string, alpha: number): string {
  const m = /^#([0-9a-f]{6})$/i.exec(hex);
  if (!m) return hex;
  const n = parseInt(m[1], 16);
  return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, ${alpha})`;
}

function formatDuration(ms: number): string {
  const s = Math.floor(ms / 1000);
  const m = Math.floor(s / 60);
  return `${m}:${String(s % 60).padStart(2, "0")}`;
}

const AUDIO_EXTS = new Set(["wav", "mp3", "flac", "ogg", "aac", "m4a"]);
const VIDEO_EXTS = new Set(["mp4", "m4v", "webm", "mov", "mkv", "avi", "ogv"]);
const IMAGE_EXTS = new Set(["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"]);

const isAudioPath = (p: string) => AUDIO_EXTS.has(p.split(".").pop()?.toLowerCase() ?? "");
const isVideoPath = (p: string) => VIDEO_EXTS.has(p.split(".").pop()?.toLowerCase() ?? "");
const isImagePath = (p: string) => IMAGE_EXTS.has(p.split(".").pop()?.toLowerCase() ?? "");
function cueTypeForPath(p: string): "video" | "image" | "audio" {
  if (isVideoPath(p)) return "video";
  if (isImagePath(p)) return "image";
  return "audio";
}
async function setFileForCue(cueType: "audio" | "video" | "image", cueId: string, path: string) {
  if (cueType === "video") await setVideoFile(cueId, path);
  else if (cueType === "image") await setImageFile(cueId, path);
  else await setAudioFile(cueId, path);
}
function basenameNoExt(p: string) {
  return (p.split(/[\\/]/).pop() ?? p).replace(/\.[^.]+$/, "");
}

// ---------------------------------------------------------------------------
// DragGhost — floating card following cursor with inertia-based tilt
// ---------------------------------------------------------------------------

function DragGhost({ cue, x, y, rotation }: { cue: CueSummary; x: number; y: number; rotation: number }) {
  const colorAccent = COLOR_SWATCHES[cue.color] ?? "transparent";
  const hasColor    = colorAccent !== "transparent";

  return (
    <div
      className="wc-drag-ghost"
      style={{
        position: "fixed",
        left: x, top: y,
        width: 164,
        transform: `translate(-50%, -50%) rotate(${rotation.toFixed(2)}deg) scale(1.08)`,
        pointerEvents: "none",
        zIndex: 99999,
        background: "var(--wc-bg-surface)",
        border: "1px solid var(--wc-border-strong)",
        borderRadius: 8,
        minHeight: 90,
        display: "flex",
        flexDirection: "column",
        boxShadow: "0 28px 56px rgba(0,0,0,0.65), 0 8px 20px rgba(0,0,0,0.45)",
        overflow: "hidden",
      }}
    >
      {hasColor && (
        <div style={{
          position: "absolute", left: 0, top: 0, bottom: 0, width: 4,
          background: colorAccent, borderRadius: "8px 0 0 8px",
        }} />
      )}
      <div style={{
        display: "flex", justifyContent: "space-between", alignItems: "center",
        padding: hasColor ? "8px 10px 0 14px" : "8px 10px 0 10px", flexShrink: 0,
      }}>
        <span style={{ fontSize: 10, color: "var(--wc-text-faint)", fontFamily: "monospace", lineHeight: 1 }}>
          {cue.number ?? "–"}
        </span>
        <span style={{ fontSize: 11, lineHeight: 1, opacity: 0.7 }}>
          {CUE_TYPE_ICONS[cue.cue_type] ?? ""}
        </span>
      </div>
      <div style={{
        flex: 1, display: "flex", alignItems: "center",
        padding: hasColor ? "6px 10px 8px 14px" : "6px 10px 8px 10px",
      }}>
        <span style={{
          fontSize: 13, fontWeight: 600, color: "var(--wc-text)", lineHeight: 1.3,
          display: "-webkit-box", WebkitLineClamp: 2, WebkitBoxOrient: "vertical",
          overflow: "hidden", wordBreak: "break-word", width: "100%",
        }}>
          {cue.name || "(untitled)"}
        </span>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// DropSlot — placeholder cell shown at the insertion target position
// ---------------------------------------------------------------------------

function DropSlot() {
  return (
    <div style={{
      border: "2px dashed var(--wc-accent)",
      borderRadius: 8,
      minHeight: 90,
      background: hexToRgba("#3b82f6", 0.06),
      pointerEvents: "none",
    }} />
  );
}

// ---------------------------------------------------------------------------
// CartTile
// ---------------------------------------------------------------------------

interface TileProps {
  cue: CueSummary;
  onMouseDown: (e: React.MouseEvent) => void;
  onFire: () => void;
}

function CartTile({ cue, onMouseDown, onFire }: TileProps) {
  const timing = useTimingStore((s) => s.timings[cue.id]);
  const [hovered, setHovered] = useState(false);
  const [stopHovered, setStopHovered] = useState(false);

  const isRunning   = cue.state === "running";
  const isPaused    = cue.state === "paused";
  const isCompleted = cue.state === "completed";

  const colorAccent = COLOR_SWATCHES[cue.color] ?? "transparent";
  const hasColor    = colorAccent !== "transparent";

  const statusBorderColor = isRunning ? "#22c55e" : isPaused ? "#f59e0b" : "var(--wc-border)";
  const statusTint =
    isRunning ? hexToRgba("#22c55e", 0.07) :
    isPaused  ? hexToRgba("#f59e0b", 0.07) : "var(--wc-bg-surface)";

  const statusShadow =
    isRunning ? `0 0 0 1px ${hexToRgba("#22c55e", 0.25)}` :
    isPaused  ? `0 0 0 1px ${hexToRgba("#f59e0b", 0.25)}` : "none";

  const loopPeriodMs = cue.file_duration_ms ?? cue.duration_ms;
  const progressPct =
    isRunning && timing && loopPeriodMs && loopPeriodMs > 0
      ? Math.min(100, ((timing.action_elapsed_ms % loopPeriodMs) / loopPeriodMs) * 100)
      : null;

  const remainingMs = timing?.remaining_ms ?? null;

  function handleStop(e: React.MouseEvent) {
    e.stopPropagation();
    stopCue(cue.id).catch(console.error);
  }

  return (
    <div
      data-cue-id={cue.id}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => { setHovered(false); setStopHovered(false); }}
      onMouseDown={onMouseDown}
      onClick={onFire}
      title={cue.name || "(untitled)"}
      style={{
        position: "relative",
        background: statusTint,
        border: `1px solid ${hovered && !isRunning && !isPaused ? "var(--wc-border-strong)" : statusBorderColor}`,
        borderRadius: 8,
        cursor: "pointer",
        userSelect: "none",
        overflow: "hidden",
        display: "flex",
        flexDirection: "column",
        minHeight: 90,
        opacity: isCompleted ? 0.5 : 1,
        transition: "border-color 0.1s, opacity 0.2s",
        boxShadow: statusShadow,
      }}
    >
      {/* Color stripe */}
      {hasColor && (
        <div style={{
          position: "absolute", left: 0, top: 0, bottom: 0, width: 4,
          background: colorAccent, borderRadius: "8px 0 0 8px", zIndex: 0,
        }} />
      )}

      {/* Header: number + type icon */}
      <div style={{
        display: "flex", justifyContent: "space-between", alignItems: "center",
        padding: hasColor ? "8px 10px 0 14px" : "8px 10px 0 10px", flexShrink: 0,
      }}>
        <span style={{ fontSize: 10, color: "var(--wc-text-faint)", fontFamily: "monospace", lineHeight: 1 }}>
          {cue.number ?? "–"}
        </span>
        <span style={{ fontSize: 11, lineHeight: 1, opacity: 0.7 }}>
          {CUE_TYPE_ICONS[cue.cue_type] ?? ""}
        </span>
      </div>

      {/* Name */}
      <div style={{
        flex: 1, display: "flex", alignItems: "center",
        padding: hasColor ? "4px 10px 6px 14px" : "4px 10px 6px 10px", minHeight: 0,
      }}>
        <span style={{
          fontSize: 13, fontWeight: 600,
          color: isCompleted ? "var(--wc-text-muted)" : "var(--wc-text)",
          lineHeight: 1.3,
          display: "-webkit-box", WebkitLineClamp: 2, WebkitBoxOrient: "vertical",
          overflow: "hidden", wordBreak: "break-word", width: "100%",
        }}>
          {cue.name || "(untitled)"}
        </span>
      </div>

      {/* Footer: status + stop */}
      <div style={{
        display: "flex", alignItems: "center", justifyContent: "space-between",
        padding: "0 10px 6px", paddingLeft: hasColor ? 14 : 10,
        flexShrink: 0, minHeight: 20,
      }}>
        {(isRunning || isPaused) ? (
          <div style={{ display: "flex", alignItems: "center", gap: 5 }}>
            {isRunning && (
              <span
                className="wc-led-pulse"
                style={{
                  display: "inline-block", width: 7, height: 7, borderRadius: "50%",
                  background: "#22c55e", flexShrink: 0,
                  animationDelay: `-${(Date.now() % 1800) / 1000}s`,
                }}
              />
            )}
            {isPaused && (
              <span style={{
                display: "inline-block", width: 7, height: 7, borderRadius: "50%",
                background: "#f59e0b", flexShrink: 0,
              }} />
            )}
            <span style={{ fontSize: 10, color: isRunning ? "#22c55e" : "#f59e0b", fontFamily: "monospace" }}>
              {remainingMs != null && remainingMs >= 0
                ? formatDuration(remainingMs)
                : isRunning ? "running" : "paused"}
            </span>
          </div>
        ) : (
          <span style={{ fontSize: 10, color: "var(--wc-text-faint)" }}>
            {cue.duration_ms != null ? formatDuration(cue.duration_ms) : ""}
          </span>
        )}
        {(isRunning || isPaused) && hovered && (
          <button
            onMouseEnter={() => setStopHovered(true)}
            onMouseLeave={() => setStopHovered(false)}
            onClick={handleStop}
            title="Stop"
            style={{
              background: stopHovered ? "#ef4444" : "rgba(239,68,68,0.2)",
              border: "1px solid #ef4444", borderRadius: 4,
              color: "#f1f5f9", fontSize: 9, fontWeight: 700,
              padding: "1px 5px", cursor: "pointer", letterSpacing: "0.05em",
              transition: "background 0.1s",
            }}
          >
            STOP
          </button>
        )}
      </div>

      {/* Progress bar */}
      {progressPct != null && (
        <div style={{
          position: "absolute", bottom: 0, left: 0, height: 3,
          width: `${progressPct}%`, background: "#22c55e",
          borderRadius: "0 0 0 8px", transition: "width 0.15s linear",
        }} />
      )}

      {/* Hover overlay */}
      {hovered && !isRunning && !isPaused && (
        <div style={{
          position: "absolute", inset: 0,
          background: "rgba(255,255,255,0.04)", borderRadius: 8, pointerEvents: "none",
        }} />
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// CartView
// ---------------------------------------------------------------------------

export function CartView({ onRefresh }: { onRefresh: () => void }) {
  const { cues } = useWorkspaceStore();
  const containerRef = useRef<HTMLDivElement>(null);
  const onRefreshRef = useRef(onRefresh);
  useEffect(() => { onRefreshRef.current = onRefresh; }, [onRefresh]);
  const cuesRef = useRef(cues);
  useEffect(() => { cuesRef.current = cues; }, [cues]);

  // Tile reorder drag
  const [draggingCueId, setDraggingCueId] = useState<string | null>(null);
  const [ghostState, setGhostState] = useState<{ x: number; y: number; rotation: number } | null>(null);
  const prevMouseXRef  = useRef<number | null>(null);
  const smoothedVelRef = useRef(0);
  const dragRef = useRef<{
    id: string;
    fromIndex: number;
    startX: number; startY: number;
    active: boolean;
    insertIndex: number | null;
  } | null>(null);

  // New-cue drag from toolbar
  const [newCueDragType, setNewCueDragType] = useState<CueType | null>(null);
  const newCueDragRef = useRef<{
    cueType: CueType;
    startX: number; startY: number;
    active: boolean;
    insertIndex: number | null;
  } | null>(null);

  // File drag (Tauri)
  const [isDraggingFile, setIsDraggingFile] = useState(false);

  // Shared insert index for all drag types
  const [insertIndex, setInsertIndex] = useState<number | null>(null);

  const justDroppedRef = useRef(false);

  // ---------------------------------------------------------------------------
  // Display array — remove dragged tile, insert DropSlot at target position.
  // This drives the live tile reflow during drag.
  // ---------------------------------------------------------------------------

  const displayItems = useMemo((): (CueSummary | null)[] => {
    const activeInsert = insertIndex;
    if (activeInsert === null) return cues;

    if (draggingCueId !== null) {
      // Tile reorder: dragged tile is removed from grid, DropSlot marks landing spot.
      // insertIndex is relative to the filtered array (cues without dragged tile),
      // which matches the `to` index expected by moveCue's remove-then-insert logic.
      const without = cues.filter(c => c.id !== draggingCueId);
      const result: (CueSummary | null)[] = [...without];
      result.splice(Math.min(activeInsert, without.length), 0, null);
      return result;
    }

    // Toolbar or file drag: inject DropSlot without removing a tile.
    const result: (CueSummary | null)[] = [...cues];
    result.splice(Math.min(activeInsert, cues.length), 0, null);
    return result;
  }, [cues, draggingCueId, insertIndex]);

  // ---------------------------------------------------------------------------
  // Grid insert index from cursor — queries only real tiles (DropSlot excluded)
  // ---------------------------------------------------------------------------

  function calcInsertIndex(clientX: number, clientY: number): number {
    const tiles = containerRef.current
      ? (Array.from(containerRef.current.querySelectorAll("[data-cue-id]")) as HTMLElement[])
      : [];
    for (let i = 0; i < tiles.length; i++) {
      const rect = tiles[i].getBoundingClientRect();
      if (clientY < rect.top) return i;
      if (clientY <= rect.bottom && clientX < rect.left + rect.width / 2) return i;
    }
    return tiles.length;
  }

  // ---------------------------------------------------------------------------
  // Document-level drag handlers
  // ---------------------------------------------------------------------------

  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      if (newCueDragRef.current) {
        const drag = newCueDragRef.current;
        if (!drag.active) {
          if (Math.hypot(e.clientX - drag.startX, e.clientY - drag.startY) < 5) return;
          drag.active = true;
          document.body.style.cursor = "copy";
          setNewCueDragType(drag.cueType);
        }
        const idx = calcInsertIndex(e.clientX, e.clientY);
        drag.insertIndex = idx;
        setInsertIndex(idx);
        return;
      }
      if (dragRef.current) {
        const drag = dragRef.current;
        if (!drag.active) {
          if (Math.hypot(e.clientX - drag.startX, e.clientY - drag.startY) < 5) return;
          drag.active = true;
          document.body.style.cursor = "none";
          prevMouseXRef.current = e.clientX;
          smoothedVelRef.current = 0;
          setDraggingCueId(drag.id);
        }
        const idx = calcInsertIndex(e.clientX, e.clientY);
        drag.insertIndex = idx;
        setInsertIndex(idx);
        // Inertia-based tilt
        const dx = e.clientX - (prevMouseXRef.current ?? e.clientX);
        prevMouseXRef.current = e.clientX;
        smoothedVelRef.current = smoothedVelRef.current * 0.78 + dx * 0.22;
        const rotation = Math.max(-13, Math.min(13, smoothedVelRef.current * 0.72));
        setGhostState({ x: e.clientX, y: e.clientY, rotation });
      }
    };

    const resetTileDrag = () => {
      dragRef.current = null;
      document.body.style.cursor = "";
      smoothedVelRef.current = 0;
      prevMouseXRef.current = null;
      setDraggingCueId(null);
      setInsertIndex(null);
      setGhostState(null);
    };

    const onUp = (e: MouseEvent) => {
      if (newCueDragRef.current) {
        const drag = newCueDragRef.current;
        if (drag.active && drag.insertIndex !== null) {
          const container = containerRef.current;
          if (container) {
            const rect = container.getBoundingClientRect();
            const inBounds =
              e.clientX >= rect.left && e.clientX <= rect.right &&
              e.clientY >= rect.top  && e.clientY <= rect.bottom;
            if (inBounds) {
              addCue(drag.cueType, drag.insertIndex)
                .then(() => onRefreshRef.current())
                .catch(console.error);
            }
          }
        }
        newCueDragRef.current = null;
        document.body.style.cursor = "";
        setNewCueDragType(null);
        setInsertIndex(null);
        return;
      }
      if (dragRef.current) {
        const drag = dragRef.current;
        if (drag.active && drag.insertIndex !== null) {
          // insertIndex is already the after-removal index (relative to the filtered array),
          // which is exactly what moveCue expects (it removes first, then inserts at `to`).
          const to   = drag.insertIndex;
          const from = drag.fromIndex;
          if (to !== from) {
            moveCue(drag.id, to)
              .then(() => onRefreshRef.current())
              .catch(console.error);
          }
          justDroppedRef.current = true;
          setTimeout(() => { justDroppedRef.current = false; }, 0);
        }
        resetTileDrag();
      }
    };

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      if (dragRef.current?.active) resetTileDrag();
      if (newCueDragRef.current?.active) {
        newCueDragRef.current = null;
        document.body.style.cursor = "";
        setNewCueDragType(null);
        setInsertIndex(null);
      }
    };

    const onNewCueDragStart = (e: Event) => {
      const { cueType, startX, startY } = (e as CustomEvent).detail as {
        cueType: CueType; startX: number; startY: number;
      };
      newCueDragRef.current = { cueType, startX, startY, active: false, insertIndex: null };
    };

    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup",   onUp);
    document.addEventListener("keydown",   onKeyDown);
    document.addEventListener("wincue:cue-drag-start", onNewCueDragStart);
    return () => {
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup",   onUp);
      document.removeEventListener("keydown",   onKeyDown);
      document.removeEventListener("wincue:cue-drag-start", onNewCueDragStart);
    };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // ---------------------------------------------------------------------------
  // Tauri file drag-drop
  // ---------------------------------------------------------------------------

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    (async () => {
      const fn_ = await getCurrentWindow().onDragDropEvent(async (event) => {
        const { type } = event.payload;
        if (type === "enter" || type === "over") {
          setIsDraggingFile(true);
          const pos = event.payload.position;
          if (pos) {
            setInsertIndex(calcInsertIndex(pos.x, pos.y));
          }
        } else if (type === "leave") {
          setIsDraggingFile(false);
          setInsertIndex(null);
        } else if (type === "drop") {
          setIsDraggingFile(false);
          setInsertIndex(null);
          const paths: string[] = (event.payload as { paths?: string[] }).paths ?? [];
          const pos = event.payload.position;
          const mediaPaths = paths.filter(p => isAudioPath(p) || isVideoPath(p) || isImagePath(p));
          if (mediaPaths.length === 0) return;
          let at = pos ? calcInsertIndex(pos.x, pos.y) : cuesRef.current.length;
          for (const p of mediaPaths) {
            const ct = cueTypeForPath(p);
            const newId = await addCue(ct, at).catch(() => null);
            if (newId) {
              await setFileForCue(ct, newId, p).catch(console.error);
              await updateCue(newId, { name: basenameNoExt(p) }).catch(console.error);
              at++;
            }
          }
          await onRefreshRef.current();
        }
      });
      if (cancelled) fn_(); else unlisten = fn_;
    })().catch(console.error);

    return () => { cancelled = true; unlisten?.(); };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // ---------------------------------------------------------------------------
  // Tile mousedown — start potential drag
  // ---------------------------------------------------------------------------

  function startTileDrag(e: React.MouseEvent, cueId: string, originalIndex: number) {
    if (e.button !== 0) return;
    e.preventDefault();
    dragRef.current = {
      id: cueId, fromIndex: originalIndex,
      startX: e.clientX, startY: e.clientY,
      active: false, insertIndex: null,
    };
  }

  // ---------------------------------------------------------------------------
  // Render
  // ---------------------------------------------------------------------------

  const isEmpty = cues.length === 0;
  const isDragging = draggingCueId !== null || newCueDragType !== null || isDraggingFile;

  return (
    <div
      ref={containerRef}
      style={{
        flex: 1, overflow: "auto", padding: 12,
        display: "grid",
        gridTemplateColumns: "repeat(auto-fill, minmax(160px, 1fr))",
        gap: 8,
        alignContent: "start",
      }}
      onDragOver={(e) => e.preventDefault()}
      onDrop={(e) => e.preventDefault()}
    >
      {isEmpty && !isDragging && (
        <div style={{
          gridColumn: "1 / -1", padding: 40, textAlign: "center",
          color: "var(--wc-text-faint)", fontSize: 14,
        }}>
          Drag cues from the toolbar to add them here.
        </div>
      )}

      {isEmpty && isDragging && <DropSlot key="__drop_slot__" />}

      {!isEmpty && displayItems.map((item, _i) =>
        item === null ? (
          <DropSlot key="__drop_slot__" />
        ) : (
          <CartTile
            key={item.id}
            cue={item}
            onMouseDown={(e) => {
              const originalIndex = cues.findIndex(c => c.id === item.id);
              startTileDrag(e, item.id, originalIndex);
            }}
            onFire={() => {
              if (justDroppedRef.current) return;
              goCue(item.id).catch(console.error);
            }}
          />
        )
      )}

      {/* Floating ghost with inertia tilt */}
      {draggingCueId !== null && ghostState !== null && (() => {
        const cue = cues.find(c => c.id === draggingCueId);
        return cue
          ? <DragGhost cue={cue} x={ghostState.x} y={ghostState.y} rotation={ghostState.rotation} />
          : null;
      })()}
    </div>
  );
}
