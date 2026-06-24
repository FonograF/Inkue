// Root application layout — mirrors QLab's three-zone layout.

import { useEffect, useState, useCallback, useRef } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { CueListView } from "./components/CueList/CueListView";
import { CartView } from "./components/CueList/CartView";
import { ShowModeView } from "./components/ShowMode/ShowModeView";
import { CueListTabs } from "./components/CueList/CueListTabs";
import { InspectorPanel } from "./components/Inspector/InspectorPanel";
import { TransportBar } from "./components/Transport/TransportBar";
import { useTauriEvents } from "./hooks/useTauriEvents";
import { useKeyboardShortcuts } from "./hooks/useKeyboardShortcuts";
import { useWorkspaceStore } from "./stores/workspaceStore";
import { addCue, collectAndSave, saveWorkspace, loadWorkspace, newWorkspace, setPlayhead, toggleOutputWindow, getOutputWindowVisible, openPreferencesWindow, getCueLists } from "./lib/commands";
import type { CollectReport } from "./lib/types";
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
          background: "var(--wc-bg-surface)", border: "1px solid var(--wc-border-strong)",
          borderRadius: 10, padding: "28px 32px", width: 360,
          boxShadow: "0 16px 48px rgba(0,0,0,0.8)",
        }}
      >
        <div style={{ fontSize: 15, fontWeight: 600, color: "var(--wc-text-bright)", marginBottom: 8 }}>
          Unsaved Changes
        </div>
        <div style={{ fontSize: 13, color: "var(--wc-text-secondary)", marginBottom: 24 }}>
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
    ? hov ? "var(--wc-accent-hover)" : "var(--wc-accent)"
    : danger
      ? hov ? "#dc2626" : "#b91c1c"
      : hov ? "var(--wc-bg-hover)" : "var(--wc-bg-surface)";
  const color = primary ? "var(--wc-accent-fg)" : danger ? "#fff" : "var(--wc-text)";
  return (
    <button
      onClick={onClick}
      onMouseEnter={() => setHov(true)}
      onMouseLeave={() => setHov(false)}
      style={{
        padding: "6px 16px", border: "1px solid var(--wc-border-strong)", borderRadius: 6,
        background: bg, color, fontSize: 13, cursor: "pointer",
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
          background: "var(--wc-bg-surface)", border: "1px solid var(--wc-text-faint)",
          borderRadius: 8, padding: "16px 20px", width: 280,
          boxShadow: "0 12px 40px rgba(0,0,0,0.8)",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <div style={{ fontSize: 12, color: "var(--wc-text-secondary)", marginBottom: 8 }}>
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
            background: "var(--wc-bg-app)", border: "1px solid var(--wc-border-strong)",
            borderRadius: 5, color: "var(--wc-text-bright)", fontSize: 14,
            padding: "7px 10px", outline: "none",
          }}
        />
        <div style={{ fontSize: 11, color: "var(--wc-text-faint)", marginTop: 8 }}>
          Enter to confirm · Escape to cancel
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// File menu
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Collect & Save result dialog
// ---------------------------------------------------------------------------

function CollectResultDialog({
  report,
  onClose,
}: {
  report: CollectReport;
  onClose: () => void;
}) {
  const hasMissing = report.files_missing.length > 0;
  return (
    <div
      style={{
        position: "fixed", inset: 0, zIndex: 99999,
        background: "rgba(0,0,0,0.6)", display: "flex",
        alignItems: "center", justifyContent: "center",
      }}
      onClick={onClose}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        style={{
          background: "var(--wc-bg-surface)", border: "1px solid var(--wc-border-strong)",
          borderRadius: 10, padding: "24px 28px", maxWidth: 520, width: "90%",
          boxShadow: "0 16px 48px rgba(0,0,0,0.8)",
        }}
      >
        <h3 style={{ margin: "0 0 16px", fontSize: 15, color: "var(--wc-text)" }}>
          Collect &amp; Save Complete
        </h3>

        <div style={{ fontSize: 12, color: "var(--wc-text-muted)", marginBottom: 14, fontFamily: "monospace", wordBreak: "break-all" }}>
          {report.workspace_path}
        </div>

        <div style={{ display: "flex", flexDirection: "column", gap: 6, fontSize: 13, marginBottom: 16 }}>
          <div style={{ color: "var(--wc-text)" }}>
            <span style={{ color: "#4ade80" }}>✓</span>{" "}
            {report.files_copied} file{report.files_copied !== 1 ? "s" : ""} copied
          </div>
          {report.files_skipped > 0 && (
            <div style={{ color: "var(--wc-text-muted)" }}>
              — {report.files_skipped} already in place (skipped)
            </div>
          )}
          {hasMissing && (
            <div style={{ color: "#f87171" }}>
              ⚠ {report.files_missing.length} file{report.files_missing.length !== 1 ? "s" : ""} missing from disk
            </div>
          )}
        </div>

        {hasMissing && (
          <div style={{
            background: "rgba(239,68,68,0.08)", border: "1px solid rgba(239,68,68,0.3)",
            borderRadius: 6, padding: "8px 12px", marginBottom: 16,
            maxHeight: 140, overflowY: "auto",
          }}>
            {report.files_missing.map((p) => (
              <div key={p} style={{ fontSize: 11, color: "#fca5a5", fontFamily: "monospace", wordBreak: "break-all", marginBottom: 4 }}>
                {p}
              </div>
            ))}
          </div>
        )}

        <div style={{ display: "flex", justifyContent: "flex-end" }}>
          <button
            onClick={onClose}
            style={{
              background: "var(--wc-bg-hover)", border: "1px solid var(--wc-border-strong)",
              borderRadius: 6, color: "var(--wc-text)", cursor: "pointer",
              fontSize: 13, padding: "6px 18px",
            }}
          >
            OK
          </button>
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
  onCollect,
  onPreferences,
}: {
  onSave: () => void;
  onSaveAs: () => void;
  onOpen: () => void;
  onNew: () => void;
  onCollect: () => void;
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
    { type: "item", label: "Save",              shortcut: "Ctrl+S",       action: act(onSave) },
    { type: "item", label: "Save As…",         shortcut: "Ctrl+Shift+S", action: act(onSaveAs) },
    { type: "item", label: "Collect and Save…",                          action: act(onCollect) },
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
          background: open ? "var(--wc-bg-surface)" : "transparent",
          border: "none", color: "var(--wc-text)", cursor: "pointer",
          fontSize: 12, padding: "3px 8px", borderRadius: 4, userSelect: "none",
        }}
      >
        File
      </button>
      {open && (
        <div
          style={{
            position: "absolute", left: 0, top: "100%", marginTop: 2,
            background: "var(--wc-bg-surface)", border: "1px solid var(--wc-border-strong)", borderRadius: 6,
            padding: "4px 0", minWidth: 220,
            boxShadow: "0 8px 24px rgba(0,0,0,0.7)", zIndex: 9999,
          }}
        >
          {menuItems.map((item, i) =>
            item.type === "separator" ? (
              <div key={i} style={{ height: 1, background: "var(--wc-border-strong)", margin: "4px 0" }} />
            ) : (
              <button
                key={item.label}
                onMouseEnter={() => setHovered(item.label)}
                onMouseLeave={() => setHovered(null)}
                onClick={(e) => { e.stopPropagation(); item.action(); }}
                style={{
                  display: "flex", alignItems: "center", justifyContent: "space-between",
                  width: "100%", padding: "6px 14px",
                  background: hovered === item.label ? "var(--wc-bg-hover)" : "transparent",
                  border: "none", color: "var(--wc-text)", fontSize: 13,
                  cursor: "pointer", textAlign: "left", gap: 24,
                }}
              >
                <span>{item.label}</span>
                {item.shortcut && (
                  <span style={{ color: "var(--wc-text-muted)", fontSize: 11 }}>{item.shortcut}</span>
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
// View menu
// ---------------------------------------------------------------------------

interface ViewMenuItem {
  label: string;
  checked: boolean;
  onClick: () => void;
  shortcut?: string;
}

function ViewMenu({ items }: { items: ViewMenuItem[] }) {
  const [open, setOpen] = useState(false);
  const [hovered, setHovered] = useState<string | null>(null);

  const close = () => setOpen(false);

  return (
    <div style={{ position: "relative", flexShrink: 0 }}>
      {open && (
        <div style={{ position: "fixed", inset: 0, zIndex: 9990 }} onClick={close} />
      )}
      <button
        onClick={(e) => { e.stopPropagation(); setOpen((v) => !v); }}
        style={{
          background: open ? "var(--wc-bg-surface)" : "transparent",
          border: "none", color: "var(--wc-text)", cursor: "pointer",
          fontSize: 12, padding: "3px 8px", borderRadius: 4, userSelect: "none",
        }}
      >
        View
      </button>
      {open && (
        <div
          style={{
            position: "absolute", left: 0, top: "100%", marginTop: 2,
            background: "var(--wc-bg-surface)", border: "1px solid var(--wc-border-strong)", borderRadius: 6,
            padding: "4px 0", minWidth: 200,
            boxShadow: "0 8px 24px rgba(0,0,0,0.7)", zIndex: 9999,
          }}
        >
          {items.map((item) => (
            <button
              key={item.label}
              onMouseEnter={() => setHovered(item.label)}
              onMouseLeave={() => setHovered(null)}
              onClick={(e) => { e.stopPropagation(); close(); item.onClick(); }}
              style={{
                display: "flex", alignItems: "center", gap: 8,
                width: "100%", padding: "6px 14px",
                background: hovered === item.label ? "var(--wc-bg-hover)" : "transparent",
                border: "none", color: "var(--wc-text)", fontSize: 13,
                cursor: "pointer", textAlign: "left",
              }}
            >
              <span style={{ width: 14, textAlign: "center", color: "var(--wc-text-secondary)" }}>
                {item.checked ? "✓" : ""}
              </span>
              <span style={{ flex: 1 }}>{item.label}</span>
              {item.shortcut && (
                <span style={{ color: "var(--wc-text-muted)", fontSize: 11 }}>{item.shortcut}</span>
              )}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Persisted UI layout (panel visibility) — mirrors the column-config pattern.
// ---------------------------------------------------------------------------

const LS_LAYOUT_KEY = "wincue_ui_layout";

interface UiLayout {
  showCueListTabs: boolean;
  inspectorOpen: boolean;
}

const DEFAULT_UI_LAYOUT: UiLayout = { showCueListTabs: true, inspectorOpen: true };

function loadUiLayout(): UiLayout {
  try {
    const raw = localStorage.getItem(LS_LAYOUT_KEY);
    if (!raw) return DEFAULT_UI_LAYOUT;
    const parsed = JSON.parse(raw) as Partial<UiLayout>;
    return {
      showCueListTabs: parsed.showCueListTabs ?? true,
      inspectorOpen: parsed.inspectorOpen ?? true,
    };
  } catch {
    return DEFAULT_UI_LAYOUT;
  }
}

function saveUiLayout(layout: UiLayout): void {
  try {
    localStorage.setItem(LS_LAYOUT_KEY, JSON.stringify(layout));
  } catch {
    // ignore (private / storage-full)
  }
}

// ---------------------------------------------------------------------------
// Root component
// ---------------------------------------------------------------------------

function findCueRecursive(cues: CueSummary[], id: string | null): CueSummary | undefined {
  if (!id) return undefined;
  for (const cue of cues) {
    if (cue.id === id) return cue;
    if (cue.children) {
      const found = findCueRecursive(cue.children, id);
      if (found) return found;
    }
  }
  return undefined;
}

export default function App() {
  const { refreshCues, refreshWorkspaceInfo, loadGeneralPrefs, loadDisplayPrefs, displayPrefs, workspaceInfo, selectedCueId, selectedCueIds, cues, cueLists, activeCueListId } =
    useWorkspaceStore();

  const [inspectorOpen, setInspectorOpen]         = useState(() => loadUiLayout().inspectorOpen);
  const [showCueListTabs, setShowCueListTabs]     = useState(() => loadUiLayout().showCueListTabs);
  const [showMode, setShowMode]                   = useState(false);
  const [closeDialogOpen, setCloseDialogOpen]     = useState(false);
  const [gotoOpen, setGotoOpen]                   = useState(false);
  const [outputSurfaceVisible, setOutputSurfaceVisible] = useState(false);
  const [loadError, setLoadError]                 = useState<string | null>(null);
  const [collectReport, setCollectReport]         = useState<CollectReport | null>(null);

  // Persist panel visibility across launches.
  useEffect(() => {
    saveUiLayout({ showCueListTabs, inspectorOpen });
  }, [showCueListTabs, inspectorOpen]);

  // Apply data-theme whenever display prefs change
  useEffect(() => {
    const root = document.documentElement;
    const theme = displayPrefs.theme ?? "system";

    if (theme === "system") {
      const mq = window.matchMedia("(prefers-color-scheme: dark)");
      const apply = (dark: boolean) => {
        const effective = dark ? "dark" : "light";
        root.setAttribute("data-theme", effective);
        try { localStorage.setItem("wc_theme", effective); } catch { /* ignore */ }
      };
      apply(mq.matches);
      const handler = (e: MediaQueryListEvent) => apply(e.matches);
      mq.addEventListener("change", handler);
      return () => mq.removeEventListener("change", handler);
    } else {
      root.setAttribute("data-theme", theme);
      try { localStorage.setItem("wc_theme", theme); } catch { /* ignore */ }
    }
  }, [displayPrefs.theme]);

  // Bootstrap
  useEffect(() => {
    refreshCues();
    // Use getState() inside the .then() so we read the store at resolution time,
    // not at call time. This prevents a stale response from overwriting a
    // cue-lists-changed event that fired while the IPC was in flight.
    void getCueLists().then((lists) => {
      const store = useWorkspaceStore.getState();
      if (store.cueLists.length === 0 && lists.length > 0) {
        store.setCueLists(lists, lists[0].id);
      }
    }).catch(console.error);
    refreshWorkspaceInfo();
    loadGeneralPrefs();
    loadDisplayPrefs();
    void getOutputWindowVisible().then(setOutputSurfaceVisible);

    let unlistenVisible: (() => void) | undefined;
    listen<boolean>("output-window-visible", (e) => {
      setOutputSurfaceVisible(e.payload);
    }).then((u) => { unlistenVisible = u; }).catch(console.error);
    return () => { unlistenVisible?.(); };
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
      // cue-lists-changed is emitted by the backend and handled in useTauriEvents.
      // workspace-modified triggers refreshCues + refreshWorkspaceInfo.
    }
  }, []);

  const handleNew = useCallback(async () => {
    await newWorkspace().catch(console.error);
    // cue-lists-changed and workspace-modified are emitted by the backend.
  }, []);

  const handleCollectAndSave = useCallback(async () => {
    const dir = await openDialog({ directory: true });
    if (typeof dir !== "string") return;
    try {
      const report = await collectAndSave(dir);
      setCollectReport(report);
    } catch (err) {
      setLoadError(String(err));
    }
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

  const handleLoadError = useCallback((_cueId: string, error: string) => {
    setLoadError(error);
  }, []);

  useTauriEvents({ onLoadError: handleLoadError });

  const handleRefresh = useCallback(async () => {
    await refreshCues();
  }, [refreshCues]);

  useKeyboardShortcuts(
    handleRefresh,
    () => void openPreferencesWindow(),
    () => void handleSave(),
    () => void handleOpen(),
    () => setInspectorOpen((v) => !v),
    () => setGotoOpen(true),
    () => void handleToggleSurface(),
    () => setShowMode((v) => !v),
  );

  const selectedCue = findCueRecursive(cues, selectedCueId) ?? null;

  const handleToggleSurface = async () => {
    await toggleOutputWindow().catch(console.error);
    // outputSurfaceVisible is driven by the "output-window-visible" event from Rust
  };

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

  const handleAddWait = async () => {
    const { selectedCueId, cues } = useWorkspaceStore.getState();
    const idx = cues.findIndex((c) => c.id === selectedCueId);
    await addCue("wait", idx >= 0 ? idx + 1 : -1).catch(console.error);
    await refreshCues();
  };

  const handleAddGroup = async () => {
    const { selectedCueId, cues } = useWorkspaceStore.getState();
    const idx = cues.findIndex((c) => c.id === selectedCueId);
    await addCue("group", idx >= 0 ? idx + 1 : -1).catch(console.error);
    await refreshCues();
  };

  const handleAddOsc = async () => {
    const { selectedCueId, cues } = useWorkspaceStore.getState();
    const idx = cues.findIndex((c) => c.id === selectedCueId);
    await addCue("osc", idx >= 0 ? idx + 1 : -1).catch(console.error);
    await refreshCues();
  };

  const handleAddFade = async () => {
    const { selectedCueId, cues } = useWorkspaceStore.getState();
    const idx = cues.findIndex((c) => c.id === selectedCueId);
    await addCue("fade", idx >= 0 ? idx + 1 : -1).catch(console.error);
    await refreshCues();
  };

  const handleAddMidi = async () => {
    const { selectedCueId, cues } = useWorkspaceStore.getState();
    const idx = cues.findIndex((c) => c.id === selectedCueId);
    await addCue("midi", idx >= 0 ? idx + 1 : -1).catch(console.error);
    await refreshCues();
  };

  const handleAddLight = async () => {
    const { selectedCueId, cues } = useWorkspaceStore.getState();
    const idx = cues.findIndex((c) => c.id === selectedCueId);
    await addCue("light", idx >= 0 ? idx + 1 : -1).catch(console.error);
    await refreshCues();
  };

  const handleAddMic = async () => {
    const { selectedCueId, cues } = useWorkspaceStore.getState();
    const idx = cues.findIndex((c) => c.id === selectedCueId);
    await addCue("mic", idx >= 0 ? idx + 1 : -1).catch(console.error);
    await refreshCues();
  };

  const handleAddTimecode = async () => {
    const { selectedCueId, cues } = useWorkspaceStore.getState();
    const idx = cues.findIndex((c) => c.id === selectedCueId);
    await addCue("timecode", idx >= 0 ? idx + 1 : -1).catch(console.error);
    await refreshCues();
  };

  const dispatchCueDrag = (cueType: "audio" | "stop" | "video" | "image" | "group" | "wait" | "osc" | "fade" | "midi" | "light" | "mic" | "timecode", e: React.MouseEvent) => {
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
        background: "var(--wc-bg-app)", color: "var(--wc-text)",
        fontFamily: "'Segoe UI', system-ui, -apple-system, BlinkMacSystemFont, sans-serif",
        overflow: "hidden",
      }}
      onContextMenu={(e) => e.preventDefault()}
    >
      {/* Audio file load error toast */}
      {loadError && (
        <div
          style={{
            position: "fixed", bottom: 20, left: "50%", transform: "translateX(-50%)",
            zIndex: 99999, background: "#7f1d1d", border: "1px solid #ef4444",
            borderRadius: 8, padding: "10px 16px", maxWidth: 520,
            display: "flex", alignItems: "flex-start", gap: 12,
            boxShadow: "0 8px 24px rgba(0,0,0,0.8)",
          }}
        >
          <span style={{ color: "#fca5a5", fontSize: 13, flex: 1 }}>
            <strong style={{ color: "#fecaca" }}>Failed to load audio file.</strong>
            <br />
            <span style={{ opacity: 0.85, fontFamily: "monospace", fontSize: 11 }}>{loadError}</span>
          </span>
          <button
            onClick={() => setLoadError(null)}
            style={{
              background: "transparent", border: "none", color: "#fca5a5",
              cursor: "pointer", fontSize: 16, padding: 0, lineHeight: 1, flexShrink: 0,
            }}
          >
            ✕
          </button>
        </div>
      )}

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

      {/* Collect & Save result dialog */}
      {collectReport && (
        <CollectResultDialog
          report={collectReport}
          onClose={() => setCollectReport(null)}
        />
      )}

      {/* Custom title bar — no drag-region on the container so menus/buttons work on Linux */}
      <div
        style={{
          display: "flex", alignItems: "center", height: 36, padding: "0 12px",
          background: "var(--wc-bg-surface)", borderBottom: "1px solid var(--wc-border)",
          flexShrink: 0, gap: 12, userSelect: "none", WebkitUserSelect: "none",
        }}
      >
        <WindowControls />
        <FileMenu
          onSave={() => void handleSave()}
          onSaveAs={() => void handleSaveAs()}
          onOpen={() => void handleOpen()}
          onNew={() => void handleNew()}
          onCollect={() => void handleCollectAndSave()}
          onPreferences={() => void openPreferencesWindow()}
        />
        <ViewMenu
          items={[
            { label: "Show Mode",      checked: showMode,             onClick: () => setShowMode((v) => !v),       shortcut: "F5" },
            { label: "Cue List Tabs",  checked: showCueListTabs,      onClick: () => setShowCueListTabs((v) => !v) },
            { label: "Inspector",      checked: inspectorOpen,         onClick: () => setInspectorOpen((v) => !v) },
            { label: "Output Surface", checked: outputSurfaceVisible,  onClick: () => void handleToggleSurface() },
          ]}
        />

        {/* Drag region: app name + workspace name */}
        <div
          data-tauri-drag-region
          style={{ flex: 1, display: "flex", alignItems: "center", gap: 10, overflow: "hidden" }}
        >
          <span
            data-tauri-drag-region
            style={{ fontWeight: 700, fontSize: 13, color: "var(--wc-text-bright)", flexShrink: 0 }}
          >
            WinCue
          </span>
          <span
            data-tauri-drag-region
            style={{ fontSize: 12, color: "var(--wc-text-muted)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}
          >
            {titleBarName}
          </span>
        </div>

        {/* Toolbar — hidden in Show Mode */}
        <div style={{ display: showMode ? "none" : "flex", gap: 6, flexShrink: 0 }}>
          <button
            style={{ ...toolbarBtn, color: "var(--wc-accent)", cursor: "grab", userSelect: "none" }}
            onClick={handleAddAudio}
            onMouseDown={(e) => dispatchCueDrag("audio", e)}
            title="Add Audio Cue after selection (Ctrl+N) · Drag to insert at position"
          >
            + Audio
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
          <button
            style={{ ...toolbarBtn, color: "#ef4444", cursor: "grab", userSelect: "none" }}
            onClick={handleAddStop}
            onMouseDown={(e) => dispatchCueDrag("stop", e)}
            title="Add Stop Cue after selection · Drag to insert at position"
          >
            + Stop
          </button>
          <button
            style={{ ...toolbarBtn, color: "#ec4899", cursor: "grab", userSelect: "none" }}
            onClick={handleAddFade}
            onMouseDown={(e) => dispatchCueDrag("fade", e)}
            title="Add Fade Cue after selection · Drag to insert at position"
          >
            + Fade
          </button>
          <button
            style={{ ...toolbarBtn, color: "#fb923c", cursor: "grab", userSelect: "none" }}
            onClick={handleAddWait}
            onMouseDown={(e) => dispatchCueDrag("wait", e)}
            title="Add Wait Cue after selection · Drag to insert at position"
          >
            + Wait
          </button>
          <button
            style={{ ...toolbarBtn, color: "#fde047", cursor: "grab", userSelect: "none" }}
            onClick={handleAddGroup}
            onMouseDown={(e) => dispatchCueDrag("group", e)}
            title="Add Group Cue after selection · Drag to insert at position"
          >
            + Group
          </button>
          <button
            style={{ ...toolbarBtn, color: "var(--wc-text-bright)", cursor: "grab", userSelect: "none" }}
            onClick={handleAddMidi}
            onMouseDown={(e) => dispatchCueDrag("midi", e)}
            title="Add MIDI Cue after selection · Drag to insert at position"
          >
            + MIDI
          </button>
          <button
            style={{ ...toolbarBtn, color: "#06b6d4", cursor: "grab", userSelect: "none" }}
            onClick={handleAddOsc}
            onMouseDown={(e) => dispatchCueDrag("osc", e)}
            title="Add OSC Cue after selection · Drag to insert at position"
          >
            + OSC
          </button>
          <button
            style={{ ...toolbarBtn, color: "#fbbf24", cursor: "grab", userSelect: "none" }}
            onClick={handleAddLight}
            onMouseDown={(e) => dispatchCueDrag("light", e)}
            title="Add Light Cue after selection · Drag to insert at position"
          >
            + Light
          </button>
          <button
            style={{ ...toolbarBtn, color: "#86efac", cursor: "grab", userSelect: "none" }}
            onClick={handleAddMic}
            onMouseDown={(e) => dispatchCueDrag("mic", e)}
            title="Add Mic Cue after selection · Drag to insert at position"
          >
            + Mic
          </button>
          <button
            style={{ ...toolbarBtn, color: "#67e8f9", cursor: "grab", userSelect: "none" }}
            onClick={handleAddTimecode}
            onMouseDown={(e) => dispatchCueDrag("timecode", e)}
            title="Add Timecode Cue after selection · Drag to insert at position"
          >
            + TC
          </button>
          <button style={toolbarBtn} onClick={() => setInspectorOpen((v) => !v)} title="Toggle Inspector (Ctrl+I)">
            Inspector
          </button>
        </div>
      </div>

      {/* Main area */}
      <div style={{ display: "flex", flex: 1, overflow: "hidden" }}>
        {showMode ? (
          <ShowModeView />
        ) : (
          <>
            <div style={{ flex: 1, minWidth: 0, minHeight: 0, overflow: "hidden", display: "flex", flexDirection: "column" }}>
              {showCueListTabs && <CueListTabs onRefresh={handleRefresh} />}
              {(() => {
                const activeList = cueLists.find((l) => l.id === activeCueListId);
                if (activeList?.mode === "cart") {
                  return <CartView onRefresh={handleRefresh} />;
                }
                return (
                  <CueListView
                    onCueDoubleClick={(cue: CueSummary) => {
                      useWorkspaceStore.getState().setSelectedCueId(cue.id);
                      setInspectorOpen(true);
                    }}
                    onRefresh={handleRefresh}
                  />
                );
              })()}
            </div>
            {inspectorOpen && (() => {
              const activeList = cueLists.find((l) => l.id === activeCueListId);
              if (activeList?.mode === "cart") return null;
              return (
                <div
                  style={{
                    width: 300, borderLeft: "1px solid var(--wc-border)",
                    overflow: "hidden", display: "flex", flexDirection: "column", flexShrink: 0,
                  }}
                >
                  <InspectorPanel selectedCue={selectedCue} selectedCueIds={selectedCueIds} onRefresh={handleRefresh} />
                </div>
              );
            })()}
          </>
        )}
      </div>

      <TransportBar onRefresh={handleRefresh} />
    </div>
  );
}

const toolbarBtn: React.CSSProperties = {
  padding: "3px 10px", background: "var(--wc-bg-surface)", border: "1px solid var(--wc-border-strong)",
  borderRadius: 4, color: "var(--wc-text)", cursor: "pointer", fontSize: 12,
};
