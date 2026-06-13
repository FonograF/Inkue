// Tab bar for switching between multiple cue lists.

import { useState, useRef, useEffect } from "react";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import { addCueList, removeCueList, renameCueList, setActiveCueList } from "../../lib/commands";

export function CueListTabs({ onRefresh }: { onRefresh: () => void }) {
  const { cueLists, activeCueListId, refreshCueLists } = useWorkspaceStore();

  const [renamingId, setRenamingId]   = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; id: string } | null>(null);
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
        background: "var(--wc-bg-surface, #0f172a)",
        borderBottom: "1px solid #1e293b",
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
              background: isActive ? "var(--wc-bg-panel, #1e293b)" : "transparent",
              color: isActive ? "var(--wc-text, #e2e8f0)" : "#64748b",
              border: isActive ? "1px solid #334155" : "1px solid transparent",
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
              <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", maxWidth: 140 }}>
                {list.name}
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
          background: "transparent", border: "none", color: "#475569",
          cursor: "pointer", fontSize: 16, lineHeight: 1,
          padding: "0 6px", height: 26, borderRadius: 4, flexShrink: 0,
          display: "flex", alignItems: "center",
        }}
        onMouseEnter={(e) => { (e.currentTarget as HTMLButtonElement).style.color = "#94a3b8"; }}
        onMouseLeave={(e) => { (e.currentTarget as HTMLButtonElement).style.color = "#475569"; }}
      >
        +
      </button>

      {/* Context menu */}
      {contextMenu && (
        <div
          style={{
            position: "fixed", left: contextMenu.x, top: contextMenu.y,
            background: "#1e293b", border: "1px solid #334155", borderRadius: 6,
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
          <div style={{ height: 1, background: "#334155", margin: "4px 0" }} />
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
        background: hov && !disabled ? "#334155" : "transparent",
        border: "none", textAlign: "left", fontSize: 13, cursor: disabled ? "default" : "pointer",
        color: disabled ? "#475569" : danger ? "#f87171" : "#e2e8f0",
      }}
    >
      {label}
    </button>
  );
}
