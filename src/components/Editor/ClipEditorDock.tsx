// Clip editor dock: a second inspector under the cue list for precise trim +
// slice editing on Audio and Video cues. Opened by the ⤢ button next to the
// inline waveform / filmstrip; audio cues can be auditioned in place.

import { useCallback, useEffect, useMemo, useState } from "react";
import type { AudioCueData, SliceList, VideoCueData, WaveformData } from "../../lib/types";
import { PLAY_COUNT_INFINITE } from "../../lib/types";
import { getCue, getVideoFilmstrip, getVideoFilmstripRange, getWaveformPeaks, previewCue, stopPreview, updateCue } from "../../lib/commands";
import { SliceTimeline } from "./SliceTimeline";
import type { TrimPainter, TrimView } from "../Inspector/TrimStrip";

const TIMELINE_HEIGHT = 140;
const FILMSTRIP_TILES = 16;
const FILMSTRIP_TILE_WIDTH = 160;

type ClipCue = AudioCueData | VideoCueData;

function normalizeSlices(s: SliceList | undefined): SliceList {
  return { markers: s?.markers ?? [], play_counts: s?.play_counts ?? [1] };
}

function loadImage(dataUrl: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = reject;
    img.src = dataUrl;
  });
}

export function ClipEditorDock({
  cueId,
  onClose,
  onSaved,
  reloadToken,
}: {
  cueId: string;
  onClose: () => void;
  /** Called after every save so the cue list + inspector refresh. */
  onSaved: () => void;
  /** Bump to re-fetch the cue (the inspector edited it). */
  reloadToken?: number;
}) {
  const [cue, setCue] = useState<ClipCue | null>(null);
  const [waveform, setWaveform] = useState<WaveformData | null>(null);
  const [tiles, setTiles] = useState<HTMLImageElement[] | null>(null);
  const [previewVoice, setPreviewVoice] = useState<string | null>(null);
  /** Set on first zoom ≥ 2× — swaps in high-resolution media data. */
  const [wantDetail, setWantDetail] = useState(false);
  /** Visible window while zoomed (video: drives the range filmstrip). */
  const [zoomView, setZoomView] = useState<TrimView | null>(null);
  /** Window-matched frames streamed in while zoomed. */
  const [rangeStrip, setRangeStrip] =
    useState<{ startMs: number; endMs: number; tiles: HTMLImageElement[] } | null>(null);

  const isVideo = cue?.cue_type === "video";

  // Fetch the cue JSON — again whenever the inspector saved it (reloadToken).
  useEffect(() => {
    getCue(cueId)
      .then((data) => setCue(data as ClipCue))
      .catch(onClose); // cue deleted — close the dock
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cueId, reloadToken]);

  // (Re)load the media preview when the target's file changes — not on every
  // cue re-fetch, so inspector edits don't re-decode the waveform/filmstrip.
  useEffect(() => {
    setWaveform(null);
    setTiles(null);
    setWantDetail(false);
    setZoomView(null);
    setRangeStrip(null);
    if (!cue?.file_path) return;
    if (cue.cue_type === "video") {
      getVideoFilmstrip(cue.file_path, FILMSTRIP_TILES, FILMSTRIP_TILE_WIDTH)
        .then((urls) => Promise.all(urls.map(loadImage)))
        .then(setTiles)
        .catch(() => setTiles([]));
    } else {
      getWaveformPeaks(cueId, 2000)
        .then(setWaveform)
        .catch(() => setWaveform({ peaks: [], rms: [], file_duration_s: 0 }));
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cue?.file_path, cue?.cue_type]);

  // Stop the audition when the dock closes or the cue changes.
  useEffect(() => {
    return () => {
      if (previewVoice) void stopPreview(previewVoice).catch(() => {});
    };
  }, [previewVoice, cueId]);

  // High-resolution swap once the operator zooms in: finer waveform bins /
  // a denser filmstrip (both cached, so this costs once per file).
  useEffect(() => {
    if (!wantDetail || !cue?.file_path) return;
    if (cue.cue_type === "video") {
      getVideoFilmstrip(cue.file_path, 48, FILMSTRIP_TILE_WIDTH)
        .then((urls) => Promise.all(urls.map(loadImage)))
        .then(setTiles)
        .catch(() => {});
    } else {
      getWaveformPeaks(cueId, 16000)
        .then(setWaveform)
        .catch(() => {});
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wantDetail]);

  // Zoomed video: stream in frames for the visible window (debounced — waits
  // for the zoom/pan to settle; results are disk-cached on a ½ s grid).
  useEffect(() => {
    if (!isVideo || !zoomView || !cue?.file_path) return;
    const path = cue.file_path;
    const startS = Math.max(0, Math.floor((zoomView.startMs / 1000) * 2) / 2);
    const endS = Math.ceil((zoomView.endMs / 1000) * 2) / 2;
    let stale = false;
    const timer = setTimeout(() => {
      getVideoFilmstripRange(path, startS, endS, 12, FILMSTRIP_TILE_WIDTH)
        .then((urls) => Promise.all(urls.map(loadImage)))
        .then((images) => {
          if (!stale) {
            setRangeStrip({ startMs: startS * 1000, endMs: endS * 1000, tiles: images });
          }
        })
        .catch(() => {});
    }, 300);
    return () => {
      stale = true;
      clearTimeout(timer);
    };
  }, [isVideo, zoomView, cue?.file_path]);

  const save = async (partial: Partial<ClipCue>) => {
    await updateCue(cueId, partial).catch(console.error);
    setCue((prev) => (prev ? ({ ...prev, ...partial } as ClipCue) : prev));
    onSaved();
  };

  // getCue returns the *serialized* cue: video duration lives in
  // cached_duration_ms there (file_duration_ms only exists on summaries).
  const durationMs = isVideo
    ? (cue as VideoCueData | null)?.cached_duration_ms ??
      cue?.file_duration_ms ?? cue?.duration_ms ?? 0
    : (waveform?.file_duration_s ?? 0) * 1000;

  // Stable identity: a fresh object every render would retrigger the
  // timeline's reset effect and cancel drags / close the badge editor.
  const slices = useMemo(() => normalizeSlices(cue?.slices), [cue?.slices]);

  // ── Painters ─────────────────────────────────────────────────────────────
  const fileDurationS = waveform?.file_duration_s ?? 0;
  const paintWaveform = useCallback<TrimPainter>(
    (ctx, W, H, startX, endX, view) => {
      ctx.fillStyle = "#0f172a";
      ctx.fillRect(0, 0, W, H);
      const peaks = waveform?.peaks ?? [];
      const rms = waveform?.rms ?? [];
      if (peaks.length === 0) {
        ctx.fillStyle = "#475569";
        ctx.font = "12px sans-serif";
        ctx.textAlign = "center";
        ctx.fillText(waveform === null ? "Loading waveform…" : "No audio data", W / 2, H / 2);
        return;
      }
      ctx.fillStyle = "#0d2818";
      ctx.fillRect(startX, 0, endX - startX, H);
      const mid = H / 2;
      const amp = H * 0.46;
      const fileMs = fileDurationS * 1000;
      const viewSpan = view.endMs - view.startMs;
      // Bin index for a time (ms) — bins always cover the whole file.
      const binAt = (ms: number) =>
        fileMs > 0 ? Math.floor((ms / fileMs) * peaks.length) : 0;
      const rangeMax = (v: number[], a: number, b: number) => {
        let m = 0;
        for (let i = Math.max(0, a); i < b && i < v.length; i++) if (v[i] > m) m = v[i];
        return m;
      };
      for (let x = 0; x < W; x++) {
        const t0 = view.startMs + (x / W) * viewSpan;
        const t1 = view.startMs + ((x + 1) / W) * viewSpan;
        const from = binAt(t0);
        const to = Math.max(from + 1, binAt(t1));
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
      ctx.fillStyle = "rgba(74, 222, 128, 0.35)";
      ctx.fillRect(0, mid - 0.5, W, 1);
    },
    [waveform, fileDurationS],
  );

  const paintFilmstrip = useCallback<TrimPainter>(
    (ctx, W, H, startX, endX, view) => {
      ctx.fillStyle = "#000";
      ctx.fillRect(0, 0, W, H);
      if (!tiles || tiles.length === 0) {
        ctx.fillStyle = "#475569";
        ctx.font = "12px sans-serif";
        ctx.textAlign = "center";
        ctx.fillText(tiles === null ? "Generating preview…" : "No preview", W / 2, H / 2);
        return;
      }
      // Tiles are time-anchored: map each tile's time range into the visible
      // window so zooming stays aligned.
      const viewSpan = view.endMs - view.startMs;
      const drawStrip = (
        strip: HTMLImageElement[],
        stripStartMs: number,
        stripEndMs: number,
      ) => {
        const tileMs = (stripEndMs - stripStartMs) / strip.length;
        for (let i = 0; i < strip.length; i++) {
          const t0 = stripStartMs + i * tileMs;
          const x0 = ((t0 - view.startMs) / viewSpan) * W;
          const x1 = ((t0 + tileMs - view.startMs) / viewSpan) * W;
          if (x1 < 0 || x0 > W) continue;
          const img = strip[i];
          const cellW = x1 - x0;
          const scale = Math.max(cellW / img.width, H / img.height);
          const dw = img.width * scale;
          const dh = img.height * scale;
          ctx.save();
          ctx.beginPath();
          ctx.rect(x0, 0, cellW, H);
          ctx.clip();
          ctx.drawImage(img, x0 + (cellW - dw) / 2, (H - dh) / 2, dw, dh);
          ctx.restore();
        }
      };
      drawStrip(tiles, 0, durationMs);
      // Window-matched frames on top while zoomed in (crisper than the
      // stretched whole-file tiles).
      if (rangeStrip && rangeStrip.tiles.length > 0 && viewSpan < durationMs) {
        drawStrip(rangeStrip.tiles, rangeStrip.startMs, rangeStrip.endMs);
      }
      ctx.fillStyle = "rgba(0, 0, 0, 0.65)";
      ctx.fillRect(0, 0, startX, H);
      ctx.fillRect(endX, 0, W - endX, H);
    },
    [tiles, durationMs, rangeStrip],
  );

  const dragPreview = useCallback(
    (ms: number) => {
      if (!tiles || tiles.length === 0 || durationMs <= 0) return null;
      const idx = Math.min(
        tiles.length - 1,
        Math.max(0, Math.round((ms / durationMs) * (tiles.length - 1))),
      );
      return (
        <div style={{ background: "#000", border: "1px solid var(--wc-border-strong)", borderRadius: 4, overflow: "hidden", boxShadow: "0 4px 16px rgba(0,0,0,0.6)" }}>
          <img src={tiles[idx].src} alt="" style={{ display: "block", width: 200 }} />
          <div style={{ textAlign: "center", fontSize: 11, padding: "2px 0", color: "var(--wc-text)", fontVariantNumeric: "tabular-nums", background: "var(--wc-bg-deepest)" }}>
            {(ms / 1000).toFixed(3)}s
          </div>
        </div>
      );
    },
    [tiles, durationMs],
  );

  // ── Audition (audio only) ─────────────────────────────────────────────────
  const toggleAudition = async () => {
    if (previewVoice) {
      await stopPreview(previewVoice).catch(() => {});
      setPreviewVoice(null);
      return;
    }
    const vid = await previewCue(cueId).catch(() => null);
    if (typeof vid === "string") setPreviewVoice(vid);
  };

  if (!cue) {
    return (
      <div style={dockShell}>
        <span style={{ color: "var(--wc-text-faint)", fontSize: 12, padding: 12 }}>Loading…</span>
      </div>
    );
  }

  const hasVamp = slices.play_counts.some((c) => c === PLAY_COUNT_INFINITE);

  return (
    <div style={dockShell}>
      {/* Header */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 10,
          padding: "6px 12px",
          borderBottom: "1px solid var(--wc-border)",
          background: "var(--wc-bg-deepest)",
        }}
      >
        <span style={{ fontWeight: 600, fontSize: 13 }}>
          {isVideo ? "🎬" : "🔊"} {cue.name}
        </span>
        {cue.number && (
          <span style={{ fontSize: 11, color: "var(--wc-text-muted)" }}>#{cue.number}</span>
        )}
        <span style={{ fontSize: 11, color: "var(--wc-text-muted)" }}>
          {durationMs > 0 ? `${(durationMs / 1000).toFixed(2)}s` : ""}
        </span>
        {!isVideo && (
          <button onClick={toggleAudition} style={headerBtn} title="Audition the cue (preview playback)">
            {previewVoice ? "■ Stop" : "▶ Audition"}
          </button>
        )}
        {hasVamp && (
          <span style={{ fontSize: 11, color: "#facc15" }} title="This cue vamps — add a Devamp Cue to release it during the show.">
            ∞ vamp — release with a Devamp Cue
          </span>
        )}
        <span style={{ flex: 1 }} />
        <span style={{ fontSize: 11, color: "var(--wc-text-faint)" }}>
          Double-click: add slice · drag ▾: move · right-click: remove · badge: play count
        </span>
        <button onClick={onClose} style={headerBtn} title="Close editor">✕</button>
      </div>

      {/* Timeline */}
      <div style={{ padding: "10px 12px 6px" }}>
        {durationMs > 0 ? (
          <SliceTimeline
            durationMs={durationMs}
            startMs={cue.start_time_ms}
            endMs={cue.end_time_ms}
            slices={slices}
            height={TIMELINE_HEIGHT}
            paint={isVideo ? paintFilmstrip : paintWaveform}
            paintKey={isVideo ? tiles : waveform}
            onCommitStart={(ms) => save({ start_time_ms: ms })}
            onCommitEnd={(ms) => save({ end_time_ms: ms })}
            onSlicesChange={(s) => save({ slices: s })}
            dragPreview={isVideo ? dragPreview : undefined}
            onZoomDetail={() => setWantDetail(true)}
            onViewChange={setZoomView}
          />
        ) : (
          <div style={{ color: "var(--wc-text-faint)", fontSize: 12, padding: 16, textAlign: "center" }}>
            {cue.file_path ? "Waiting for media duration…" : "No file assigned."}
          </div>
        )}
      </div>
    </div>
  );
}

const dockShell: React.CSSProperties = {
  borderTop: "1px solid var(--wc-border)",
  background: "var(--wc-bg-app)",
  display: "flex",
  flexDirection: "column",
  flexShrink: 0,
};

const headerBtn: React.CSSProperties = {
  background: "var(--wc-bg-surface)",
  border: "1px solid var(--wc-border-strong)",
  borderRadius: 4,
  color: "var(--wc-text)",
  cursor: "pointer",
  fontSize: 11,
  padding: "2px 8px",
};
