import { useEffect, useState } from "react";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import { restoreAudioDevice } from "../../lib/commands";
import type { HealthAlert, HealthLevel } from "../../lib/types";

const LEVEL_STYLE: Record<HealthLevel, { bg: string; border: string; fg: string; icon: string }> = {
  error:   { bg: "rgba(239,68,68,0.16)",  border: "rgba(239,68,68,0.55)",  fg: "#fca5a5", icon: "●" },
  warning: { bg: "rgba(251,191,36,0.16)", border: "rgba(251,191,36,0.55)", fg: "#fcd34d", icon: "⚠" },
  info:    { bg: "rgba(56,189,248,0.16)", border: "rgba(56,189,248,0.55)", fg: "#7dd3fc", icon: "ℹ" },
};

/** Maps a backend action id to the command that resolves it. */
const ACTIONS: Record<string, () => Promise<void>> = {
  restore_audio_device: () => restoreAudioDevice(),
};

/** Stable signature of an alert's *content* — a dismiss only hides this exact
 *  message, so a changed/recurring alert (e.g. "device is back") shows again. */
const sig = (a: HealthAlert) => `${a.key}|${a.message}`;

/** Non-blocking banner stack for runtime device/network faults. */
export function HealthBanner() {
  const alerts = useWorkspaceStore((s) => s.healthAlerts);
  const refreshHealth = useWorkspaceStore((s) => s.refreshHealth);
  const [dismissed, setDismissed] = useState<Set<string>>(new Set());

  // Prune dismissals whose alert is gone, so an identical fault later re-shows.
  useEffect(() => {
    const live = new Set(alerts.map(sig));
    setDismissed((prev) => {
      const next = new Set([...prev].filter((s) => live.has(s)));
      return next.size === prev.size ? prev : next;
    });
  }, [alerts]);

  const visible = alerts.filter((a) => !dismissed.has(sig(a)));
  if (visible.length === 0) return null;

  const runAction = async (alert: HealthAlert) => {
    const fn = alert.action ? ACTIONS[alert.action] : undefined;
    if (!fn) return;
    try {
      await fn();
      await refreshHealth();
    } catch (e) {
      console.error("health action failed", e);
    }
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", flexShrink: 0 }}>
      {visible.map((a) => {
        const s = LEVEL_STYLE[a.level];
        return (
          <div
            key={a.key}
            style={{
              display: "flex", alignItems: "center", gap: 10,
              padding: "5px 8px 5px 12px", fontSize: 12,
              background: s.bg, borderBottom: `1px solid ${s.border}`, color: s.fg,
            }}
          >
            <span style={{ flexShrink: 0, fontSize: 10 }}>{s.icon}</span>
            <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
              {a.message}
            </span>
            {a.action && a.action_label && (
              <button
                onClick={() => void runAction(a)}
                style={{
                  flexShrink: 0,
                  background: "rgba(255,255,255,0.10)", border: `1px solid ${s.border}`,
                  borderRadius: 5, color: s.fg, cursor: "pointer",
                  fontSize: 11, padding: "2px 10px",
                }}
              >
                {a.action_label}
              </button>
            )}
            <button
              onClick={() => setDismissed((prev) => new Set(prev).add(sig(a)))}
              title="Dismiss"
              style={{
                flexShrink: 0, width: 20, height: 20, lineHeight: "18px",
                background: "transparent", border: "none", color: s.fg,
                cursor: "pointer", fontSize: 15, opacity: 0.7, padding: 0,
              }}
            >
              ×
            </button>
          </div>
        );
      })}
    </div>
  );
}
