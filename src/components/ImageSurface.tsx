import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";

interface SurfaceData {
  data_url: string;
  fade_in_ms: number;
}

export function ImageSurface({ voiceId }: { voiceId: string }) {
  const [dataUrl, setDataUrl] = useState<string | null>(null);
  const [opacity, setOpacity] = useState(0);
  const [transitionMs, setTransitionMs] = useState(0);

  useEffect(() => {
    const win = getCurrentWindow();
    const unlisteners: Array<() => void> = [];

    // Fetch image data from backend on mount (avoids window-creation timing race)
    invoke<SurfaceData>("get_image_surface_data", { voiceId })
      .then((data) => {
        setTransitionMs(data.fade_in_ms);
        setDataUrl(data.data_url);
        // Defer opacity change so the transition fires after the image is painted
        requestAnimationFrame(() => {
          requestAnimationFrame(() => setOpacity(1));
        });
      })
      .catch(console.error);

    win
      .listen<{ fade_ms: number }>("hide-image", (e) => {
        setTransitionMs(e.payload.fade_ms);
        setOpacity(0);
        setTimeout(() => {
          void invoke("report_image_faded_out", { voiceId });
        }, e.payload.fade_ms);
      })
      .then((u) => unlisteners.push(u));

    return () => {
      unlisteners.forEach((u) => u());
    };
  }, [voiceId]);

  return (
    <div
      style={{
        width: "100vw",
        height: "100vh",
        background: "black",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        overflow: "hidden",
      }}
    >
      {dataUrl && (
        <img
          src={dataUrl}
          alt=""
          style={{
            maxWidth: "100%",
            maxHeight: "100%",
            objectFit: "contain",
            opacity,
            transition: `opacity ${transitionMs}ms ease`,
            pointerEvents: "none",
            userSelect: "none",
          }}
        />
      )}
    </div>
  );
}
