import { useCallback, useEffect, useState } from "react";
import type { AudioCueData, VideoCueData } from "../../lib/types";
import { getNormalizeDb } from "../../lib/commands";
import { Field, inputStyle } from "./Field";

export function LevelsTab({
  cue,
  isAudio,
  onSave,
}: {
  cue: AudioCueData | VideoCueData;
  isAudio: boolean;
  onSave: (p: Partial<AudioCueData | VideoCueData>) => void;
}) {
  const [volumeDb, setVolumeDb] = useState(cue.volume_db);
  const [pan, setPan] = useState(isAudio ? (cue as AudioCueData).pan : 0);
  const [normalizing, setNormalizing] = useState(false);
  const [normalizeError, setNormalizeError] = useState<string | null>(null);

  // Sync when the selected cue changes or after an external save
  useEffect(() => {
    setVolumeDb(cue.volume_db);
    if (isAudio) setPan((cue as AudioCueData).pan);
    setNormalizeError(null);
  }, [cue.id, cue.volume_db, isAudio, (cue as AudioCueData).pan]);

  const commitVolume = useCallback(
    (v: number) => onSave({ volume_db: v }),
    [onSave]
  );
  const commitPan = useCallback(
    (v: number) => onSave({ pan: v } as Partial<AudioCueData>),
    [onSave]
  );

  const handleNormalize = useCallback(async () => {
    setNormalizing(true);
    setNormalizeError(null);
    try {
      const db = await getNormalizeDb(cue.id);
      const rounded = Math.round(db * 10) / 10;
      setVolumeDb(rounded);
      commitVolume(rounded);
    } catch (e) {
      setNormalizeError(String(e));
    } finally {
      setNormalizing(false);
    }
  }, [cue.id, commitVolume]);

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

      {isAudio && (
        <Field label="">
          <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
            <button
              onClick={() => void handleNormalize()}
              disabled={normalizing}
              style={{
                background: normalizing ? "var(--wc-bg-surface)" : "var(--wc-bg-app)",
                border: "1px solid var(--wc-border-strong)",
                borderRadius: 4,
                color: normalizing ? "var(--wc-text-faint)" : "var(--wc-text-secondary)",
                cursor: normalizing ? "default" : "pointer",
                fontSize: 12,
                padding: "4px 10px",
                textAlign: "center",
              }}
              onMouseEnter={(e) => {
                if (!normalizing)
                  (e.currentTarget as HTMLButtonElement).style.color = "var(--wc-text)";
              }}
              onMouseLeave={(e) => {
                if (!normalizing)
                  (e.currentTarget as HTMLButtonElement).style.color = "var(--wc-text-secondary)";
              }}
            >
              {normalizing ? "Analyzing…" : "Normalize to 0 dBFS"}
            </button>
            {normalizeError && (
              <span style={{ fontSize: 11, color: "#f87171" }}>{normalizeError}</span>
            )}
          </div>
        </Field>
      )}
      {isAudio && (
        <Field label="Pan">
          <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
            <span style={{ color: "var(--wc-text-secondary)", fontSize: 11, flexShrink: 0 }}>L</span>
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
            <span style={{ color: "var(--wc-text-secondary)", fontSize: 11, flexShrink: 0 }}>R</span>
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
      )}
    </>
  );
}
