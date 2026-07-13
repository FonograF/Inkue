// Inline audio waveform with trim markers, drawn DAW-style: a dim peak
// envelope with a brighter RMS body inside it, one column per CSS pixel.

import { useCallback, useEffect, useState } from "react";
import type { AudioCueData, WaveformData } from "../../lib/types";
import { getWaveformPeaks } from "../../lib/commands";
import { TrimStrip, type TrimPainter } from "./TrimStrip";

/** Bins requested from the backend — enough for one bin per device pixel at
 *  the widest inspector on a 2× display. */
const WAVEFORM_BINS = 1600;

/** Max over a bin range, for downsampling peaks to columns. */
function rangeMax(values: number[], from: number, to: number): number {
  let max = 0;
  for (let i = from; i < to && i < values.length; i++) {
    if (values[i] > max) max = values[i];
  }
  return max;
}

export function WaveformViewer({
  cue,
  onSave,
  onExpand,
}: {
  cue: AudioCueData;
  onSave: (p: Partial<AudioCueData>) => void;
  onExpand: () => void;
}) {
  const [waveform, setWaveform] = useState<WaveformData | null>(null);

  useEffect(() => {
    setWaveform(null);
    if (!cue.file_path) return;
    getWaveformPeaks(cue.id, WAVEFORM_BINS)
      .then(setWaveform)
      .catch(() => setWaveform({ peaks: [], rms: [], file_duration_s: 0 }));
  }, [cue.id, cue.file_path]);

  const paint = useCallback<TrimPainter>(
    (ctx, W, H, startX, endX) => {
      ctx.fillStyle = "#0f172a";
      ctx.fillRect(0, 0, W, H);

      const peaks = waveform?.peaks ?? [];
      const rms = waveform?.rms ?? [];
      if (peaks.length === 0) {
        ctx.fillStyle = "#475569";
        ctx.font = "11px sans-serif";
        ctx.textAlign = "center";
        ctx.fillText(
          waveform === null ? "Loading waveform…" : "No audio data",
          W / 2,
          H / 2 + 4,
        );
        return;
      }

      // Shaded active region
      ctx.fillStyle = "#0d2818";
      ctx.fillRect(startX, 0, endX - startX, H);

      // One column per CSS pixel: peak envelope (dim) + RMS body (bright).
      const mid = H / 2;
      const amp = H * 0.46;
      for (let x = 0; x < W; x++) {
        const from = Math.floor((x / W) * peaks.length);
        const to = Math.max(from + 1, Math.floor(((x + 1) / W) * peaks.length));
        const peak = rangeMax(peaks, from, to);
        const body = rangeMax(rms, from, to);
        const inRegion = x >= startX && x <= endX;

        const peakH = Math.max(1, peak * amp);
        ctx.fillStyle = inRegion ? "#15803d" : "#14532d";
        ctx.fillRect(x, mid - peakH, 1, peakH * 2);

        if (body > 0) {
          const bodyH = Math.max(1, body * amp);
          ctx.fillStyle = inRegion ? "#4ade80" : "#166534";
          ctx.fillRect(x, mid - bodyH, 1, bodyH * 2);
        }
      }

      // Center line ties the two half-waves together visually.
      ctx.fillStyle = "rgba(74, 222, 128, 0.35)";
      ctx.fillRect(0, mid - 0.5, W, 1);
    },
    [waveform],
  );

  if (!cue.file_path) return null;

  const fileDurMs = (waveform?.file_duration_s ?? 0) * 1000;

  return (
    <TrimStrip
      durationMs={fileDurMs}
      startMs={cue.start_time_ms}
      endMs={cue.end_time_ms}
      height={80}
      paint={paint}
      paintKey={waveform}
      onCommitStart={(ms) => onSave({ start_time_ms: ms })}
      onCommitEnd={(ms) => onSave({ end_time_ms: ms })}
      onExpand={onExpand}
      sliceMarkersMs={cue.slices?.markers ?? []}
    />
  );
}
