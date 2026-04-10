// Full-screen waveform editor modal with preview playback.
// Opened from the Time tab in the Inspector.

import { useEffect, useRef, useState, useCallback } from "react";
import type { AudioCueData, WaveformData } from "../lib/types";
import { getWaveformPeaks, previewCue, stopPreview } from "../lib/commands";

interface Props {
  cue: AudioCueData;
  onClose: () => void;
  onSave: (p: Partial<AudioCueData>) => void;
}

export function WaveformModal({ cue, onClose, onSave }: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [waveform, setWaveform] = useState<WaveformData | null>(null);

  // Working copies of start/end — not committed until Apply is clicked.
  const [startMs, setStartMs] = useState(cue.start_time_ms ?? 0);
  const [endMs, setEndMs] = useState<number>(0); // 0 = use fileDurMs (initialised below)

  const [dragging, setDragging] = useState<"start" | "end" | null>(null);

  const [isPlaying, setIsPlaying] = useState(false);
  const [playheadMs, setPlayheadMs] = useState<number | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);

  const voiceIdRef = useRef<string | null>(null);
  const animRef = useRef<number>(0);
  const isPlayingRef = useRef(false);

  // ---------------------------------------------------------------------------
  // Derived values
  // ---------------------------------------------------------------------------

  const fileDurMs = (waveform?.file_duration_s ?? 0) * 1000;
  const effectiveEndMs = endMs > 0 ? endMs : fileDurMs;

  // Once the waveform loads, initialise endMs from the cue or the file length.
  useEffect(() => {
    if (fileDurMs > 0 && endMs === 0) {
      setEndMs(cue.end_time_ms ?? fileDurMs);
    }
  }, [fileDurMs]); // eslint-disable-line react-hooks/exhaustive-deps

  // ---------------------------------------------------------------------------
  // Load waveform
  // ---------------------------------------------------------------------------

  useEffect(() => {
    if (!cue.file_path) return;
    getWaveformPeaks(cue.id, 800)
      .then(setWaveform)
      .catch(() => setWaveform({ peaks: [], file_duration_s: 0 }));
  }, [cue.id, cue.file_path]);

  // ---------------------------------------------------------------------------
  // Canvas drawing
  // ---------------------------------------------------------------------------

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const rect = canvas.getBoundingClientRect();
    const W = rect.width || 800;
    const H = 200;
    canvas.width = W;
    canvas.height = H;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    ctx.fillStyle = "#0f172a";
    ctx.fillRect(0, 0, W, H);

    const peaks = waveform?.peaks ?? [];

    if (peaks.length === 0) {
      ctx.fillStyle = "#475569";
      ctx.font = "13px system-ui, sans-serif";
      ctx.textAlign = "center";
      ctx.fillText(
        waveform === null ? "Loading waveform…" : "No audio data",
        W / 2,
        H / 2 + 5
      );
      return;
    }

    const sX = fileDurMs > 0 ? (startMs / fileDurMs) * W : 0;
    const eX = fileDurMs > 0 ? (effectiveEndMs / fileDurMs) * W : W;

    // Active region background
    ctx.fillStyle = "#0d2818";
    ctx.fillRect(sX, 0, eX - sX, H);

    // Waveform bars
    const binW = W / peaks.length;
    for (let i = 0; i < peaks.length; i++) {
      const x = i * binW;
      const h = Math.max(1, peaks[i] * H * 0.85);
      const y = (H - h) / 2;
      const inRegion = x >= sX - binW && x <= eX;
      ctx.fillStyle = inRegion ? "#22c55e" : "#166534";
      ctx.fillRect(x, y, Math.max(1, binW - 0.5), h);
    }

    // Playhead (white line)
    if (playheadMs !== null && fileDurMs > 0) {
      const phX = (playheadMs / fileDurMs) * W;
      ctx.strokeStyle = "rgba(255,255,255,0.85)";
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.moveTo(phX, 0);
      ctx.lineTo(phX, H);
      ctx.stroke();
    }

    // Start marker — blue vertical + triangle
    ctx.strokeStyle = "#60a5fa";
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(sX, 0);
    ctx.lineTo(sX, H);
    ctx.stroke();
    ctx.fillStyle = "#60a5fa";
    ctx.beginPath();
    ctx.moveTo(sX - 8, 0);
    ctx.lineTo(sX + 8, 0);
    ctx.lineTo(sX, 18);
    ctx.closePath();
    ctx.fill();

    // End marker — orange vertical + triangle
    ctx.strokeStyle = "#fb923c";
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(eX, 0);
    ctx.lineTo(eX, H);
    ctx.stroke();
    ctx.fillStyle = "#fb923c";
    ctx.beginPath();
    ctx.moveTo(eX - 8, 0);
    ctx.lineTo(eX + 8, 0);
    ctx.lineTo(eX, 18);
    ctx.closePath();
    ctx.fill();
  }, [waveform, startMs, effectiveEndMs, fileDurMs, playheadMs]);

  // ---------------------------------------------------------------------------
  // Preview playback
  // ---------------------------------------------------------------------------

  const stopPlayback = useCallback(async () => {
    isPlayingRef.current = false;
    cancelAnimationFrame(animRef.current);
    setPlayheadMs(null);
    setIsPlaying(false);
    if (voiceIdRef.current) {
      await stopPreview(voiceIdRef.current).catch(() => {});
      voiceIdRef.current = null;
    }
  }, []);

  const handlePlay = async () => {
    if (isPlayingRef.current) return stopPlayback();
    setPreviewError(null);

    const fromMs = startMs;
    const toMs = effectiveEndMs;

    try {
      // Capture time before the IPC call so we can estimate when the backend
      // actually started the audio (which happens during the round-trip).
      const t_before = performance.now();
      const vid = await previewCue(
        cue.id,
        fromMs > 0 ? fromMs : undefined,
        toMs < fileDurMs ? toMs : undefined
      );
      // The backend calls play_voice near the start of its return journey.
      // Using the midpoint of the IPC round-trip as wallStart minimises the
      // systematic drift: instead of lagging behind the audio by the full
      // round-trip (~10-30 ms), the indicator is off by only ~half-buffer
      // (~5 ms), which is imperceptible.
      const wallStart = t_before + (performance.now() - t_before) / 2;
      voiceIdRef.current = vid;
      isPlayingRef.current = true;
      setIsPlaying(true);

      const tick = () => {
        if (!isPlayingRef.current) return;
        const elapsed = performance.now() - wallStart;
        const currentMs = fromMs + elapsed;
        setPlayheadMs(currentMs);
        if (currentMs >= toMs) {
          isPlayingRef.current = false;
          setIsPlaying(false);
          setPlayheadMs(null);
          voiceIdRef.current = null;
          return;
        }
        animRef.current = requestAnimationFrame(tick);
      };
      animRef.current = requestAnimationFrame(tick);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setPreviewError(msg);
      console.error("Preview failed:", e);
    }
  };

  // Stop playback and release resources when the modal unmounts.
  useEffect(() => {
    return () => {
      isPlayingRef.current = false;
      cancelAnimationFrame(animRef.current);
      if (voiceIdRef.current) {
        stopPreview(voiceIdRef.current).catch(() => {});
      }
    };
  }, []);

  // ---------------------------------------------------------------------------
  // Keyboard shortcuts
  // ---------------------------------------------------------------------------

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" || e.key === " ") {
        // Capture phase + stopImmediatePropagation prevents these keys from
        // ever reaching the global keyboard shortcuts handler (Go / Stop All).
        e.preventDefault();
        e.stopImmediatePropagation();
        if (e.key === "Escape") {
          stopPlayback().then(onClose);
        } else {
          isPlayingRef.current ? stopPlayback() : handlePlay();
        }
      }
    };
    // Register in CAPTURE phase so we intercept before bubble-phase listeners
    // (including useKeyboardShortcuts which listens in bubble phase on window).
    window.addEventListener("keydown", onKey, { capture: true });
    return () => window.removeEventListener("keydown", onKey, { capture: true });
  }, [startMs, effectiveEndMs, fileDurMs, isPlaying]); // eslint-disable-line react-hooks/exhaustive-deps

  // ---------------------------------------------------------------------------
  // Drag logic
  // ---------------------------------------------------------------------------

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
    const eX = (effectiveEndMs / fileDurMs) * W;
    const dS = Math.abs(x - sX);
    const dE = Math.abs(x - eX);
    if (dS <= dE && dS < 20) setDragging("start");
    else if (dE < 20) setDragging("end");
  };

  const handleMouseMove = (e: React.MouseEvent<HTMLCanvasElement>) => {
    if (!dragging) return;
    const ms = xToMs(e.clientX);
    if (dragging === "start") {
      setStartMs(Math.max(0, Math.min(ms, effectiveEndMs - 50)));
    } else {
      setEndMs(Math.min(fileDurMs, Math.max(ms, startMs + 50)));
    }
  };

  const handleMouseUp = () => setDragging(null);

  // ---------------------------------------------------------------------------
  // Apply / Close
  // ---------------------------------------------------------------------------

  const handleApply = async () => {
    await stopPlayback();
    onSave({
      start_time_ms: startMs > 0 ? Math.round(startMs) : null,
      end_time_ms: effectiveEndMs < fileDurMs ? Math.round(effectiveEndMs) : null,
    });
    onClose();
  };

  const handleClose = async () => {
    await stopPlayback();
    onClose();
  };

  // ---------------------------------------------------------------------------
  // Helpers
  // ---------------------------------------------------------------------------

  const fmtTime = (ms: number) => {
    const totalSec = ms / 1000;
    const min = Math.floor(totalSec / 60);
    const sec = (totalSec % 60).toFixed(3);
    return min > 0 ? `${min}:${sec.padStart(6, "0")}` : `${sec}s`;
  };

  // ---------------------------------------------------------------------------
  // Render
  // ---------------------------------------------------------------------------

  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        zIndex: 10000,
        background: "rgba(0,0,0,0.72)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
      }}
      onClick={(e) => { if (e.target === e.currentTarget) handleClose(); }}
    >
      <div
        style={{
          background: "#0f172a",
          border: "1px solid #334155",
          borderRadius: 8,
          width: "min(920px, 94vw)",
          padding: 20,
          display: "flex",
          flexDirection: "column",
          gap: 14,
          boxShadow: "0 24px 64px rgba(0,0,0,0.8)",
        }}
      >
        {/* Header */}
        <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
          <span style={{ fontWeight: 600, fontSize: 14, color: "#f1f5f9", flex: 1 }}>
            Waveform — {cue.name}
          </span>
          <span style={{ fontSize: 11, color: "#475569" }}>
            Space = preview · Esc = close
          </span>
          <button onClick={handleClose} style={closeBtnStyle} title="Close (Esc)">
            ✕
          </button>
        </div>

        {/* Canvas */}
        <canvas
          ref={canvasRef}
          style={{
            width: "100%",
            height: 200,
            display: "block",
            borderRadius: 4,
            border: "1px solid #1e293b",
            cursor: dragging ? "ew-resize" : "crosshair",
          }}
          onMouseDown={handleMouseDown}
          onMouseMove={handleMouseMove}
          onMouseUp={handleMouseUp}
          onMouseLeave={handleMouseUp}
        />

        {/* Time labels */}
        <div
          style={{
            display: "flex",
            justifyContent: "space-between",
            fontSize: 11,
            color: "#94a3b8",
            marginTop: -6,
          }}
        >
          <span style={{ color: "#60a5fa" }}>▶ {fmtTime(startMs)}</span>
          <span>{waveform ? fmtTime(fileDurMs) : "—"}</span>
          <span style={{ color: "#fb923c" }}>■ {fmtTime(effectiveEndMs)}</span>
        </div>

        {/* Controls */}
        <div style={{ display: "flex", gap: 10, alignItems: "center", flexWrap: "wrap" }}>
          {/* Start time */}
          <label style={labelStyle}>
            <span style={{ color: "#60a5fa" }}>Start (s)</span>
            <input
              style={numInputStyle}
              type="number"
              step="0.001"
              min="0"
              value={(startMs / 1000).toFixed(3)}
              onChange={(e) =>
                setStartMs(
                  Math.max(0, Math.min(parseFloat(e.target.value) * 1000, effectiveEndMs - 50))
                )
              }
            />
          </label>

          {/* End time */}
          <label style={labelStyle}>
            <span style={{ color: "#fb923c" }}>End (s)</span>
            <input
              style={numInputStyle}
              type="number"
              step="0.001"
              min="0"
              value={(effectiveEndMs / 1000).toFixed(3)}
              onChange={(e) =>
                setEndMs(
                  Math.min(fileDurMs, Math.max(parseFloat(e.target.value) * 1000, startMs + 50))
                )
              }
            />
          </label>

          {/* Duration badge */}
          <span style={{ fontSize: 11, color: "#64748b", flexShrink: 0 }}>
            Duration: {fmtTime(effectiveEndMs - startMs)}
          </span>

          <div style={{ flex: 1 }} />

          {/* Play / Stop preview */}
          <button
            style={{
              ...actionBtnStyle,
              background: isPlaying ? "#7c3aed" : "#1d4ed8",
              minWidth: 100,
            }}
            onClick={isPlaying ? stopPlayback : handlePlay}
          >
            {isPlaying ? "■ Stop" : "▶ Preview"}
          </button>

          <button
            style={{ ...actionBtnStyle, background: "#334155" }}
            onClick={handleClose}
          >
            Cancel
          </button>
          <button
            style={{ ...actionBtnStyle, background: "#16a34a" }}
            onClick={handleApply}
          >
            Apply
          </button>
        </div>

        {previewError && (
          <div
            style={{
              background: "#450a0a",
              border: "1px solid #7f1d1d",
              borderRadius: 4,
              padding: "6px 10px",
              fontSize: 12,
              color: "#fca5a5",
            }}
          >
            Preview error: {previewError}
          </div>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Styles
// ---------------------------------------------------------------------------

const closeBtnStyle: React.CSSProperties = {
  background: "transparent",
  border: "none",
  color: "#64748b",
  cursor: "pointer",
  fontSize: 16,
  padding: "2px 6px",
  borderRadius: 4,
  lineHeight: 1,
};

const labelStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 6,
  fontSize: 12,
};

const numInputStyle: React.CSSProperties = {
  background: "#1e293b",
  border: "1px solid #334155",
  borderRadius: 4,
  color: "#e2e8f0",
  padding: "3px 7px",
  fontSize: 12,
  width: 90,
};

const actionBtnStyle: React.CSSProperties = {
  padding: "6px 16px",
  border: "none",
  borderRadius: 4,
  color: "#f1f5f9",
  cursor: "pointer",
  fontSize: 12,
  fontWeight: 500,
  flexShrink: 0,
};
