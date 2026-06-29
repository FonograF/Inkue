// Column definitions and config helpers for the Cue List table.

export type ColumnId =
  | "playhead"
  | "led"
  | "number"
  | "name"
  | "notes"
  | "target"
  | "type"
  | "pre_wait"
  | "duration"
  | "post_wait"
  | "continue";

export interface ColumnDef {
  id: ColumnId;
  label: string;
  /** Default pixel width. */
  defaultWidth: number;
  /** Minimum drag width in px. */
  minWidth: number;
  /** Cannot be hidden or reordered. */
  fixed: boolean;
  /** Shows a resize drag handle on the right edge. */
  resizable: boolean;
  /** Sticks to the right edge of the scroll container — always visible. */
  stickyRight?: boolean;
}

export const DEFAULT_COLUMNS: ColumnDef[] = [
  { id: "playhead",  label: "▶",      defaultWidth: 28,  minWidth: 24, fixed: true,  resizable: false },
  { id: "led",       label: "",        defaultWidth: 20,  minWidth: 16, fixed: true,  resizable: false },
  { id: "number",    label: "#",       defaultWidth: 60,  minWidth: 36, fixed: false, resizable: true  },
  { id: "name",      label: "Name",    defaultWidth: 200, minWidth: 80, fixed: true,  resizable: true  },
  { id: "notes",     label: "Notes",   defaultWidth: 220, minWidth: 60, fixed: false, resizable: true  },
  { id: "target",    label: "Target",  defaultWidth: 180, minWidth: 80, fixed: false, resizable: true  },
  { id: "type",      label: "T",       defaultWidth: 32,  minWidth: 28, fixed: false, resizable: true  },
  { id: "pre_wait",  label: "Pre-W",   defaultWidth: 64,  minWidth: 48, fixed: false, resizable: true  },
  { id: "duration",  label: "Dur",     defaultWidth: 64,  minWidth: 48, fixed: false, resizable: true  },
  { id: "post_wait", label: "Post-W",  defaultWidth: 64,  minWidth: 48, fixed: false, resizable: true  },
  { id: "continue",  label: "C",       defaultWidth: 36,  minWidth: 28, fixed: false, resizable: true  },
];

const DEFAULT_ORDER: ColumnId[] = DEFAULT_COLUMNS.map((d) => d.id);

// ---------------------------------------------------------------------------
// Persisted config shape
// ---------------------------------------------------------------------------

export interface ColumnConfig {
  /** Per-column pixel width overrides (absent = use defaultWidth). */
  widths: Partial<Record<ColumnId, number>>;
  /** Columns explicitly hidden by the user. */
  hidden: Partial<Record<ColumnId, boolean>>;
  /** User-defined display order (all column IDs, including hidden). */
  order: ColumnId[];
}

export const DEFAULT_COLUMN_CONFIG: ColumnConfig = {
  widths: {},
  hidden: {},
  order: DEFAULT_ORDER,
};

const LS_KEY = "wincue_column_config_v2";

export function loadColumnConfig(): ColumnConfig {
  try {
    const raw = localStorage.getItem(LS_KEY);
    if (!raw) return DEFAULT_COLUMN_CONFIG;
    const parsed = JSON.parse(raw) as Partial<ColumnConfig>;
    // Keep only IDs that still exist; append any new columns at the end.
    const savedOrder = ((parsed.order ?? []) as string[]).filter((id) =>
      DEFAULT_ORDER.includes(id as ColumnId),
    ) as ColumnId[];
    const missing = DEFAULT_ORDER.filter((id) => !savedOrder.includes(id));
    const order: ColumnId[] = [...savedOrder, ...missing];

    // Ensure "led" always sits right after "playhead" (migration for older configs).
    const phPos = order.indexOf("playhead");
    const ldPos = order.indexOf("led");
    if (phPos >= 0 && ldPos >= 0 && ldPos !== phPos + 1) {
      order.splice(ldPos, 1);
      order.splice(phPos + 1, 0, "led");
    }

    return {
      widths: parsed.widths ?? {},
      hidden: parsed.hidden ?? {},
      order,
    };
  } catch {
    return DEFAULT_COLUMN_CONFIG;
  }
}

export function saveColumnConfig(c: ColumnConfig): void {
  try {
    localStorage.setItem(LS_KEY, JSON.stringify(c));
  } catch {
    // ignore (private / storage-full)
  }
}

// ---------------------------------------------------------------------------
// Derived helpers
// ---------------------------------------------------------------------------

/** All column defs in user-defined order (including hidden). */
export function getOrderedDefs(config: ColumnConfig): ColumnDef[] {
  return config.order
    .map((id) => DEFAULT_COLUMNS.find((d) => d.id === id))
    .filter((d): d is ColumnDef => d != null);
}

/** Visible column defs in user-defined order. */
export function getVisibleDefs(config: ColumnConfig): ColumnDef[] {
  return getOrderedDefs(config).filter((d) => !config.hidden[d.id]);
}

/** CSS grid-template-columns string — all pixel values, no fr units. */
export function buildGridCols(visibleDefs: ColumnDef[], config: ColumnConfig): string {
  return visibleDefs
    .map((d) => {
      const px = config.widths[d.id] ?? d.defaultWidth;
      return `${px}px`;
    })
    .join(" ");
}
