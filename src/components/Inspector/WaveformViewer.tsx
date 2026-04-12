import { useEffect, useRef, useState } from "react";
import type { AudioCueData, WaveformData } from "../../lib/types";
import { getWaveformPeaks } from "../../lib/commands";

export function WaveformViewer({
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
      const inRegion = x >= startX - binW && x <= endX;
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
