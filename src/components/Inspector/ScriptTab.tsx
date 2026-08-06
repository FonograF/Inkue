// Script Cue main tab: the command to run and how long to let it run.
//
// Arguments are edited one per line rather than as a single string: no shell
// is involved, so a path containing spaces is one argument and quoting it
// would be wrong. One per line makes that unambiguous.

import type { ScriptCueData } from "../../lib/types";
import { Field, Section, inputStyle } from "./Field";
import { DragNumber } from "../common/DragNumber";

export function ScriptTab({
  cue,
  onSave,
}: {
  cue: ScriptCueData;
  onSave: (p: Partial<ScriptCueData>) => void;
}) {
  return (
    <Section title="Command">
      <Field label="Command">
        <input
          style={inputStyle}
          defaultValue={cue.command ?? ""}
          key={`cmd-${cue.id}`}
          placeholder="ffmpeg, python, C:\\tools\\go.bat…"
          onBlur={(e) => onSave({ command: e.target.value })}
        />
      </Field>

      <Field label="Arguments">
        <textarea
          style={{ ...inputStyle, minHeight: 64, resize: "vertical", fontFamily: "monospace" }}
          defaultValue={(cue.args ?? []).join("\n")}
          key={`args-${cue.id}`}
          placeholder={"one per line\n-i\nmy file.mov"}
          onBlur={(e) =>
            onSave({
              args: e.target.value.split("\n").map((l) => l.trim()).filter((l) => l !== ""),
            })
          }
        />
      </Field>

      <Field label="Working dir">
        <input
          style={inputStyle}
          defaultValue={cue.working_dir ?? ""}
          key={`wd-${cue.id}`}
          placeholder="(inherit Inkue's)"
          onBlur={(e) => onSave({ working_dir: e.target.value.trim() || null })}
        />
      </Field>

      <Field label="Timeout (s)">
        <DragNumber
          style={{ ...inputStyle, width: 90 }}
          step="1"
          min="0"
          value={((cue.timeout_ms ?? 0) / 1000).toString()}
          onChange={(e) =>
            onSave({ timeout_ms: Math.max(0, Math.round(parseFloat(e.target.value) * 1000) || 0) })
          }
        />
      </Field>

      <div style={{ fontSize: 11, color: "var(--wc-text-faint)", marginTop: 4 }}>
        The cue completes as soon as the process starts — a GO never waits on a
        script. 0 = no timeout; anything else kills a runaway process so it
        cannot outlive the show. Output goes to the log viewer.
      </div>
    </Section>
  );
}
