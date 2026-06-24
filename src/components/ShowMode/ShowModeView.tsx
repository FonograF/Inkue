// Read-only "Show Mode" cue list — bubble cards, no editing.

import { useEffect, useRef, useMemo } from "react";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import { useTimingStore } from "../../stores/timingStore";
import type { CueSummary } from "../../lib/types";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function msToMmss(ms: number): string {
  const s = Math.floor(ms / 1000);
  return `${String(Math.floor(s / 60)).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`;
}

function flattenAll(cues: CueSummary[]): CueSummary[] {
  const out: CueSummary[] = [];
  for (const c of cues) {
    if (c.cue_type === "group") {
      if (c.children?.length) out.push(...flattenAll(c.children));
    } else {
      out.push(c);
    }
  }
  return out;
}

// Returns the set of cue IDs that are effectively "armed" (next to fire).
// Handles sequential groups (armed = active_child_id) and simultaneous
// groups (armed = all children), mirroring CueListView's inner-playhead logic.
function computeArmedIds(cues: CueSummary[], playheadId: string | null): Set<string> {
  const result = new Set<string>();
  if (!playheadId) return result;

  function walk(list: CueSummary[]) {
    for (const cue of list) {
      const atPlayhead = cue.id === playheadId;
      const running    = cue.state === "running";
      if (cue.cue_type === "group") {
        if (cue.group_mode === "sequential") {
          if ((atPlayhead || running) && cue.active_child_id) result.add(cue.active_child_id);
        } else {
          if (atPlayhead || running) for (const child of cue.children ?? []) result.add(child.id);
        }
        if (cue.children?.length) walk(cue.children);
      } else if (atPlayhead) {
        result.add(cue.id);
      }
    }
  }
  walk(cues);
  return result;
}

// ---------------------------------------------------------------------------
// Card
// ---------------------------------------------------------------------------

interface CardProps {
  cue: CueSummary;
  isArmed: boolean;
  isCompleted: boolean;
}

function ShowCard({ cue, isArmed, isCompleted }: CardProps) {
  const timing    = useTimingStore((s) => s.timings[cue.id]);
  const isRunning = cue.state === "running";
  const isPaused  = cue.state === "paused";

  let statusLabel: string;
  let statusColor: string;
  let borderColor: string;
  let bgTint: string;
  let opacity = 1;

  if (isRunning) {
    const elapsed = timing?.action_elapsed_ms ?? 0;
    statusLabel = `Running ${msToMmss(elapsed)}`;
    statusColor = "#4ade80";
    borderColor = "#22c55e";
    bgTint      = "rgba(34, 197, 94, 0.07)";
  } else if (isPaused) {
    const elapsed = timing?.action_elapsed_ms ?? 0;
    statusLabel = `Paused ${msToMmss(elapsed)}`;
    statusColor = "#fbbf24";
    borderColor = "#f59e0b";
    bgTint      = "rgba(245, 158, 11, 0.06)";
  } else if (cue.is_loading) {
    statusLabel = "Loading…";
    statusColor = "#f59e0b";
    borderColor = "var(--wc-border)";
    bgTint      = "transparent";
  } else if (isArmed) {
    statusLabel = "Armed";
    statusColor = "#22d3ee";
    borderColor = "#06b6d4";
    bgTint      = "rgba(6, 182, 212, 0.06)";
  } else if (isCompleted) {
    statusLabel = "Completed";
    statusColor = "var(--wc-text-faint)";
    borderColor = "transparent";
    bgTint      = "transparent";
    opacity     = 0.45;
  } else {
    statusLabel = "Ready";
    statusColor = "var(--wc-text-muted)";
    borderColor = "var(--wc-border)";
    bgTint      = "transparent";
  }

  const progressPct =
    isRunning && timing && cue.duration_ms && cue.duration_ms > 0
      ? Math.min(100, (timing.action_elapsed_ms / cue.duration_ms) * 100)
      : null;

  return (
    <div
      data-cue-id={cue.id}
      data-armed={isArmed ? "true" : undefined}
      data-running={isRunning ? "true" : undefined}
      style={{
        position: "relative",
        border: `1px solid ${borderColor}`,
        borderRadius: 8,
        padding: "10px 16px",
        background: bgTint !== "transparent" ? bgTint : "var(--wc-bg-surface)",
        display: "flex",
        alignItems: "center",
        gap: 12,
        opacity,
        overflow: "hidden",
        flexShrink: 0,
      }}
    >
      {/* Progress bar */}
      {progressPct !== null && (
        <div
          style={{
            position: "absolute",
            bottom: 0, left: 0,
            height: 2,
            width: `${progressPct}%`,
            background: "#22c55e",
            borderRadius: "0 1px 0 8px",
            transition: "width 0.1s linear",
          }}
        />
      )}

      {/* Cue number */}
      <span
        style={{
          width: 48,
          flexShrink: 0,
          textAlign: "right",
          fontSize: 12,
          fontFamily: "monospace",
          color: "var(--wc-text-secondary)",
        }}
      >
        {cue.number ?? ""}
      </span>

      {/* Name */}
      <span
        style={{
          flex: 1,
          fontSize: 15,
          fontWeight: 600,
          color: "var(--wc-text-bright)",
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
        }}
      >
        {cue.name}
      </span>

      {/* Status */}
      <span
        style={{
          flexShrink: 0,
          fontSize: 12,
          color: statusColor,
          minWidth: 100,
          textAlign: "right",
        }}
      >
        {statusLabel}
      </span>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main view
// ---------------------------------------------------------------------------

export function ShowModeView() {
  const { cues, playheadCueId } = useWorkspaceStore();
  const scrollRef = useRef<HTMLDivElement>(null);

  const flatCues  = useMemo(() => flattenAll(cues), [cues]);
  const armedIds  = useMemo(() => computeArmedIds(cues, playheadCueId ?? null), [cues, playheadCueId]);

  const boundaryIndex = useMemo(() => {
    for (let i = 0; i < flatCues.length; i++) {
      if (armedIds.has(flatCues[i].id)) return i;
    }
    return -1;
  }, [flatCues, armedIds]);

  // Scroll armed cue into center when playhead changes.
  useEffect(() => {
    if (!scrollRef.current) return;
    const target = (
      scrollRef.current.querySelector("[data-armed='true']") ??
      scrollRef.current.querySelector("[data-running='true']")
    ) as HTMLElement | null;
    target?.scrollIntoView({ block: "center", behavior: "smooth" });
  }, [playheadCueId]);

  return (
    <div
      ref={scrollRef}
      style={{
        flex: 1,
        overflow: "auto",
        padding: "16px 24px",
        display: "flex",
        flexDirection: "column",
        gap: 6,
      }}
    >
      {flatCues.map((cue, index) => {
        const isRunning   = cue.state === "running";
        const isPaused    = cue.state === "paused";
        const isArmed     = armedIds.has(cue.id);
        const isCompleted = !isRunning && !isPaused && !isArmed && boundaryIndex >= 0 && index < boundaryIndex;

        return (
          <ShowCard
            key={cue.id}
            cue={cue}
            isArmed={isArmed}
            isCompleted={isCompleted}
          />
        );
      })}
    </div>
  );
}
