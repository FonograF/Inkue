import { useCallback, useEffect, useState } from "react";
import type { AudioCueData, OutputPatch, VideoCueData } from "../../lib/types";
import { getNormalizeDb, getOutputPatchTable, setLiveLevel } from "../../lib/commands";
import { Field, inputStyle } from "./Field";
import { Select } from "../common/Select";
import { LevelMatrixGrid } from "./LevelMatrixGrid";
import { DragNumber } from "../common/DragNumber";

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
  const [patches, setPatches] = useState<OutputPatch[]>([]);
  const [defaultPatchId, setDefaultPatchId] = useState<string | null>(null);

  useEffect(() => {
    getOutputPatchTable()
      .then((t) => { setPatches(t.patches); setDefaultPatchId(t.default_patch_id); })
      .catch(console.error);
  }, []);

  // The patch the cue actually plays through: its own, or the workspace
  // default when it has none. Resolving only `output_patch_id` left every
  // cue on "Default patch" showing a two-column matrix regardless of how many
  // outputs the patch really has.
  const activePatch = patches.find((p) => p.id === (cue.output_patch_id ?? defaultPatchId));

  // Sync when the selected cue changes or after an external save
  useEffect(() => {
    setVolumeDb(cue.volume_db);
    if (isAudio) setPan((cue as AudioCueData).pan);
    setNormalizeError(null);
  }, [cue.id, cue.volume_db, isAudio, (cue as AudioCueData).pan]);

  // While dragging, the value goes straight to the engine so a playing cue
  // follows the slider. Persisting through onSave on every step would
  // re-serialise the cue and push an undo snapshot per pixel moved.
  const previewLevels = useCallback(
    (v: number, p: number) => { void setLiveLevel(cue.id, v, p).catch(console.error); },
    [cue.id]
  );
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
      <Field label="Output Patch">
        <Select
          style={{ ...inputStyle, cursor: "pointer" }}
          value={cue.output_patch_id ?? ""}
          onChange={(e) => onSave({ output_patch_id: e.target.value || null })}
        >
          <option value="">Default patch</option>
          {patches.map((p) => (
            <option key={p.id} value={p.id}>{p.name}</option>
          ))}
          {cue.output_patch_id && !patches.some((p) => p.id === cue.output_patch_id) && (
            <option value={cue.output_patch_id}>(deleted patch — using default)</option>
          )}
        </Select>
      </Field>

      <Field label="Volume (dB)">
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <input
            style={{ ...inputStyle, flex: 1, padding: "2px 4px" }}
            type="range"
            min="-60"
            max="12"
            step="0.5"
            value={volumeDb}
            onChange={(e) => {
              const v = parseFloat(e.target.value);
              setVolumeDb(v);
              previewLevels(v, pan);
            }}
            onMouseUp={() => commitVolume(volumeDb)}
          />
          <DragNumber
            style={{ ...inputStyle, width: 60 }}
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
              onChange={(e) => {
                const p = parseFloat(e.target.value);
                setPan(p);
                previewLevels(volumeDb, p);
              }}
              onMouseUp={() => commitPan(pan)}
            />
            <span style={{ color: "var(--wc-text-secondary)", fontSize: 11, flexShrink: 0 }}>R</span>
            <DragNumber
              style={{ ...inputStyle, width: 60 }}
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

      <LevelMatrixGrid
        cueId={cue.id}
        matrix={cue.level_matrix ?? null}
        patchName={activePatch?.name ?? "Default patch"}
        deviceChannels={activePatch?.channels ?? []}
        onSave={(m) => onSave({ level_matrix: m } as Partial<AudioCueData>)}
      />
    </>
  );
}
