// Preferences — draggable floating modal overlaid on the workspace.
// Opened via File → Preferences or Ctrl+,

import { useEffect, useRef, useState, useCallback } from "react";
import { createPortal } from "react-dom";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { emit } from "@tauri-apps/api/event";
import type { AppPreferences, AudioPreferences, CueColorStyle, DeviceInfo, DisplayPreferences, GeneralPreferences, MachineAudioConfig, OscReceiveConfig, ScreenInfo, TimerPosition } from "../../lib/types";
import { DEFAULT_DISPLAY_PREFS, DEFAULT_MACHINE_AUDIO_CONFIG } from "../../lib/types";
import { CurveSelect } from "../common/CurveSelect";
import { Select } from "../common/Select";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import {
  getAsioOutputPairs,
  getAvailableBackends,
  getMachineAudioConfig,
  getOscConfig,
  getOutputScreen,
  getPreferences,
  listAudioDevices,
  listSystemFonts,
  listVideoScreens,
  previewOutputTimer,
  setOscConfig,
  setOutputScreen,
  testAudioDevice,
  updateAudioPreferences,
  updateDisplayPreferences,
  updateGeneralPreferences,
  updateMachineAudioConfig,
} from "../../lib/commands";
import { OscPatchesPanel } from "../OscPatches/OscPatchesPanel";

// ---------------------------------------------------------------------------
// Sidebar categories
// ---------------------------------------------------------------------------

type Category = "audio" | "general" | "network" | "display" | "personalization";

const CATEGORIES: { id: Category; icon: string; label: string }[] = [
  { id: "audio",           icon: "🔊", label: "Audio"           },
  { id: "general",         icon: "⚙️",  label: "General"         },
  { id: "network",         icon: "🌐", label: "Network"          },
  { id: "display",         icon: "🖥",  label: "Display"          },
  { id: "personalization", icon: "🎨", label: "Personalization" },
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

const BACKEND_LABELS: Record<string, string> = {
  wasapi_shared:    "WASAPI Shared",
  wasapi_exclusive: "WASAPI Exclusive",
  asio:             "ASIO",
  system_default:   "System Default (CoreAudio / ALSA)",
};

// ---------------------------------------------------------------------------
// Audio content
// ---------------------------------------------------------------------------

function AudioContent({
  machineConfig,
  audioPrefs,
  onMachineConfigChange,
  onAudioPrefsChange,
  availableBackends,
  onImmediateApplyMachine,
}: {
  machineConfig: MachineAudioConfig;
  audioPrefs: AudioPreferences;
  onMachineConfigChange: (c: MachineAudioConfig) => void;
  onAudioPrefsChange: (p: AudioPreferences) => void;
  availableBackends: string[];
  onImmediateApplyMachine?: (c: MachineAudioConfig) => Promise<void>;
}) {
  const [devices, setDevices] = useState<DeviceInfo[]>([]);
  const [devicesError, setDevicesError] = useState<string | null>(null);
  const [devicesLoading, setDevicesLoading] = useState(false);
  const [asioPairs, setAsioPairs] = useState<number>(1);
  const asioAvailable = availableBackends.includes("asio");
  const isAsio = machineConfig.backend === "asio";
  const isShared = machineConfig.backend === "wasapi_shared";

  const loadDevices = useCallback(async (backend: MachineAudioConfig["backend"]) => {
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
    void loadDevices(machineConfig.backend);
    if (machineConfig.backend === "asio") {
      getAsioOutputPairs().then(setAsioPairs).catch(() => setAsioPairs(1));
    }
  }, [machineConfig.backend, loadDevices]);

  const currentDevice = devices.find((d) => d.id === machineConfig.device_id) ?? devices[0] ?? null;
  const latencyMs = !isShared && currentDevice && machineConfig.buffer_size
    ? ((machineConfig.buffer_size / currentDevice.sample_rate) * 1000).toFixed(1)
    : "—";

  return (
    <>
      <Section title="Audio Engine">
        <Row label="Backend">
          <Select
            style={selectStyle}
            value={machineConfig.backend}
            onChange={(e) =>
              onMachineConfigChange({
                ...machineConfig,
                backend: e.target.value as MachineAudioConfig["backend"],
                device_id: null,
              })
            }
          >
            {availableBackends.map((id) => (
              <option key={id} value={id} disabled={id === "asio" && !asioAvailable}>
                {BACKEND_LABELS[id] ?? id}
                {id === "asio" && !asioAvailable ? " (install ASIO4ALL or ASIO drivers)" : ""}
              </option>
            ))}
          </Select>
        </Row>

        <Row label="Output Device">
          {devicesLoading ? (
            <span style={{ fontSize: 12, color: "#64748b" }}>Loading…</span>
          ) : devicesError ? (
            <span style={{ fontSize: 12, color: "#ef4444" }}>{devicesError}</span>
          ) : (
            <>
              <Select
                style={selectStyle}
                value={machineConfig.device_id ?? ""}
                onChange={(e) =>
                  onMachineConfigChange({ ...machineConfig, device_id: e.target.value || null })
                }
              >
                <option value="">— System Default —</option>
                {devices.map((d) => (
                  <option key={d.id} value={d.id}>{d.name}</option>
                ))}
              </Select>
              <button
                style={btnStyle}
                onClick={() => void testAudioDevice(machineConfig.device_id ?? "", machineConfig.backend)}
                title="Play 440 Hz test tone on selected device"
              >
                Test
              </button>
            </>
          )}
        </Row>

        {isAsio && (
          <Row label="Output Pair">
            <Select
              style={selectStyle}
              value={machineConfig.asio_out_pair}
              onChange={async (e) => {
                const next = { ...machineConfig, asio_out_pair: Number(e.target.value) };
                onMachineConfigChange(next);
                if (onImmediateApplyMachine) {
                  await onImmediateApplyMachine(next);
                  getAsioOutputPairs().then(setAsioPairs).catch(() => setAsioPairs(1));
                }
              }}
            >
              {Array.from({ length: Math.max(asioPairs, 1) }, (_, i) => (
                <option key={i} value={i}>
                  Out {i * 2 + 1}-{i * 2 + 2}
                </option>
              ))}
            </Select>
            <span style={{ fontSize: 11, color: "#475569" }}>
              {asioPairs <= 1 ? "Apply first to detect pairs" : `${asioPairs} pair${asioPairs > 1 ? "s" : ""} available`}
            </span>
          </Row>
        )}

        <Row label="Buffer Size">
          <Select
            style={{ ...selectStyle, opacity: isShared ? 0.4 : 1 }}
            value={machineConfig.buffer_size}
            disabled={isShared}
            onChange={(e) => onMachineConfigChange({ ...machineConfig, buffer_size: Number(e.target.value) })}
          >
            {[64, 128, 256, 512, 1024, 2048].map((s) => (
              <option key={s} value={s}>{s} samples</option>
            ))}
          </Select>
          {isShared && (
            <span style={{ fontSize: 11, color: "#475569" }}>managed by Windows in Shared mode</span>
          )}
          {isAsio && (
            <span style={{ fontSize: 11, color: "#475569" }}>set in ASIO driver control panel</span>
          )}
        </Row>

        <Row label="Sample Rate">
          <span style={{ fontSize: 12, color: "#94a3b8" }}>
            {currentDevice?.sample_rate ?? "—"} Hz
            <span style={{ fontSize: 11, color: "#475569", marginLeft: 8 }}>(set by device)</span>
          </span>
        </Row>

        {!isShared && (
          <Row label="Estimated Latency">
            <span style={{ fontSize: 12, color: "#22c55e", fontFamily: "monospace" }}>
              {latencyMs} ms
            </span>
          </Row>
        )}
      </Section>

      <Section title="Defaults">
        <Row label="Default Volume">
          <input
            type="range" min={-60} max={0} step={0.5}
            value={audioPrefs.default_volume_db} style={{ flex: 1 }}
            onChange={(e) => onAudioPrefsChange({ ...audioPrefs, default_volume_db: Number(e.target.value) })}
          />
          <span style={{ width: 52, textAlign: "right", fontFamily: "monospace", fontSize: 12, color: "#94a3b8" }}>
            {audioPrefs.default_volume_db.toFixed(1)} dB
          </span>
        </Row>
        <Row label="Fade Out on Stop (ms)">
          <input
            type="number" min={0} max={5000} step={50}
            style={{ ...inputStyle, width: 90 }}
            value={audioPrefs.default_fade_out_ms}
            onChange={(e) => onAudioPrefsChange({ ...audioPrefs, default_fade_out_ms: Number(e.target.value) })}
          />
        </Row>
        <Row label="Default Fade Curve">
          <CurveSelect
            value={audioPrefs.default_fade_curve}
            onChange={(v) => onAudioPrefsChange({ ...audioPrefs, default_fade_curve: v })}
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
          <Select
            style={selectStyle}
            value={prefs.cue_row_height}
            onChange={(e) => onChange({ ...prefs, cue_row_height: e.target.value as GeneralPreferences["cue_row_height"] })}
          >
            <option value="compact">Compact</option>
            <option value="normal">Normal</option>
            <option value="tall">Tall</option>
          </Select>
        </Row>
      </Section>
    </>
  );
}

// ---------------------------------------------------------------------------
// Display content
// ---------------------------------------------------------------------------

const THEME_COLORS: { key: keyof typeof DEFAULT_DISPLAY_PREFS; label: string; hint: string }[] = [
  { key: "bg_app",       label: "App Background", hint: "Main window background" },
  { key: "bg_surface",   label: "Surface",        hint: "Title bar, modals, inputs" },
  { key: "bg_panel",     label: "Panel",          hint: "Sidebars, buttons, menus" },
  { key: "accent",       label: "Accent",         hint: "Selection, playhead, active states" },
  { key: "text_primary", label: "Primary Text",   hint: "Main text colour" },
];

// ---------------------------------------------------------------------------
// Timer position picker — 3×3 grid with only the 5 valid positions active
// ---------------------------------------------------------------------------

const TIMER_POSITIONS: { pos: TimerPosition; gridArea: string; label: string }[] = [
  { pos: "top_left",     gridArea: "1 / 1", label: "↖" },
  { pos: "top_right",    gridArea: "1 / 3", label: "↗" },
  { pos: "center",       gridArea: "2 / 2", label: "⊙" },
  { pos: "bottom_left",  gridArea: "3 / 1", label: "↙" },
  { pos: "bottom_right", gridArea: "3 / 3", label: "↘" },
];

function TimerPositionPicker({ value, onChange }: { value: TimerPosition; onChange: (v: TimerPosition) => void }) {
  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: "repeat(3, 32px)",
        gridTemplateRows: "repeat(3, 32px)",
        gap: 3,
        background: "#0f172a",
        border: "1px solid #334155",
        borderRadius: 5,
        padding: 4,
      }}
    >
      {TIMER_POSITIONS.map(({ pos, gridArea, label }) => (
        <button
          key={pos}
          title={pos.replace(/_/g, " ")}
          onClick={() => onChange(pos)}
          style={{
            gridArea,
            width: 32, height: 32,
            border: value === pos ? "2px solid #3b82f6" : "1px solid #334155",
            borderRadius: 4,
            background: value === pos ? "#1d4ed8" : "#1e293b",
            color: value === pos ? "#fff" : "#94a3b8",
            fontSize: 16,
            cursor: "pointer",
            display: "flex", alignItems: "center", justifyContent: "center",
            lineHeight: 1,
          }}
        >
          {label}
        </button>
      ))}
    </div>
  );
}


function DisplayContent({
  outputScreen, onScreenChange,
  showOutputTimer, onTimerChange,
  timerFloating, onTimerFloatingChange,
  timerCountDown, onTimerModeChange,
  timerFont, onTimerFontChange,
  timerFontSize, onTimerFontSizeChange,
  timerPosition, onTimerPositionChange,
  timerShowMs, onTimerShowMsChange,
  timerMargin, onTimerMarginChange,
  timerPreview, onTimerPreviewChange,
  committedTimerStyle,
}: {
  outputScreen: number | null;
  onScreenChange: (screen: number | null) => void;
  showOutputTimer: boolean;
  onTimerChange: (v: boolean) => void;
  timerFloating: boolean;
  onTimerFloatingChange: (v: boolean) => void;
  timerCountDown: boolean;
  onTimerModeChange: (v: boolean) => void;
  timerFont: string;
  onTimerFontChange: (v: string) => void;
  timerFontSize: number;
  onTimerFontSizeChange: (v: number) => void;
  timerPosition: TimerPosition;
  onTimerPositionChange: (v: TimerPosition) => void;
  timerShowMs: boolean;
  onTimerShowMsChange: (v: boolean) => void;
  timerMargin: number;
  onTimerMarginChange: (v: number) => void;
  timerPreview: boolean;
  onTimerPreviewChange: (v: boolean) => void;
  /** Committed (applied) style, used to restore mpv state on cancel. */
  committedTimerStyle: { font: string; fontSize: number; position: TimerPosition; margin: number };
}) {
  const [screens, setScreens] = useState<ScreenInfo[]>([]);
  const [systemFonts, setSystemFonts] = useState<string[]>([]);

  useEffect(() => {
    listVideoScreens().then(setScreens).catch(console.error);
    listSystemFonts().then(setSystemFonts).catch(console.error);
  }, []);

  // Derive preview text from current draft show_ms setting.
  const previewText = timerShowMs ? "00:00.000" : "00:00";

  // Whenever draft style settings change while preview is on, push them live.
  useEffect(() => {
    if (!timerPreview) return;
    void previewOutputTimer(timerFont, timerFontSize, timerPosition, timerMargin, previewText);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [timerPreview, timerFont, timerFontSize, timerPosition, timerMargin, previewText]);

  const handlePreviewToggle = (on: boolean) => {
    onTimerPreviewChange(on);
    if (on) {
      void previewOutputTimer(timerFont, timerFontSize, timerPosition, timerMargin, previewText);
    } else {
      void previewOutputTimer(
        committedTimerStyle.font, committedTimerStyle.fontSize,
        committedTimerStyle.position, committedTimerStyle.margin,
        null,
      );
    }
  };

  return (
    <>
      <Section title="Output Surface">
        <Row label="Output Screen">
          <Select
            style={selectStyle}
            value={outputScreen ?? "floating"}
            onChange={(e) => {
              const v = e.target.value;
              onScreenChange(v === "floating" ? null : parseInt(v));
            }}
          >
            <option value="floating">Floating window</option>
            {screens.map((s) => (
              <option key={s.index} value={s.index}>
                {s.is_primary
                  ? `Screen ${s.index + 1} (primary, ${s.width}×${s.height})`
                  : `Screen ${s.index + 1} (${s.width}×${s.height})`}
              </option>
            ))}
          </Select>
        </Row>
        <Row label="">
          <span style={{ fontSize: 11, color: "#475569" }}>
            Applies to all Video and Image cues. Takes effect on the next GO.
          </span>
        </Row>
        <Row label="Output Timer">
          <label style={{ display: "flex", alignItems: "center", gap: 8, cursor: "pointer" }}>
            <input
              type="checkbox"
              checked={showOutputTimer}
              onChange={(e) => onTimerChange(e.target.checked)}
              style={{ width: 14, height: 14, cursor: "pointer" }}
            />
            <span style={{ fontSize: 13, color: "#cbd5e1" }}>
              Show cue timer on output window
            </span>
          </label>
        </Row>
        {showOutputTimer && (
          <>
            <Row label="Display mode">
              <label style={{ display: "flex", alignItems: "center", gap: 8, cursor: "pointer" }}>
                <input
                  type="checkbox"
                  checked={timerFloating}
                  onChange={(e) => onTimerFloatingChange(e.target.checked)}
                  style={{ width: 14, height: 14, cursor: "pointer" }}
                />
                <span style={{ fontSize: 13, color: "#cbd5e1" }}>
                  Floating window (replaces output overlay)
                </span>
              </label>
            </Row>
            <Row label="Timer mode">
              <div style={{ display: "flex", gap: 16 }}>
                <label style={{ display: "flex", alignItems: "center", gap: 6, cursor: "pointer" }}>
                  <input
                    type="radio"
                    checked={!timerCountDown}
                    onChange={() => onTimerModeChange(false)}
                    style={{ cursor: "pointer" }}
                  />
                  <span style={{ fontSize: 13, color: "#cbd5e1" }}>Elapsed (count up)</span>
                </label>
                <label style={{ display: "flex", alignItems: "center", gap: 6, cursor: "pointer" }}>
                  <input
                    type="radio"
                    checked={timerCountDown}
                    onChange={() => onTimerModeChange(true)}
                    style={{ cursor: "pointer" }}
                  />
                  <span style={{ fontSize: 13, color: "#cbd5e1" }}>Remaining (countdown)</span>
                </label>
              </div>
            </Row>
            <Row label="Milliseconds">
              <label style={{ display: "flex", alignItems: "center", gap: 8, cursor: "pointer" }}>
                <input
                  type="checkbox"
                  checked={timerShowMs}
                  onChange={(e) => onTimerShowMsChange(e.target.checked)}
                  style={{ width: 14, height: 14, cursor: "pointer" }}
                />
                <span style={{ fontSize: 13, color: "#cbd5e1" }}>Show milliseconds (1:23.456)</span>
              </label>
            </Row>
            <Row label="Position">
              <div style={{ opacity: timerFloating ? 0.35 : 1, pointerEvents: timerFloating ? "none" : undefined }}>
                <TimerPositionPicker value={timerPosition} onChange={onTimerPositionChange} />
              </div>
              {timerFloating && (
                <span style={{ fontSize: 11, color: "#475569", marginLeft: 8 }}>
                  n/a — window is freely positioned
                </span>
              )}
            </Row>
            {timerPosition !== "center" && !timerFloating && (
              <Row label="Corner margin">
                <input
                  type="range" min={0} max={300} step={5}
                  value={timerMargin} style={{ flex: 1 }}
                  onChange={(e) => onTimerMarginChange(Number(e.target.value))}
                />
                <span style={{ width: 40, textAlign: "right", fontFamily: "monospace", fontSize: 12, color: "#94a3b8" }}>
                  {timerMargin}px
                </span>
              </Row>
            )}
            <Row label="Preview">
              <label style={{ display: "flex", alignItems: "center", gap: 8, cursor: "pointer" }}>
                <input
                  type="checkbox"
                  checked={timerPreview}
                  onChange={(e) => handlePreviewToggle(e.target.checked)}
                  style={{ width: 14, height: 14, cursor: "pointer" }}
                />
                <span style={{ fontSize: 13, color: "#cbd5e1" }}>
                  Show on output window while configuring
                </span>
              </label>
            </Row>
            <Row label="Font">
              <>
                <input
                  type="text"
                  list="timer-font-list"
                  value={timerFont}
                  onChange={(e) => onTimerFontChange(e.target.value)}
                  placeholder="Font family name…"
                  style={{ ...inputStyle, flex: 1, fontFamily: timerFont }}
                />
                <datalist id="timer-font-list">
                  {systemFonts.map((f) => <option key={f} value={f} />)}
                </datalist>
              </>
            </Row>
            <Row label="Font size">
              <input
                type="number"
                min={20}
                max={400}
                step={4}
                value={timerFontSize}
                onChange={(e) => onTimerFontSizeChange(Number(e.target.value))}
                style={{ ...inputStyle, width: 80 }}
              />
              <span style={{ fontSize: 11, color: "#475569" }}>
                pt — 120 center, 60–80 corner
              </span>
            </Row>
          </>
        )}
      </Section>
    </>
  );
}

// ---------------------------------------------------------------------------
// Personalization content
// ---------------------------------------------------------------------------

const CUE_COLOR_STYLES: { value: CueColorStyle; label: string }[] = [
  { value: "stripe",   label: "Stripe (left edge only)" },
  { value: "full_row", label: "Full row"                 },
];

function PersonalizationContent({
  theme, onThemeChange,
}: {
  theme: typeof DEFAULT_DISPLAY_PREFS;
  onThemeChange: (t: typeof DEFAULT_DISPLAY_PREFS) => void;
}) {
  return (
    <>
      <Section title="Cue Appearance">
        <Row label="Cue Colour Style">
          <Select
            style={selectStyle}
            value={theme.cue_color_style}
            onChange={(e) => onThemeChange({ ...theme, cue_color_style: e.target.value as CueColorStyle })}
          >
            {CUE_COLOR_STYLES.map(({ value, label }) => (
              <option key={value} value={value}>{label}</option>
            ))}
          </Select>
        </Row>
      </Section>

      <Section title="Colour Theme">
        {THEME_COLORS.map(({ key, label, hint }) => (
          <Row key={key} label={label}>
            <div style={{ display: "flex", alignItems: "center", gap: 10, flex: 1 }}>
              <input
                type="color"
                value={theme[key]}
                onChange={(e) => onThemeChange({ ...theme, [key]: e.target.value })}
                style={{
                  width: 36, height: 26, padding: 2, cursor: "pointer",
                  background: "#1e293b", border: "1px solid #334155", borderRadius: 4,
                }}
              />
              <input
                type="text"
                value={theme[key]}
                onChange={(e) => {
                  const v = e.target.value;
                  if (/^#[0-9a-fA-F]{0,6}$/.test(v)) onThemeChange({ ...theme, [key]: v });
                }}
                style={{ ...inputStyle, width: 90, fontFamily: "monospace" }}
              />
              <span style={{ fontSize: 11, color: "#475569" }}>{hint}</span>
            </div>
          </Row>
        ))}
        <Row label="">
          <button
            onClick={() => onThemeChange({ ...DEFAULT_DISPLAY_PREFS })}
            style={{ ...btnStyle, padding: "4px 14px", fontSize: 11 }}
          >
            Reset to defaults
          </button>
        </Row>
      </Section>
    </>
  );
}

// ---------------------------------------------------------------------------
// OSC / Network content
// ---------------------------------------------------------------------------

function OscContent({
  config,
  onChange,
}: {
  config: OscReceiveConfig;
  onChange: (c: OscReceiveConfig) => void;
}) {
  return (
    <>
      <Section title="OSC Receive">
        <Row label="Enable">
          <label style={{ display: "flex", alignItems: "center", gap: 8, cursor: "pointer" }}>
            <input
              type="checkbox"
              checked={config.enabled}
              onChange={(e) => onChange({ ...config, enabled: e.target.checked })}
              style={{ width: 14, height: 14, cursor: "pointer" }}
            />
            <span style={{ fontSize: 13, color: "#cbd5e1" }}>
              Listen for OSC commands
            </span>
          </label>
        </Row>
        <Row label="Port">
          <input
            type="number"
            min={1024}
            max={65535}
            value={config.port}
            onChange={(e) => onChange({ ...config, port: Number(e.target.value) })}
            style={{ ...inputStyle, width: 100 }}
          />
          <span style={{ fontSize: 11, color: "#475569" }}>default 53001</span>
        </Row>
        <Row label="IP Allowlist">
          <div style={{ flex: 1 }}>
            <textarea
              rows={3}
              placeholder={"Leave empty to accept all.\nOne IP per line."}
              value={config.allowed_ips.join("\n")}
              onChange={(e) => {
                const ips = e.target.value.split("\n").map(s => s.trim()).filter(Boolean);
                onChange({ ...config, allowed_ips: ips });
              }}
              style={{ ...inputStyle, width: "100%", resize: "vertical", fontFamily: "monospace" }}
            />
            <span style={{ fontSize: 11, color: "#475569", display: "block", marginTop: 2 }}>
              Empty = accept all. One IP per line.
            </span>
          </div>
        </Row>
        <Row label="Address reference">
          <div style={{ flex: 1, fontSize: 11, color: "#64748b", lineHeight: 1.6 }}>
            <code style={{ fontFamily: "monospace" }}>/wincue/go</code> · <code>/wincue/stop</code> · <code>/wincue/hardstop</code><br />
            <code>/wincue/pause</code> · <code>/wincue/resume</code><br />
            <code>/wincue/cue/&#123;n&#125;/go</code> · <code>/wincue/cue/&#123;n&#125;/select</code> · <code>/wincue/cue/&#123;n&#125;/stop</code>
          </div>
        </Row>
      </Section>

      <OscPatchesPanel />

      <Section title="OSC Feedback">
        <Row label="Enable">
          <label style={{ display: "flex", alignItems: "center", gap: 8, cursor: "pointer" }}>
            <input
              type="checkbox"
              checked={config.feedback_enabled}
              onChange={(e) => onChange({ ...config, feedback_enabled: e.target.checked })}
              style={{ width: 14, height: 14, cursor: "pointer" }}
            />
            <span style={{ fontSize: 13, color: "#cbd5e1" }}>
              Broadcast active cue info via OSC
            </span>
          </label>
        </Row>
        {config.feedback_enabled && (
          <>
            <Row label="Destination">
              <input
                type="text"
                value={config.feedback_host}
                onChange={(e) => onChange({ ...config, feedback_host: e.target.value })}
                placeholder="127.0.0.1"
                style={{ ...inputStyle, flex: 1 }}
              />
              <span style={{ fontSize: 12, color: "#64748b" }}>:</span>
              <input
                type="number"
                min={1024}
                max={65535}
                value={config.feedback_port}
                onChange={(e) => onChange({ ...config, feedback_port: Number(e.target.value) })}
                style={{ ...inputStyle, width: 80 }}
              />
            </Row>
            <Row label="Messages sent">
              <div style={{ flex: 1, fontSize: 11, color: "#64748b", lineHeight: 1.7 }}>
                <code style={{ fontFamily: "monospace" }}>/wincue/cue/number</code> — cue number (string)<br />
                <code style={{ fontFamily: "monospace" }}>/wincue/cue/name</code> &nbsp;&nbsp;&nbsp;— cue name (string)<br />
                <code style={{ fontFamily: "monospace" }}>/wincue/cue/active</code> &nbsp;— 1 running / 0 stopped (int)<br />
                <span style={{ color: "#475569" }}>Sent on every active-cue change (GO, stop, auto-follow).</span>
              </div>
            </Row>
          </>
        )}
      </Section>
    </>
  );
}

// ---------------------------------------------------------------------------
// Draggable floating modal
// ---------------------------------------------------------------------------

interface Props {
  onClose: () => void;
  standalone?: boolean;
}

const MODAL_W = 740;
const MODAL_H = 520;

export function PreferencesModal({ onClose, standalone = false }: Props) {
  const { setGeneralPrefs, setDisplayPrefs } = useWorkspaceStore();
  const [category, setCategory] = useState<Category>("audio");
  const [prefs, setPrefs] = useState<AppPreferences | null>(null);
  const [draft, setDraft] = useState<AppPreferences | null>(null);
  // Machine audio config is stored separately from the workspace prefs.
  const [machineConfig, setMachineConfig] = useState<MachineAudioConfig>(DEFAULT_MACHINE_AUDIO_CONFIG);
  const [draftMachineConfig, setDraftMachineConfig] = useState<MachineAudioConfig>(DEFAULT_MACHINE_AUDIO_CONFIG);
  const [outputScreen, setOutputScreen_] = useState<number | null>(null);
  const [draftOutputScreen, setDraftOutputScreen] = useState<number | null>(null);
  const [showOutputTimer, setShowOutputTimer] = useState(false);
  const [draftShowOutputTimer, setDraftShowOutputTimer] = useState(false);
  const [timerCountDown, setTimerCountDown] = useState(false);
  const [draftTimerCountDown, setDraftTimerCountDown] = useState(false);
  const [timerFont, setTimerFont] = useState("DSEG7 Classic");
  const [draftTimerFont, setDraftTimerFont] = useState("DSEG7 Classic");
  const [timerFontSize, setTimerFontSize] = useState(120);
  const [draftTimerFontSize, setDraftTimerFontSize] = useState(120);
  const [timerPosition, setTimerPosition] = useState<TimerPosition>("center");
  const [draftTimerPosition, setDraftTimerPosition] = useState<TimerPosition>("center");
  const [timerShowMs, setTimerShowMs] = useState(false);
  const [draftTimerShowMs, setDraftTimerShowMs] = useState(false);
  const [timerMargin, setTimerMargin] = useState(50);
  const [draftTimerMargin, setDraftTimerMargin] = useState(50);
  const [timerPreview, setTimerPreview] = useState(false);
  const [timerFloating, setTimerFloating] = useState(false);
  const [draftTimerFloating, setDraftTimerFloating] = useState(false);
  const [theme, setTheme] = useState({ ...DEFAULT_DISPLAY_PREFS });
  const [draftTheme, setDraftTheme] = useState({ ...DEFAULT_DISPLAY_PREFS });
  const [availableBackends, setAvailableBackends] = useState<string[]>(["wasapi_shared", "wasapi_exclusive"]);
  const [oscConfig, setOscConfig_] = useState<OscReceiveConfig>({ enabled: false, port: 53001, allowed_ips: [], feedback_enabled: false, feedback_host: "127.0.0.1", feedback_port: 53000 });
  const [applyError, setApplyError] = useState<string | null>(null);

  // Drag state
  const posRef = useRef({ x: Math.round((window.innerWidth - MODAL_W) / 2), y: Math.round((window.innerHeight - MODAL_H) / 2) });
  const [pos, setPos] = useState(posRef.current);
  const dragRef = useRef<{ startMouseX: number; startMouseY: number; startPosX: number; startPosY: number } | null>(null);

  useEffect(() => {
    getPreferences().then((p) => {
      setPrefs(p);
      setDraft(p);
      const t = { ...DEFAULT_DISPLAY_PREFS, ...p.display };
      setTheme(t);
      setDraftTheme(t);
      const timer = p.display.show_output_timer ?? false;
      setShowOutputTimer(timer);
      setDraftShowOutputTimer(timer);
      const countDown = p.display.timer_count_down ?? false;
      setTimerCountDown(countDown);
      setDraftTimerCountDown(countDown);
      const font = p.display.timer_font ?? "DSEG7 Classic";
      setTimerFont(font);
      setDraftTimerFont(font);
      const fontSize = p.display.timer_font_size ?? 120;
      setTimerFontSize(fontSize);
      setDraftTimerFontSize(fontSize);
      const pos = p.display.timer_position ?? "center";
      setTimerPosition(pos);
      setDraftTimerPosition(pos);
      const showMs = p.display.timer_show_ms ?? false;
      setTimerShowMs(showMs);
      setDraftTimerShowMs(showMs);
      const margin = p.display.timer_margin ?? 50;
      setTimerMargin(margin);
      setDraftTimerMargin(margin);
      const floating = p.display.timer_floating ?? false;
      setTimerFloating(floating);
      setDraftTimerFloating(floating);
    }).catch(console.error);
    getMachineAudioConfig().then((c) => { setMachineConfig(c); setDraftMachineConfig(c); }).catch(console.error);
    getAvailableBackends().then(setAvailableBackends).catch(console.error);
    getOutputScreen().then((s) => { setOutputScreen_(s); setDraftOutputScreen(s); }).catch(console.error);
    getOscConfig().then(setOscConfig_).catch(console.error);
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
    if ((e.target as HTMLElement).closest("button")) return;
    e.preventDefault();
    if (standalone) {
      void getCurrentWindow().startDragging();
      return;
    }
    dragRef.current = {
      startMouseX: e.clientX, startMouseY: e.clientY,
      startPosX: posRef.current.x, startPosY: posRef.current.y,
    };
    document.body.style.cursor = "grabbing";
  };

  const handleApply = async () => {
    if (!draft) return;
    setApplyError(null);
    const displayPayload: DisplayPreferences = {
      ...draftTheme,
      output_screen: draftOutputScreen ?? undefined,
      show_output_timer: draftShowOutputTimer,
      timer_floating: draftTimerFloating,
      timer_count_down: draftTimerCountDown,
      timer_font: draftTimerFont,
      timer_font_size: draftTimerFontSize,
      timer_position: draftTimerPosition,
      timer_show_ms: draftTimerShowMs,
      timer_margin: draftTimerMargin,
    } as DisplayPreferences;
    try {
      await Promise.all([
        updateMachineAudioConfig(draftMachineConfig),
        updateAudioPreferences(draft.audio),
        updateGeneralPreferences(draft.general),
        setOutputScreen(draftOutputScreen),
        updateDisplayPreferences(displayPayload),
      ]);
      if (standalone) {
        await emit("preferences-applied");
      } else {
        setGeneralPrefs(draft.general);
        setDisplayPrefs({
          ...draftTheme,
          output_screen: draftOutputScreen,
          show_output_timer: draftShowOutputTimer,
          timer_floating: draftTimerFloating,
          timer_count_down: draftTimerCountDown,
          timer_font: draftTimerFont,
          timer_font_size: draftTimerFontSize,
          timer_position: draftTimerPosition,
          timer_show_ms: draftTimerShowMs,
          timer_margin: draftTimerMargin,
        });
      }
      setPrefs(draft);
      setMachineConfig(draftMachineConfig);
      setOutputScreen_(draftOutputScreen);
      setShowOutputTimer(draftShowOutputTimer);
      setTimerCountDown(draftTimerCountDown);
      setTimerFont(draftTimerFont);
      setTimerFontSize(draftTimerFontSize);
      setTimerPosition(draftTimerPosition);
      setTimerShowMs(draftTimerShowMs);
      setTimerMargin(draftTimerMargin);
      setTimerFloating(draftTimerFloating);
      setTheme(draftTheme);
      onClose();
    } catch (e) {
      setApplyError(String(e));
    }
  };

  // Apply machine config immediately (without closing the modal).
  // Used for ASIO output pair which must restart the engine to take effect.
  const handleImmediateApplyMachine = useCallback(async (config: MachineAudioConfig) => {
    setApplyError(null);
    try {
      await updateMachineAudioConfig(config);
      setMachineConfig(config);
      setDraftMachineConfig(config);
    } catch (e) {
      setApplyError(String(e));
    }
  }, []);

  const handleCancel = () => {
    // Restore committed style and clear preview.
    void previewOutputTimer(timerFont, timerFontSize, timerPosition, timerMargin, null);
    setTimerPreview(false);
    setDraft(prefs);
    setDraftMachineConfig(machineConfig);
    setDraftOutputScreen(outputScreen);
    setDraftShowOutputTimer(showOutputTimer);
    setDraftTimerCountDown(timerCountDown);
    setDraftTimerFont(timerFont);
    setDraftTimerFontSize(timerFontSize);
    setDraftTimerPosition(timerPosition);
    setDraftTimerShowMs(timerShowMs);
    setDraftTimerMargin(timerMargin);
    setDraftTimerFloating(timerFloating);
    setDraftTheme(theme);
    onClose();
  };

  const modalStyle: React.CSSProperties = standalone
    ? { position: "fixed", inset: 0, background: "#0f172a", display: "flex", flexDirection: "column", overflow: "hidden" }
    : {
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
      };

  const inner = (
    <>
      {!standalone && (
        <div
          style={{ position: "fixed", inset: 0, zIndex: 49999, background: "rgba(0,0,0,0.45)" }}
          onClick={handleCancel}
        />
      )}

      {/* Floating window */}
      <div
        style={modalStyle}
        onClick={standalone ? undefined : (e) => e.stopPropagation()}
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
            {draft === null ? (
              <div style={{ display: "flex", alignItems: "center", justifyContent: "center", height: "100%", color: "#64748b", fontSize: 13 }}>
                Loading…
              </div>
            ) : (
              <>
                {category === "audio" && (
                  <AudioContent
                    machineConfig={draftMachineConfig}
                    audioPrefs={draft.audio}
                    onMachineConfigChange={setDraftMachineConfig}
                    onAudioPrefsChange={(audio) => setDraft({ ...draft, audio })}
                    availableBackends={availableBackends}
                    onImmediateApplyMachine={handleImmediateApplyMachine}
                  />
                )}
                {category === "general" && (
                  <GeneralContent
                    prefs={draft.general}
                    onChange={(general) => setDraft({ ...draft, general })}
                  />
                )}
                {category === "display" && (
                  <DisplayContent
                    outputScreen={draftOutputScreen}
                    onScreenChange={setDraftOutputScreen}
                    showOutputTimer={draftShowOutputTimer}
                    onTimerChange={setDraftShowOutputTimer}
                    timerFloating={draftTimerFloating}
                    onTimerFloatingChange={setDraftTimerFloating}
                    timerCountDown={draftTimerCountDown}
                    onTimerModeChange={setDraftTimerCountDown}
                    timerFont={draftTimerFont}
                    onTimerFontChange={setDraftTimerFont}
                    timerFontSize={draftTimerFontSize}
                    onTimerFontSizeChange={setDraftTimerFontSize}
                    timerPosition={draftTimerPosition}
                    onTimerPositionChange={setDraftTimerPosition}
                    timerShowMs={draftTimerShowMs}
                    onTimerShowMsChange={setDraftTimerShowMs}
                    timerMargin={draftTimerMargin}
                    onTimerMarginChange={setDraftTimerMargin}
                    timerPreview={timerPreview}
                    onTimerPreviewChange={setTimerPreview}
                    committedTimerStyle={{ font: timerFont, fontSize: timerFontSize, position: timerPosition, margin: timerMargin }}
                  />
                )}
                {category === "personalization" && (
                  <PersonalizationContent
                    theme={draftTheme}
                    onThemeChange={setDraftTheme}
                  />
                )}
                {category === "network" && (
                  <OscContent
                    config={oscConfig}
                    onChange={async (c) => {
                      setOscConfig_(c);
                      try { await setOscConfig(c); } catch (e) { console.error(e); }
                    }}
                  />
                )}
              </>
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
              disabled={draft === null}
              style={{ ...btnStyle, padding: "5px 16px", fontSize: 12, background: "#1d4ed8", border: "1px solid #2563eb", color: "#fff", opacity: draft === null ? 0.5 : 1 }}
            >
              Apply
            </button>
          </div>
        </div>
      </div>
    </>
  );

  return standalone ? inner : createPortal(inner, document.body);
}
