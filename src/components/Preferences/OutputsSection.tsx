// Preferences → Outputs: the extra video outputs of the show (a façade
// projector, a return monitor…).  The main output stays in the Display tab; a
// Video / Image / Camera / Text cue picks its output in the Inspector.
//
// Everything applies live and is saved in the workspace.

import { useCallback, useEffect, useState } from "react";
import { confirm } from "@tauri-apps/plugin-dialog";
import type { ScreenInfo, VideoOutputInfo } from "../../lib/types";
import { MAIN_OUTPUT_ID } from "../../lib/types";
import {
  addVideoOutput,
  deleteVideoOutput,
  identifyOutputScreen,
  listVideoOutputs,
  listVideoScreens,
  renameVideoOutput,
  setVideoOutputScreen,
} from "../../lib/commands";
import { Select } from "../common/Select";
import { ProjectorToolsSection } from "./ProjectorToolsSection";

const labelStyle: React.CSSProperties = {
  fontSize: 10, fontWeight: 600, color: "var(--wc-text-muted)",
  textTransform: "uppercase", letterSpacing: "0.07em",
  marginBottom: 10, paddingBottom: 5,
  borderBottom: "1px solid var(--wc-border)",
};

const hintStyle: React.CSSProperties = {
  fontSize: 11, color: "var(--wc-text-faint)", marginBottom: 12, lineHeight: 1.5,
};

const inputStyle: React.CSSProperties = {
  background: "var(--wc-bg-app)",
  border: "1px solid var(--wc-border-strong)",
  borderRadius: 4,
  color: "var(--wc-text)",
  fontSize: 12,
  padding: "4px 8px",
  boxSizing: "border-box",
};

const buttonStyle: React.CSSProperties = {
  padding: "4px 12px",
  fontSize: 12,
  borderRadius: 4,
  border: "1px solid var(--wc-border-strong)",
  background: "var(--wc-bg-surface)",
  color: "var(--wc-text)",
  cursor: "pointer",
  whiteSpace: "nowrap",
};

/** The screen choices offered for an output. */
function screenOptions(screens: ScreenInfo[]) {
  return [
    <option key="floating" value="floating">Floating window</option>,
    ...screens.map((s) => (
      <option key={s.index} value={s.index}>
        {s.is_primary
          ? `Screen ${s.index + 1} (primary, ${s.width}×${s.height})`
          : `Screen ${s.index + 1} (${s.width}×${s.height})`}
      </option>
    )),
  ];
}

function OutputRow({
  output,
  screens,
  selected,
  onSelect,
  onChanged,
  onError,
}: {
  output: VideoOutputInfo;
  screens: ScreenInfo[];
  selected: boolean;
  onSelect: () => void;
  onChanged: () => void;
  onError: (message: string | null) => void;
}) {
  const [name, setName] = useState(output.name);
  useEffect(() => setName(output.name), [output.name]);

  const run = (action: Promise<unknown>) => {
    onError(null);
    action.then(onChanged).catch((e) => onError(String(e)));
  };

  const commitName = () => {
    const trimmed = name.trim();
    if (trimmed === output.name) return;
    if (trimmed === "") {
      setName(output.name);
      return;
    }
    run(renameVideoOutput(output.id, trimmed));
  };

  return (
    <div
      onClick={onSelect}
      style={{
        display: "flex", alignItems: "center", gap: 8, padding: "6px 8px", marginBottom: 6,
        borderRadius: 6, cursor: "pointer",
        border: selected ? "1px solid var(--wc-accent)" : "1px solid var(--wc-border)",
        background: selected ? "var(--wc-bg-hover)" : "transparent",
      }}
    >
      <input
        value={name}
        onChange={(e) => setName(e.target.value)}
        onBlur={commitName}
        onKeyDown={(e) => { if (e.key === "Enter") e.currentTarget.blur(); }}
        onClick={(e) => e.stopPropagation()}
        style={{ ...inputStyle, width: 150 }}
        aria-label="Output name"
      />
      <div style={{ flex: 1, minWidth: 0 }} onClick={(e) => e.stopPropagation()}>
        <Select
          style={inputStyle}
          value={output.screen ?? "floating"}
          onChange={(e) => {
            const v = e.target.value;
            run(setVideoOutputScreen(output.id, v === "floating" ? null : parseInt(v, 10)));
          }}
        >
          {screenOptions(screens)}
        </Select>
      </div>
      <button
        title="Flash this output's name on its screen"
        style={buttonStyle}
        onClick={(e) => {
          e.stopPropagation();
          onError(null);
          identifyOutputScreen(output.screen, output.id).catch((err) => onError(String(err)));
        }}
      >
        Identify
      </button>
      <button
        title="Delete this output — cues that point at it will report it missing"
        style={{ ...buttonStyle, color: "#ef4444" }}
        onClick={(e) => {
          e.stopPropagation();
          void confirm(
            `Delete the output "${output.name}"? Its window closes and anything playing on it stops. ` +
              "Cues set to play on it will need another output.",
            { title: "Delete Output", kind: "warning" },
          ).then((ok) => {
            if (ok) run(deleteVideoOutput(output.id));
          });
        }}
      >
        Delete
      </button>
    </div>
  );
}

export function OutputsSection() {
  const [outputs, setOutputs] = useState<VideoOutputInfo[]>([]);
  const [screens, setScreens] = useState<ScreenInfo[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [newName, setNewName] = useState("");
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(() => {
    listVideoOutputs()
      .then((all) => setOutputs(all.filter((o) => o.id !== MAIN_OUTPUT_ID)))
      .catch((e) => setError(String(e)));
  }, []);

  useEffect(() => {
    reload();
    listVideoScreens().then(setScreens).catch(console.error);
  }, [reload]);

  // Keep a valid selection: the first output when none is chosen or the chosen one went away.
  useEffect(() => {
    if (outputs.length === 0) {
      setSelectedId(null);
    } else if (!outputs.some((o) => o.id === selectedId)) {
      setSelectedId(outputs[0].id);
    }
  }, [outputs, selectedId]);

  const add = () => {
    const name = newName.trim();
    if (name === "") return;
    setError(null);
    addVideoOutput(name)
      .then((created) => {
        setNewName("");
        setSelectedId(created.id);
        reload();
      })
      .catch((e) => setError(String(e)));
  };

  const selected = outputs.find((o) => o.id === selectedId) ?? null;

  return (
    <>
      <div style={{ marginBottom: 24 }}>
        <div style={labelStyle}>Extra Video Outputs</div>
        <div style={hintStyle}>
          The main output (Display tab) plays every cue that has no output of its own. Add an
          output for each additional projector or monitor, assign it a screen, then pick it
          in the cue's Inspector (Compositing). An output on a missing screen never plays on
          another display — its cues report it instead. A floating output opens as a window,
          handy to rehearse with a single monitor.
        </div>

        {outputs.length === 0 && (
          <div style={{ ...hintStyle, fontStyle: "italic" }}>No extra output yet.</div>
        )}
        {outputs.map((o) => (
          <OutputRow
            key={o.id}
            output={o}
            screens={screens}
            selected={o.id === selectedId}
            onSelect={() => setSelectedId(o.id)}
            onChanged={reload}
            onError={setError}
          />
        ))}

        <div style={{ display: "flex", gap: 8, marginTop: 10 }}>
          <input
            placeholder="New output name (e.g. Façade)"
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") add(); }}
            style={{ ...inputStyle, flex: 1 }}
          />
          <button style={buttonStyle} disabled={newName.trim() === ""} onClick={add}>
            Add output
          </button>
        </div>
        {error && (
          <div style={{ marginTop: 8, fontSize: 12, color: "#ef4444" }}>{error}</div>
        )}
      </div>

      {selected && (
        <>
          <div style={{ ...labelStyle, color: "var(--wc-text)" }}>
            Alignment &amp; test patterns — {selected.name}
          </div>
          <ProjectorToolsSection key={selected.id} outputId={selected.id} />
        </>
      )}
    </>
  );
}
