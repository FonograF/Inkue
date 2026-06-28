import { useState, useEffect } from "react";

/**
 * Running-cue indicator that blinks via a discrete JS interval (~1.4 Hz) instead
 * of a continuous CSS keyframe animation.
 *
 * A CSS keyframe animation forces WebKitGTK to commit a fresh frame for the entire
 * UI surface every display refresh (~60 fps) for the whole lifetime of the
 * animation.  On a weak shared-memory iGPU that monopolises the compositor and
 * starves the UI's own paint while a Video Cue's output window is also presenting —
 * the WinCue UI froze to ~0 fps during video playback.  A discrete opacity toggle
 * repaints only a few times per second and leaves the UI surface idle in between,
 * so the compositor has room for both the UI and the video output.
 */
export function RunningLed({ size = 8 }: { size?: number }) {
  const [on, setOn] = useState(true);
  useEffect(() => {
    const id = setInterval(() => setOn((v) => !v), 700);
    return () => clearInterval(id);
  }, []);
  return (
    <span
      style={{
        display: "inline-block",
        width: size,
        height: size,
        borderRadius: "50%",
        background: "#22c55e",
        boxShadow: "0 0 4px #22c55e",
        flexShrink: 0,
        opacity: on ? 1 : 0.3,
      }}
    />
  );
}
