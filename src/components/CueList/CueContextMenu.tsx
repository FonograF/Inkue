// Right-click menu of the cue list. On a cue it acts on the whole selection
// (the right-clicked cue is selected first when it was not part of it): edit
// shared settings in one undo step, create a Fade / Stop / … aimed at the
// selection, renumber, group, duplicate, delete. On empty space it adds a cue.

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import type { BatchEdit, BatchEditKind, CueJson } from "../../lib/batchEdit";
import { TARGETING_PRESETS, countSupporting } from "../../lib/batchEdit";
import { addCue, getOutputPatches, removeCueFromGroup, ungroup } from "../../lib/commands";
import {
  applyBatchEdit,
  createTargetingCue,
  deleteSelection,
  duplicateSelection,
  groupSelection,
  loadCueJson,
  movePlayheadToSelection,
  renumberSelection,
} from "../../lib/cueOperations";
import type { CueSummary, CueType, OutputPatch } from "../../lib/types";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import { BatchEditMenu } from "./BatchEditMenu";
import { BatchValueDialog, type BatchValuePrompt } from "./BatchValueDialog";
import { ASSIGN_FILE_LABELS, CUE_TYPES, FILE_FILTERS, setFileForCue, type MediaCueType } from "./cueCatalog";
import { CtxHeader, CtxItem, CtxSeparator, CtxSubmenu, menuPanelStyle } from "./ContextMenuParts";
import { RenumberDialog } from "./RenumberDialog";

export interface CueContextMenuState {
  x: number;
  y: number;
  /** Right-clicked cue; null on empty space. */
  cueId: string | null;
  parentGroupId?: string | null;
  /** Cues the menu acts on: the selection, or just the right-clicked cue. */
  targetIds: string[];
}

interface Props {
  menu: CueContextMenuState;
  /** Every visible row (top level and expanded group children). */
  flatItems: { cue: CueSummary; parentGroupId: string | null }[];
  onClose: () => void;
  onRefresh: () => void;
}

/** Room a flyout needs before the menu opens its submenus to the left. */
const FLYOUT_ROOM_PX = 380;

export function CueContextMenu({ menu, flatItems, onClose, onRefresh }: Props) {
  const [cueJson, setCueJson] = useState<CueJson[] | null>(null);
  const [outputPatches, setOutputPatches] = useState<OutputPatch[]>([]);
  const [prompt, setPrompt] = useState<{ kind: BatchEditKind; prompt: BatchValuePrompt } | null>(null);
  const [renumberOpen, setRenumberOpen] = useState(false);
  const videoOutputs = useWorkspaceStore((s) => s.videoOutputs);
  const confirmBeforeDelete = useWorkspaceStore((s) => s.generalPrefs.confirm_before_delete);

  const { targetIds } = menu;
  const targetKey = targetIds.join(",");
  useEffect(() => {
    let cancelled = false;
    if (targetIds.length === 0) return;
    loadCueJson(targetIds)
      .then((cues) => { if (!cancelled) setCueJson(cues); })
      .catch(console.error);
    getOutputPatches()
      .then((patches) => { if (!cancelled) setOutputPatches(patches); })
      .catch(console.error);
    return () => { cancelled = true; };
  }, [targetKey]); // eslint-disable-line react-hooks/exhaustive-deps

  const panelRef = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState({ left: menu.x, top: menu.y });
  useLayoutEffect(() => {
    const panel = panelRef.current;
    if (!panel) return;
    const { width, height } = panel.getBoundingClientRect();
    setPosition({
      left: Math.max(4, Math.min(menu.x, window.innerWidth - width - 4)),
      top: Math.max(4, Math.min(menu.y, window.innerHeight - height - 4)),
    });
  }, [menu.x, menu.y, cueJson]);

  const run = (action: () => Promise<unknown> | void) => {
    onClose();
    void Promise.resolve(action()).catch(console.error);
  };
  const applyEdit = (edit: BatchEdit) => run(() => applyBatchEdit(targetIds, edit, onRefresh));
  const askValue = (kind: BatchEditKind, valuePrompt: BatchValuePrompt) => setPrompt({ kind, prompt: valuePrompt });

  if (prompt && cueJson) {
    return (
      <BatchValueDialog
        prompt={{ ...prompt.prompt, onConfirm: (value) => { prompt.prompt.onConfirm(value); onClose(); } }}
        affected={countSupporting(prompt.kind, cueJson)}
        selected={cueJson.length}
        onCancel={onClose}
      />
    );
  }
  if (renumberOpen) {
    return (
      <RenumberDialog
        cueCount={targetIds.length}
        onCancel={onClose}
        onConfirm={(start, increment) => run(() => renumberSelection(start, increment, onRefresh))}
      />
    );
  }

  const openLeft = menu.x > window.innerWidth - FLYOUT_ROOM_PX;
  const clicked = flatItems.find((item) => item.cue.id === menu.cueId);

  return (
    <>
      <div
        style={{ position: "fixed", inset: 0, zIndex: 9998 }}
        onClick={onClose}
        onContextMenu={(e) => { e.preventDefault(); onClose(); }}
      />
      <div
        ref={panelRef}
        style={{ ...menuPanelStyle, position: "fixed", ...position, zIndex: 9999, minWidth: 220, fontSize: 13 }}
      >
        {!clicked ? (
          <CtxSubmenu label="Add Cue" openLeft={openLeft}>
            <CueTypeItems onPick={(type) => run(() => addCueAt(type, -1, onRefresh))} />
          </CtxSubmenu>
        ) : (
          <CueMenuBody
            clicked={clicked}
            menu={menu}
            flatItems={flatItems}
            cueJson={cueJson}
            openLeft={openLeft}
            outputPatches={outputPatches}
            videoOutputs={videoOutputs}
            confirmBeforeDelete={confirmBeforeDelete}
            run={run}
            onApply={applyEdit}
            onPrompt={askValue}
            onRenumber={() => setRenumberOpen(true)}
            onRefresh={onRefresh}
          />
        )}
      </div>
    </>
  );
}

interface BodyProps {
  clicked: { cue: CueSummary; parentGroupId: string | null };
  menu: CueContextMenuState;
  flatItems: Props["flatItems"];
  cueJson: CueJson[] | null;
  openLeft: boolean;
  outputPatches: OutputPatch[];
  videoOutputs: ReturnType<typeof useWorkspaceStore.getState>["videoOutputs"];
  confirmBeforeDelete: boolean;
  run: (action: () => Promise<unknown> | void) => void;
  onApply: (edit: BatchEdit) => void;
  onPrompt: (kind: BatchEditKind, prompt: BatchValuePrompt) => void;
  onRenumber: () => void;
  onRefresh: () => void;
}

function CueMenuBody(props: BodyProps) {
  const { clicked, menu, flatItems, cueJson, openLeft, run, onRefresh } = props;
  const { targetIds } = menu;
  const count = targetIds.length;
  const multiple = count > 1;
  const inGroup = clicked.parentGroupId !== null;
  const assignType = mediaTypeOf(clicked.cue.cue_type);
  const topLevelIndex = useWorkspaceStore.getState().cues.findIndex((c) => c.id === clicked.cue.id);
  const presets = cueJson ? TARGETING_PRESETS.filter((preset) => cueJson.some(preset.appliesTo)) : [];

  return (
    <>
      {multiple && <CtxHeader>{count} cues selected</CtxHeader>}

      {!inGroup && topLevelIndex >= 0 && (
        <>
          <CtxSubmenu label="Add Cue Above" openLeft={openLeft}>
            <CueTypeItems onPick={(type) => run(() => addCueAt(type, topLevelIndex, onRefresh))} />
          </CtxSubmenu>
          <CtxSubmenu label="Add Cue Below" openLeft={openLeft}>
            <CueTypeItems onPick={(type) => run(() => addCueAt(type, topLevelIndex + 1, onRefresh))} />
          </CtxSubmenu>
          <CtxSeparator />
        </>
      )}

      {cueJson ? (
        <BatchEditMenu
          cues={cueJson}
          openLeft={openLeft}
          outputPatches={props.outputPatches}
          videoOutputs={props.videoOutputs}
          onApply={props.onApply}
          onPrompt={props.onPrompt}
        />
      ) : (
        <CtxItem label="Loading…" disabled onClick={() => {}} />
      )}

      <CtxSeparator />
      {presets.length > 0 && (
        <CtxSubmenu label={multiple ? "Create Cue Targeting Selection" : "Create Cue Targeting This"} openLeft={openLeft}>
          {presets.map((preset) => (
            <CtxItem
              key={preset.label}
              label={preset.label}
              onClick={() => run(() => createTargetingCue(targetIds, preset, onRefresh))}
            />
          ))}
        </CtxSubmenu>
      )}
      <CtxItem label={multiple ? `Renumber ${count} Cues…` : "Renumber…"} onClick={props.onRenumber} />
      {!multiple && (
        <CtxItem label="Set Playhead Here" onClick={() => run(() => movePlayheadToSelection(onRefresh))} />
      )}

      <GroupItems clicked={clicked} targetIds={targetIds} flatItems={flatItems} run={run} onRefresh={onRefresh} />

      {!multiple && !inGroup && assignType && (
        <>
          <CtxSeparator />
          <CtxItem
            label={`Assign ${ASSIGN_FILE_LABELS[assignType]} File…`}
            onClick={() => run(() => assignFile(assignType, clicked.cue.id, onRefresh))}
          />
        </>
      )}

      <CtxSeparator />
      <CtxItem label={multiple ? `Duplicate ${count} Cues` : "Duplicate"} onClick={() => run(() => duplicateSelection(onRefresh))} />
      <CtxItem
        label={multiple ? `Delete ${count} Cues` : "Delete"}
        danger
        onClick={() => run(() => deleteSelection(onRefresh, props.confirmBeforeDelete))}
      />
    </>
  );
}

function GroupItems({ clicked, targetIds, flatItems, run, onRefresh }: {
  clicked: BodyProps["clicked"];
  targetIds: string[];
  flatItems: Props["flatItems"];
  run: BodyProps["run"];
  onRefresh: () => void;
}) {
  const groupId = clicked.parentGroupId;
  const isGroup = clicked.cue.cue_type === "group";
  const count = targetIds.length;

  if (groupId) {
    // Every selected cue that shares this parent group leaves it together.
    const siblings = targetIds.filter((id) => flatItems.find((f) => f.cue.id === id)?.parentGroupId === groupId);
    return (
      <>
        <CtxSeparator />
        <CtxItem
          label={siblings.length > 1 ? `Remove ${siblings.length} Cues from Group` : "Remove from Group"}
          onClick={() => run(async () => {
            await Promise.all(siblings.map((id) => removeCueFromGroup(groupId, id).catch(console.error)));
            onRefresh();
          })}
        />
      </>
    );
  }
  return (
    <>
      <CtxSeparator />
      <CtxItem label={count > 1 ? `Group ${count} Cues` : "Group Cue"} onClick={() => run(() => groupSelection(onRefresh))} />
      {isGroup && count === 1 && (
        <CtxItem
          label="Ungroup"
          onClick={() => run(async () => {
            await ungroup(clicked.cue.id).catch(console.error);
            onRefresh();
          })}
        />
      )}
    </>
  );
}

function CueTypeItems({ onPick }: { onPick: (type: CueType) => void }) {
  return (
    <>
      {CUE_TYPES.map((ct) => (
        <CtxItem key={ct.type} color={ct.color} label={ct.label} onClick={() => onPick(ct.type)} />
      ))}
    </>
  );
}

async function addCueAt(type: CueType, position: number, onRefresh: () => void) {
  await addCue(type, position).catch(console.error);
  onRefresh();
}

async function assignFile(cueType: MediaCueType, cueId: string, onRefresh: () => void) {
  const filter = FILE_FILTERS[cueType];
  if (!filter) return;
  const result = await open({ multiple: false, filters: [filter] });
  if (typeof result !== "string") return;
  await setFileForCue(cueType, cueId, result).catch(console.error);
  onRefresh();
}

function mediaTypeOf(type: CueType): MediaCueType | null {
  return type === "audio" || type === "video" || type === "image" || type === "midi_file" ? type : null;
}
