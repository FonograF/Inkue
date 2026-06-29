// Root application layout — mirrors QLab's three-zone layout.

import { useEffect, useState, useCallback, useRef } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { CueListView } from "./components/CueList/CueListView";
import { CartView } from "./components/CueList/CartView";
import { ShowModeView } from "./components/ShowMode/ShowModeView";
import { ActiveCuesView } from "./components/ActiveCues/ActiveCuesView";
import { CueListTabs } from "./components/CueList/CueListTabs";
import { InspectorPanel } from "./components/Inspector/InspectorPanel";
import { TransportBar } from "./components/Transport/TransportBar";
import { useTauriEvents } from "./hooks/useTauriEvents";
import { useKeyboardShortcuts } from "./hooks/useKeyboardShortcuts";
import { useWorkspaceStore } from "./stores/workspaceStore";
import { addCue, collectAndSave, saveWorkspace, loadWorkspace, newWorkspace, setPlayhead, toggleOutputWindow, getOutputWindowVisible, openPreferencesWindow, getCueLists, checkRecovery, restoreRecovery, discardRecovery } from "./lib/commands";
import { AboutDialog } from "./components/About/AboutDialog";
import { PreflightModal } from "./components/Preflight/PreflightModal";
import { LogViewerModal } from "./components/Logs/LogViewerModal";
import { HealthBanner } from "./components/Health/HealthBanner";
import type { CollectReport, RecoveryInfo } from "./lib/types";
import type { CueSummary } from "./lib/types";
import { CUE_TYPE_COLORS } from "./lib/types";

// ---------------------------------------------------------------------------
// Recent files
// ---------------------------------------------------------------------------

const RECENT_FILES_KEY = "inkue_recent_files";
const MAX_RECENT = 8;

function loadRecentFiles(): string[] {
  try { return JSON.parse(localStorage.getItem(RECENT_FILES_KEY) ?? "[]") as string[]; }
  catch { return []; }
}

function pushRecentFile(path: string): string[] {
  const list = loadRecentFiles().filter((p) => p !== path);
  list.unshift(path);
  const trimmed = list.slice(0, MAX_RECENT);
  try { localStorage.setItem(RECENT_FILES_KEY, JSON.stringify(trimmed)); } catch { /* ignore */ }
  return trimmed;
}

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
// Crash-recovery dialog — shown on startup when a recovery snapshot exists
// ---------------------------------------------------------------------------

function RecoveryDialog({
  info,
  onRecover,
  onDiscard,
}: {
  info: RecoveryInfo;
  onRecover: () => void;
  onDiscard: () => void;
}) {
  const label = info.name || "Untitled";
  const when = info.modified_at
    ? new Date(info.modified_at).toLocaleString()
    : "recently";

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
          borderRadius: 10, padding: "28px 32px", width: 380,
          boxShadow: "0 16px 48px rgba(0,0,0,0.8)",
        }}
      >
        <div style={{ fontSize: 15, fontWeight: 600, color: "var(--wc-text-bright)", marginBottom: 8 }}>
          Recover Unsaved Work
        </div>
        <div style={{ fontSize: 13, color: "var(--wc-text-secondary)", marginBottom: 6 }}>
          Inkue did not close properly.
        </div>
        <div style={{ fontSize: 13, color: "var(--wc-text)", marginBottom: 24 }}>
          Recover unsaved work from{" "}
          <span style={{ color: "var(--wc-text-bright)", fontWeight: 500 }}>"{label}"</span>
          {" "}(last edited {when})?
        </div>
        <div style={{ display: "flex", gap: 8, justifyContent: "flex-end" }}>
          <DialogBtn label="Discard" onClick={onDiscard} danger />
          <DialogBtn label="Recover" onClick={onRecover} primary />
        </div>
      </div>
    </div>
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
  onCheck,
  onLogs,
  onPreferences,
  onAbout,
  recentFiles,
  onOpenRecent,
}: {
  onSave: () => void;
  onSaveAs: () => void;
  onOpen: () => void;
  onNew: () => void;
  onCollect: () => void;
  onCheck: () => void;
  onLogs: () => void;
  onPreferences: () => void;
  onAbout: () => void;
  recentFiles: string[];
  onOpenRecent: (path: string) => void;
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
    | { type: "item"; label: string; shortcut?: string; action: () => void; muted?: boolean }
    | { type: "separator" }
  > = [
    { type: "item", label: "New Workspace",    shortcut: "Ctrl+N",         action: act(onNew) },
    { type: "item", label: "Open…",           shortcut: "Ctrl+O",         action: act(onOpen) },
    ...(recentFiles.length > 0
      ? [
          { type: "separator" as const },
          ...recentFiles.map((p) => ({
            type: "item" as const,
            label: p.split(/[\\/]/).pop() ?? p,
            action: act(() => onOpenRecent(p)),
            muted: true,
          })),
        ]
      : []),
    { type: "separator" },
    { type: "item", label: "Save",              shortcut: "Ctrl+S",         action: act(onSave) },
    { type: "item", label: "Save As…",         shortcut: "Ctrl+Shift+S",   action: act(onSaveAs) },
    { type: "item", label: "Collect and Save…",                            action: act(onCollect) },
    { type: "separator" },
    { type: "item", label: "Check Workspace…",                             action: act(onCheck) },
    { type: "item", label: "Logs…",                                        action: act(onLogs) },
    { type: "separator" },
    { type: "item", label: "Preferences",     shortcut: "Ctrl+,",          action: act(onPreferences) },
    { type: "separator" },
    { type: "item", label: "About Inkue",                                 action: act(onAbout) },
    { type: "separator" },
    { type: "item", label: "Quit",                                         action: handleQuit },
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
                  border: "none",
                  color: item.muted ? "var(--wc-text-secondary)" : "var(--wc-text)",
                  fontSize: 13, cursor: "pointer", textAlign: "left", gap: 24,
                }}
              >
                <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", maxWidth: 180 }}>
                  {item.label}
                </span>
                {item.shortcut && (
                  <span style={{ color: "var(--wc-text-muted)", fontSize: 11, flexShrink: 0 }}>{item.shortcut}</span>
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

const LS_LAYOUT_KEY = "inkue_ui_layout";

interface UiLayout {
  showCueListTabs: boolean;
  inspectorOpen: boolean;
  showSearchBar: boolean;
}

const DEFAULT_UI_LAYOUT: UiLayout = { showCueListTabs: true, inspectorOpen: true, showSearchBar: true };

function loadUiLayout(): UiLayout {
  try {
    const raw = localStorage.getItem(LS_LAYOUT_KEY);
    if (!raw) return DEFAULT_UI_LAYOUT;
    const parsed = JSON.parse(raw) as Partial<UiLayout>;
    return {
      showCueListTabs: parsed.showCueListTabs ?? true,
      inspectorOpen: parsed.inspectorOpen ?? true,
      showSearchBar: parsed.showSearchBar ?? true,
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
// Search results overlay
// ---------------------------------------------------------------------------

function flattenCues(cues: CueSummary[]): CueSummary[] {
  return cues.flatMap((c) => [c, ...(c.children ? flattenCues(c.children) : [])]);
}

function SearchResults({
  query,
  allCues,
  onSelect,
}: {
  query: string;
  allCues: CueSummary[];
  onSelect: (id: string) => void;
}) {
  const q = query.toLowerCase();
  const matches = flattenCues(allCues).filter(
    (c) =>
      c.name?.toLowerCase().includes(q) ||
      (c.number ?? "").toLowerCase().includes(q),
  );

  if (matches.length === 0) {
    return (
      <div style={{ padding: "16px", fontSize: 12, color: "var(--wc-text-faint)", textAlign: "center" }}>
        No cues match "{query}"
      </div>
    );
  }

  return (
    <div style={{ flex: 1, overflowY: "auto" }}>
      {matches.map((cue) => (
        <button
          key={cue.id}
          onClick={() => onSelect(cue.id)}
          style={{
            display: "flex", alignItems: "center", gap: 10,
            width: "100%", padding: "7px 12px",
            background: "transparent", border: "none",
            borderBottom: "1px solid var(--wc-border)",
            color: "var(--wc-text)", cursor: "pointer", textAlign: "left",
          }}
          onMouseEnter={(e) => { (e.currentTarget as HTMLElement).style.background = "var(--wc-bg-hover)"; }}
          onMouseLeave={(e) => { (e.currentTarget as HTMLElement).style.background = "transparent"; }}
        >
          {cue.color && (
            <div style={{ width: 3, height: 22, borderRadius: 2, background: cue.color, flexShrink: 0 }} />
          )}
          <span style={{ fontSize: 11, color: "var(--wc-text-muted)", width: 36, flexShrink: 0, fontFamily: "monospace" }}>
            {cue.number ?? ""}
          </span>
          <span style={{ fontSize: 13, flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            {cue.name || <span style={{ color: "var(--wc-text-faint)" }}>(unnamed)</span>}
          </span>
          <span style={{ fontSize: 11, color: "var(--wc-text-faint)", flexShrink: 0 }}>
            {cue.cue_type}
          </span>
        </button>
      ))}
    </div>
  );
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

/** Quick press-and-flash micro-interaction when a toolbar button is clicked.
 *  Uses the Web Animations API so it needs no React state or re-render and
 *  plays in the button's own accent color (via the brightness boost). */
function pulseButton(el: HTMLElement) {
  el.animate(
    [
      { transform: "scale(1)",    filter: "brightness(1)" },
      { transform: "scale(0.88)", filter: "brightness(1.7)", offset: 0.35 },
      { transform: "scale(1)",    filter: "brightness(1)" },
    ],
    { duration: 260, easing: "cubic-bezier(.2,.7,.3,1)" },
  );
}

export default function App() {
  const { refreshCues, refreshWorkspaceInfo, refreshValidation, refreshHealth, brokenCueIds, loadGeneralPrefs, loadDisplayPrefs, displayPrefs, workspaceInfo, selectedCueId, selectedCueIds, cues, cueLists, activeCueListId } =
    useWorkspaceStore();

  const [inspectorOpen, setInspectorOpen]         = useState(() => loadUiLayout().inspectorOpen);
  const [showCueListTabs, setShowCueListTabs]     = useState(() => loadUiLayout().showCueListTabs);
  const [showSearchBar, setShowSearchBar]         = useState(() => loadUiLayout().showSearchBar);
  const [showMode, setShowMode]                   = useState(false);
  const [closeDialogOpen, setCloseDialogOpen]     = useState(false);
  const [gotoOpen, setGotoOpen]                   = useState(false);
  const [outputSurfaceVisible, setOutputSurfaceVisible] = useState(false);
  const [loadError, setLoadError]                 = useState<string | null>(null);
  const [collectReport, setCollectReport]         = useState<CollectReport | null>(null);
  const [recentFiles, setRecentFiles]             = useState<string[]>(loadRecentFiles);
  const [showAbout, setShowAbout]                 = useState(false);
  const [preflightOpen, setPreflightOpen]         = useState(false);
  const [logsOpen, setLogsOpen]                   = useState(false);
  const [searchQuery, setSearchQuery]             = useState("");
  const [recoveryInfo, setRecoveryInfo]           = useState<RecoveryInfo | null>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);

  // Persist panel visibility across launches.
  useEffect(() => {
    saveUiLayout({ showCueListTabs, inspectorOpen, showSearchBar });
  }, [showCueListTabs, inspectorOpen, showSearchBar]);

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
    void refreshValidation();
    void refreshHealth();
    loadGeneralPrefs();
    loadDisplayPrefs();
    void getOutputWindowVisible().then(setOutputSurfaceVisible);

    let unlistenVisible: (() => void) | undefined;
    listen<boolean>("output-window-visible", (e) => {
      setOutputSurfaceVisible(e.payload);
    }).then((u) => { unlistenVisible = u; }).catch(console.error);
    return () => { unlistenVisible?.(); };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // One-time crash-recovery prompt: a snapshot left by a previous session means
  // it ended abnormally (crash / power loss) with unsaved work. Offer to restore.
  const recoveryPrompted = useRef(false);
  useEffect(() => {
    if (recoveryPrompted.current) return;
    recoveryPrompted.current = true;
    void checkRecovery()
      .then((info) => { if (info) setRecoveryInfo(info); })
      .catch((err) => console.error("recovery check failed", err));
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const handleRecover = useCallback(async () => {
    setRecoveryInfo(null);
    try {
      await restoreRecovery();
      await refreshCues();
      await refreshWorkspaceInfo();
    } catch (err) {
      console.error("restore recovery failed", err);
      setLoadError(`Recovery failed: ${String(err)}`);
    }
  }, [refreshCues, refreshWorkspaceInfo]);

  const handleDiscardRecovery = useCallback(async () => {
    setRecoveryInfo(null);
    await discardRecovery().catch((err) => console.error("discard recovery failed", err));
  }, []);

  // -------------------------------------------------------------------------
  // Shared save helpers (used by FileMenu AND CloseConfirmDialog)
  // -------------------------------------------------------------------------

  /** Save to the existing path, or open Save As if no path is set yet.
   *  Returns true if the save completed, false if the user cancelled. */
  const handleSaveAs = useCallback(async (): Promise<boolean> => {
    const path = await saveDialog({
      filters: [{ name: "Inkue Workspace", extensions: ["inkue"] }],
      defaultPath: (workspaceInfo?.name ?? "Untitled") + ".inkue",
    });
    if (typeof path !== "string") return false;
    const filePath = path.endsWith(".inkue") ? path : path + ".inkue";
    await saveWorkspace(filePath).catch(console.error);
    await refreshWorkspaceInfo();
    setRecentFiles(pushRecentFile(filePath));
    return true;
  }, [workspaceInfo, refreshWorkspaceInfo]);

  const handleSave = useCallback(async (): Promise<boolean> => {
    const path = workspaceInfo?.file_path;
    if (path) {
      await saveWorkspace(path).catch(console.error);
      await refreshWorkspaceInfo();
      setRecentFiles(pushRecentFile(path));
      return true;
    }
    return handleSaveAs();
  }, [workspaceInfo, refreshWorkspaceInfo, handleSaveAs]);

  const openWorkspacePath = useCallback(async (path: string) => {
    await loadWorkspace(path).catch(console.error);
    setRecentFiles(pushRecentFile(path));
    setSearchQuery("");
  }, []);

  const handleOpen = useCallback(async () => {
    const path = await openDialog({
      multiple: false,
      filters: [{ name: "Inkue Workspace", extensions: ["inkue", "wincue"] }],
    });
    if (typeof path === "string") {
      await openWorkspacePath(path);
    }
  }, [openWorkspacePath]);

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

  // On Linux, GNOME/Mutter may restore a previous maximised *or* fullscreen state
  // from the session — overriding tauri.conf.json ("maximized": false,
  // "fullscreen": false) — and it applies that state *after* the window is mapped.
  // Normalise the window on mount and once more on the next tick so we reliably win
  // that race and always start at the configured 1280×800 size.
  useEffect(() => {
    const normalize = () => {
      const win = getCurrentWindow();
      void win.setFullscreen(false).catch(() => {});
      void win.unmaximize().catch(() => {});
    };
    normalize();
    const id = setTimeout(normalize, 150);
    return () => clearTimeout(id);
  }, []);

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
    () => handleToggleSearch(),
  );

  const selectedCue = findCueRecursive(cues, selectedCueId) ?? null;

  const handleToggleSurface = async () => {
    await toggleOutputWindow().catch(console.error);
    // outputSurfaceVisible is driven by the "output-window-visible" event from Rust
  };

  // Show/hide the search bar. Showing it focuses the input; hiding clears the
  // query so the cue list reappears without a lingering filter.
  const handleToggleSearch = () => {
    if (showSearchBar) {
      setSearchQuery("");
      setShowSearchBar(false);
    } else {
      setShowSearchBar(true);
      requestAnimationFrame(() => searchInputRef.current?.focus());
    }
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

  const handleAddText = async () => {
    const { selectedCueId, cues } = useWorkspaceStore.getState();
    const idx = cues.findIndex((c) => c.id === selectedCueId);
    await addCue("text", idx >= 0 ? idx + 1 : -1).catch(console.error);
    await refreshCues();
  };

  const dispatchCueDrag = (cueType: "audio" | "stop" | "video" | "image" | "group" | "wait" | "osc" | "fade" | "midi" | "light" | "mic" | "timecode" | "text", e: React.MouseEvent) => {
    if (e.button !== 0) return;
    document.dispatchEvent(
      new CustomEvent("inkue:cue-drag-start", {
        detail: { cueType, startX: e.clientX, startY: e.clientY },
      }),
    );
  };

  const titleBarName = workspaceInfo
    ? `${workspaceInfo.name}${workspaceInfo.is_modified ? " •" : ""}`
    : "Inkue";

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

      {/* Crash-recovery dialog */}
      {recoveryInfo && (
        <RecoveryDialog
          info={recoveryInfo}
          onRecover={() => void handleRecover()}
          onDiscard={() => void handleDiscardRecovery()}
        />
      )}

      {/* About dialog */}
      {showAbout && <AboutDialog onClose={() => setShowAbout(false)} />}
      {preflightOpen && <PreflightModal onClose={() => setPreflightOpen(false)} />}
      {logsOpen && <LogViewerModal onClose={() => setLogsOpen(false)} />}

      {/* Custom title bar — two rows: Row 1 holds the window controls, menus and a
          full-width drag area; Row 2 holds the cue toolbar.  Splitting them means
          the toolbar can never squeeze the drag area down to an ungrabbable sliver
          when the window is narrow (the previous single-row layout collapsed it to
          ~40px, so only the "Inkue" label was draggable).  No drag-region on the
          row containers so menus/buttons keep working on Linux/WebKitGTK. */}
      <div
        style={{
          display: "flex", flexDirection: "column",
          background: "var(--wc-bg-surface)", borderBottom: "1px solid var(--wc-border)",
          flexShrink: 0, userSelect: "none", WebkitUserSelect: "none",
        }}
      >
        {/* Row 1 — window controls, File/View menus, draggable workspace title */}
        <div style={{ display: "flex", alignItems: "center", height: 36, padding: "0 12px", gap: 12 }}>
        <WindowControls />
        <FileMenu
          onSave={() => void handleSave()}
          onSaveAs={() => void handleSaveAs()}
          onOpen={() => void handleOpen()}
          onNew={() => void handleNew()}
          onCollect={() => void handleCollectAndSave()}
          onCheck={() => setPreflightOpen(true)}
          onLogs={() => setLogsOpen(true)}
          onPreferences={() => void openPreferencesWindow()}
          onAbout={() => setShowAbout(true)}
          recentFiles={recentFiles}
          onOpenRecent={(p) => void openWorkspacePath(p)}
        />
        <ViewMenu
          items={[
            { label: "Show Mode",      checked: showMode,             onClick: () => setShowMode((v) => !v),       shortcut: "F5" },
            { label: "Cue List Tabs",  checked: showCueListTabs,      onClick: () => setShowCueListTabs((v) => !v) },
            { label: "Search Bar",     checked: showSearchBar,         onClick: handleToggleSearch, shortcut: "Ctrl+F" },
            { label: "Inspector",      checked: inspectorOpen,         onClick: () => setInspectorOpen((v) => !v) },
            { label: "Output Surface", checked: outputSurfaceVisible,  onClick: () => void handleToggleSurface() },
          ]}
        />

        {/* Drag region: app name + workspace name */}
        <div
          data-tauri-drag-region
          style={{ flex: 1, minWidth: 40, display: "flex", alignItems: "center", gap: 10, overflow: "hidden" }}
        >
          <span
            data-tauri-drag-region
            style={{ fontWeight: 700, fontSize: 13, color: "var(--wc-text-bright)", flexShrink: 0 }}
          >
            Inkue
          </span>
          <span
            data-tauri-drag-region
            style={{ fontSize: 12, color: "var(--wc-text-muted)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}
          >
            {titleBarName}
          </span>
          {brokenCueIds.size > 0 && (
            <button
              onClick={() => setPreflightOpen(true)}
              title="Some cues have problems — click to check"
              style={{
                flexShrink: 0, display: "flex", alignItems: "center", gap: 4,
                background: "rgba(239,68,68,0.15)", border: "1px solid rgba(239,68,68,0.5)",
                borderRadius: 5, color: "#ef4444", cursor: "pointer",
                fontSize: 11, padding: "2px 8px",
              }}
            >
              ⚠ {brokenCueIds.size}
            </button>
          )}
        </div>

        </div>{/* end Row 1 */}

        {/* Row 2 — cue toolbar on its own row.  It never competes with the Row 1
            drag area, and wraps onto extra lines when the window is too narrow to
            fit every button, so they all stay reachable.  Hidden in Show Mode. */}
        <div
          style={{ display: showMode ? "none" : "flex", flexWrap: "wrap", gap: 6, padding: "0 12px 6px", alignItems: "center" }}
          onClick={(e) => { const btn = (e.target as HTMLElement).closest("button"); if (btn) pulseButton(btn); }}
        >
          <button
            style={{ ...toolbarBtn, color: CUE_TYPE_COLORS.audio, cursor: "pointer", userSelect: "none" }}
            onClick={handleAddAudio}
            onMouseDown={(e) => dispatchCueDrag("audio", e)}
            title="Add Audio Cue after selection (Ctrl+N) · Drag to insert at position"
          >
            + Audio
          </button>
          <button
            style={{ ...toolbarBtn, color: CUE_TYPE_COLORS.video, cursor: "pointer", userSelect: "none" }}
            onClick={handleAddVideo}
            onMouseDown={(e) => dispatchCueDrag("video", e)}
            title="Add Video Cue after selection · Drag to insert at position"
          >
            + Video
          </button>
          <button
            style={{ ...toolbarBtn, color: CUE_TYPE_COLORS.image, cursor: "pointer", userSelect: "none" }}
            onClick={handleAddImage}
            onMouseDown={(e) => dispatchCueDrag("image", e)}
            title="Add Image Cue after selection · Drag to insert at position"
          >
            + Image
          </button>
          <button
            style={{ ...toolbarBtn, color: CUE_TYPE_COLORS.stop, cursor: "pointer", userSelect: "none" }}
            onClick={handleAddStop}
            onMouseDown={(e) => dispatchCueDrag("stop", e)}
            title="Add Stop Cue after selection · Drag to insert at position"
          >
            + Stop
          </button>
          <button
            style={{ ...toolbarBtn, color: CUE_TYPE_COLORS.fade, cursor: "pointer", userSelect: "none" }}
            onClick={handleAddFade}
            onMouseDown={(e) => dispatchCueDrag("fade", e)}
            title="Add Fade Cue after selection · Drag to insert at position"
          >
            + Fade
          </button>
          <button
            style={{ ...toolbarBtn, color: CUE_TYPE_COLORS.wait, cursor: "pointer", userSelect: "none" }}
            onClick={handleAddWait}
            onMouseDown={(e) => dispatchCueDrag("wait", e)}
            title="Add Wait Cue after selection · Drag to insert at position"
          >
            + Wait
          </button>
          <button
            style={{ ...toolbarBtn, color: CUE_TYPE_COLORS.group, cursor: "pointer", userSelect: "none" }}
            onClick={handleAddGroup}
            onMouseDown={(e) => dispatchCueDrag("group", e)}
            title="Add Group Cue after selection · Drag to insert at position"
          >
            + Group
          </button>
          <button
            style={{ ...toolbarBtn, color: CUE_TYPE_COLORS.midi, cursor: "pointer", userSelect: "none" }}
            onClick={handleAddMidi}
            onMouseDown={(e) => dispatchCueDrag("midi", e)}
            title="Add MIDI Cue after selection · Drag to insert at position"
          >
            + MIDI
          </button>
          <button
            style={{ ...toolbarBtn, color: CUE_TYPE_COLORS.osc, cursor: "pointer", userSelect: "none" }}
            onClick={handleAddOsc}
            onMouseDown={(e) => dispatchCueDrag("osc", e)}
            title="Add OSC Cue after selection · Drag to insert at position"
          >
            + OSC
          </button>
          <button
            style={{ ...toolbarBtn, color: CUE_TYPE_COLORS.light, cursor: "pointer", userSelect: "none" }}
            onClick={handleAddLight}
            onMouseDown={(e) => dispatchCueDrag("light", e)}
            title="Add Light Cue after selection · Drag to insert at position"
          >
            + Light
          </button>
          <button
            style={{ ...toolbarBtn, color: CUE_TYPE_COLORS.mic, cursor: "pointer", userSelect: "none" }}
            onClick={handleAddMic}
            onMouseDown={(e) => dispatchCueDrag("mic", e)}
            title="Add Mic Cue after selection · Drag to insert at position"
          >
            + Mic
          </button>
          <button
            style={{ ...toolbarBtn, color: CUE_TYPE_COLORS.timecode, cursor: "pointer", userSelect: "none" }}
            onClick={handleAddTimecode}
            onMouseDown={(e) => dispatchCueDrag("timecode", e)}
            title="Add Timecode Cue after selection · Drag to insert at position"
          >
            + TC
          </button>
          <button
            style={{ ...toolbarBtn, color: CUE_TYPE_COLORS.text, cursor: "pointer", userSelect: "none" }}
            onClick={handleAddText}
            onMouseDown={(e) => dispatchCueDrag("text", e)}
            title="Add Text Cue after selection · Drag to insert at position"
          >
            + Text
          </button>
          <button style={toolbarBtn} onClick={() => setInspectorOpen((v) => !v)} title="Toggle Inspector (Ctrl+I)">
            Inspector
          </button>
        </div>
      </div>

      {/* Runtime health banner (device/network faults) */}
      <HealthBanner />

      {/* Main area */}
      <div style={{ display: "flex", flex: 1, overflow: "hidden" }}>
        {showMode ? (
          <ShowModeView />
        ) : (
          <>
            <div style={{ flex: 1, minWidth: 0, minHeight: 0, overflow: "hidden", display: "flex", flexDirection: "column" }}>
              {showCueListTabs && <CueListTabs onRefresh={handleRefresh} />}
              <ActiveCuesView />

              {searchQuery.trim() ? (
                <SearchResults
                  query={searchQuery.trim()}
                  allCues={cues}
                  onSelect={(id) => {
                    useWorkspaceStore.getState().setSelectedCueId(id);
                    setInspectorOpen(true);
                    setSearchQuery("");
                  }}
                />
              ) : (
                (() => {
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
                })()
              )}

              {/* Search bar — anchored at the bottom of the cue list */}
              {showSearchBar && (
                <div style={{ padding: "4px 8px", borderTop: "1px solid var(--wc-border)", flexShrink: 0 }}>
                  <input
                    ref={searchInputRef}
                    value={searchQuery}
                    onChange={(e) => setSearchQuery(e.target.value)}
                    onKeyDown={(e) => { if (e.key === "Escape") setSearchQuery(""); }}
                    placeholder="Search cues…"
                    style={{
                      width: "100%", boxSizing: "border-box",
                      background: searchQuery ? "var(--wc-bg-surface)" : "transparent",
                      border: searchQuery ? "1px solid var(--wc-accent)" : "1px solid transparent",
                      borderRadius: 4, color: "var(--wc-text)", fontSize: 12,
                      padding: "4px 8px", outline: "none",
                      transition: "border-color 0.15s, background 0.15s",
                    }}
                  />
                </div>
              )}
            </div>
            {inspectorOpen && (() => {
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
