// Contextual inspector panel shown on the right side.
// Shows AudioCue properties across four tabs: Basics, Time, Levels, Fade.

import { useEffect, useRef, useState, useCallback } from "react";
import type { AudioCueData, CueSummary, FadeCurve, WaveformData } from "../../lib/types";
import { getCue, updateCue, setAudioFile, getWaveformPeaks } from "../../lib/commands";
import { WaveformModal } from "../WaveformModal";
import { open } from "@tauri-apps/plugin-dialog";

interface Props {
  selectedCue: CueSummary | null;
  onRefresh: () => void;
}

type Tab = "basics" | "time" | "levels" | "fade";

export function InspectorPanel({ selectedCue, onRefresh }: Props) {
  const [cueData, setCueData] = useState<AudioCueData | null>(null);
  const [activeTab, setActiveTab] = useState<Tab>("basics");
  const [waveformModalOpen, setWaveformModalOpen] = useState(false);

  useEffect(() => {
    if (!selectedCue) {
      setCueData(null);
      return;
    }
    getCue(selectedCue.id)
      .then((data) => {
        // Merge cue_type from the summary in case the serialised form uses
        // a different key ("type" vs "cue_type").
        setCueData({ ...data, cue_type: selectedCue.cue_type });
      })
      .catch(console.error);
  }, [selectedCue?.id]);

  if (!selectedCue || !cueData) {
    return (
      <div
        style={{
          padding: 24,
          color: "#475569",
          textAlign: "center",
          fontSize: 13,
        }}
      >
        Select a cue to inspect it.
      </div>
    );
  }

  const isAudio = selectedCue.cue_type === "audio";

  const save = async (partial: Partial<AudioCueData>) => {
    await updateCue(cueData.id, partial).catch(console.error);
    setCueData((prev) => (prev ? { ...prev, ...partial } : prev));
    onRefresh();
  };

  const handleBrowse = async () => {
    const result = await open({
      multiple: false,
      filters: [
        { name: "Audio Files", extensions: ["wav", "mp3", "flac", "ogg", "aac"] },
      ],
    });
    if (typeof result === "string") {
      await setAudioFile(cueData.id, result).catch(console.error);
      setCueData((prev) => (prev ? { ...prev, file_path: result } : prev));
      onRefresh();
    }
  };

  const tabStyle = (tab: Tab): React.CSSProperties => ({
    padding: "6px 14px",
    cursor: "pointer",
    fontSize: 12,
    background: activeTab === tab ? "#1e293b" : "transparent",
    color: activeTab === tab ? "#e2e8f0" : "#64748b",
    border: "none",
    borderBottom:
      activeTab === tab ? "2px solid #3b82f6" : "2px solid transparent",
    outline: "none",
  });

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        background: "#0f172a",
        color: "#e2e8f0",
        fontSize: 13,
      }}
    >
      {/* Title */}
      <div
        style={{
          padding: "8px 12px",
          fontWeight: 600,
          borderBottom: "1px solid #1e293b",
          background: "#020617",
        }}
      >
        {isAudio ? "🔊" : "📝"} {cueData.name}
      </div>

      {/* Tabs */}
      <div style={{ display: "flex", borderBottom: "1px solid #1e293b" }}>
        <button style={tabStyle("basics")} onClick={() => setActiveTab("basics")}>
          Basics
        </button>
        <button style={tabStyle("time")} onClick={() => setActiveTab("time")}>
          Time
        </button>
        {isAudio && (
          <button style={tabStyle("levels")} onClick={() => setActiveTab("levels")}>
            Levels
          </button>
        )}
        {isAudio && (
          <button style={tabStyle("fade")} onClick={() => setActiveTab("fade")}>
            Fade
          </button>
        )}
      </div>

      {/* Tab content */}
      <div style={{ flex: 1, overflowY: "auto", padding: 12 }}>
        {activeTab === "basics" && (
          <BasicsTab
            cue={cueData}
            isAudio={isAudio}
            onSave={save}
            onBrowse={handleBrowse}
          />
        )}
        {activeTab === "time" && (
          <TimeTab
            cue={cueData}
            isAudio={isAudio}
            onSave={save}
            onOpenWaveform={() => setWaveformModalOpen(true)}
          />
        )}
        {activeTab === "levels" && isAudio && (
          <LevelsTab cue={cueData} onSave={save} />
        )}
        {activeTab === "fade" && isAudio && (
          <FadeTab cue={cueData} onSave={save} />
        )}
      </div>

      {waveformModalOpen && cueData && (
        <WaveformModal
          cue={cueData}
          onClose={() => setWaveformModalOpen(false)}
          onSave={save}
        />
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Shared primitives
// ---------------------------------------------------------------------------

function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        marginBottom: 10,
        gap: 8,
      }}
    >
      <label style={{ width: 100, color: "#94a3b8", flexShrink: 0 }}>
        {label}
      </label>
      <div style={{ flex: 1 }}>{children}</div>
    </div>
  );
}

const inputStyle: React.CSSProperties = {
  background: "#1e293b",
  border: "1px solid #334155",
  borderRadius: 4,
  color: "#e2e8f0",
  padding: "3px 8px",
  fontSize: 13,
  width: "100%",
  boxSizing: "border-box",
};

// ---------------------------------------------------------------------------
// Basics tab
// ---------------------------------------------------------------------------

function BasicsTab({
  cue,
  isAudio,
  onSave,
  onBrowse,
}: {
  cue: AudioCueData;
  isAudio: boolean;
  onSave: (p: Partial<AudioCueData>) => void;
  onBrowse: () => void;
}) {
  return (
    <>
      <Field label="Cue #">
        <input
          style={inputStyle}
          defaultValue={cue.number ?? ""}
          onBlur={(e) => onSave({ number: e.target.value || null })}
        />
      </Field>
      <Field label="Name">
        <input
          style={inputStyle}
          defaultValue={cue.name}
          onBlur={(e) => onSave({ name: e.target.value })}
        />
      </Field>
      <Field label="Notes">
        <textarea
          style={{ ...inputStyle, resize: "vertical", minHeight: 60 }}
          defaultValue={cue.notes}
          onBlur={(e) => onSave({ notes: e.target.value })}
        />
      </Field>
      {isAudio && (
        <Field label="File">
          <div style={{ display: "flex", gap: 4 }}>
            <input
              style={{ ...inputStyle, flex: 1 }}
              readOnly
              value={cue.file_path ? cue.file_path.split(/[\\/]/).pop() ?? cue.file_path : "(no file)"}
              title={cue.file_path ?? ""}
            />
            <button
              style={{
                padding: "3px 10px",
                background: "#334155",
                border: "none",
                borderRadius: 4,
                color: "#e2e8f0",
                cursor: "pointer",
                fontSize: 12,
                flexShrink: 0,
              }}
              onClick={onBrowse}
            >
              Browse…
            </button>
          </div>
        </Field>
      )}
      <Field label="Continue">
        <select
          style={inputStyle}
          value={cue.continue_mode}
          onChange={(e) =>
            onSave({
              continue_mode: e.target.value as AudioCueData["continue_mode"],
            })
          }
        >
          <option value="do_not_continue">Do Not Continue</option>
          <option value="auto_continue">Auto-Continue</option>
          <option value="auto_follow">Auto-Follow</option>
        </select>
      </Field>
    </>
  );
}

// ---------------------------------------------------------------------------
// Waveform viewer with draggable start/end markers
// ---------------------------------------------------------------------------

function WaveformViewer({
  cue,
  onSave,
  onExpand,
}: {
  cue: AudioCueData;
  onSave: (p: Partial<AudioCueData>) => void;
  onExpand: () => void;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [waveform, setWaveform] = useState<WaveformData | null>(null);
  const [dragging, setDragging] = useState<"start" | "end" | null>(null);
  const [localStartMs, setLocalStartMs] = useState<number | null>(null);
  const [localEndMs, setLocalEndMs] = useState<number | null>(null);

  // Reload peaks whenever the file changes
  useEffect(() => {
    setWaveform(null);
    setLocalStartMs(null);
    setLocalEndMs(null);
    if (!cue.file_path) return;
    getWaveformPeaks(cue.id, 400)
      .then(setWaveform)
      .catch(() => setWaveform({ peaks: [], file_duration_s: 0 }));
  }, [cue.id, cue.file_path]);

  // Reset local drag state when cue changes
  useEffect(() => {
    setLocalStartMs(null);
    setLocalEndMs(null);
  }, [cue.id]);

  const fileDurMs = (waveform?.file_duration_s ?? 0) * 1000;
  const startMs = localStartMs ?? cue.start_time_ms ?? 0;
  const endMs = localEndMs ?? cue.end_time_ms ?? fileDurMs;

  // Draw the waveform canvas
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const rect = canvas.getBoundingClientRect();
    const W = rect.width || 380;
    const H = 80;
    canvas.width = W;
    canvas.height = H;

    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    ctx.fillStyle = "#0f172a";
    ctx.fillRect(0, 0, W, H);

    const peaks = waveform?.peaks ?? [];

    if (peaks.length === 0) {
      ctx.fillStyle = "#475569";
      ctx.font = "11px sans-serif";
      ctx.textAlign = "center";
      ctx.fillText(
        waveform === null ? "Loading waveform…" : "No audio data",
        W / 2,
        H / 2 + 4
      );
      return;
    }

    const startX = fileDurMs > 0 ? (startMs / fileDurMs) * W : 0;
    const endX = fileDurMs > 0 ? (endMs / fileDurMs) * W : W;

    // Shaded active region
    ctx.fillStyle = "#0d2818";
    ctx.fillRect(startX, 0, endX - startX, H);

    // Waveform bars
    const binW = W / peaks.length;
    for (let i = 0; i < peaks.length; i++) {
      const x = i * binW;
      const h = Math.max(1, peaks[i] * H * 0.9);
      const y = (H - h) / 2;
      // Colour bars inside region brighter
      const inRegion =
        x >= startX - binW && x <= endX;
      ctx.fillStyle = inRegion ? "#22c55e" : "#166534";
      ctx.fillRect(x, y, Math.max(1, binW - 0.5), h);
    }

    // Start marker (blue)
    ctx.strokeStyle = "#60a5fa";
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(startX, 0);
    ctx.lineTo(startX, H);
    ctx.stroke();
    // Handle triangle
    ctx.fillStyle = "#60a5fa";
    ctx.beginPath();
    ctx.moveTo(startX - 6, 0);
    ctx.lineTo(startX + 6, 0);
    ctx.lineTo(startX, 10);
    ctx.closePath();
    ctx.fill();

    // End marker (orange)
    ctx.strokeStyle = "#fb923c";
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(endX, 0);
    ctx.lineTo(endX, H);
    ctx.stroke();
    ctx.fillStyle = "#fb923c";
    ctx.beginPath();
    ctx.moveTo(endX - 6, 0);
    ctx.lineTo(endX + 6, 0);
    ctx.lineTo(endX, 10);
    ctx.closePath();
    ctx.fill();
  }, [waveform, startMs, endMs, fileDurMs]);

  const xToMs = (clientX: number): number => {
    const canvas = canvasRef.current;
    if (!canvas || fileDurMs === 0) return 0;
    const rect = canvas.getBoundingClientRect();
    const relX = Math.max(0, Math.min(clientX - rect.left, rect.width));
    return (relX / rect.width) * fileDurMs;
  };

  const handleMouseDown = (e: React.MouseEvent<HTMLCanvasElement>) => {
    if (!waveform || fileDurMs === 0) return;
    const canvas = canvasRef.current!;
    const rect = canvas.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const W = rect.width;
    const sX = (startMs / fileDurMs) * W;
    const eX = (endMs / fileDurMs) * W;
    if (Math.abs(x - sX) <= Math.abs(x - eX) && Math.abs(x - sX) < 14) {
      setDragging("start");
    } else if (Math.abs(x - eX) < 14) {
      setDragging("end");
    }
  };

  const handleMouseMove = (e: React.MouseEvent<HTMLCanvasElement>) => {
    if (!dragging) return;
    const ms = xToMs(e.clientX);
    if (dragging === "start") {
      setLocalStartMs(Math.max(0, Math.min(ms, (localEndMs ?? endMs) - 50)));
    } else {
      setLocalEndMs(Math.min(fileDurMs, Math.max(ms, (localStartMs ?? startMs) + 50)));
    }
  };

  const handleMouseUp = () => {
    if (!dragging) return;
    if (dragging === "start" && localStartMs !== null) {
      const ms = Math.round(localStartMs);
      onSave({ start_time_ms: ms <= 0 ? null : ms });
    } else if (dragging === "end" && localEndMs !== null) {
      const ms = Math.round(localEndMs);
      onSave({ end_time_ms: ms >= fileDurMs ? null : ms });
    }
    setDragging(null);
  };

  if (!cue.file_path) return null;

  const fmtS = (ms: number) => (ms / 1000).toFixed(3);

  return (
    <div style={{ marginBottom: 16 }}>
      {/* Time labels + expand button */}
      <div
        style={{
          display: "flex",
          justifyContent: "space-between",
          alignItems: "center",
          fontSize: 11,
          marginBottom: 4,
          color: "#94a3b8",
        }}
      >
        <span style={{ color: "#60a5fa" }}>▶ {fmtS(startMs)}s</span>
        <span>{waveform ? `${waveform.file_duration_s.toFixed(2)}s` : "—"}</span>
        <span style={{ color: "#fb923c" }}>■ {fmtS(endMs)}s</span>
        <button
          onClick={onExpand}
          title="Open waveform editor"
          style={{
            background: "#1e293b",
            border: "1px solid #334155",
            borderRadius: 3,
            color: "#94a3b8",
            cursor: "pointer",
            fontSize: 11,
            padding: "1px 5px",
            lineHeight: 1.4,
          }}
        >
          ⤢
        </button>
      </div>
      <canvas
        ref={canvasRef}
        style={{
          width: "100%",
          height: 80,
          display: "block",
          borderRadius: 4,
          cursor: dragging ? "ew-resize" : "default",
        }}
        onMouseDown={handleMouseDown}
        onMouseMove={handleMouseMove}
        onMouseUp={handleMouseUp}
        onMouseLeave={handleMouseUp}
      />
      <div
        style={{
          fontSize: 10,
          color: "#475569",
          marginTop: 3,
          textAlign: "center",
        }}
      >
        Drag blue (start) or orange (end) marker to trim
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Time tab
// ---------------------------------------------------------------------------

function TimeTab({
  cue,
  isAudio,
  onSave,
  onOpenWaveform,
}: {
  cue: AudioCueData;
  isAudio: boolean;
  onSave: (p: Partial<AudioCueData>) => void;
  onOpenWaveform: () => void;
}) {
  return (
    <>
      <Field label="Pre-Wait (s)">
        <input
          style={inputStyle}
          type="number"
          step="0.1"
          min="0"
          defaultValue={(cue.pre_wait_ms / 1000).toFixed(1)}
          onBlur={(e) =>
            onSave({
              pre_wait_ms: Math.round(parseFloat(e.target.value) * 1000),
            })
          }
        />
      </Field>
      <Field label="Post-Wait (s)">
        <input
          style={inputStyle}
          type="number"
          step="0.1"
          min="0"
          defaultValue={(cue.post_wait_ms / 1000).toFixed(1)}
          onBlur={(e) =>
            onSave({
              post_wait_ms: Math.round(parseFloat(e.target.value) * 1000),
            })
          }
        />
      </Field>
      {isAudio && cue.file_path && (
        <WaveformViewer cue={cue} onSave={onSave} onExpand={onOpenWaveform} />
      )}
      {isAudio && (
        <>
          <Field label="Start Time (s)">
            <input
              style={inputStyle}
              type="number"
              step="0.001"
              min="0"
              key={`start-${cue.start_time_ms}`}
              defaultValue={
                cue.start_time_ms != null
                  ? (cue.start_time_ms / 1000).toFixed(3)
                  : ""
              }
              placeholder="0.000"
              onBlur={(e) =>
                onSave({
                  start_time_ms: e.target.value
                    ? Math.round(parseFloat(e.target.value) * 1000)
                    : null,
                })
              }
            />
          </Field>
          <Field label="End Time (s)">
            <input
              style={inputStyle}
              type="number"
              step="0.001"
              min="0"
              key={`end-${cue.end_time_ms}`}
              defaultValue={
                cue.end_time_ms != null
                  ? (cue.end_time_ms / 1000).toFixed(3)
                  : ""
              }
              placeholder="end of file"
              onBlur={(e) =>
                onSave({
                  end_time_ms: e.target.value
                    ? Math.round(parseFloat(e.target.value) * 1000)
                    : null,
                })
              }
            />
          </Field>
          <Field label="Loop Count">
            <input
              style={inputStyle}
              type="number"
              min="0"
              defaultValue={cue.loop_count}
              onBlur={(e) =>
                onSave({ loop_count: parseInt(e.target.value, 10) })
              }
            />
          </Field>
          <Field label="Rate">
            <input
              style={inputStyle}
              type="number"
              step="0.1"
              min="0.1"
              max="4.0"
              defaultValue={cue.rate}
              onBlur={(e) => onSave({ rate: parseFloat(e.target.value) })}
            />
          </Field>
        </>
      )}
    </>
  );
}

// ---------------------------------------------------------------------------
// Levels tab — volume and pan only
// ---------------------------------------------------------------------------

function LevelsTab({
  cue,
  onSave,
}: {
  cue: AudioCueData;
  onSave: (p: Partial<AudioCueData>) => void;
}) {
  const [volumeDb, setVolumeDb] = useState(cue.volume_db);
  const [pan, setPan] = useState(cue.pan);

  // Sync when the selected cue changes or after an external save
  useEffect(() => {
    setVolumeDb(cue.volume_db);
    setPan(cue.pan);
  }, [cue.id, cue.volume_db, cue.pan]);

  const commitVolume = useCallback(
    (v: number) => onSave({ volume_db: v }),
    [onSave]
  );
  const commitPan = useCallback(
    (v: number) => onSave({ pan: v }),
    [onSave]
  );

  return (
    <>
      <Field label="Volume (dB)">
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <input
            style={{ ...inputStyle, flex: 1, padding: "2px 4px" }}
            type="range"
            min="-60"
            max="12"
            step="0.5"
            value={volumeDb}
            onChange={(e) => setVolumeDb(parseFloat(e.target.value))}
            onMouseUp={() => commitVolume(volumeDb)}
          />
          <input
            style={{ ...inputStyle, width: 60 }}
            type="number"
            step="0.5"
            min="-60"
            max="12"
            value={volumeDb.toFixed(1)}
            onChange={(e) => setVolumeDb(parseFloat(e.target.value))}
            onBlur={() => commitVolume(volumeDb)}
          />
        </div>
      </Field>
      <Field label="Pan">
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <span style={{ color: "#94a3b8", fontSize: 11, flexShrink: 0 }}>L</span>
          <input
            style={{ ...inputStyle, flex: 1, padding: "2px 4px" }}
            type="range"
            min="-1"
            max="1"
            step="0.05"
            value={pan}
            onChange={(e) => setPan(parseFloat(e.target.value))}
            onMouseUp={() => commitPan(pan)}
          />
          <span style={{ color: "#94a3b8", fontSize: 11, flexShrink: 0 }}>R</span>
          <input
            style={{ ...inputStyle, width: 60 }}
            type="number"
            step="0.05"
            min="-1"
            max="1"
            value={pan.toFixed(2)}
            onChange={(e) => setPan(parseFloat(e.target.value))}
            onBlur={() => commitPan(pan)}
          />
        </div>
      </Field>
    </>
  );
}

// ---------------------------------------------------------------------------
// Fade tab — fade in/out with duration and curve selection
// ---------------------------------------------------------------------------

const FADE_CURVES: { value: FadeCurve; label: string }[] = [
  { value: "s_curve", label: "S-Curve (default)" },
  { value: "linear", label: "Linear" },
  { value: "exponential", label: "Exponential" },
];

function FadeTab({
  cue,
  onSave,
}: {
  cue: AudioCueData;
  onSave: (p: Partial<AudioCueData>) => void;
}) {
  return (
    <>
      {/* Fade In */}
      <div
        style={{
          marginBottom: 14,
          paddingBottom: 14,
          borderBottom: "1px solid #1e293b",
        }}
      >
        <div
          style={{ fontSize: 11, color: "#64748b", marginBottom: 8, textTransform: "uppercase", letterSpacing: "0.05em" }}
        >
          Fade In
        </div>
        <Field label="Duration (s)">
          <input
            style={inputStyle}
            type="number"
            step="0.1"
            min="0"
            defaultValue={
              cue.fade_in_ms != null ? (cue.fade_in_ms / 1000).toFixed(2) : ""
            }
            placeholder="none"
            onBlur={(e) =>
              onSave({
                fade_in_ms: e.target.value
                  ? Math.round(parseFloat(e.target.value) * 1000)
                  : null,
              })
            }
          />
        </Field>
        <Field label="Curve">
          <select
            style={inputStyle}
            value={cue.fade_in_curve ?? "s_curve"}
            onChange={(e) =>
              onSave({ fade_in_curve: e.target.value as FadeCurve })
            }
          >
            {FADE_CURVES.map((c) => (
              <option key={c.value} value={c.value}>
                {c.label}
              </option>
            ))}
          </select>
        </Field>
      </div>

      {/* Fade Out */}
      <div>
        <div
          style={{ fontSize: 11, color: "#64748b", marginBottom: 8, textTransform: "uppercase", letterSpacing: "0.05em" }}
        >
          Fade Out
        </div>
        <Field label="Duration (s)">
          <input
            style={inputStyle}
            type="number"
            step="0.1"
            min="0"
            defaultValue={
              cue.fade_out_ms != null
                ? (cue.fade_out_ms / 1000).toFixed(2)
                : ""
            }
            placeholder="none (0.5s on Stop)"
            onBlur={(e) =>
              onSave({
                fade_out_ms: e.target.value
                  ? Math.round(parseFloat(e.target.value) * 1000)
                  : null,
              })
            }
          />
        </Field>
        <Field label="Curve">
          <select
            style={inputStyle}
            value={cue.fade_out_curve ?? "s_curve"}
            onChange={(e) =>
              onSave({ fade_out_curve: e.target.value as FadeCurve })
            }
          >
            {FADE_CURVES.map((c) => (
              <option key={c.value} value={c.value}>
                {c.label}
              </option>
            ))}
          </select>
        </Field>
      </div>
    </>
  );
}
