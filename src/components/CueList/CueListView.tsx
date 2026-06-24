// Main cue list table — resizable / hideable / reorderable columns,
// playhead indicator, file drag-drop, and cue context menu.
//
// Layout: the header strip and the rows area share the same gridTemplateColumns
// string (all pixel widths, no fr). Horizontal overflow is handled by a
// scroll-sync pair: the rows container has overflow:auto and scrolls freely;
// the header container has its scrollbar hidden and its scrollLeft is kept in
// sync via an onScroll handler. This way the header always aligns with the
// rows regardless of window width.

import { useEffect, useRef, useState, useMemo, Fragment } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import { CueRow } from "./CueRow";
import {
  DEFAULT_COLUMNS,
  buildGridCols,
  getVisibleDefs,
  loadColumnConfig,
  saveColumnConfig,
  type ColumnConfig,
  type ColumnDef,
  type ColumnId,
} from "./columns";
import type { CueSummary } from "../../lib/types";
import {
  addCue,
  removeCue,
  duplicateCue,
  groupCues,
  moveCue,
  moveCues,
  ungroup,
  removeCueFromGroup,
  addCueToGroup,
  moveToTopLevel,
  setAudioFile,
  setVideoFile,
  setImageFile,
  setPlayhead,
  stopCue,
  updateCue,
} from "../../lib/commands";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const AUDIO_EXTS = new Set(["wav", "mp3", "flac", "ogg", "aac", "m4a"]);
const VIDEO_EXTS = new Set(["mp4", "m4v", "webm", "mov", "mkv", "avi", "ogv"]);
const IMAGE_EXTS = new Set(["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"]);

function isAudioPath(p: string) {
  return AUDIO_EXTS.has(p.split(".").pop()?.toLowerCase() ?? "");
}
function isVideoPath(p: string) {
  return VIDEO_EXTS.has(p.split(".").pop()?.toLowerCase() ?? "");
}
function isImagePath(p: string) {
  return IMAGE_EXTS.has(p.split(".").pop()?.toLowerCase() ?? "");
}
function cueTypeForPath(p: string): "video" | "image" | "audio" {
  if (isVideoPath(p)) return "video";
  if (isImagePath(p)) return "image";
  return "audio";
}
async function setFileForCue(
  cueType: "audio" | "video" | "image",
  cueId: string,
  path: string,
) {
  if (cueType === "video") await setVideoFile(cueId, path);
  else if (cueType === "image") await setImageFile(cueId, path);
  else await setAudioFile(cueId, path);
}
function basenameNoExt(p: string) {
  return (p.split(/[\\/]/).pop() ?? p).replace(/\.[^.]+$/, "");
}

// ---------------------------------------------------------------------------
// Cue context-menu item
// ---------------------------------------------------------------------------

/** Compute the set of child cue IDs that should show the inner playhead indicator.
 *
 * Rules per group mode:
 * - Sequential (at outer playhead, Standby): show first child (previews what fires on GO).
 * - Sequential (running): show `active_child_id` (the next child that fires on GO).
 * - Simultaneous (at outer playhead OR running): show all direct children.
 *
 * `outerPlayheadId` is the ID of the cue currently at the outer playhead. */
function computeInnerPlayheadIds(cues: CueSummary[], outerPlayheadId: string | null): Set<string> {
  const result = new Set<string>();
  for (const cue of cues) {
    if (cue.cue_type === "group") {
      const isAtPlayhead = cue.id === outerPlayheadId;
      const isRunning = cue.state === "running";

      if (cue.group_mode === "sequential") {
        // active_child_id is the child a GO fires next, in every state. Show it
        // whenever the group is running or parked at the outer playhead, so a
        // child the user parked the Playhead on is highlighted — not just the first.
        if ((isRunning || isAtPlayhead) && cue.active_child_id) {
          result.add(cue.active_child_id);
        }
      } else if (cue.group_mode === "simultaneous") {
        if (isAtPlayhead || isRunning) {
          // All children will fire (or are firing) — show playhead on all.
          for (const child of cue.children ?? []) {
            result.add(child.id);
          }
        }
      }
    }
    if (cue.children?.length) {
      computeInnerPlayheadIds(cue.children, outerPlayheadId).forEach((id) => result.add(id));
    }
  }
  return result;
}

function CtxItem({ label, danger, onClick }: { label: string; danger?: boolean; onClick: () => void }) {
  const [hov, setHov] = useState(false);
  return (
    <button
      style={{
        display: "block", width: "100%", padding: "6px 16px",
        background: hov ? "var(--wc-bg-hover)" : "transparent", border: "none",
        textAlign: "left", color: danger ? "#ef4444" : "var(--wc-text)",
        fontSize: 13, cursor: "pointer", whiteSpace: "nowrap",
      }}
      onMouseEnter={() => setHov(true)}
      onMouseLeave={() => setHov(false)}
      onClick={onClick}
    >
      {label}
    </button>
  );
}

// ---------------------------------------------------------------------------
// Column visibility menu (shown on header right-click)
// ---------------------------------------------------------------------------

function ColumnMenu({
  config,
  pos,
  onToggle,
  onClose,
}: {
  config: ColumnConfig;
  pos: { x: number; y: number };
  onToggle: (id: ColumnId, visible: boolean) => void;
  onClose: () => void;
}) {
  const items = config.order
    .map((id) => DEFAULT_COLUMNS.find((d) => d.id === id)!)
    .filter((d) => d && !d.fixed);

  const menuW = 200;
  const left = Math.min(pos.x, window.innerWidth - menuW - 8);
  const top  = Math.min(pos.y, window.innerHeight - items.length * 30 - 50);

  return (
    <>
      <div style={{ position: "fixed", inset: 0, zIndex: 9998 }} onClick={onClose} />
      <div
        style={{
          position: "fixed", left, top,
          background: "var(--wc-bg-surface)", border: "1px solid var(--wc-border-strong)",
          borderRadius: 6, padding: "6px 0", zIndex: 9999,
          minWidth: menuW, boxShadow: "0 4px 16px rgba(0,0,0,0.6)",
        }}
      >
        <div style={{ padding: "2px 12px 6px", fontSize: 10, color: "var(--wc-text-muted)", textTransform: "uppercase", letterSpacing: "0.05em" }}>
          Columns
        </div>
        {items.map((d) => (
          <label
            key={d.id}
            style={{
              display: "flex", alignItems: "center", gap: 8,
              padding: "5px 12px", cursor: "pointer",
              fontSize: 12, color: "var(--wc-text)", userSelect: "none",
            }}
          >
            <input
              type="checkbox"
              checked={!config.hidden[d.id]}
              onChange={(e) => onToggle(d.id, e.target.checked)}
              style={{ accentColor: "var(--wc-accent)" }}
            />
            {d.label || d.id.replace(/_/g, " ")}
          </label>
        ))}
      </div>
    </>
  );
}

// ---------------------------------------------------------------------------
// Main component
// ---------------------------------------------------------------------------

interface Props {
  onCueDoubleClick: (cue: CueSummary) => void;
  onRefresh: () => void;
}

export function CueListView({ onCueDoubleClick, onRefresh }: Props) {
  const {
    cues, selectedCueId, selectedCueIds, playheadCueId,
    setSelectedCueId, setSelectedCueIds, setPlayheadCueId, generalPrefs, displayPrefs,
  } = useWorkspaceStore();

  const rowHeight = generalPrefs.cue_row_height === "compact" ? 22
    : generalPrefs.cue_row_height === "tall" ? 32
    : 26;

  // Auto-scroll to playhead when it moves
  useEffect(() => {
    if (!generalPrefs.auto_scroll_to_playhead || !playheadCueId) return;
    const el = rowsScrollRef.current?.querySelector(`[data-cue-id="${playheadCueId}"]`);
    if (el) el.scrollIntoView({ block: "nearest" });
  }, [playheadCueId, generalPrefs.auto_scroll_to_playhead]);

  // ---------- Column config ----------
  const [colConfig, setColConfig] = useState<ColumnConfig>(loadColumnConfig);
  const [colMenuPos, setColMenuPos] = useState<{ x: number; y: number } | null>(null);
  const [draggingColId, setDraggingColId] = useState<ColumnId | null>(null);
  const [hoveredResizeId, setHoveredResizeId] = useState<ColumnId | null>(null);

  // ---------- Group expand/collapse ----------
  const [expandedGroupIds, setExpandedGroupIds] = useState<Set<string>>(new Set());

  function toggleGroupExpand(groupId: string) {
    setExpandedGroupIds(prev => {
      const next = new Set(prev);
      if (next.has(groupId)) next.delete(groupId);
      else next.add(groupId);
      return next;
    });
  }

  // Flatten nested cue tree into a display list respecting expansion state.
  const flatItems = useMemo(() => {
    function flatten(
      src: CueSummary[],
      depth: number,
      parentGroupId: string | null,
    ): Array<{ cue: CueSummary; depth: number; parentGroupId: string | null }> {
      const result: Array<{ cue: CueSummary; depth: number; parentGroupId: string | null }> = [];
      for (const cue of src) {
        result.push({ cue, depth, parentGroupId });
        if (cue.cue_type === "group" && expandedGroupIds.has(cue.id) && cue.children?.length) {
          result.push(...flatten(cue.children, depth + 1, cue.id));
        }
      }
      return result;
    }
    return flatten(cues, 0, null);
  }, [cues, expandedGroupIds]);

  // Inner playhead IDs — children shown with the playhead indicator because
  // their parent group is running and they are the active child (sequential)
  // or all children (simultaneous).
  const innerPlayheadIds = useMemo(
    () => computeInnerPlayheadIds(cues, playheadCueId ?? null),
    [cues, playheadCueId],
  );

  // ---------- Multi-selection anchors ----------
  // anchorCueIdRef: the fixed end of a range selection (set by plain click / plain arrow).
  // selectionEndRef: the moving end (updated by shift+click / shift+arrow).
  const anchorCueIdRef = useRef<string | null>(null);
  const selectionEndRef = useRef<string | null>(null);
  const selectedCueSet = useMemo(() => new Set(selectedCueIds), [selectedCueIds]);

  useEffect(() => { saveColumnConfig(colConfig); }, [colConfig]);

  const visibleDefs = useMemo(() => getVisibleDefs(colConfig), [colConfig]);
  const gridCols    = useMemo(() => buildGridCols(visibleDefs, colConfig), [visibleDefs, colConfig]);

  const setColConfigRef = useRef(setColConfig);
  setColConfigRef.current = setColConfig;

  // ---------- Cue drag-and-drop state ----------
  const [draggingCueId,     setDraggingCueId]     = useState<string | null>(null);
  const [dropInsertIndex,   setDropInsertIndex]    = useState<number | null>(null);
  const [dropTargetGroupId, setDropTargetGroupId] = useState<string | null>(null);

  // ---------- New-cue drag state (toolbar buttons dragged into list) ----------
  // Driven by a CustomEvent "wincue:cue-drag-start" dispatched by external buttons.
  const [newCueDragType,     setNewCueDragType]     = useState<import("../../lib/types").CueType | null>(null);
  const [newCueDragInsertIdx, setNewCueDragInsertIdx] = useState<number | null>(null);

  const newCueDragRef = useRef<{
    cueType: import("../../lib/types").CueType;
    startX: number;
    startY: number;
    active: boolean;
    insertIdx: number | null;
  } | null>(null);

  // Keep a fresh ref to onRefresh so the stale useEffect closure can call it.
  const onRefreshRef = useRef(onRefresh);
  useEffect(() => { onRefreshRef.current = onRefresh; }, [onRefresh]);

  // Tracks an in-progress cue drag entirely in a ref (no re-render on every
  // pixel moved). dropIdx is mirrored here so the mouseup closure can read it.
  // ids: all cues being dragged (single or multi-selection drag).
  const cueDragRef = useRef<{
    id: string;
    ids: string[];
    fromIndex: number;
    parentGroupId: string | null;
    startX: number;
    startY: number;
    active: boolean;
    dropIdx: number | null;
    dropGroupId: string | null;
    /** true = drop at end of group; false = insert at specific position within group */
    dropGroupAtEnd: boolean;
  } | null>(null);

  // Set to true for one event loop tick after a drag completes so the row's
  // onClick handler can ignore the spurious click that follows mouseup.
  const justDroppedRef = useRef(false);

  // ---------- Scroll-sync refs ----------
  // The header scrollbar is hidden (class="no-scrollbar"); its scrollLeft is
  // driven programmatically by the rows container's onScroll handler.
  const headerScrollRef = useRef<HTMLDivElement>(null);
  const rowsScrollRef   = useRef<HTMLDivElement>(null);

  // ---------- Resize drag state ----------
  const resizingRef = useRef<{ id: ColumnId; startX: number; startW: number } | null>(null);

  // ---------- Column reorder drag state ----------
  const colDragRef = useRef<{
    id: ColumnId;
    startX: number;
    active: boolean;
    originalOrder: ColumnId[];
    lastTargetId: ColumnId | null;
  } | null>(null);

  // Combined document-level pointer tracker for resize + reorder.
  useEffect(() => {
    // Compute insert index from cursor Y, identical to the previous logic.
    function calcInsertIdxFromY(clientY: number): number {
      const rowEls = rowsScrollRef.current
        ? (Array.from(rowsScrollRef.current.querySelectorAll("[data-cue-id]")) as HTMLElement[])
        : [];
      for (const el of rowEls) {
        const rect = el.getBoundingClientRect();
        if (clientY < rect.top + rect.height / 2) {
          return Number(el.dataset.cueIndex ?? 0);
        }
      }
      return flatItemsRef.current.length;
    }

    // Return the child-list position for inserting into `groupId` such that
    // the new child lands just before flatIndex `dropIdx`.
    function resolveChildPositionInGroup(groupId: string, dropIdx: number): number {
      const items = flatItemsRef.current;
      let pos = 0;
      for (let i = 0; i < dropIdx; i++) {
        if (items[i].parentGroupId === groupId) pos++;
      }
      return pos;
    }

    // Return the ID of the first top-level cue at or after flatIndex `idx`.
    // Used when a child cue is dragged to a between-rows drop position.
    function resolveTopLevelBeforeId(idx: number): string | null {
      const items = flatItemsRef.current;
      for (let i = idx; i < items.length; i++) {
        if (items[i].parentGroupId === null) return items[i].cue.id;
      }
      return null; // append at end
    }

    // Determine where to drop a dragged cue.
    //
    // Two-pass approach:
    //   Pass 1 — check if cursor is in the MIDDLE zone of a group header row
    //             (the only case where we drop at end without a positional insert line).
    //   Pass 2 — standard midpoint logic gives a stable insertIdx, then derive
    //             the target group purely from flatItems data.
    //             This is flicker-free: groupId only changes when insertIdx crosses
    //             a real group boundary, not on pixel-level gaps between rows.
    function calcDropTarget(clientY: number): { insertIdx: number; groupId: string | null; atEnd: boolean } {
      const rowEls = rowsScrollRef.current
        ? (Array.from(rowsScrollRef.current.querySelectorAll("[data-cue-id]")) as HTMLElement[])
        : [];

      // Pass 1: group-header middle zone → drop at end of that group.
      for (const el of rowEls) {
        if (el.dataset.isGroup !== "true") continue;
        const rect = el.getBoundingClientRect();
        if (clientY < rect.top || clientY > rect.bottom) continue;
        const relY = clientY - rect.top;
        const DEAD = rect.height * 0.28;
        if (relY >= DEAD && relY <= rect.height - DEAD) {
          const idx = Number(el.dataset.cueIndex ?? 0);
          return { insertIdx: idx, groupId: el.dataset.cueId ?? null, atEnd: true };
        }
      }

      // Pass 2: midpoint insert position.
      let insertIdx = flatItemsRef.current.length;
      for (const el of rowEls) {
        const rect = el.getBoundingClientRect();
        if (clientY < rect.top + rect.height / 2) {
          insertIdx = Number(el.dataset.cueIndex ?? 0);
          break;
        }
      }

      // Derive group from flat-items data: if the item we're inserting BEFORE
      // is a child, we're within its parent group. This is stable — groupId only
      // changes when insertIdx crosses into a different parentGroupId territory.
      const items = flatItemsRef.current;
      const itemAt = insertIdx < items.length ? items[insertIdx] : null;
      const groupId = itemAt?.parentGroupId ?? null;

      return { insertIdx, groupId, atEnd: false };
    }

    const onMove = (e: MouseEvent) => {
      // ── Column resize ─────────────────────────────────────────────────────
      if (resizingRef.current) {
        const { id, startX, startW } = resizingRef.current;
        const def = DEFAULT_COLUMNS.find((d) => d.id === id)!;
        const newW = Math.max(def.minWidth, startW + (e.clientX - startX));
        setColConfigRef.current((prev) => ({
          ...prev,
          widths: { ...prev.widths, [id]: newW },
        }));
        return;
      }

      // ── Column reorder ────────────────────────────────────────────────────
      if (colDragRef.current) {
        const drag = colDragRef.current;
        if (!drag.active) {
          if (Math.abs(e.clientX - drag.startX) < 6) return;
          drag.active = true;
          document.body.style.cursor = "grabbing";
          setDraggingColId(drag.id);
        }
        const colEl = document.elementFromPoint(e.clientX, e.clientY)
          ?.closest("[data-col-id]") as HTMLElement | null;
        const targetId = (colEl?.dataset.colId ?? null) as ColumnId | null;
        if (
          !targetId || targetId === drag.id || targetId === drag.lastTargetId ||
          DEFAULT_COLUMNS.find((d) => d.id === targetId)?.fixed
        ) return;
        drag.lastTargetId = targetId;
        setColConfigRef.current((prev) => {
          const order = [...prev.order];
          const from = order.indexOf(drag.id);
          const to   = order.indexOf(targetId);
          if (from < 0 || to < 0) return prev;
          order.splice(from, 1);
          order.splice(to, 0, drag.id);
          return { ...prev, order };
        });
        return;
      }

      // ── New-cue drag (toolbar button → insert position in list) ──────────
      if (newCueDragRef.current) {
        const drag = newCueDragRef.current;
        if (!drag.active) {
          if (Math.hypot(e.clientX - drag.startX, e.clientY - drag.startY) < 5) return;
          drag.active = true;
          document.body.style.cursor = "copy";
          setNewCueDragType(drag.cueType);
        }
        const newDrop = calcInsertIdxFromY(e.clientY);
        drag.insertIdx = newDrop;
        setNewCueDragInsertIdx(newDrop);
        return;
      }

      // ── Cue reorder ───────────────────────────────────────────────────────
      if (cueDragRef.current) {
        const drag = cueDragRef.current;
        if (!drag.active) {
          if (Math.hypot(e.clientX - drag.startX, e.clientY - drag.startY) < 5) return;
          drag.active = true;
          document.body.style.cursor = "grabbing";
          setDraggingCueId(drag.id);
        }
        const { insertIdx, groupId, atEnd } = calcDropTarget(e.clientY);
        // Don't allow dropping a cue onto itself as a group target.
        const resolvedGroupId = drag.ids.includes(groupId ?? "") ? null : groupId;
        drag.dropIdx = insertIdx;
        drag.dropGroupId = resolvedGroupId;
        drag.dropGroupAtEnd = atEnd;
        if (resolvedGroupId && atEnd) {
          // Drop onto group header at end → highlight group, no insert line.
          setDropTargetGroupId(resolvedGroupId);
          setDropInsertIndex(null);
        } else if (resolvedGroupId && !atEnd) {
          // Insert between group children → highlight group + show insert line.
          // No flickering: calcDropTarget always returns the same groupId for child rows.
          setDropTargetGroupId(resolvedGroupId);
          setDropInsertIndex(insertIdx);
        } else {
          setDropTargetGroupId(null);
          setDropInsertIndex(insertIdx);
        }
        return;
      }
    };

    const onUp = (e: MouseEvent) => {
      if (resizingRef.current) {
        resizingRef.current = null;
        document.body.style.cursor = "";
        return;
      }
      if (newCueDragRef.current) {
        const drag = newCueDragRef.current;
        if (drag.active && drag.insertIdx !== null) {
          const rowsEl = rowsScrollRef.current;
          if (rowsEl) {
            const rect = rowsEl.getBoundingClientRect();
            const inBounds =
              e.clientX >= rect.left && e.clientX <= rect.right &&
              e.clientY >= rect.top  && e.clientY <= rect.bottom;
            if (inBounds) {
              addCue(drag.cueType, drag.insertIdx)
                .then(() => onRefreshRef.current())
                .catch(console.error);
            }
          }
        }
        newCueDragRef.current = null;
        document.body.style.cursor = "";
        setNewCueDragType(null);
        setNewCueDragInsertIdx(null);
        return;
      }
      if (colDragRef.current) {
        colDragRef.current = null;
        document.body.style.cursor = "";
        setDraggingColId(null);
        return;
      }
      if (cueDragRef.current) {
        const drag = cueDragRef.current;
        if (drag.active) {
          if (drag.dropGroupId) {
            // Drop onto / insert into a group.
            const pos = drag.dropGroupAtEnd
              ? -1  // append at end
              : resolveChildPositionInGroup(drag.dropGroupId, drag.dropIdx ?? 0);
            const promises = drag.ids.map((id) =>
              addCueToGroup(id, drag.dropGroupId!, pos).catch(console.error)
            );
            Promise.all(promises).then(onRefresh).catch(console.error);
          } else if (drag.dropIdx !== null) {
            if (drag.parentGroupId) {
              // Cue(s) sourced from inside a group.
              const dropItem = flatItemsRef.current[drag.dropIdx];
              const prevItem = drag.dropIdx > 0 ? flatItemsRef.current[drag.dropIdx - 1] : null;
              const sameGroup =
                dropItem?.parentGroupId === drag.parentGroupId ||
                prevItem?.parentGroupId === drag.parentGroupId;
              if (sameGroup) {
                // Reorder within the same group.
                const childPos = resolveChildPositionInGroup(drag.parentGroupId, drag.dropIdx);
                const promises = drag.ids.map((id) =>
                  addCueToGroup(id, drag.parentGroupId!, childPos).catch(console.error)
                );
                Promise.all(promises).then(onRefresh).catch(console.error);
              } else {
                // Extract to top level.
                const beforeId = resolveTopLevelBeforeId(drag.dropIdx);
                const promises = drag.ids.map((id) =>
                  moveToTopLevel(id, beforeId).catch(console.error)
                );
                Promise.all(promises).then(onRefresh).catch(console.error);
              }
            } else if (drag.ids.length > 1) {
              const beforeId = flatItemsRef.current[drag.dropIdx]?.cue.id ?? null;
              const draggingSet = new Set(drag.ids);
              if (!draggingSet.has(beforeId ?? "")) {
                moveCues(drag.ids, beforeId).then(onRefresh).catch(console.error);
              }
            } else {
              const from   = cuesRef.current.findIndex((c) => c.id === drag.id);
              const newPos = from < drag.dropIdx ? drag.dropIdx - 1 : drag.dropIdx;
              if (from >= 0 && newPos !== from) {
                moveCue(drag.id, newPos).then(onRefresh).catch(console.error);
              }
            }
          }
          // Suppress the spurious onClick that fires after mouseup.
          justDroppedRef.current = true;
          setTimeout(() => { justDroppedRef.current = false; }, 0);
        }
        cueDragRef.current = null;
        document.body.style.cursor = "";
        setDraggingCueId(null);
        setDropInsertIndex(null);
        setDropTargetGroupId(null);
        return;
      }
    };

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (colDragRef.current?.active) {
          const orig = colDragRef.current.originalOrder;
          setColConfigRef.current((prev) => ({ ...prev, order: orig }));
          colDragRef.current = null;
          document.body.style.cursor = "";
          setDraggingColId(null);
        }
        if (cueDragRef.current?.active) {
          cueDragRef.current = null;
          document.body.style.cursor = "";
          setDraggingCueId(null);
          setDropInsertIndex(null);
          setDropTargetGroupId(null);
        }
        if (newCueDragRef.current?.active) {
          newCueDragRef.current = null;
          document.body.style.cursor = "";
          setNewCueDragType(null);
          setNewCueDragInsertIdx(null);
        }
      }
    };

    const onNewCueDragStart = (e: Event) => {
      const { cueType, startX, startY } = (e as CustomEvent).detail as {
        cueType: import("../../lib/types").CueType;
        startX: number;
        startY: number;
      };
      newCueDragRef.current = { cueType, startX, startY, active: false, insertIdx: null };
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
  }, []);

  // ---------- Column header handlers ----------

  function startResize(e: React.MouseEvent, def: ColumnDef) {
    e.preventDefault();
    e.stopPropagation();
    document.body.style.cursor = "col-resize";
    const startW = colConfig.widths[def.id] ?? def.defaultWidth;
    resizingRef.current = { id: def.id, startX: e.clientX, startW };
  }

  function startColDrag(e: React.MouseEvent, def: ColumnDef) {
    if (e.button !== 0 || def.fixed) return;
    e.preventDefault();
    colDragRef.current = {
      id: def.id,
      startX: e.clientX,
      active: false,
      originalOrder: [...colConfig.order],
      lastTargetId: null,
    };
  }

  // ---------- Cue drag start ----------
  function startCueDrag(e: React.MouseEvent, cueId: string, index: number) {
    if (e.button !== 0) return;
    // Don't steal the event if a column resize handle was clicked.
    if ((e.target as HTMLElement).closest("[data-resize]")) return;
    e.preventDefault();

    const flatItem = flatItemsRef.current[index];
    const parentGroupId = flatItem?.parentGroupId ?? null;

    // Allow multi-drag for top-level cues and for children within the same group.
    const dragIds = selectedCueSet.has(cueId) && selectedCueIds.length > 1
      ? [...selectedCueIds]
      : [cueId];

    cueDragRef.current = {
      id: cueId,
      ids: dragIds,
      fromIndex: index,
      parentGroupId,
      startX: e.clientX,
      startY: e.clientY,
      active: false,
      dropIdx: null,
      dropGroupId: null,
      dropGroupAtEnd: true,
    };
  }

  // ---------- File drag-drop state ----------
  const [dragOverCueId,    setDragOverCueId]    = useState<string | null>(null);
  const [dragOverGroupId,  setDragOverGroupId]  = useState<string | null>(null);
  const [isDragging,       setIsDragging]       = useState(false);
  // When a file is dragged in insert-between mode (cursor near row edge),
  // this holds the insertion index; dragOverCueId is null in that case.
  const [fileDragInsertIdx, setFileDragInsertIdx] = useState<number | null>(null);
  const [contextMenu,   setContextMenu]   = useState<{ x: number; y: number; cueId: string | null; parentGroupId?: string | null } | null>(null);

  const cuesRef = useRef(cues);
  useEffect(() => { cuesRef.current = cues; }, [cues]);

  const flatItemsRef = useRef(flatItems);
  useEffect(() => { flatItemsRef.current = flatItems; }, [flatItems]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    (async () => {
      // Tauri drag-drop positions are in physical (DPI-scaled) pixels.
      // Convert to logical CSS pixels before comparing with getBoundingClientRect().
      // Cursor in the top/bottom 8 logical px of a row → insert line.
      // Cursor in the middle of a non-group row → assign/replace that cue.
      // Cursor in the middle of a top-level group row → drop into the group.
      const EDGE_PX = 8;
      function resolveFileDragMode(_physX: number, physY: number): {
        insertIdx: number | null;
        assignId: string | null;
        groupId: string | null;
      } {
        const dpr = window.devicePixelRatio || 1;
        const py  = physY / dpr;
        const rowEls = rowsScrollRef.current
          ? (Array.from(rowsScrollRef.current.querySelectorAll("[data-cue-id]")) as HTMLElement[])
          : [];
        for (const el of rowEls) {
          const rect = el.getBoundingClientRect();
          if (py < rect.top) {
            return { insertIdx: Number(el.dataset.cueIndex ?? 0), assignId: null, groupId: null };
          }
          if (py < rect.bottom) {
            const idx = Number(el.dataset.cueIndex ?? -1);
            const id  = el.dataset.cueId ?? null;
            if (py - rect.top    < EDGE_PX) return { insertIdx: idx >= 0 ? idx     : 0,                           assignId: null, groupId: null };
            if (rect.bottom - py < EDGE_PX) return { insertIdx: idx >= 0 ? idx + 1 : flatItemsRef.current.length, assignId: null, groupId: null };
            // Middle of row — group vs normal cue.
            if (el.dataset.isGroup === "true") {
              return { insertIdx: null, assignId: null, groupId: id };
            }
            return { insertIdx: null, assignId: id, groupId: null };
          }
        }
        return { insertIdx: flatItemsRef.current.length, assignId: null, groupId: null };
      }

      const fn_ = await getCurrentWindow().onDragDropEvent(async (event) => {
        const { type } = event.payload;
        if (type === "enter" || type === "over") {
          setIsDragging(true);
          const pos = event.payload.position;
          if (pos) {
            const { insertIdx, assignId, groupId } = resolveFileDragMode(pos.x, pos.y);
            setFileDragInsertIdx(insertIdx);
            setDragOverCueId(assignId);
            setDragOverGroupId(groupId);
          }
        } else if (type === "leave") {
          setIsDragging(false);
          setDragOverCueId(null);
          setDragOverGroupId(null);
          setFileDragInsertIdx(null);
        } else if (type === "drop") {
          setIsDragging(false);
          setDragOverCueId(null);
          setDragOverGroupId(null);
          setFileDragInsertIdx(null);
          const paths: string[] = (event.payload as { paths?: string[] }).paths ?? [];
          const pos = event.payload.position;
          const mediaPaths = paths.filter(
            (p) => isAudioPath(p) || isVideoPath(p) || isImagePath(p),
          );
          if (mediaPaths.length === 0) return;

          const { insertIdx, assignId, groupId } = pos
            ? resolveFileDragMode(pos.x, pos.y)
            : { insertIdx: null, assignId: null, groupId: null };

          if (groupId) {
            // Drop into group: create cue(s) then move into the group.
            for (const p of mediaPaths) {
              const cueType = cueTypeForPath(p);
              const newId = await addCue(cueType, -1).catch(() => null);
              if (newId) {
                await setFileForCue(cueType, newId, p).catch(console.error);
                await updateCue(newId, { name: basenameNoExt(p) }).catch(console.error);
                await addCueToGroup(newId, groupId, -1).catch(console.error);
                setSelectedCueId(newId);
              }
            }
            await onRefresh();
          } else if (insertIdx !== null) {
            // Insert mode: create new cue(s) at the target position.
            let at = insertIdx;
            for (const p of mediaPaths) {
              const cueType = cueTypeForPath(p);
              const newId = await addCue(cueType, at).catch(() => null);
              if (newId) {
                await setFileForCue(cueType, newId, p).catch(console.error);
                await updateCue(newId, { name: basenameNoExt(p) }).catch(console.error);
                setSelectedCueId(newId);
                at++;
              }
            }
            await onRefresh();
          } else if (mediaPaths.length === 1) {
            await handleFileDrop(mediaPaths[0], assignId);
          } else {
            for (const p of mediaPaths) await handleFileDrop(p, null);
          }
        }
      });
      if (cancelled) fn_(); else unlisten = fn_;
    })().catch(console.error);

    return () => { cancelled = true; unlisten?.(); };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  async function handleFileDrop(filePath: string, targetCueId: string | null) {
    const cueType = cueTypeForPath(filePath);
    if (targetCueId) {
      // Assign to an existing cue only if the file type matches the cue type.
      const targetCue = cuesRef.current.find((c) => c.id === targetCueId);
      if (targetCue?.cue_type === cueType) {
        await setFileForCue(cueType, targetCueId, filePath).catch(console.error);
      } else {
        // Type mismatch — insert a new cue at the end instead.
        const newId = await addCue(cueType, -1).catch(() => null);
        if (newId) {
          await setFileForCue(cueType, newId, filePath).catch(console.error);
          await updateCue(newId, { name: basenameNoExt(filePath) }).catch(console.error);
          setSelectedCueId(newId);
        }
      }
    } else {
      const newId = await addCue(cueType, -1).catch(() => null);
      if (newId) {
        await setFileForCue(cueType, newId, filePath).catch(console.error);
        await updateCue(newId, { name: basenameNoExt(filePath) }).catch(console.error);
        setSelectedCueId(newId);
      }
    }
    await onRefresh();
  }

  // ---------- Cue context menu ----------
  const closeCtx = () => setContextMenu(null);

  const ctxAddAudio  = async () => { closeCtx(); await addCue("audio", -1).catch(console.error); await onRefresh(); };
  const ctxAddAbove  = async () => {
    closeCtx();
    if (!contextMenu?.cueId) return;
    const idx = cuesRef.current.findIndex((c) => c.id === contextMenu.cueId);
    if (idx >= 0) { await addCue("audio", idx).catch(console.error); await onRefresh(); }
  };
  const ctxAddBelow  = async () => {
    closeCtx();
    if (!contextMenu?.cueId) return;
    const idx = cuesRef.current.findIndex((c) => c.id === contextMenu.cueId);
    if (idx >= 0) { await addCue("audio", idx + 1).catch(console.error); await onRefresh(); }
  };
  const ctxDuplicate = async () => {
    closeCtx();
    if (!contextMenu?.cueId) return;
    await duplicateCue(contextMenu.cueId).catch(console.error);
    await onRefresh();
  };
  const ctxDelete    = async () => {
    closeCtx();
    if (!contextMenu?.cueId) return;
    await removeCue(contextMenu.cueId).catch(console.error);
    await onRefresh();
  };
  const ctxAssignFile = async () => {
    closeCtx();
    if (!contextMenu?.cueId) return;
    const result = await open({
      multiple: false,
      filters: [{ name: "Audio Files", extensions: ["wav", "mp3", "flac", "ogg", "aac"] }],
    });
    if (typeof result === "string") {
      await setAudioFile(contextMenu.cueId, result).catch(console.error);
      await onRefresh();
    }
  };

  // ---------- Keyboard navigation ----------
  function handleKeyDown(e: React.KeyboardEvent) {
    if (cues.length === 0) return;
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;

    e.preventDefault();

    // The "moving end" for arrow navigation is selectionEndRef when set (for
    // Shift+Arrow continuation), otherwise fall back to selectedCueId.
    const endId = selectionEndRef.current ?? selectedCueId;
    if (!endId) return;
    const endIdx = cues.findIndex((c) => c.id === endId);
    if (endIdx < 0) return;
    const nextIdx = e.key === "ArrowDown" ? endIdx + 1 : endIdx - 1;
    if (nextIdx < 0 || nextIdx >= cues.length) return;
    const nextId = cues[nextIdx].id;

    if (e.shiftKey) {
      // Extend / shrink the range from anchor to the new end.
      const anchorId = anchorCueIdRef.current ?? endId;
      const anchorIdx = cues.findIndex((c) => c.id === anchorId);
      const effAnchor = anchorIdx >= 0 ? anchorIdx : endIdx;
      const [lo, hi] = effAnchor <= nextIdx
        ? [effAnchor, nextIdx]
        : [nextIdx, effAnchor];
      setSelectedCueIds(cues.slice(lo, hi + 1).map((c) => c.id));
      selectionEndRef.current = nextId;
    } else {
      // Plain arrow: single-select, update both anchor and end.
      setSelectedCueId(nextId);
      setSelectedCueIds([nextId]);
      anchorCueIdRef.current = nextId;
      selectionEndRef.current = nextId;
    }
  }

  // ---------------------------------------------------------------------------
  // Render
  // ---------------------------------------------------------------------------

  // Common grid style used by both header and rows.
  const gridStyle: React.CSSProperties = {
    display: "grid",
    gridTemplateColumns: gridCols,
    gap: "0 8px",
    padding: "0 8px",
    // min-width: max-content ensures pixel-based columns never get squeezed when
    // the container is narrower than the total column width. The scroll containers
    // handle the overflow.
    minWidth: "max-content",
  };

  return (
    <div
      style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0, outline: "none", position: "relative" }}
      tabIndex={0}
      onKeyDown={handleKeyDown}
      onContextMenu={(e) => { e.preventDefault(); setContextMenu({ x: e.clientX, y: e.clientY, cueId: null }); }}
      onDragOver={(e) => e.preventDefault()}
      onDrop={(e) => e.preventDefault()}
    >
      {/* ── Column header (scrollbar hidden, scroll driven by rows) ───────── */}
      <div
        ref={headerScrollRef}
        className="no-scrollbar"
        style={{
          overflowX: "scroll",
          overflowY: "hidden",
          flexShrink: 0,
          background: "var(--wc-bg-app)",
          borderBottom: "2px solid var(--wc-border-strong)",
        }}
        onContextMenu={(e) => {
          e.preventDefault();
          e.stopPropagation();
          setContextMenu(null);
          setColMenuPos({ x: e.clientX, y: e.clientY });
        }}
      >
        <div
          style={{
            ...gridStyle,
            height: 28,
            alignItems: "center",
            fontSize: 11,
            color: "var(--wc-text-muted)",
            textTransform: "uppercase",
            letterSpacing: "0.05em",
            userSelect: "none",
          }}
        >
          {visibleDefs.map((def, i) => (
            <div
              key={def.id}
              data-col-id={def.id}
              title={def.fixed ? undefined : "Drag to reorder · Right-click for options"}
              style={{
                position: "relative",
                display: "flex",
                alignItems: "center",
                height: "100%",
                minWidth: 0,
                opacity: draggingColId === def.id ? 0.4 : 1,
                cursor: def.fixed ? "default" : "grab",
                transition: "opacity 0.1s",
                borderRight: i < visibleDefs.length - 1
                  ? `1px solid ${hoveredResizeId === def.id ? "var(--wc-text-faint)" : "var(--wc-border)"}`
                  : undefined,
              }}
              onMouseDown={(e) => startColDrag(e, def)}
            >
              <span
                style={{
                  flex: 1,
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                  paddingLeft: 5,
                  paddingRight: 5,
                  pointerEvents: "none",
                }}
              >
                {def.label}
              </span>

              {/* Resize handle — 8 px wide, centred on the right border.
                  Invisible zone (cursor-only feedback); border colour shifts on hover. */}
              {def.resizable && i < visibleDefs.length - 1 && (
                <div
                  data-resize="true"
                  style={{
                    position: "absolute",
                    right: -4,
                    top: 0,
                    bottom: 0,
                    width: 8,
                    cursor: "col-resize",
                    zIndex: 10,
                  }}
                  onMouseDown={(e) => startResize(e, def)}
                  onMouseEnter={() => setHoveredResizeId(def.id)}
                  onMouseLeave={() => setHoveredResizeId(null)}
                />
              )}
            </div>
          ))}
        </div>
      </div>

      {/* ── Cue rows (scrolls both axes; drives header horizontal sync) ────── */}
      <div
        ref={rowsScrollRef}
        style={{ flex: 1, overflow: "auto" }}
        onClick={(e) => {
          setContextMenu(null);
          setColMenuPos(null);
          // Clear selection when clicking on empty space (not on a cue row).
          if (!(e.target as HTMLElement).closest("[data-cue-id]")) {
            setSelectedCueId(null);
            setSelectedCueIds([]);
            anchorCueIdRef.current = null;
            selectionEndRef.current = null;
          }
        }}
        onScroll={(e) => {
          if (headerScrollRef.current) {
            headerScrollRef.current.scrollLeft = e.currentTarget.scrollLeft;
          }
        }}
      >
        {cues.length === 0 && (
          <div style={{ padding: 32, textAlign: "center", color: "var(--wc-text-faint)", fontSize: 14 }}>
            No cues. Press Ctrl+N or drag an audio file here.
          </div>
        )}

        {flatItems.map(({ cue, depth, parentGroupId }, flatIndex) => (
          <Fragment key={cue.id}>
            {/* Drop-target indicator ABOVE (file insert, cue reorder, new-cue drag) */}
            {isDragging && fileDragInsertIdx === flatIndex && (
              <div style={{ height: 2, background: "var(--wc-accent)", margin: `0 ${8 + depth * 20}px`, borderRadius: 1, pointerEvents: "none" }} />
            )}
            {draggingCueId !== null && dropInsertIndex === flatIndex && (
              <div style={{
                height: 2, background: "var(--wc-accent)",
                margin: `0 ${8 + depth * 20}px`,
                borderRadius: 1, pointerEvents: "none",
              }} />
            )}
            {newCueDragType !== null && newCueDragInsertIdx === flatIndex && (
              <div style={{ height: 2, background: "#ef4444", margin: "0 8px", borderRadius: 1, pointerEvents: "none" }} />
            )}
            <CueRow
              cue={cue}
              cueIndex={flatIndex}
              gridStyle={gridStyle}
              visibleDefs={visibleDefs}
              rowHeight={rowHeight}
              cueColorStyle={displayPrefs.cue_color_style}
              depth={depth}
              isGroup={cue.cue_type === "group"}
              isGroupExpanded={expandedGroupIds.has(cue.id)}
              onToggleExpand={() => toggleGroupExpand(cue.id)}
              isSelected={selectedCueSet.has(cue.id)}
              isAtPlayhead={playheadCueId === cue.id || innerPlayheadIds.has(cue.id)}
              isDragOver={dragOverCueId === cue.id}
              isGroupDropTarget={
                cue.cue_type === "group" &&
                (dropTargetGroupId === cue.id || dragOverGroupId === cue.id)
              }
              parentGroupId={parentGroupId}
              isDragSource={
                draggingCueId !== null &&
                (cueDragRef.current?.ids.includes(cue.id) ?? false)
              }
              onCueDragStart={(e) => {
                startCueDrag(e, cue.id, flatIndex);
              }}
              onClick={(e) => {
                if (justDroppedRef.current) return;
                setContextMenu(null);

                if (e.shiftKey && anchorCueIdRef.current) {
                  const anchorIdx = flatItems.findIndex((fi) => fi.cue.id === anchorCueIdRef.current);
                  const [lo, hi] = anchorIdx <= flatIndex
                    ? [anchorIdx, flatIndex]
                    : [flatIndex, anchorIdx];
                  setSelectedCueIds(flatItems.slice(lo, hi + 1).map((fi) => fi.cue.id));
                  selectionEndRef.current = cue.id;
                } else if (e.ctrlKey) {
                  const next = selectedCueSet.has(cue.id)
                    ? selectedCueIds.filter((id) => id !== cue.id)
                    : [...selectedCueIds, cue.id];
                  setSelectedCueIds(next);
                  setSelectedCueId(cue.id);
                  anchorCueIdRef.current = cue.id;
                  selectionEndRef.current = cue.id;
                } else {
                  setSelectedCueId(cue.id);
                  setSelectedCueIds([cue.id]);
                  // Park the Playhead on this exact cue. The backend routes a
                  // child of a Sequential group to its ancestor group and points
                  // the group's inner sequence at the child, so GO fires it.
                  // Optimistically reflect the outer Playhead (the group for a child).
                  setPlayheadCueId(parentGroupId ?? cue.id);
                  setPlayhead(cue.id).catch(console.error);
                  anchorCueIdRef.current = cue.id;
                  selectionEndRef.current = cue.id;
                }
              }}
              onDoubleClick={() => onCueDoubleClick(cue)}
              onContextMenu={(e) => {
                e.preventDefault();
                e.stopPropagation();
                setContextMenu({ x: e.clientX, y: e.clientY, cueId: cue.id, parentGroupId });
              }}
              onStop={(id) => stopCue(id).catch(console.error)}
            />
          </Fragment>
        ))}

        {/* Drop-target indicators AFTER the last row */}
        {isDragging && fileDragInsertIdx === flatItems.length && (
          <div style={{ height: 2, background: "var(--wc-accent)", margin: "0 8px", borderRadius: 1, pointerEvents: "none" }} />
        )}
        {draggingCueId !== null && dropInsertIndex === flatItems.length && (
          <div style={{ height: 2, background: "var(--wc-accent)", margin: "0 8px", borderRadius: 1, pointerEvents: "none" }} />
        )}
        {/* Drop-target indicator line AFTER the last row (new-cue drag from toolbar) */}
        {newCueDragType !== null && newCueDragInsertIdx === cues.length && (
          <div
            style={{
              height: 2,
              background: "#ef4444",
              margin: "0 8px",
              borderRadius: 1,
              pointerEvents: "none",
            }}
          />
        )}

        {isDragging && !dragOverCueId && fileDragInsertIdx === null && (
          <div
            style={{
              margin: "8px 16px",
              border: "2px dashed var(--wc-accent)",
              borderRadius: 6,
              padding: 16,
              textAlign: "center",
              color: "var(--wc-accent)",
              fontSize: 13,
              pointerEvents: "none",
            }}
          >
            Drop to create new Audio Cue
          </div>
        )}
      </div>

      {/* ── Column visibility menu ────────────────────────────────────────── */}
      {colMenuPos && (
        <ColumnMenu
          config={colConfig}
          pos={colMenuPos}
          onToggle={(id, visible) =>
            setColConfig((prev) => ({ ...prev, hidden: { ...prev.hidden, [id]: !visible } }))
          }
          onClose={() => setColMenuPos(null)}
        />
      )}

      {/* ── Cue context menu ──────────────────────────────────────────────── */}
      {contextMenu && (
        <>
          <div
            style={{ position: "fixed", inset: 0, zIndex: 9998 }}
            onClick={closeCtx}
            onContextMenu={(e) => { e.preventDefault(); closeCtx(); }}
          />
          <div
            style={{
              position: "fixed",
              left: contextMenu.x,
              top: contextMenu.y,
              background: "var(--wc-bg-surface)",
              border: "1px solid var(--wc-border-strong)",
              borderRadius: 6,
              padding: "4px 0",
              zIndex: 9999,
              minWidth: 200,
              boxShadow: "0 4px 16px rgba(0,0,0,0.6)",
              fontSize: 13,
            }}
          >
            {contextMenu.cueId ? (
              <>
                {!contextMenu.parentGroupId && (
                  <>
                    <CtxItem label="Add Audio Cue Above" onClick={ctxAddAbove} />
                    <CtxItem label="Add Audio Cue Below" onClick={ctxAddBelow} />
                    <div style={{ height: 1, background: "var(--wc-border-strong)", margin: "4px 0" }} />
                  </>
                )}
                <CtxItem label="Duplicate" onClick={ctxDuplicate} />
                <CtxItem label="Delete" danger onClick={ctxDelete} />
                {/* Group / ungroup */}
                {!contextMenu.parentGroupId && (() => {
                  const ids = selectedCueIds.length > 1 && contextMenu.cueId && selectedCueIds.includes(contextMenu.cueId)
                    ? selectedCueIds
                    : contextMenu.cueId ? [contextMenu.cueId] : [];
                  const label = ids.length > 1 ? `Group ${ids.length} Cues` : "Group Cue";
                  return ids.length > 0 ? (
                    <>
                      <div style={{ height: 1, background: "var(--wc-border-strong)", margin: "4px 0" }} />
                      <CtxItem
                        label={label}
                        onClick={async () => {
                          closeCtx();
                          const newGroupId = await groupCues(ids).catch(() => null);
                          if (newGroupId) {
                            setSelectedCueId(newGroupId);
                            setSelectedCueIds([newGroupId]);
                            await onRefresh();
                          }
                        }}
                      />
                    </>
                  ) : null;
                })()}
                {/* Group-specific actions */}
                {(() => {
                  const cueItem = flatItems.find(fi => fi.cue.id === contextMenu.cueId);
                  const isGroup = cueItem?.cue.cue_type === "group";
                  const inGroup = !!contextMenu.parentGroupId;
                  return (
                    <>
                      {isGroup && (
                        <>
                          <div style={{ height: 1, background: "var(--wc-border-strong)", margin: "4px 0" }} />
                          <CtxItem
                            label="Ungroup"
                            onClick={async () => {
                              closeCtx();
                              if (!contextMenu.cueId) return;
                              await ungroup(contextMenu.cueId).catch(console.error);
                              await onRefresh();
                            }}
                          />
                        </>
                      )}
                      {inGroup && (
                        <>
                          <div style={{ height: 1, background: "var(--wc-border-strong)", margin: "4px 0" }} />
                          <CtxItem
                            label="Remove from Group"
                            onClick={async () => {
                              closeCtx();
                              if (!contextMenu.cueId || !contextMenu.parentGroupId) return;
                              // Remove all selected cues that share this parent group,
                              // or just the right-clicked cue if it's not in the selection.
                              const groupId = contextMenu.parentGroupId;
                              const targets =
                                selectedCueIds.includes(contextMenu.cueId)
                                  ? selectedCueIds.filter((id) => {
                                      const fi = flatItems.find((f) => f.cue.id === id);
                                      return fi?.parentGroupId === groupId;
                                    })
                                  : [contextMenu.cueId];
                              await Promise.all(
                                targets.map((id) => removeCueFromGroup(groupId, id).catch(console.error))
                              );
                              await onRefresh();
                            }}
                          />
                        </>
                      )}
                    </>
                  );
                })()}
                {!contextMenu.parentGroupId && (
                  <>
                    <div style={{ height: 1, background: "var(--wc-border-strong)", margin: "4px 0" }} />
                    <CtxItem label="Assign Audio File…" onClick={ctxAssignFile} />
                  </>
                )}
              </>
            ) : (
              <CtxItem label="Add Audio Cue" onClick={ctxAddAudio} />
            )}
          </div>
        </>
      )}
    </div>
  );
}
