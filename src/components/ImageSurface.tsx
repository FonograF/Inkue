import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";

interface VoiceInitData {
  voice_id: string;
  data_url: string;
  fade_in_ms: number;
}

interface ImageLayer {
  voiceId: string;
  dataUrl: string;
}

export function OutputSurface() {
  const win = getCurrentWindow();
  const label = win.label;
  const isFloating = label === "output-surface-float";

  const [layer, setLayer] = useState<ImageLayer | null>(null);

  // Refs — stable across renders, no stale-closure risk in event listeners.
  const imgRef = useRef<HTMLImageElement>(null);
  const currentVoiceRef = useRef<string | null>(null);
  const pendingFadeInRef = useRef<number>(0); // fade-in ms for the next onLoad
  const hideTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const showImage = useCallback(
    (voiceId: string, dataUrl: string, fadeInMs: number) => {
      // Cancel any in-progress hide.
      if (hideTimerRef.current) {
        clearTimeout(hideTimerRef.current);
        hideTimerRef.current = null;
      }
      // Reset any fade-out in progress on the existing element.
      if (imgRef.current) {
        imgRef.current.style.transition = "none";
        imgRef.current.style.opacity = "0";
      }

      currentVoiceRef.current = voiceId;
      pendingFadeInRef.current = fadeInMs;
      setLayer({ voiceId, dataUrl });
    },
    [],
  );

  useEffect(() => {
    const unlisteners: Array<() => void> = [];

    // On mount: fetch any voice the engine queued before React loaded.
    invoke<VoiceInitData | null>("get_surface_current_voice", {
      surfaceLabel: label,
    })
      .then((data) => {
        if (data) showImage(data.voice_id, data.data_url, data.fade_in_ms);
      })
      .catch(console.error);

    win
      .listen<{ voice_id: string; data_url: string; fade_in_ms: number }>(
        "surface-show-image",
        (e) => {
          showImage(
            e.payload.voice_id,
            e.payload.data_url,
            e.payload.fade_in_ms,
          );
        },
      )
      .then((u) => unlisteners.push(u));

    win
      .listen<{ voice_id: string; fade_ms: number }>(
        "surface-hide-image",
        (e) => {
          if (currentVoiceRef.current !== e.payload.voice_id) return;

          const fadeMs = e.payload.fade_ms;
          const vid = e.payload.voice_id;
          currentVoiceRef.current = null;

          const el = imgRef.current;
          if (el) {
            // Direct DOM: start the fade-out transition.
            el.style.transition = `opacity ${fadeMs}ms ease`;
            el.style.opacity = "0";
          }

          hideTimerRef.current = setTimeout(() => {
            setLayer(null);
            void invoke("report_image_faded_out", { voiceId: vid });
          }, fadeMs);
        },
      )
      .then((u) => unlisteners.push(u));

    return () => {
      unlisteners.forEach((u) => u());
    };
    // showImage is stable (useCallback with no deps).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Called by the <img> once the image data is decoded and ready to paint.
  // At this point the element is in the DOM at opacity:0. We force a reflow
  // (getBoundingClientRect) so the browser commits that state, then immediately
  // start the CSS transition to opacity:1.  This is more reliable than the
  // double-rAF + React state approach, which can be collapsed by React 18
  // automatic batching before the browser gets a paint opportunity.
  const handleLoad = useCallback(() => {
    const el = imgRef.current;
    if (!el) return;
    const fadeInMs = pendingFadeInRef.current;
    // Force the browser to acknowledge opacity:0 before we change it.
    el.getBoundingClientRect();
    if (fadeInMs > 0) {
      el.style.transition = `opacity ${fadeInMs}ms ease`;
    }
    el.style.opacity = "1";
  }, []);

  const handleDragStart = (e: React.MouseEvent) => {
    if (e.button !== 0) return;
    e.preventDefault();
    void win.startDragging();
  };

  return (
    <div
      onMouseDown={isFloating ? handleDragStart : undefined}
      style={{
        width: "100vw",
        height: "100vh",
        background: "black",
        overflow: "hidden",
        position: "relative",
        cursor: isFloating ? "move" : "none",
      }}
    >
      {layer && (
        <img
          // Key forces a fresh DOM element (and a fresh onLoad) for each new
          // voice, ensuring the fade-in always starts from opacity:0.
          key={`img-${layer.voiceId}`}
          ref={imgRef}
          src={layer.dataUrl}
          alt=""
          onLoad={handleLoad}
          style={{
            position: "absolute",
            inset: 0,
            width: "100%",
            height: "100%",
            objectFit: "contain",
            opacity: 0, // always start invisible; handleLoad drives the reveal
            pointerEvents: "none",
            userSelect: "none",
          }}
        />
      )}
    </div>
  );
}
