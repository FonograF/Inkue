// Group Cue main tab: playback mode and per-mode options.

import type { GroupMode } from "../../lib/types";
import { setGroupMode, setPlaylistLoop } from "../../lib/commands";
import { Section, ToggleRow, inputStyle } from "./Field";
import { Select } from "../common/Select";

const MODE_HINTS: Record<GroupMode, string> = {
  simultaneous: "GO starts every child at once (use child pre-waits for a timeline).",
  sequential: "GO starts the first child; each child triggers the next.",
  playlist: "One child plays at a time; GO advances to the next.",
  start_random: "GO starts one random child (shuffle-bag, no repeats until all played).",
};

export function GroupTab({
  cue,
  onRefresh,
}: {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  cue: any;
  onRefresh: () => void;
}) {
  const mode: GroupMode = cue.group_mode ?? "simultaneous";

  return (
    <Section title="Mode">
      <Select
        style={inputStyle}
        value={mode}
        onChange={async (e) => {
          await setGroupMode(cue.id, e.target.value as GroupMode).catch(console.error);
          onRefresh();
        }}
      >
        <option value="simultaneous">Simultaneous</option>
        <option value="sequential">Sequential</option>
        <option value="playlist">Playlist</option>
        <option value="start_random">Start Random</option>
      </Select>
      <div style={{ fontSize: 11, color: "var(--wc-text-faint)", margin: "6px 0 10px" }}>
        {MODE_HINTS[mode]}
      </div>
      {mode === "playlist" && (
        <ToggleRow
          label="Loop back to the first child"
          checked={cue.playlist_loop ?? false}
          onToggle={async (v) => {
            await setPlaylistLoop(cue.id, v).catch(console.error);
            onRefresh();
          }}
        />
      )}
    </Section>
  );
}
