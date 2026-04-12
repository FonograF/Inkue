// Preferences — draggable floating modal overlaid on the workspace.
// Opened via File → Preferences or Ctrl+,

import { useEffect, useRef, useState, useCallback } from "react";
import { createPortal } from "react-dom";
import type { AppPreferences, AudioPreferences, DeviceInfo, GeneralPreferences } from "../../lib/types";
import { CurveSelect } from "../common/CurveSelect";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import {
  getAsioOutputPairs,
  getAvailableBackends,
  getPreferences,
  listAudioDevices,
  testAudioDevice,
  updateAudioPreferences,
  updateGeneralPreferences,
} from "../../lib/commands";

// ---------------------------------------------------------------------------
// Sidebar categories
// ---------------------------------------------------------------------------

type Category = "audio" | "general" | "network" | "display";

const CATEGORIES: { id: Category; icon: string; label: string }[] = [
  { id: "audio",   icon: "🔊", label: "Audio"   },
  { id: "general", icon: "⚙️",  label: "General" },
  { id: "network", icon: "🌐", label: "Network"  },
  { id: "display", icon: "🖥",  label: "Display"  },
];

// ---------------------------------------------------------------------------
// Small reusable atoms
// ---------------------------------------------------------------------------

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div style={{ marginBottom: 24 }}>
      <div style={{
        fontSize: 11, fontWeight: 600, color: "#64748b",
        textTransform: "uppercase", letterSpacing: "0.07em",
        marginBottom: 10, paddingBottom: 5,
        borderBottom: "1px solid #1e293b",
      }}>
        {title}
      </div>
      {children}
    </div>
  );
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 12, marginBottom: 8, minHeight: 28 }}>
      <label style={{ width: 170, fontSize: 12, color: "#94a3b8", flexShrink: 0, textAlign: "right" }}>
        {label}
      </label>
      <div style={{ flex: 1, display: "flex", alignItems: "center", gap: 8 }}>
        {children}
      </div>
    </div>
  );
}

const inputStyle: React.CSSProperties = {
  background: "#0f172a", border: "1px solid #334155", borderRadius: 4,
  color: "#e2e8f0", fontSize: 12, padding: "3px 7px", width: "100%",
};
const selectStyle: React.CSSProperties = { ...inputStyle, cursor: "pointer" };
const btnStyle: React.CSSProperties = {
  padding: "3px 10px", background: "#1e293b", border: "1px solid #334155",
  borderRadius: 4, color: "#cbd5e1", fontSize: 11, cursor: "pointer",
};

// ---------------------------------------------------------------------------
// Audio content
// ---------------------------------------------------------------------------

function AudioContent({ prefs, onChange, availableBackends, onImmediateApply }: {
  prefs: AudioPreferences;
  onChange: (p: AudioPreferences) => void;
  availableBackends: string[];
  onImmediateApply?: (p: AudioPreferences) => Promise<void>;
}) {
  const [devices, setDevices] = useState<DeviceInfo[]>([]);
  const [devicesError, setDevicesError] = useState<string | null>(null);
  const [devicesLoading, setDevicesLoading] = useState(false);
  const [asioPairs, setAsioPairs] = useState<number>(1);
  const asioAvailable = availableBackends.includes("asio");
  const isAsio = prefs.backend === "asio";

  const loadDevices = useCallback(async (backend: AudioPreferences["backend"]) => {
    setDevicesLoading(true);
    setDevicesError(null);
    try {
      const list = await listAudioDevices(backend);
      setDevices(list);
    } catch (e) {
      setDevicesError(String(e));
      setDevices([]);
    } finally {
      setDevicesLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadDevices(prefs.backend);
    if (prefs.backend === "asio") {
      getAsioOutputPairs().then(setAsioPairs).catch(() => setAsioPairs(1));
    }
  }, [prefs.backend, loadDevices]);

  const currentDevice = devices.find((d) => d.id === prefs.device_id) ?? devices[0] ?? null;
  const latencyMs = currentDevice && prefs.buffer_size
    ? ((prefs.buffer_size / currentDevice.sample_rate) * 1000).toFixed(1)
    : "—";

  return (
    <>
      <Section title="Audio Engine">
        <Row label="Backend">
          <select
            style={selectStyle}
            value={prefs.backend}
            onChange={(e) =>
              onChange({
                ...prefs,
                backend: e.target.value as AudioPreferences["backend"],
                device_id: null,
              })
            }
          >
            <option value="wasapi_shared">WASAPI Shared</option>
            <option value="wasapi_exclusive">WASAPI Exclusive</option>
            <option value="asio" disabled={!asioAvailable}>
              ASIO{!asioAvailable ? " (install ASIO4ALL or ASIO drivers)" : ""}
            </option>
          </select>
        </Row>

        <Row label="Output Device">
          {devicesLoading ? (
            <span style={{ fontSize: 12, color: "#64748b" }}>Loading…</span>
          ) : devicesError ? (
            <span style={{ fontSize: 12, color: "#ef4444" }}>{devicesError}</span>
          ) : (
            <>
              <select
                style={selectStyle}
                value={prefs.device_id ?? ""}
                onChange={(e) =>
                  onChange({ ...prefs, device_id: e.target.value || null })
                }
              >
                <option value="">— System Default —</option>
                {devices.map((d) => (
                  <option key={d.id} value={d.id}>{d.name}</option>
                ))}
              </select>
              <button
                  style={btnStyle}
                  onClick={() => void testAudioDevice(prefs.device_id ?? "", prefs.backend)}
                  title="Play 440 Hz test tone on selected device"
                >
                Test
              </button>
            </>
          )}
        </Row>

        {isAsio && (
          <Row label="Output Pair">
            <select
              style={selectStyle}
              value={prefs.asio_out_pair}
              onChange={async (e) => {
                const next = { ...prefs, asio_out_pair: Number(e.target.value) };
                onChange(next);
                if (onImmediateApply) {
                  await onImmediateApply(next);
                  getAsioOutputPairs().then(setAsioPairs).catch(() => setAsioPairs(1));
                }
              }}
            >
              {Array.from({ length: Math.max(asioPairs, 1) }, (_, i) => (
                <option key={i} value={i}>
                  Out {i * 2 + 1}-{i * 2 + 2}
                </option>
              ))}
            </select>
            <span style={{ fontSize: 11, color: "#475569" }}>
              {asioPairs <= 1 ? "Apply first to detect pairs" : `${asioPairs} pair${asioPairs > 1 ? "s" : ""} available`}
            </span>
          </Row>
        )}

        <Row label="Buffer Size">
          <select
            style={selectStyle}
            value={prefs.buffer_size}
            onChange={(e) => onChange({ ...prefs, buffer_size: Number(e.target.value) })}
          >
            {[64, 128, 256, 512, 1024, 2048].map((s) => (
              <option key={s} value={s}>{s} samples</option>
            ))}
          </select>
          {prefs.backend === "wasapi_shared" && (
            <span style={{ fontSize: 11, color: "#475569" }}>ignored in shared mode</span>
          )}
        </Row>

        <Row label="Sample Rate">
          <span style={{ fontSize: 12, color: "#94a3b8" }}>
            {currentDevice?.sample_rate ?? "—"} Hz
            <span style={{ fontSize: 11, color: "#475569", marginLeft: 8 }}>(set by device)</span>
          </span>
        </Row>

        <Row label="Estimated Latency">
          <span style={{ fontSize: 12, color: "#22c55e", fontFamily: "monospace" }}>
            {latencyMs} ms
          </span>
        </Row>
      </Section>

      <Section title="Defaults">
        <Row label="Default Volume">
          <input
            type="range" min={-60} max={0} step={0.5}
            value={prefs.default_volume_db} style={{ flex: 1 }}
            onChange={(e) => onChange({ ...prefs, default_volume_db: Number(e.target.value) })}
          />
          <span style={{ width: 52, textAlign: "right", fontFamily: "monospace", fontSize: 12, color: "#94a3b8" }}>
            {prefs.default_volume_db.toFixed(1)} dB
          </span>
        </Row>
        <Row label="Fade Out on Stop (ms)">
          <input
            type="number" min={0} max={5000} step={50}
            style={{ ...inputStyle, width: 90 }}
            value={prefs.default_fade_out_ms}
            onChange={(e) => onChange({ ...prefs, default_fade_out_ms: Number(e.target.value) })}
          />
        </Row>
        <Row label="Default Fade Curve">
          <CurveSelect
            value={prefs.default_fade_curve}
            onChange={(v) => onChange({ ...prefs, default_fade_curve: v })}
            baseStyle={selectStyle}
          />
        </Row>
      </Section>
    </>
  );
}

// ---------------------------------------------------------------------------
// General content
// ---------------------------------------------------------------------------

function GeneralContent({ prefs, onChange }: {
  prefs: GeneralPreferences;
  onChange: (p: GeneralPreferences) => void;
}) {
  return (
    <>
      <Section title="Transport">
        <Row label="Double GO Protection">
          <input
            type="number" min={0} max={5000} step={50}
            style={{ ...inputStyle, width: 90 }}
            value={prefs.double_go_protection_ms}
            onChange={(e) => onChange({ ...prefs, double_go_protection_ms: Number(e.target.value) })}
          />
          <span style={{ fontSize: 11, color: "#475569" }}>ms (0 = disabled)</span>
        </Row>
      </Section>
      <Section title="Cue List">
        <Row label="Confirm Before Delete">
          <input
            type="checkbox"
            checked={prefs.confirm_before_delete}
            onChange={(e) => onChange({ ...prefs, confirm_before_delete: e.target.checked })}
            style={{ accentColor: "#3b82f6", width: 14, height: 14 }}
          />
        </Row>
        <Row label="Auto-Scroll to Playhead">
          <input
            type="checkbox"
            checked={prefs.auto_scroll_to_playhead}
            onChange={(e) => onChange({ ...prefs, auto_scroll_to_playhead: e.target.checked })}
            style={{ accentColor: "#3b82f6", width: 14, height: 14 }}
          />
        </Row>
        <Row label="Row Height">
          <select
            style={selectStyle}
            value={prefs.cue_row_height}
            onChange={(e) => onChange({ ...prefs, cue_row_height: e.target.value as GeneralPreferences["cue_row_height"] })}
          >
            <option value="compact">Compact</option>
            <option value="normal">Normal</option>
            <option value="tall">Tall</option>
          </select>
        </Row>
      </Section>
    </>
  );
}

// ---------------------------------------------------------------------------
// Draggable floating modal
// ---------------------------------------------------------------------------

interface Props {
  onClose: () => void;
}

const MODAL_W = 740;
const MODAL_H = 520;

export function PreferencesModal({ onClose }: Props) {
  const { setGeneralPrefs } = useWorkspaceStore();
  const [category, setCategory] = useState<Category>("audio");
  const [prefs, setPrefs] = useState<AppPreferences | null>(null);
  const [draft, setDraft] = useState<AppPreferences | null>(null);
  const [availableBackends, setAvailableBackends] = useState<string[]>(["wasapi_shared", "wasapi_exclusive"]);
  const [applyError, setApplyError] = useState<string | null>(null);

  // Drag state
  const posRef = useRef({ x: Math.round((window.innerWidth - MODAL_W) / 2), y: Math.round((window.innerHeight - MODAL_H) / 2) });
  const [pos, setPos] = useState(posRef.current);
  const dragRef = useRef<{ startMouseX: number; startMouseY: number; startPosX: number; startPosY: number } | null>(null);

  useEffect(() => {
    getPreferences().then((p) => { setPrefs(p); setDraft(p); }).catch(console.error);
    getAvailableBackends().then(setAvailableBackends).catch(console.error);
  }, []);

  // Escape closes without applying
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") { e.stopImmediatePropagation(); onClose(); }
    };
    window.addEventListener("keydown", handler, { capture: true });
    return () => window.removeEventListener("keydown", handler, { capture: true });
  }, [onClose]);

  // Drag handlers registered on document to track mouse outside the element
  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      if (!dragRef.current) return;
      const dx = e.clientX - dragRef.current.startMouseX;
      const dy = e.clientY - dragRef.current.startMouseY;
      const newPos = {
        x: dragRef.current.startPosX + dx,
        y: dragRef.current.startPosY + dy,
      };
      posRef.current = newPos;
      setPos({ ...newPos });
    };
    const onUp = () => { dragRef.current = null; document.body.style.cursor = ""; };
    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
    return () => { document.removeEventListener("mousemove", onMove); document.removeEventListener("mouseup", onUp); };
  }, []);

  const startDrag = (e: React.MouseEvent) => {
    // Don't start drag if clicking on a button
    if ((e.target as HTMLElement).closest("button")) return;
    e.preventDefault();
    dragRef.current = {
      startMouseX: e.clientX, startMouseY: e.clientY,
      startPosX: posRef.current.x, startPosY: posRef.current.y,
    };
    document.body.style.cursor = "grabbing";
  };

  const handleApply = async () => {
    if (!draft) return;
    setApplyError(null);
    try {
      await updateAudioPreferences(draft.audio);
      await updateGeneralPreferences(draft.general);
      setGeneralPrefs(draft.general);
      setPrefs(draft);
      onClose();
    } catch (e) {
      setApplyError(String(e));
    }
  };

  // Apply audio prefs immediately (without closing the modal).
  // Used by controls that should take effect in real-time, e.g. ASIO output pair.
  const handleImmediateApply = useCallback(async (audio: AudioPreferences) => {
    if (!draft) return;
    const newDraft = { ...draft, audio };
    setApplyError(null);
    try {
      await updateAudioPreferences(audio);
      setPrefs(newDraft);
      setDraft(newDraft);
    } catch (e) {
      setApplyError(String(e));
    }
  }, [draft]);

  const handleCancel = () => { setDraft(prefs); onClose(); };

  if (!draft) return null;

  return createPortal(
    <>
      {/* Dim backdrop — click outside closes */}
      <div
        style={{ position: "fixed", inset: 0, zIndex: 49999, background: "rgba(0,0,0,0.45)" }}
        onClick={handleCancel}
      />

      {/* Floating window */}
      <div
        style={{
          position: "fixed",
          left: pos.x, top: pos.y,
          width: MODAL_W, height: MODAL_H,
          zIndex: 50000,
          background: "#0f172a",
          border: "1px solid #334155",
          borderRadius: 8,
          boxShadow: "0 24px 64px rgba(0,0,0,0.8)",
          display: "flex", flexDirection: "column",
          overflow: "hidden",
        }}
        // Prevent backdrop click from firing when clicking inside
        onClick={(e) => e.stopPropagation()}
      >
        {/* Draggable title bar */}
        <div
          onMouseDown={startDrag}
          style={{
            height: 40, display: "flex", alignItems: "center",
            padding: "0 14px", flexShrink: 0,
            background: "#0f172a", borderBottom: "1px solid #1e293b",
            cursor: "grab", userSelect: "none",
          }}
        >
          <span style={{ fontSize: 13, fontWeight: 600, color: "#f1f5f9" }}>
            Preferences
          </span>
          <button
            onClick={handleCancel}
            style={{
              marginLeft: "auto", background: "transparent", border: "none",
              color: "#64748b", cursor: "pointer", fontSize: 16, lineHeight: 1, padding: 4,
            }}
          >
            ✕
          </button>
        </div>

        {/* Body */}
        <div style={{ display: "flex", flex: 1, overflow: "hidden" }}>
          {/* Sidebar */}
          <div style={{ width: 150, background: "#020617", borderRight: "1px solid #1e293b", padding: "8px 0", flexShrink: 0 }}>
            {CATEGORIES.map((cat) => (
              <button
                key={cat.id}
                onClick={() => setCategory(cat.id)}
                style={{
                  display: "flex", alignItems: "center", gap: 8,
                  width: "100%", padding: "7px 14px",
                  background: category === cat.id ? "#1d4ed8" : "transparent",
                  border: "none",
                  color: category === cat.id ? "#fff" : "#94a3b8",
                  fontSize: 12, cursor: "pointer", textAlign: "left",
                }}
              >
                <span style={{ fontSize: 14 }}>{cat.icon}</span>
                {cat.label}
              </button>
            ))}
          </div>

          {/* Content */}
          <div style={{ flex: 1, overflowY: "auto", padding: "18px 22px" }}>
            {category === "audio" && (
              <AudioContent
                prefs={draft.audio}
                onChange={(audio) => setDraft({ ...draft, audio })}
                availableBackends={availableBackends}
                onImmediateApply={handleImmediateApply}
              />
            )}
            {category === "general" && (
              <GeneralContent
                prefs={draft.general}
                onChange={(general) => setDraft({ ...draft, general })}
              />
            )}
            {category !== "audio" && category !== "general" && (
              <div style={{ height: "100%", display: "flex", alignItems: "center", justifyContent: "center", color: "#475569", fontSize: 13 }}>
                Coming soon
              </div>
            )}
          </div>
        </div>

        {/* Footer */}
        <div style={{
          height: applyError ? "auto" : 46, display: "flex", flexDirection: "column",
          justifyContent: "center",
          borderTop: "1px solid #1e293b", flexShrink: 0,
          background: "#0f172a",
        }}>
          {applyError && (
            <div style={{ padding: "6px 16px 0", fontSize: 11, color: "#ef4444" }}>
              {applyError}
            </div>
          )}
          <div style={{ display: "flex", alignItems: "center", justifyContent: "flex-end", gap: 8, padding: "8px 16px" }}>
          <button onClick={handleCancel} style={{ ...btnStyle, padding: "5px 16px", fontSize: 12 }}>
            Cancel
          </button>
          <button
            onClick={() => void handleApply()}
            style={{ ...btnStyle, padding: "5px 16px", fontSize: 12, background: "#1d4ed8", border: "1px solid #2563eb", color: "#fff" }}
          >
            Apply
          </button>
          </div>
        </div>
      </div>
    </>,
    document.body,
  );
}
