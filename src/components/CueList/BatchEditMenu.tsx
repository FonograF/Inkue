// The "edit the selection" part of the cue context menu. Each entry becomes a
// `BatchEdit`; a section shows up only when some selected cue has the setting,
// with an "n/total" hint when only part of the selection does.

import type { ReactNode } from "react";

import type { BatchEdit, BatchEditKind, CueJson } from "../../lib/batchEdit";
import {
  VOLUME_MAX_DB,
  VOLUME_MIN_DB,
  countSupporting,
  layerStyleOf,
  playCountOf,
  sharedValue,
} from "../../lib/batchEdit";
import type { ContinueMode, OutputPatch, VideoOutputInfo } from "../../lib/types";
import { PLAY_COUNT_INFINITE } from "../../lib/types";
import { COLOR_OPTIONS } from "../Inspector/ColorPicker";
import { BLEND_MODES } from "../Inspector/LayerTab";
import type { BatchValuePrompt } from "./BatchValueDialog";
import { CtxItem, CtxSeparator, CtxSubmenu } from "./ContextMenuParts";

const CONTINUE_MODES: { value: ContinueMode; label: string }[] = [
  { value: "do_not_continue", label: "Do Not Continue" },
  { value: "auto_continue", label: "Auto-Continue" },
  { value: "auto_follow", label: "Auto-Follow" },
];

interface Props {
  cues: CueJson[];
  openLeft: boolean;
  outputPatches: OutputPatch[];
  videoOutputs: VideoOutputInfo[];
  onApply: (edit: BatchEdit) => void;
  onPrompt: (kind: BatchEditKind, prompt: BatchValuePrompt) => void;
}

export function BatchEditMenu({ cues, openLeft, outputPatches, videoOutputs, onApply, onPrompt }: Props) {
  const count = (kind: BatchEditKind) => countSupporting(kind, cues);
  const hint = (kind: BatchEditKind) => {
    const n = count(kind);
    return n < cues.length ? `${n}/${cues.length}` : undefined;
  };
  const shared = <T,>(kind: BatchEditKind, read: (cue: CueJson) => T) => sharedValue(kind, cues, read);
  const section = (kind: BatchEditKind, label: string, children: ReactNode) =>
    count(kind) > 0 ? (
      <CtxSubmenu label={label} openLeft={openLeft} hint={hint(kind)}>{children}</CtxSubmenu>
    ) : null;

  const color = shared("color", (c) => c.color);
  const continueMode = shared("continue_mode", (c) => c.continue_mode);
  const allDisabled = cues.every((c) => c.is_disabled === true);
  const msToSeconds = (ms: unknown) => (typeof ms === "number" ? ms / 1000 : undefined);

  const promptSeconds = (
    kind: "pre_wait" | "post_wait" | "fade_in" | "fade_out",
    title: string,
    readMs: (cue: CueJson) => unknown,
  ) =>
    onPrompt(kind, {
      title, label: "Duration", unit: "s", min: 0, step: 0.1,
      initial: msToSeconds(shared(kind, (c) => readMs(c) ?? 0)),
      onConfirm: (seconds) => onApply({ kind, ms: seconds * 1000 } as BatchEdit),
    });

  return (
    <>
      <CtxSubmenu label="Color" openLeft={openLeft}>
        {COLOR_OPTIONS.map((option) => (
          <CtxItem
            key={option.value}
            label={option.label}
            color={option.hex}
            checked={color === option.value}
            onClick={() => onApply({ kind: "color", color: option.value })}
          />
        ))}
      </CtxSubmenu>

      <CtxSubmenu label="Continue" openLeft={openLeft}>
        {CONTINUE_MODES.map((mode) => (
          <CtxItem
            key={mode.value}
            label={mode.label}
            checked={continueMode === mode.value}
            onClick={() => onApply({ kind: "continue_mode", mode: mode.value })}
          />
        ))}
      </CtxSubmenu>

      <CtxSubmenu label="Timing" openLeft={openLeft}>
        <CtxItem label="Set Pre-Wait…" onClick={() => promptSeconds("pre_wait", "Set Pre-Wait", (c) => c.pre_wait_ms)} />
        <CtxItem label="Set Post-Wait…" onClick={() => promptSeconds("post_wait", "Set Post-Wait", (c) => c.post_wait_ms)} />
        <CtxSeparator />
        <CtxItem label="Clear Pre-Wait" onClick={() => onApply({ kind: "pre_wait", ms: 0 })} />
        <CtxItem label="Clear Post-Wait" onClick={() => onApply({ kind: "post_wait", ms: 0 })} />
      </CtxSubmenu>

      {section("volume_set", "Levels", (
        <>
          <CtxItem
            label="Set Volume…"
            onClick={() => onPrompt("volume_set", {
              title: "Set Volume", label: "Volume", unit: "dB", min: VOLUME_MIN_DB, max: VOLUME_MAX_DB, step: 0.5,
              initial: shared("volume_set", (c) => c.volume_db as number),
              onConfirm: (db) => onApply({ kind: "volume_set", db }),
            })}
          />
          <CtxItem
            label="Adjust Volume…"
            onClick={() => onPrompt("volume_adjust", {
              title: "Adjust Volume", label: "Change by", unit: "dB", step: 0.5, initial: 0,
              onConfirm: (deltaDb) => onApply({ kind: "volume_adjust", deltaDb }),
            })}
          />
          {count("pan") > 0 && (
            <CtxItem
              label="Set Pan…"
              hint={hint("pan")}
              onClick={() => onPrompt("pan", {
                title: "Set Pan", label: "Pan", unit: "L/R", min: -1, max: 1, step: 0.05,
                initial: shared("pan", (c) => c.pan as number),
                onConfirm: (pan) => onApply({ kind: "pan", pan }),
              })}
            />
          )}
        </>
      ))}

      {section("output_patch", "Output Patch", (
        <OutputPatchItems
          patches={outputPatches}
          current={shared("output_patch", (c) => c.output_patch_id ?? null)}
          onPick={(patchId) => onApply({ kind: "output_patch", patchId })}
        />
      ))}

      {section("fade_in", "Fades", (
        <>
          <CtxItem label="Set Fade In…" onClick={() => promptSeconds("fade_in", "Set Fade In", (c) => c.fade_in_ms ?? c.video_fade_in_ms)} />
          <CtxItem label="Set Fade Out…" onClick={() => promptSeconds("fade_out", "Set Fade Out", (c) => c.fade_out_ms ?? c.video_fade_out_ms)} />
          <CtxSeparator />
          <CtxItem label="Remove Fade In" onClick={() => onApply({ kind: "fade_in", ms: 0 })} />
          <CtxItem label="Remove Fade Out" onClick={() => onApply({ kind: "fade_out", ms: 0 })} />
        </>
      ))}

      {section("play_count", "Looping", (
        <LoopItems
          current={shared("play_count", playCountOf)}
          onPick={(count) => onApply({ kind: "play_count", count })}
          onCustom={() => onPrompt("play_count", {
            title: "Set Play Count", label: "Plays", unit: "×", min: 1, step: 1, integer: true,
            initial: 2,
            onConfirm: (count) => onApply({ kind: "play_count", count }),
          })}
        />
      ))}

      {count("video_output") + count("opacity") > 0 && (
        <CtxSubmenu label="Video" openLeft={openLeft} hint={hint("video_output")}>
          {count("video_output") > 0 && videoOutputs.length > 1 && (
            <CtxSubmenu label="Output" openLeft={openLeft}>
              {videoOutputs.map((output) => {
                const outputId = output.is_main ? null : output.id;
                return (
                  <CtxItem
                    key={output.id}
                    label={output.is_main ? "Main" : output.name}
                    checked={shared("video_output", (c) => c.output_id ?? null) === outputId}
                    onClick={() => onApply({ kind: "video_output", outputId })}
                  />
                );
              })}
            </CtxSubmenu>
          )}
          {count("opacity") > 0 && (
            <>
              <CtxItem
                label="Set Opacity…"
                onClick={() => onPrompt("opacity", {
                  title: "Set Opacity", label: "Opacity", unit: "%", min: 0, max: 100, step: 1,
                  initial: scaleOrUndefined(shared("opacity", (c) => layerStyleOf(c).opacity), 100),
                  onConfirm: (percent) => onApply({ kind: "opacity", opacity: percent / 100 }),
                })}
              />
              <CtxSubmenu label="Blend Mode" openLeft={openLeft}>
                {BLEND_MODES.map((mode) => (
                  <CtxItem
                    key={mode.value}
                    label={mode.label}
                    checked={shared("blend_mode", (c) => layerStyleOf(c).blend_mode) === mode.value}
                    onClick={() => onApply({ kind: "blend_mode", mode: mode.value })}
                  />
                ))}
              </CtxSubmenu>
            </>
          )}
          {count("hold_last_frame") > 0 && (
            <CtxItem
              label="Hold Last Frame"
              hint={hint("hold_last_frame")}
              checked={shared("hold_last_frame", (c) => c.hold_last_frame) === true}
              onClick={() => onApply({
                kind: "hold_last_frame",
                hold: shared("hold_last_frame", (c) => c.hold_last_frame) !== true,
              })}
            />
          )}
        </CtxSubmenu>
      )}

      <CtxItem
        label={allDisabled ? enableLabel("Enable", cues.length) : enableLabel("Disable", cues.length)}
        onClick={() => onApply({ kind: "disabled", disabled: !allDisabled })}
      />
    </>
  );
}

function OutputPatchItems({ patches, current, onPick }: {
  patches: OutputPatch[];
  current: unknown;
  onPick: (patchId: string | null) => void;
}) {
  return (
    <>
      <CtxItem label="Default" checked={current === null} onClick={() => onPick(null)} />
      {patches.length > 0 && <CtxSeparator />}
      {patches.map((patch) => (
        <CtxItem key={patch.id} label={patch.name} checked={current === patch.id} onClick={() => onPick(patch.id)} />
      ))}
    </>
  );
}

function LoopItems({ current, onPick, onCustom }: {
  current: number | undefined;
  onPick: (count: number) => void;
  onCustom: () => void;
}) {
  const custom = current !== undefined && current !== 1 && current !== PLAY_COUNT_INFINITE;
  return (
    <>
      <CtxItem label="Play Once" checked={current === 1} onClick={() => onPick(1)} />
      <CtxItem label="Loop Forever" checked={current === PLAY_COUNT_INFINITE} onClick={() => onPick(PLAY_COUNT_INFINITE)} />
      <CtxItem label="Play Count…" checked={custom} hint={custom ? `${current}×` : undefined} onClick={onCustom} />
    </>
  );
}

function enableLabel(verb: string, count: number): string {
  return count > 1 ? `${verb} ${count} Cues` : `${verb} Cue`;
}

function scaleOrUndefined(value: number | undefined, factor: number): number | undefined {
  return value === undefined ? undefined : value * factor;
}
