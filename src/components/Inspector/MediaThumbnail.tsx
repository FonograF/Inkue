// Inline media preview for the inspector's Media section: a thumbnail of the
// selected image, or a representative frame of the selected video, so the
// operator can identify content at a glance even with unclear cue names.

import { useEffect, useState } from "react";
import { getMediaThumbnail } from "../../lib/commands";

// Session-lifetime cache: the backend caches JPEGs on disk, but this avoids
// re-invoking (and re-transferring the data URL) on every cue re-selection.
const thumbnailCache = new Map<string, string>();

export function MediaThumbnail({
  path,
  seekInto,
}: {
  path: string;
  /** Pick a frame ~15% in (videos — frame 0 is often black). */
  seekInto: boolean;
}) {
  const [url, setUrl] = useState<string | null>(() => thumbnailCache.get(path) ?? null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    const cached = thumbnailCache.get(path);
    setUrl(cached ?? null);
    setFailed(false);
    if (cached) return;

    let stale = false;
    getMediaThumbnail(path, seekInto)
      .then((dataUrl) => {
        thumbnailCache.set(path, dataUrl);
        if (!stale) setUrl(dataUrl);
      })
      .catch(() => {
        if (!stale) setFailed(true);
      });
    return () => { stale = true; };
  }, [path, seekInto]);

  if (failed) return null;

  return (
    <div
      style={{
        marginBottom: 10,
        borderRadius: 4,
        overflow: "hidden",
        border: "1px solid var(--wc-border-strong)",
        background: "#000",
        minHeight: url ? undefined : 90,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
      }}
    >
      {url ? (
        <img
          src={url}
          alt=""
          style={{ display: "block", width: "100%", maxHeight: 180, objectFit: "contain" }}
        />
      ) : (
        <span style={{ fontSize: 11, color: "var(--wc-text-faint)" }}>Generating preview…</span>
      )}
    </div>
  );
}
