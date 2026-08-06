// Edit operations on the cue list, shared by the keyboard shortcuts and the
// Edit / Action menus.
//
// They live here rather than inside `useKeyboardShortcuts` so a menu entry and
// its shortcut cannot drift apart: "Delete" asking for confirmation in one
// place but not the other is exactly the kind of divergence this prevents.
// Each reads the current selection from the store itself, so callers only pass
// what they cannot know (the refresh callback, the confirm-before-delete
// preference).

import { confirm } from "@tauri-apps/plugin-dialog";

import {
  clearCueNumbers,
  copyCue,
  duplicateCue,
  duplicateCues,
  groupCues,
  pasteCue,
  redo,
  removeCue,
  removeCues,
  renumberCues,
  renumberSelectedCues,
  setPlayhead,
  undo,
  ungroup,
} from "./commands";
import { useWorkspaceStore } from "../stores/workspaceStore";

type Refresh = () => void;

export async function undoAction(onRefresh: Refresh) {
  await undo().catch(console.error);
  onRefresh();
}

export async function redoAction(onRefresh: Refresh) {
  await redo().catch(console.error);
  onRefresh();
}

export async function copySelection() {
  const { selectedCueId } = useWorkspaceStore.getState();
  if (!selectedCueId) return;
  await copyCue(selectedCueId).catch(console.error);
}

export async function pasteAfterSelection(onRefresh: Refresh) {
  const { selectedCueId } = useWorkspaceStore.getState();
  await pasteCue(selectedCueId).catch(console.error);
  onRefresh();
}

export async function duplicateSelection(onRefresh: Refresh) {
  const { selectedCueId, selectedCueIds } = useWorkspaceStore.getState();
  if (!selectedCueId) return;
  if (selectedCueIds.length > 1) {
    await duplicateCues(selectedCueIds).catch(console.error);
  } else {
    await duplicateCue(selectedCueId).catch(console.error);
  }
  onRefresh();
}

/** Returns false when the operator cancelled the confirmation dialog. */
export async function deleteSelection(onRefresh: Refresh, confirmFirst: boolean) {
  const { selectedCueId, selectedCueIds, setSelectedCueId, setSelectedCueIds } =
    useWorkspaceStore.getState();
  if (!selectedCueId) return false;

  const multiple = selectedCueIds.length > 1;
  if (confirmFirst) {
    const message = multiple ? `Delete ${selectedCueIds.length} cues?` : "Delete this cue?";
    const ok = await confirm(message, { title: "Confirm Delete", kind: "warning" });
    if (!ok) return false;
  }
  if (multiple) {
    await removeCues(selectedCueIds).catch(console.error);
  } else {
    await removeCue(selectedCueId).catch(console.error);
  }
  setSelectedCueId(null);
  setSelectedCueIds([]);
  onRefresh();
  return true;
}

export function selectAllCues() {
  const { cues, setSelectedCueIds } = useWorkspaceStore.getState();
  setSelectedCueIds(cues.map((c) => c.id));
}

export async function groupSelection(onRefresh: Refresh) {
  const { selectedCueIds, setSelectedCueId, setSelectedCueIds } = useWorkspaceStore.getState();
  if (selectedCueIds.length === 0) return;
  const newGroupId = await groupCues(selectedCueIds).catch(() => null);
  if (!newGroupId) return;
  // Selecting the fresh group leaves the operator on what they just made,
  // rather than on cues that are now one level down.
  setSelectedCueId(newGroupId);
  setSelectedCueIds([newGroupId]);
  onRefresh();
}

export async function ungroupSelection(onRefresh: Refresh) {
  const { selectedCueId, setSelectedCueId, setSelectedCueIds } = useWorkspaceStore.getState();
  if (!selectedCueId) return;
  await ungroup(selectedCueId).catch(console.error);
  setSelectedCueId(null);
  setSelectedCueIds([]);
  onRefresh();
}

/** `true` when the current selection is a single Group cue. */
export function selectionIsGroup(): boolean {
  const { cues, selectedCueId, selectedCueIds } = useWorkspaceStore.getState();
  if (!selectedCueId || selectedCueIds.length > 1) return false;
  return cues.find((c) => c.id === selectedCueId)?.cue_type === "group";
}

/**
 * Point the Playhead at the selected cue. Selection and Playhead are
 * independent by design, so re-aligning them is an explicit action.
 */
export async function movePlayheadToSelection(onRefresh: Refresh) {
  const { selectedCueId, setPlayheadCueId } = useWorkspaceStore.getState();
  if (!selectedCueId) return;
  await setPlayhead(selectedCueId).catch(console.error);
  setPlayheadCueId(selectedCueId);
  onRefresh();
}

export async function renumberAll(onRefresh: Refresh) {
  await renumberCues().catch(console.error);
  onRefresh();
}

export async function renumberSelection(start: number, increment: number, onRefresh: Refresh) {
  const { selectedCueIds } = useWorkspaceStore.getState();
  if (selectedCueIds.length === 0) return;
  await renumberSelectedCues(selectedCueIds, start, increment).catch(console.error);
  onRefresh();
}

export async function clearAllCueNumbers(onRefresh: Refresh) {
  await clearCueNumbers().catch(console.error);
  onRefresh();
}
