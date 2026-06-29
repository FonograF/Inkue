// Tab bar for switching between multiple cue lists.

import { useState, useRef, useEffect } from "react";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import {
  addCueList, removeCueList, renameCueList, setActiveCueList, setCueListMode,
  getCuelistTcConfig, setCuelistTcConfig,
} from "../../lib/commands";
import type { CueListTcConfig, TcRate, TcOnStop } from "../../lib/types";
import { Select } from "../common/Select";

export function CueListTabs({ onRefresh }: { onRefresh: () => void }) {
  const { cueLists, activeCueListId, refreshCueLists } = useWorkspaceStore();

  const [renamingId, setRenamingId]   = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; id: string } | null>(null);
  const [addBtnHovered, setAddBtnHovered] = useState(false);
  const renameInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (renamingId) renameInputRef.current?.focus();
  }, [renamingId]);

  // Close context menu on outside click.
  useEffect(() => {
    if (!contextMenu) return;
    const close = () => setContextMenu(null);
    window.addEventListener("click", close);
    return () => window.removeEventListener("click", close);
  }, [contextMenu]);

  const handleSwitchList = async (id: string) => {
    if (id === activeCueListId) return;
    await setActiveCueList(id).catch(console.error);
    onRefresh();
  };

  const handleAddList = async () => {
    const name = `Cue List ${cueLists.length + 1}`;
    await addCueList(name).catch(console.error);
    await refreshCueLists();
    onRefresh();
  };

  const handleRemoveList = async (id: string) => {
    if (cueLists.length <= 1) return;
    await removeCueList(id).catch(console.error);
    await refreshCueLists();
    onRefresh();
  };

  const handleToggleMode = async (id: string) => {
    const list = cueLists.find((l) => l.id === id);
    if (!list) return;
    const next = list.mode === "cart" ? "sequential" : "cart";
    await setCueListMode(id, next).catch(console.error);
    await refreshCueLists();
    setContextMenu(null);
  };

  const startRename = (id: string, currentName: string) => {
    setRenamingId(id);
    setRenameValue(currentName);
    setContextMenu(null);
  };

  const commitRename = async () => {
    if (!renamingId) return;
    const trimmed = renameValue.trim();
    if (trimmed) {
      await renameCueList(renamingId, trimmed).catch(console.error);
      await refreshCueLists();
    }
    setRenamingId(null);
  };

  const handleRenameKey = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") { e.preventDefault(); void commitRename(); }
    if (e.key === "Escape") { e.preventDefault(); setRenamingId(null); }
  };

  return (
    <div
      style={{
        display: "flex", alignItems: "center", height: 30,
        background: "var(--wc-bg-app)",
        borderBottom: "1px solid var(--wc-border)",
        flexShrink: 0, gap: 1, paddingLeft: 4, paddingRight: 4,
        overflowX: "auto", overflowY: "hidden",
      }}
    >
      {cueLists.map((list) => {
        const isActive = list.id === activeCueListId;
        const isRenaming = list.id === renamingId;

        return (
          <div
            key={list.id}
            onContextMenu={(e) => {
              e.preventDefault();
              e.stopPropagation();
              setContextMenu({ x: e.clientX, y: e.clientY, id: list.id });
            }}
            onClick={() => !isRenaming && void handleSwitchList(list.id)}
            onDoubleClick={() => startRename(list.id, list.name)}
            style={{
              display: "flex", alignItems: "center",
              padding: "0 10px", height: 26, borderRadius: 4,
              cursor: "pointer", flexShrink: 0, userSelect: "none",
              background: isActive ? "var(--wc-bg-surface)" : "transparent",
              color: isActive ? "var(--wc-text)" : "var(--wc-text-muted)",
              border: isActive ? "1px solid var(--wc-border-strong)" : "1px solid transparent",
              fontSize: 12, fontWeight: isActive ? 600 : 400,
              minWidth: 80,
            }}
          >
            {isRenaming ? (
              <input
                ref={renameInputRef}
                value={renameValue}
                onChange={(e) => setRenameValue(e.target.value)}
                onBlur={() => void commitRename()}
                onKeyDown={handleRenameKey}
                onClick={(e) => e.stopPropagation()}
                style={{
                  background: "transparent", border: "none", outline: "none",
                  color: "inherit", fontSize: "inherit", fontWeight: "inherit",
                  width: "100%", padding: 0,
                }}
              />
            ) : (
              <span style={{ display: "flex", alignItems: "center", gap: 4, overflow: "hidden" }}>
                <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", maxWidth: 120 }}>
                  {list.name}
                </span>
                {list.mode === "cart" && (
                  <span style={{
                    fontSize: 9, fontWeight: 700, letterSpacing: "0.04em",
                    color: isActive ? "var(--wc-accent)" : "var(--wc-text-faint)",
                    border: `1px solid ${isActive ? "var(--wc-accent)" : "var(--wc-border)"}`,
                    borderRadius: 3, padding: "0 3px", lineHeight: "14px",
                    flexShrink: 0,
                  }}>
                    CART
                  </span>
                )}
              </span>
            )}
          </div>
        );
      })}

      {/* Add cue list button */}
      <button
        onClick={() => void handleAddList()}
        title="Add Cue List"
        style={{
          background: "transparent", border: "none",
          color: addBtnHovered ? "var(--wc-text-secondary)" : "var(--wc-text-faint)",
          cursor: "pointer", fontSize: 16, lineHeight: 1,
          padding: "0 6px", height: 26, borderRadius: 4, flexShrink: 0,
          display: "flex", alignItems: "center",
        }}
        onMouseEnter={() => setAddBtnHovered(true)}
        onMouseLeave={() => setAddBtnHovered(false)}
      >
        +
      </button>

      {/* Per-list timecode sync */}
      <CueListTcSync activeCueListId={activeCueListId} />

      {/* Context menu */}
      {contextMenu && (
        <div
          style={{
            position: "fixed", left: contextMenu.x, top: contextMenu.y,
            background: "var(--wc-bg-surface)", border: "1px solid var(--wc-border-strong)", borderRadius: 6,
            padding: "4px 0", zIndex: 9999, minWidth: 160,
            boxShadow: "0 8px 24px rgba(0,0,0,0.7)",
          }}
          onClick={(e) => e.stopPropagation()}
        >
          <ContextMenuItem
            label="Rename…"
            onClick={() => {
              const list = cueLists.find((l) => l.id === contextMenu.id);
              if (list) startRename(list.id, list.name);
            }}
          />
          <ContextMenuItem
            label={cueLists.find((l) => l.id === contextMenu.id)?.mode === "cart"
              ? "Switch to Sequential Mode"
              : "Switch to Cart Mode"}
            onClick={() => void handleToggleMode(contextMenu.id)}
          />
          <div style={{ height: 1, background: "var(--wc-border-strong)", margin: "4px 0" }} />
          <ContextMenuItem
            label="Delete Cue List"
            danger
            disabled={cueLists.length <= 1}
            onClick={() => void handleRemoveList(contextMenu.id)}
          />
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Per-cue-list timecode sync control (lives at the right of the tab bar).
// Gates whether incoming TC fires this list's cues (event_loop dispatcher reads
// CueList.tc_config.enabled). Edits the *active* cue list via get/setCuelistTcConfig.
// ---------------------------------------------------------------------------

const TC_RATES: TcRate[] = ["24", "25", "29.97", "29.97df", "30"];
const TC_RATE_LABELS: Record<TcRate, string> = {
  "24": "24 fps", "25": "25 fps (PAL)", "29.97": "29.97 fps",
  "29.97df": "29.97df (NTSC DF)", "30": "30 fps",
};
const ON_STOP_LABELS: Record<TcOnStop, string> = {
  continue: "Keep cues running", pause: "Pause running cues", stop: "Stop running cues",
};

const tcFieldLabel: React.CSSProperties = { fontSize: 10, color: "var(--wc-text-muted)", marginBottom: 3 };
const tcInputStyle: React.CSSProperties = {
  background: "var(--wc-bg-app)", border: "1px solid var(--wc-border-strong)",
  borderRadius: 4, color: "var(--wc-text)", fontSize: 12, padding: "4px 6px",
  width: "100%", boxSizing: "border-box",
};

function CueListTcSync({ activeCueListId }: { activeCueListId: string | null }) {
  const [cfg, setCfg] = useState<CueListTcConfig | null>(null);
  const [open, setOpen] = useState(false);
  const [anchor, setAnchor] = useState<{ x: number; y: number } | null>(null);
  const btnRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    getCuelistTcConfig()
      .then((c) => setCfg(c ?? { enabled: false, rate: "30", freewheel_ms: 500, on_stop: "continue" }))
      .catch(console.error);
  }, [activeCueListId]);

  useEffect(() => {
    if (!open) return;
    const close = () => setOpen(false);
    window.addEventListener("click", close);
    return () => window.removeEventListener("click", close);
  }, [open]);

  if (!cfg) return null;

  const apply = (patch: Partial<CueListTcConfig>) => {
    const next = { ...cfg, ...patch };
    setCfg(next);
    setCuelistTcConfig(next).catch(console.error);
  };

  return (
    <div style={{ marginLeft: "auto", position: "relative", flexShrink: 0 }}>
      <button
        ref={btnRef}
        onClick={(e) => {
          e.stopPropagation();
          const r = btnRef.current?.getBoundingClientRect();
          if (r) setAnchor({ x: r.right, y: r.bottom + 4 });
          setOpen((v) => !v);
        }}
        title="Timecode sync for this Cue List"
        style={{
          display: "flex", alignItems: "center", gap: 5, height: 22,
          padding: "0 8px", borderRadius: 4, cursor: "pointer",
          background: "transparent", fontSize: 11, fontWeight: 600, letterSpacing: "0.04em",
          color: cfg.enabled ? "var(--wc-accent)" : "var(--wc-text-faint)",
          border: `1px solid ${cfg.enabled ? "var(--wc-accent)" : "var(--wc-border)"}`,
        }}
      >
        <span style={{
          width: 6, height: 6, borderRadius: "50%",
          background: cfg.enabled ? "var(--wc-accent)" : "var(--wc-text-faint)",
        }} />
        TC SYNC
      </button>

      {open && anchor && (
        <div
          onClick={(e) => e.stopPropagation()}
          style={{
            position: "fixed", left: anchor.x, top: anchor.y, transform: "translateX(-100%)",
            background: "var(--wc-bg-surface)", border: "1px solid var(--wc-border-strong)",
            borderRadius: 6, padding: 12, zIndex: 9999, width: 240,
            boxShadow: "0 8px 24px rgba(0,0,0,0.7)",
            display: "flex", flexDirection: "column", gap: 10,
          }}
        >
          <label style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 12 }}>
            <input
              type="checkbox"
              checked={cfg.enabled}
              onChange={(e) => apply({ enabled: e.target.checked })}
              style={{ accentColor: "var(--wc-accent)", width: 14, height: 14 }}
            />
            Sync cues from incoming timecode
          </label>

          <div>
            <div style={tcFieldLabel}>Expected Rate</div>
            <Select
              style={{ ...tcInputStyle, cursor: "pointer" }}
              value={cfg.rate}
              onChange={(e) => apply({ rate: e.target.value as TcRate })}
            >
              {TC_RATES.map((r) => <option key={r} value={r}>{TC_RATE_LABELS[r]}</option>)}
            </Select>
          </div>

          <div>
            <div style={tcFieldLabel}>Freewheel (ms)</div>
            <input
              type="number"
              min={0}
              max={2000}
              step={50}
              value={cfg.freewheel_ms}
              onChange={(e) => apply({ freewheel_ms: Math.max(0, Math.min(2000, Number(e.target.value) || 0)) })}
              style={{ ...tcInputStyle, fontFamily: "monospace" }}
            />
          </div>

          <div>
            <div style={tcFieldLabel}>On Stop</div>
            <Select
              style={{ ...tcInputStyle, cursor: "pointer" }}
              value={cfg.on_stop}
              onChange={(e) => apply({ on_stop: e.target.value as TcOnStop })}
            >
              {(["continue", "pause", "stop"] as TcOnStop[]).map((s) => (
                <option key={s} value={s}>{ON_STOP_LABELS[s]}</option>
              ))}
            </Select>
          </div>

          <div style={{ fontSize: 10, color: "var(--wc-text-faint)", lineHeight: 1.4 }}>
            Enable TC receive in Preferences → Network, then set per-cue trigger
            times in the Inspector → Triggers tab.
          </div>
        </div>
      )}
    </div>
  );
}

function ContextMenuItem({
  label, onClick, danger, disabled,
}: {
  label: string;
  onClick: () => void;
  danger?: boolean;
  disabled?: boolean;
}) {
  const [hov, setHov] = useState(false);
  return (
    <button
      onMouseEnter={() => setHov(true)}
      onMouseLeave={() => setHov(false)}
      onClick={() => { if (!disabled) onClick(); }}
      style={{
        display: "block", width: "100%", padding: "6px 14px",
        background: hov && !disabled ? "var(--wc-bg-hover)" : "transparent",
        border: "none", textAlign: "left", fontSize: 13, cursor: disabled ? "default" : "pointer",
        color: disabled ? "var(--wc-text-faint)" : danger ? "#f87171" : "var(--wc-text)",
      }}
    >
      {label}
    </button>
  );
}
