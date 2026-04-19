// Root application layout — mirrors QLab's three-zone layout.

import { useEffect, useState, useCallback, useRef } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { CueListView } from "./components/CueList/CueListView";
import { InspectorPanel } from "./components/Inspector/InspectorPanel";
import { TransportBar } from "./components/Transport/TransportBar";
import { useTauriEvents } from "./hooks/useTauriEvents";
import { useKeyboardShortcuts } from "./hooks/useKeyboardShortcuts";
import { useWorkspaceStore } from "./stores/workspaceStore";
import { addCue, saveWorkspace, loadWorkspace, newWorkspace, setPlayhead } from "./lib/commands";
import { PreferencesModal } from "./components/Preferences/PreferencesModal";
import type { CueSummary } from "./lib/types";

// ---------------------------------------------------------------------------
// Window control buttons (minimize / maximize / close)
// ---------------------------------------------------------------------------

function WindowControls() {
  const [hovered, setHovered] = useState<"min" | "max" | "close" | null>(null);

  const handleMin   = () => void getCurrentWindow().minimize();
  const handleMax   = () => void getCurrentWindow().toggleMaximize();
  // Close goes through the normal close path so onCloseRequested fires.
  const handleClose = () => void getCurrentWindow().close();

  const btn = (
    key: "min" | "max" | "close",
    label: string,
    color: string,
    hoverColor: string,
    onClick: () => void,
  ) => (
    <button
      key={key}
      title={label}
      onClick={onClick}
      onMouseEnter={() => setHovered(key)}
      onMouseLeave={() => setHovered(null)}
      style={{
        width: 13, height: 13, borderRadius: "50%", border: "none",
        background: hovered === key ? hoverColor : color,
        cursor: "pointer", display: "flex", alignItems: "center",
        justifyContent: "center", padding: 0, flexShrink: 0,
        fontSize: 8,
        color: hovered === key ? "rgba(0,0,0,0.6)" : "transparent",
        transition: "background 0.1s",
      }}
    >
      {hovered === key ? label : ""}
    </button>
  );

  return (
    <div style={{ display: "flex", gap: 7, alignItems: "center", flexShrink: 0 }}>
      {btn("close", "✕", "#ef4444", "#dc2626", handleClose)}
      {btn("min",   "–", "#f59e0b", "#d97706", handleMin)}
      {btn("max",   "▢", "#22c55e", "#16a34a", handleMax)}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Unsaved-changes close confirmation dialog
// ---------------------------------------------------------------------------

function CloseConfirmDialog({
  onSave,
  onDiscard,
  onCancel,
}: {
  onSave: () => void;
  onDiscard: () => void;
  onCancel: () => void;
}) {
  return (
    <div
      style={{
        position: "fixed", inset: 0, zIndex: 99999,
        background: "rgba(0,0,0,0.6)",
        display: "flex", alignItems: "center", justifyContent: "center",
      }}
    >
      <div
        style={{
          background: "#1e293b", border: "1px solid #334155",
          borderRadius: 10, padding: "28px 32px", width: 360,
          boxShadow: "0 16px 48px rgba(0,0,0,0.8)",
        }}
      >
        <div style={{ fontSize: 15, fontWeight: 600, color: "#f1f5f9", marginBottom: 8 }}>
          Unsaved Changes
        </div>
        <div style={{ fontSize: 13, color: "#94a3b8", marginBottom: 24 }}>
          Do you want to save your workspace before quitting?
        </div>
        <div style={{ display: "flex", gap: 8, justifyContent: "flex-end" }}>
          <DialogBtn label="Cancel"    onClick={onCancel}  />
          <DialogBtn label="Don't Save" onClick={onDiscard} danger />
          <DialogBtn label="Save"       onClick={onSave}    primary />
        </div>
      </div>
    </div>
  );
}

function DialogBtn({
  label, onClick, primary, danger,
}: {
  label: string; onClick: () => void; primary?: boolean; danger?: boolean;
}) {
  const [hov, setHov] = useState(false);
  const bg = primary
    ? hov ? "#2563eb" : "#1d4ed8"
    : danger
      ? hov ? "#dc2626" : "#b91c1c"
      : hov ? "#334155" : "#1e293b";
  return (
    <button
      onClick={onClick}
      onMouseEnter={() => setHov(true)}
      onMouseLeave={() => setHov(false)}
      style={{
        padding: "6px 16px", border: "1px solid #334155", borderRadius: 6,
        background: bg, color: "#e2e8f0", fontSize: 13, cursor: "pointer",
      }}
    >
      {label}
    </button>
  );
}

// ---------------------------------------------------------------------------
// Goto cue dialog (G key) — type a cue number to move the playhead
// ---------------------------------------------------------------------------

function GotoDialog({
  onClose,
  onRefresh,
}: {
  onClose: () => void;
  onRefresh: () => void;
}) {
  const [value, setValue] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const commit = async () => {
    const q = value.trim();
    if (!q) { onClose(); return; }
    const { cues } = useWorkspaceStore.getState();
    const match = cues.find((c) => c.number != null && c.number === q);
    if (match) {
      await setPlayhead(match.id).catch(console.error);
      onRefresh();
    }
    onClose();
  };

  const handleKey = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") { e.preventDefault(); void commit(); }
    if (e.key === "Escape") { e.preventDefault(); onClose(); }
  };

  return (
    <div
      style={{
        position: "fixed", inset: 0, zIndex: 99998,
        display: "flex", alignItems: "center", justifyContent: "center",
      }}
      onClick={onClose}
    >
      <div
        style={{
          background: "#1e293b", border: "1px solid #475569",
          borderRadius: 8, padding: "16px 20px", width: 280,
          boxShadow: "0 12px 40px rgba(0,0,0,0.8)",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <div style={{ fontSize: 12, color: "#94a3b8", marginBottom: 8 }}>
          Go to cue number
        </div>
        <input
          ref={inputRef}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={handleKey}
          placeholder="e.g. 1, 1.5, Intro"
          style={{
            width: "100%", boxSizing: "border-box",
            background: "#0f172a", border: "1px solid #334155",
            borderRadius: 5, color: "#f1f5f9", fontSize: 14,
            padding: "7px 10px", outline: "none",
          }}
        />
        <div style={{ fontSize: 11, color: "#475569", marginTop: 8 }}>
          Enter to confirm · Escape to cancel
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// File menu
// ---------------------------------------------------------------------------

function FileMenu({
  onSave,
  onSaveAs,
  onOpen,
  onNew,
  onPreferences,
}: {
  onSave: () => void;
  onSaveAs: () => void;
  onOpen: () => void;
  onNew: () => void;
  onPreferences: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [hovered, setHovered] = useState<string | null>(null);

  const close = () => setOpen(false);

  const act = (fn: () => void) => () => { close(); fn(); };

  const handleQuit = () => {
    close();
    void getCurrentWindow().close();
  };

  const menuItems: Array<
    | { type: "item"; label: string; shortcut?: string; action: () => void }
    | { type: "separator" }
  > = [
    { type: "item", label: "New Workspace",  shortcut: "Ctrl+N",         action: act(onNew) },
    { type: "item", label: "Open…",         shortcut: "Ctrl+O",         action: act(onOpen) },
    { type: "separator" },
    { type: "item", label: "Save",          shortcut: "Ctrl+S",         action: act(onSave) },
    { type: "item", label: "Save As…",      shortcut: "Ctrl+Shift+S",   action: act(onSaveAs) },
    { type: "separator" },
    { type: "item", label: "Preferences",   shortcut: "Ctrl+,",         action: act(onPreferences) },
    { type: "separator" },
    { type: "item", label: "Quit",                                       action: handleQuit },
  ];

  return (
    <div style={{ position: "relative", flexShrink: 0 }}>
      {open && (
        <div style={{ position: "fixed", inset: 0, zIndex: 9990 }} onClick={close} />
      )}
      <button
        onClick={(e) => { e.stopPropagation(); setOpen((v) => !v); }}
        style={{
          background: open ? "#1e293b" : "transparent",
          border: "none", color: "#cbd5e1", cursor: "pointer",
          fontSize: 12, padding: "3px 8px", borderRadius: 4, userSelect: "none",
        }}
      >
        File
      </button>
      {open && (
        <div
          style={{
            position: "absolute", left: 0, top: "100%", marginTop: 2,
            background: "#1e293b", border: "1px solid #334155", borderRadius: 6,
            padding: "4px 0", minWidth: 220,
            boxShadow: "0 8px 24px rgba(0,0,0,0.7)", zIndex: 9999,
          }}
        >
          {menuItems.map((item, i) =>
            item.type === "separator" ? (
              <div key={i} style={{ height: 1, background: "#334155", margin: "4px 0" }} />
            ) : (
              <button
                key={item.label}
                onMouseEnter={() => setHovered(item.label)}
                onMouseLeave={() => setHovered(null)}
                onClick={(e) => { e.stopPropagation(); item.action(); }}
                style={{
                  display: "flex", alignItems: "center", justifyContent: "space-between",
                  width: "100%", padding: "6px 14px",
                  background: hovered === item.label ? "#334155" : "transparent",
                  border: "none", color: "#e2e8f0", fontSize: 13,
                  cursor: "pointer", textAlign: "left", gap: 24,
                }}
              >
                <span>{item.label}</span>
                {item.shortcut && (
                  <span style={{ color: "#64748b", fontSize: 11 }}>{item.shortcut}</span>
                )}
              </button>
            )
          )}
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Root component
// ---------------------------------------------------------------------------

export default function App() {
  const { refreshCues, refreshWorkspaceInfo, loadGeneralPrefs, workspaceInfo, selectedCueId, cues } =
    useWorkspaceStore();

  const [inspectorOpen, setInspectorOpen]     = useState(true);
  const [closeDialogOpen, setCloseDialogOpen] = useState(false);
  const [prefsOpen, setPrefsOpen]             = useState(false);
  const [gotoOpen, setGotoOpen]               = useState(false);

  // Bootstrap
  useEffect(() => {
    refreshCues();
    refreshWorkspaceInfo();
    loadGeneralPrefs();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // -------------------------------------------------------------------------
  // Shared save helpers (used by FileMenu AND CloseConfirmDialog)
  // -------------------------------------------------------------------------

  /** Save to the existing path, or open Save As if no path is set yet.
   *  Returns true if the save completed, false if the user cancelled. */
  const handleSaveAs = useCallback(async (): Promise<boolean> => {
    const path = await saveDialog({
      filters: [{ name: "WinCue Workspace", extensions: ["wincue"] }],
      defaultPath: (workspaceInfo?.name ?? "Untitled") + ".wincue",
    });
    if (typeof path !== "string") return false;
    const filePath = path.endsWith(".wincue") ? path : path + ".wincue";
    await saveWorkspace(filePath).catch(console.error);
    await refreshWorkspaceInfo();
    return true;
  }, [workspaceInfo, refreshWorkspaceInfo]);

  const handleSave = useCallback(async (): Promise<boolean> => {
    const path = workspaceInfo?.file_path;
    if (path) {
      await saveWorkspace(path).catch(console.error);
      await refreshWorkspaceInfo();
      return true;
    }
    return handleSaveAs();
  }, [workspaceInfo, refreshWorkspaceInfo, handleSaveAs]);

  const handleOpen = useCallback(async () => {
    const path = await openDialog({
      multiple: false,
      filters: [{ name: "WinCue Workspace", extensions: ["wincue"] }],
    });
    if (typeof path === "string") {
      await loadWorkspace(path).catch(console.error);
      await refreshCues();
      await refreshWorkspaceInfo();
    }
  }, [refreshCues, refreshWorkspaceInfo]);

  const handleNew = useCallback(async () => {
    await newWorkspace().catch(console.error);
  }, []);

  // -------------------------------------------------------------------------
  // Close-request interception
  // -------------------------------------------------------------------------

  useEffect(() => {
    const win = getCurrentWindow();
    let unlisten: (() => void) | undefined;

    win.onCloseRequested((event) => {
      const isModified = useWorkspaceStore.getState().workspaceInfo?.is_modified;
      if (isModified) {
        event.preventDefault();
        setCloseDialogOpen(true);
      }
      // Not modified → close proceeds normally.
    }).then((u) => { unlisten = u; });

    return () => unlisten?.();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // destroy() force-closes without re-triggering onCloseRequested.
  const confirmSaveAndClose = async () => {
    setCloseDialogOpen(false);
    const saved = await handleSave();
    if (saved) {
      void getCurrentWindow().destroy();
    }
    // If user cancelled the Save As dialog, keep the app open.
  };

  const confirmDiscardAndClose = () => {
    setCloseDialogOpen(false);
    void getCurrentWindow().destroy();
  };

  const cancelClose = () => setCloseDialogOpen(false);

  // -------------------------------------------------------------------------
  // Misc
  // -------------------------------------------------------------------------

  useTauriEvents();

  const handleRefresh = useCallback(async () => {
    await refreshCues();
  }, [refreshCues]);

  useKeyboardShortcuts(
    handleRefresh,
    () => setPrefsOpen(true),
    () => void handleSave(),
    () => void handleOpen(),
    () => setInspectorOpen((v) => !v),
    () => setGotoOpen(true),
  );

  const selectedCue = cues.find((c) => c.id === selectedCueId) ?? null;

  const handleAddAudio = async () => {
    const { selectedCueId, cues } = useWorkspaceStore.getState();
    const idx = cues.findIndex((c) => c.id === selectedCueId);
    await addCue("audio", idx >= 0 ? idx + 1 : -1).catch(console.error);
    await refreshCues();
  };

  const handleAddStop = async () => {
    const { selectedCueId, cues } = useWorkspaceStore.getState();
    const idx = cues.findIndex((c) => c.id === selectedCueId);
    await addCue("stop", idx >= 0 ? idx + 1 : -1).catch(console.error);
    await refreshCues();
  };

  const handleAddVideo = async () => {
    const { selectedCueId, cues } = useWorkspaceStore.getState();
    const idx = cues.findIndex((c) => c.id === selectedCueId);
    await addCue("video", idx >= 0 ? idx + 1 : -1).catch(console.error);
    await refreshCues();
  };

  const handleAddImage = async () => {
    const { selectedCueId, cues } = useWorkspaceStore.getState();
    const idx = cues.findIndex((c) => c.id === selectedCueId);
    await addCue("image", idx >= 0 ? idx + 1 : -1).catch(console.error);
    await refreshCues();
  };

  const dispatchCueDrag = (cueType: "audio" | "stop" | "video" | "image", e: React.MouseEvent) => {
    if (e.button !== 0) return;
    document.dispatchEvent(
      new CustomEvent("wincue:cue-drag-start", {
        detail: { cueType, startX: e.clientX, startY: e.clientY },
      }),
    );
  };

  const titleBarName = workspaceInfo
    ? `${workspaceInfo.name}${workspaceInfo.is_modified ? " •" : ""}`
    : "WinCue";

  return (
    <div
      style={{
        display: "flex", flexDirection: "column", height: "100vh",
        background: "#020617", color: "#e2e8f0",
        fontFamily: "'Segoe UI', system-ui, -apple-system, BlinkMacSystemFont, sans-serif",
        overflow: "hidden",
      }}
      onContextMenu={(e) => e.preventDefault()}
    >
      {/* Preferences modal */}
      {prefsOpen && <PreferencesModal onClose={() => setPrefsOpen(false)} />}

      {/* Goto cue dialog */}
      {gotoOpen && (
        <GotoDialog onClose={() => setGotoOpen(false)} onRefresh={handleRefresh} />
      )}

      {/* Unsaved-changes dialog */}
      {closeDialogOpen && (
        <CloseConfirmDialog
          onSave={confirmSaveAndClose}
          onDiscard={confirmDiscardAndClose}
          onCancel={cancelClose}
        />
      )}

      {/* Custom title bar */}
      <div
        data-tauri-drag-region
        style={{
          display: "flex", alignItems: "center", height: 36, padding: "0 12px",
          background: "#0f172a", borderBottom: "1px solid #1e293b",
          flexShrink: 0, gap: 12, userSelect: "none", WebkitUserSelect: "none",
        }}
      >
        <WindowControls />
        <FileMenu
          onSave={() => void handleSave()}
          onSaveAs={() => void handleSaveAs()}
          onOpen={() => void handleOpen()}
          onNew={() => void handleNew()}
          onPreferences={() => setPrefsOpen(true)}
        />

        {/* Drag region: app name + workspace name */}
        <div
          data-tauri-drag-region
          style={{ flex: 1, display: "flex", alignItems: "center", gap: 10, overflow: "hidden" }}
        >
          <span
            data-tauri-drag-region
            style={{ fontWeight: 700, fontSize: 13, color: "#f1f5f9", flexShrink: 0 }}
          >
            WinCue
          </span>
          <span
            data-tauri-drag-region
            style={{ fontSize: 12, color: "#64748b", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}
          >
            {titleBarName}
          </span>
        </div>

        {/* Toolbar */}
        <div style={{ display: "flex", gap: 6, flexShrink: 0 }}>
          <button
            style={{ ...toolbarBtn, cursor: "grab", userSelect: "none" }}
            onClick={handleAddAudio}
            onMouseDown={(e) => dispatchCueDrag("audio", e)}
            title="Add Audio Cue after selection (Ctrl+N) · Drag to insert at position"
          >
            + Audio
          </button>
          <button
            style={{ ...toolbarBtn, color: "#fca5a5", cursor: "grab", userSelect: "none" }}
            onClick={handleAddStop}
            onMouseDown={(e) => dispatchCueDrag("stop", e)}
            title="Add Stop Cue after selection · Drag to insert at position"
          >
            + Stop
          </button>
          <button
            style={{ ...toolbarBtn, color: "#a78bfa", cursor: "grab", userSelect: "none" }}
            onClick={handleAddVideo}
            onMouseDown={(e) => dispatchCueDrag("video", e)}
            title="Add Video Cue after selection · Drag to insert at position"
          >
            + Video
          </button>
          <button
            style={{ ...toolbarBtn, color: "#86efac", cursor: "grab", userSelect: "none" }}
            onClick={handleAddImage}
            onMouseDown={(e) => dispatchCueDrag("image", e)}
            title="Add Image Cue after selection · Drag to insert at position"
          >
            + Image
          </button>
          <button style={toolbarBtn} onClick={() => setInspectorOpen((v) => !v)} title="Toggle Inspector (Ctrl+I)">
            Inspector
          </button>
        </div>
      </div>

      {/* Main area */}
      <div style={{ display: "flex", flex: 1, overflow: "hidden" }}>
        <div style={{ flex: 1, overflow: "hidden", display: "flex", flexDirection: "column" }}>
          <CueListView
            onCueDoubleClick={(cue: CueSummary) => {
              useWorkspaceStore.getState().setSelectedCueId(cue.id);
              setInspectorOpen(true);
            }}
            onRefresh={handleRefresh}
          />
        </div>
        {inspectorOpen && (
          <div
            style={{
              width: 300, borderLeft: "1px solid #1e293b",
              overflow: "hidden", display: "flex", flexDirection: "column", flexShrink: 0,
            }}
          >
            <InspectorPanel selectedCue={selectedCue} onRefresh={handleRefresh} />
          </div>
        )}
      </div>

      <TransportBar onRefresh={handleRefresh} />
    </div>
  );
}

const toolbarBtn: React.CSSProperties = {
  padding: "3px 10px", background: "#1e293b", border: "1px solid #334155",
  borderRadius: 4, color: "#cbd5e1", cursor: "pointer", fontSize: 12,
};
