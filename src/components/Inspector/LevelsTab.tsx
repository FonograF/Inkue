import { useCallback, useEffect, useState } from "react";
import type { AudioCueData, VideoCueData } from "../../lib/types";
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

  // Sync when the selected cue changes or after an external save
  useEffect(() => {
    setVolumeDb(cue.volume_db);
    if (isAudio) setPan((cue as AudioCueData).pan);
  }, [cue.id, cue.volume_db, isAudio, (cue as AudioCueData).pan]);

  const commitVolume = useCallback(
    (v: number) => onSave({ volume_db: v }),
    [onSave]
  );
  const commitPan = useCallback(
    (v: number) => onSave({ pan: v } as Partial<AudioCueData>),
    [onSave]
  );

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
        <Field label="Pan">
          <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
            <span style={{ color: "#94a3b8", fontSize: 11, flexShrink: 0 }}>L</span>
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
            <span style={{ color: "#94a3b8", fontSize: 11, flexShrink: 0 }}>R</span>
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
