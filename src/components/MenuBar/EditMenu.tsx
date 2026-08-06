// "Edit" menu — undo/redo and the operations that act on the current
// selection. Every entry already existed as a keyboard shortcut and nothing
// else: undo in particular was reachable only by knowing Ctrl+Z, with no way
// to tell whether there was anything to undo.

import { useState } from "react";

import { canRedo, canUndo } from "../../lib/commands";
import {
  copySelection,
  deleteSelection,
  duplicateSelection,
  groupSelection,
  pasteAfterSelection,
  redoAction,
  selectAllCues,
  selectionIsGroup,
  undoAction,
  ungroupSelection,
} from "../../lib/cueOperations";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import { MenuBarMenu, type MenuBarItem } from "./MenuBarMenu";

const isMac = typeof navigator !== "undefined" && /mac/i.test(navigator.platform);
const mod = isMac ? "Cmd" : "Ctrl";

export function EditMenu({ onRefresh }: { onRefresh: () => void }) {
  const selectedCueId = useWorkspaceStore((s) => s.selectedCueId);
  const selectionCount = useWorkspaceStore((s) => s.selectedCueIds.length);
  const confirmBeforeDelete = useWorkspaceStore((s) => s.generalPrefs.confirm_before_delete);

  // Undo depth lives in the Rust undo stack, so availability is asked for when
  // the menu opens rather than mirrored in the store.
  const [undoAvailable, setUndoAvailable] = useState(true);
  const [redoAvailable, setRedoAvailable] = useState(true);

  const refreshUndoState = () => {
    void canUndo().then(setUndoAvailable).catch(() => setUndoAvailable(true));
    void canRedo().then(setRedoAvailable).catch(() => setRedoAvailable(true));
  };

  const hasSelection = selectedCueId !== null;
  const deleteLabel = selectionCount > 1 ? `Delete ${selectionCount} Cues` : "Delete";

  const items: MenuBarItem[] = [
    { type: "item", label: "Undo", shortcut: `${mod}+Z`, disabled: !undoAvailable,
      onClick: () => void undoAction(onRefresh) },
    { type: "item", label: "Redo", shortcut: `${mod}+Y`, disabled: !redoAvailable,
      onClick: () => void redoAction(onRefresh) },
    { type: "separator" },
    { type: "item", label: "Copy", shortcut: `${mod}+C`, disabled: !hasSelection,
      onClick: () => void copySelection() },
    { type: "item", label: "Paste", shortcut: `${mod}+V`,
      onClick: () => void pasteAfterSelection(onRefresh) },
    { type: "item", label: "Duplicate", shortcut: `${mod}+D`, disabled: !hasSelection,
      onClick: () => void duplicateSelection(onRefresh) },
    { type: "item", label: deleteLabel, shortcut: "Del", disabled: !hasSelection,
      onClick: () => void deleteSelection(onRefresh, confirmBeforeDelete) },
    { type: "separator" },
    { type: "item", label: "Select All", shortcut: `${mod}+A`, onClick: selectAllCues },
    { type: "separator" },
    { type: "item", label: "Group Selected", shortcut: `${mod}+G`, disabled: !hasSelection,
      onClick: () => void groupSelection(onRefresh) },
    { type: "item", label: "Ungroup", disabled: !selectionIsGroup(),
      onClick: () => void ungroupSelection(onRefresh) },
  ];

  return <MenuBarMenu label="Edit" items={items} minWidth={230} onOpen={refreshUndoState} />;
}
