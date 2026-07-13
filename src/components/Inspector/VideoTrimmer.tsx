// Video trim strip: a filmstrip of frames spread across the file, with the
// same draggable start/end markers as the audio waveform. While a marker is
// dragged, a popup previews the frame under the cursor (nearest tile of a
// denser, larger scrub strip prefetched in the background).

import { useCallback, useEffect, useState } from "react";
import type { VideoCueData } from "../../lib/types";
import { getVideoFilmstrip } from "../../lib/commands";
import { TrimStrip, type TrimPainter } from "./TrimStrip";

const FILMSTRIP_TILES = 8;
const FILMSTRIP_TILE_WIDTH = 160;
const SCRUB_TILES = 32;
const SCRUB_TILE_WIDTH = 320;
const SCRUB_PREVIEW_WIDTH = 200;

// Session-lifetime caches of decoded tiles (backend caches JPEGs on disk).
const filmstripCache = new Map<string, HTMLImageElement[]>();
const scrubCache = new Map<string, HTMLImageElement[]>();

function loadImage(dataUrl: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = reject;
    img.src = dataUrl;
  });
}

function fetchTiles(
  path: string,
  tiles: number,
  tileWidth: number,
  cache: Map<string, HTMLImageElement[]>,
): Promise<HTMLImageElement[]> {
  const cached = cache.get(path);
  if (cached) return Promise.resolve(cached);
  return getVideoFilmstrip(path, tiles, tileWidth)
    .then((urls) => Promise.all(urls.map(loadImage)))
    .then((images) => {
      cache.set(path, images);
      return images;
    });
}

export function VideoTrimmer({
  cue,
  durationMs,
  onSave,
  onExpand,
}: {
  cue: VideoCueData;
  durationMs: number;
  onSave: (p: Partial<VideoCueData>) => void;
  /** Open the clip editor dock (trim + slices). */
  onExpand?: () => void;
}) {
  const [tiles, setTiles] = useState<HTMLImageElement[] | null>(
    () => (cue.file_path && filmstripCache.get(cue.file_path)) || null,
  );
  const [scrubTiles, setScrubTiles] = useState<HTMLImageElement[] | null>(
    () => (cue.file_path && scrubCache.get(cue.file_path)) || null,
  );
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    const path = cue.file_path;
    setFailed(false);
    if (!path) {
      setTiles(null);
      setScrubTiles(null);
      return;
    }
    setTiles(filmstripCache.get(path) ?? null);
    setScrubTiles(scrubCache.get(path) ?? null);

    let stale = false;
    // Strip first (small, fast), then the denser scrub set in the background
    // so the drag preview is ready by the time the operator reaches for it.
    fetchTiles(path, FILMSTRIP_TILES, FILMSTRIP_TILE_WIDTH, filmstripCache)
      .then((images) => {
        if (stale) return;
        setTiles(images);
        return fetchTiles(path, SCRUB_TILES, SCRUB_TILE_WIDTH, scrubCache).then(
          (scrub) => {
            if (!stale) setScrubTiles(scrub);
          },
        );
      })
      .catch(() => {
        if (!stale) setFailed(true);
      });
    return () => { stale = true; };
  }, [cue.file_path]);

  const paint = useCallback<TrimPainter>(
    (ctx, W, H, startX, endX) => {
      ctx.fillStyle = "#000";
      ctx.fillRect(0, 0, W, H);

      if (!tiles || tiles.length === 0) {
        ctx.fillStyle = "#475569";
        ctx.font = "11px sans-serif";
        ctx.textAlign = "center";
        ctx.fillText("Generating preview…", W / 2, H / 2 + 4);
        return;
      }

      // Tiles side by side, each cover-cropped into its cell.
      const cellW = W / tiles.length;
      for (let i = 0; i < tiles.length; i++) {
        const img = tiles[i];
        const scale = Math.max(cellW / img.width, H / img.height);
        const dw = img.width * scale;
        const dh = img.height * scale;
        ctx.save();
        ctx.beginPath();
        ctx.rect(i * cellW, 0, cellW, H);
        ctx.clip();
        ctx.drawImage(img, i * cellW + (cellW - dw) / 2, (H - dh) / 2, dw, dh);
        ctx.restore();
      }

      // Dim everything outside the active region.
      ctx.fillStyle = "rgba(0, 0, 0, 0.65)";
      ctx.fillRect(0, 0, startX, H);
      ctx.fillRect(endX, 0, W - endX, H);
    },
    [tiles],
  );

  const dragPreview = useCallback(
    (ms: number) => {
      const frames = scrubTiles ?? tiles;
      if (!frames || frames.length === 0 || durationMs <= 0) return null;
      const idx = Math.min(
        frames.length - 1,
        Math.max(0, Math.round((ms / durationMs) * (frames.length - 1))),
      );
      return (
        <div
          style={{
            background: "#000",
            border: "1px solid var(--wc-border-strong)",
            borderRadius: 4,
            overflow: "hidden",
            boxShadow: "0 4px 16px rgba(0, 0, 0, 0.6)",
          }}
        >
          <img
            src={frames[idx].src}
            alt=""
            style={{ display: "block", width: SCRUB_PREVIEW_WIDTH }}
          />
          <div
            style={{
              textAlign: "center",
              fontSize: 11,
              padding: "2px 0",
              color: "var(--wc-text)",
              fontVariantNumeric: "tabular-nums",
              background: "var(--wc-bg-deepest)",
            }}
          >
            {(ms / 1000).toFixed(3)}s
          </div>
        </div>
      );
    },
    [scrubTiles, tiles, durationMs],
  );

  if (!cue.file_path || durationMs <= 0 || failed) return null;

  return (
    <TrimStrip
      durationMs={durationMs}
      startMs={cue.start_time_ms}
      endMs={cue.end_time_ms}
      height={64}
      paint={paint}
      paintKey={tiles}
      onCommitStart={(ms) => onSave({ start_time_ms: ms })}
      onCommitEnd={(ms) => onSave({ end_time_ms: ms })}
      dragPreview={dragPreview}
      onExpand={onExpand}
      sliceMarkersMs={cue.slices?.markers ?? []}
    />
  );
}
