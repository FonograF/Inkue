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
  moveCue,
  setAudioFile,
  setPlayhead,
  updateCue,
} from "../../lib/commands";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const AUDIO_EXTS = new Set(["wav", "mp3", "flac", "ogg", "aac", "m4a"]);

function isAudioPath(p: string) {
  return AUDIO_EXTS.has(p.split(".").pop()?.toLowerCase() ?? "");
}
function basenameNoExt(p: string) {
  return (p.split(/[\\/]/).pop() ?? p).replace(/\.[^.]+$/, "");
}

// ---------------------------------------------------------------------------
// Cue context-menu item
// ---------------------------------------------------------------------------

function CtxItem({ label, danger, onClick }: { label: string; danger?: boolean; onClick: () => void }) {
  const [hov, setHov] = useState(false);
  return (
    <button
      style={{
        display: "block", width: "100%", padding: "6px 16px",
        background: hov ? "#334155" : "transparent", border: "none",
        textAlign: "left", color: danger ? "#ef4444" : "#e2e8f0",
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
          background: "#1e293b", border: "1px solid #334155",
          borderRadius: 6, padding: "6px 0", zIndex: 9999,
          minWidth: menuW, boxShadow: "0 4px 16px rgba(0,0,0,0.6)",
        }}
      >
        <div style={{ padding: "2px 12px 6px", fontSize: 10, color: "#64748b", textTransform: "uppercase", letterSpacing: "0.05em" }}>
          Columns
        </div>
        {items.map((d) => (
          <label
            key={d.id}
            style={{
              display: "flex", alignItems: "center", gap: 8,
              padding: "5px 12px", cursor: "pointer",
              fontSize: 12, color: "#e2e8f0", userSelect: "none",
            }}
          >
            <input
              type="checkbox"
              checked={!config.hidden[d.id]}
              onChange={(e) => onToggle(d.id, e.target.checked)}
              style={{ accentColor: "#3b82f6" }}
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
  const { cues, selectedCueId, playheadCueId, setSelectedCueId, setPlayheadCueId } =
    useWorkspaceStore();

  // ---------- Column config ----------
  const [colConfig, setColConfig] = useState<ColumnConfig>(loadColumnConfig);
  const [colMenuPos, setColMenuPos] = useState<{ x: number; y: number } | null>(null);
  const [draggingColId, setDraggingColId] = useState<ColumnId | null>(null);

  useEffect(() => { saveColumnConfig(colConfig); }, [colConfig]);

  const visibleDefs = useMemo(() => getVisibleDefs(colConfig), [colConfig]);
  const gridCols    = useMemo(() => buildGridCols(visibleDefs, colConfig), [visibleDefs, colConfig]);

  const setColConfigRef = useRef(setColConfig);
  setColConfigRef.current = setColConfig;

  // ---------- Cue drag-and-drop state ----------
  const [draggingCueId,   setDraggingCueId]   = useState<string | null>(null);
  const [dropInsertIndex, setDropInsertIndex]  = useState<number | null>(null);

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
  const cueDragRef = useRef<{
    id: string;
    fromIndex: number;
    startX: number;
    startY: number;
    active: boolean;
    dropIdx: number | null;
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
    // Scan all visible cue rows by midpoint to find the correct insert index.
    // More robust than elementFromPoint: works between rows, over scrollbar, etc.
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
      return cuesRef.current.length;
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
        const newDrop = calcInsertIdxFromY(e.clientY);
        drag.dropIdx = newDrop;
        setDropInsertIndex(newDrop);
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
        if (drag.active && drag.dropIdx !== null) {
          // Compute the insertion index in the post-removal array.
          const from   = cuesRef.current.findIndex((c) => c.id === drag.id);
          const newPos = from < drag.dropIdx ? drag.dropIdx - 1 : drag.dropIdx;
          if (from >= 0 && newPos !== from) {
            moveCue(drag.id, newPos).then(onRefresh).catch(console.error);
          }
          // Suppress the spurious onClick that fires after mouseup.
          justDroppedRef.current = true;
          setTimeout(() => { justDroppedRef.current = false; }, 0);
        }
        cueDragRef.current = null;
        document.body.style.cursor = "";
        setDraggingCueId(null);
        setDropInsertIndex(null);
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
    cueDragRef.current = {
      id: cueId,
      fromIndex: index,
      startX: e.clientX,
      startY: e.clientY,
      active: false,
      dropIdx: null,
    };
  }

  // ---------- File drag-drop state ----------
  const [dragOverCueId,    setDragOverCueId]    = useState<string | null>(null);
  const [isDragging,       setIsDragging]       = useState(false);
  // When a file is dragged in insert-between mode (cursor near row edge),
  // this holds the insertion index; dragOverCueId is null in that case.
  const [fileDragInsertIdx, setFileDragInsertIdx] = useState<number | null>(null);
  const [contextMenu,   setContextMenu]   = useState<{ x: number; y: number; cueId: string | null } | null>(null);

  const cuesRef = useRef(cues);
  useEffect(() => { cuesRef.current = cues; }, [cues]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    (async () => {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      // Tauri drag-drop positions are in physical (DPI-scaled) pixels.
      // Convert to logical CSS pixels before comparing with getBoundingClientRect().
      // Cursor in the top/bottom 8 logical px of a row → insert line.
      // Cursor in the middle of a row → assign/replace that cue.
      const EDGE_PX = 8;
      function resolveFileDragMode(_physX: number, physY: number): { insertIdx: number | null; assignId: string | null } {
        const dpr = window.devicePixelRatio || 1;
        const py  = physY / dpr;
        const rowEls = rowsScrollRef.current
          ? (Array.from(rowsScrollRef.current.querySelectorAll("[data-cue-id]")) as HTMLElement[])
          : [];
        for (const el of rowEls) {
          const rect = el.getBoundingClientRect();
          if (py < rect.top) {
            // Cursor is above this row (gap between previous and this row).
            return { insertIdx: Number(el.dataset.cueIndex ?? 0), assignId: null };
          }
          if (py < rect.bottom) {
            // Cursor is inside this row.
            const idx = Number(el.dataset.cueIndex ?? -1);
            const id  = el.dataset.cueId ?? null;
            if (py - rect.top  < EDGE_PX) return { insertIdx: idx >= 0 ? idx     : 0,                      assignId: null };
            if (rect.bottom - py < EDGE_PX) return { insertIdx: idx >= 0 ? idx + 1 : cuesRef.current.length, assignId: null };
            return { insertIdx: null, assignId: id };
          }
        }
        // Below all rows → append at end.
        return { insertIdx: cuesRef.current.length, assignId: null };
      }

      const fn_ = await getCurrentWindow().onDragDropEvent(async (event) => {
        const { type } = event.payload;
        if (type === "enter" || type === "over") {
          setIsDragging(true);
          const pos = event.payload.position;
          if (pos) {
            const { insertIdx, assignId } = resolveFileDragMode(pos.x, pos.y);
            setFileDragInsertIdx(insertIdx);
            setDragOverCueId(assignId);
          }
        } else if (type === "leave") {
          setIsDragging(false);
          setDragOverCueId(null);
          setFileDragInsertIdx(null);
        } else if (type === "drop") {
          setIsDragging(false);
          setDragOverCueId(null);
          setFileDragInsertIdx(null);
          const paths: string[] = (event.payload as { paths?: string[] }).paths ?? [];
          const pos = event.payload.position;
          const audioPaths = paths.filter(isAudioPath);
          if (audioPaths.length === 0) return;

          const { insertIdx, assignId } = pos
            ? resolveFileDragMode(pos.x, pos.y)
            : { insertIdx: null, assignId: null };

          if (insertIdx !== null) {
            // Insert mode: create new cue(s) at the target position.
            let at = insertIdx;
            for (const p of audioPaths) {
              const newId = await addCue("audio", at).catch(() => null);
              if (newId) {
                await setAudioFile(newId, p).catch(console.error);
                await updateCue(newId, { name: basenameNoExt(p) }).catch(console.error);
                setSelectedCueId(newId);
                at++;
              }
            }
            await onRefresh();
          } else if (audioPaths.length === 1) {
            await handleFileDrop(audioPaths[0], assignId);
          } else {
            for (const p of audioPaths) await handleFileDrop(p, null);
          }
        }
      });
      if (cancelled) fn_(); else unlisten = fn_;
    })().catch(console.error);

    return () => { cancelled = true; unlisten?.(); };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  async function handleFileDrop(filePath: string, targetCueId: string | null) {
    if (targetCueId) {
      await setAudioFile(targetCueId, filePath).catch(console.error);
    } else {
      const newId = await addCue("audio", -1).catch(() => null);
      if (newId) {
        await setAudioFile(newId, filePath).catch(console.error);
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
    if (!selectedCueId || cues.length === 0) return;
    const idx = cues.findIndex((c) => c.id === selectedCueId);
    if (e.key === "ArrowDown" && idx < cues.length - 1) setSelectedCueId(cues[idx + 1].id);
    else if (e.key === "ArrowUp" && idx > 0)            setSelectedCueId(cues[idx - 1].id);
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
      style={{ display: "flex", flexDirection: "column", height: "100%", outline: "none", position: "relative" }}
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
          background: "#0f172a",
          borderBottom: "2px solid #334155",
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
            color: "#64748b",
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
                borderLeft: i > 0 ? "1px solid #1e293b" : undefined,
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
                  paddingRight: def.resizable ? 10 : 5,
                  pointerEvents: "none",
                }}
              >
                {def.label}
              </span>

              {/* Resize handle — 8 px wide, centred on the right column boundary */}
              {def.resizable && (
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
                    display: "flex",
                    alignItems: "stretch",
                    justifyContent: "center",
                  }}
                  onMouseDown={(e) => startResize(e, def)}
                >
                  <div
                    style={{
                      width: 1,
                      background: "#475569",
                      alignSelf: "stretch",
                      margin: "4px 0",
                    }}
                  />
                </div>
              )}
            </div>
          ))}
        </div>
      </div>

      {/* ── Cue rows (scrolls both axes; drives header horizontal sync) ────── */}
      <div
        ref={rowsScrollRef}
        style={{ flex: 1, overflow: "auto" }}
        onClick={() => { setContextMenu(null); setColMenuPos(null); }}
        onScroll={(e) => {
          if (headerScrollRef.current) {
            headerScrollRef.current.scrollLeft = e.currentTarget.scrollLeft;
          }
        }}
      >
        {cues.length === 0 && (
          <div style={{ padding: 32, textAlign: "center", color: "#475569", fontSize: 14 }}>
            No cues. Press Ctrl+N or drag an audio file here.
          </div>
        )}

        {cues.map((cue, index) => (
          <Fragment key={cue.id}>
            {/* Drop-target indicator line ABOVE this row (file insert) */}
            {isDragging && fileDragInsertIdx === index && (
              <div
                style={{
                  height: 2,
                  background: "#3b82f6",
                  margin: "0 8px",
                  borderRadius: 1,
                  pointerEvents: "none",
                }}
              />
            )}
            {/* Drop-target indicator line ABOVE this row (cue reorder) */}
            {draggingCueId !== null && dropInsertIndex === index && (
              <div
                style={{
                  height: 2,
                  background: "#3b82f6",
                  margin: "0 8px",
                  borderRadius: 1,
                  pointerEvents: "none",
                }}
              />
            )}
            {/* Drop-target indicator line ABOVE this row (new-cue drag from toolbar) */}
            {newCueDragType !== null && newCueDragInsertIdx === index && (
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
            <CueRow
              cue={cue}
              cueIndex={index}
              gridStyle={gridStyle}
              visibleDefs={visibleDefs}
              isSelected={selectedCueId === cue.id}
              isAtPlayhead={playheadCueId === cue.id}
              isDragOver={dragOverCueId === cue.id}
              isDragSource={draggingCueId === cue.id}
              onCueDragStart={(e) => startCueDrag(e, cue.id, index)}
              onClick={() => {
                if (justDroppedRef.current) return;
                setSelectedCueId(cue.id);
                setPlayheadCueId(cue.id);
                setPlayhead(cue.id).catch(console.error);
                setContextMenu(null);
              }}
              onDoubleClick={() => onCueDoubleClick(cue)}
              onContextMenu={(e) => {
                e.preventDefault();
                e.stopPropagation();
                setContextMenu({ x: e.clientX, y: e.clientY, cueId: cue.id });
              }}
            />
          </Fragment>
        ))}

        {/* Drop-target indicator line AFTER the last row (file insert) */}
        {isDragging && fileDragInsertIdx === cues.length && (
          <div
            style={{
              height: 2,
              background: "#3b82f6",
              margin: "0 8px",
              borderRadius: 1,
              pointerEvents: "none",
            }}
          />
        )}
        {/* Drop-target indicator line AFTER the last row (cue reorder) */}
        {draggingCueId !== null && dropInsertIndex === cues.length && (
          <div
            style={{
              height: 2,
              background: "#3b82f6",
              margin: "0 8px",
              borderRadius: 1,
              pointerEvents: "none",
            }}
          />
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
              border: "2px dashed #3b82f6",
              borderRadius: 6,
              padding: 16,
              textAlign: "center",
              color: "#3b82f6",
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
              background: "#1e293b",
              border: "1px solid #334155",
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
                <CtxItem label="Add Audio Cue Above" onClick={ctxAddAbove} />
                <CtxItem label="Add Audio Cue Below" onClick={ctxAddBelow} />
                <div style={{ height: 1, background: "#334155", margin: "4px 0" }} />
                <CtxItem label="Duplicate" onClick={ctxDuplicate} />
                <CtxItem label="Delete" danger onClick={ctxDelete} />
                <div style={{ height: 1, background: "#334155", margin: "4px 0" }} />
                <CtxItem label="Assign Audio File…" onClick={ctxAssignFile} />
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
