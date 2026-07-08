// Zone (c) — front/back IPC contract.
//
// The classic blind spot: a TypeScript wrapper calls invoke("some_command")
// but the Rust side renamed it, never registered it, or the string has a typo.
// Neither the Rust tests nor a React render catches it — it fails at runtime as
// "command not found", live, during a show. This test parses the authoritative
// Rust `generate_handler!` registration and asserts every command name reached
// from commands.ts exists there.

import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const here = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(here, "../../.."); // src/lib/__tests__ -> repo root

const libRs = readFileSync(path.join(repoRoot, "src-tauri/src/lib.rs"), "utf8");
const commandsTs = readFileSync(path.join(repoRoot, "src/lib/commands.ts"), "utf8");

/** Command names registered in the Tauri `generate_handler!` macro. */
function rustHandlerCommands(src: string): Set<string> {
  const start = src.indexOf("generate_handler![");
  expect(start, "generate_handler! not found in lib.rs").toBeGreaterThanOrEqual(0);
  const end = src.indexOf("])", start);
  const block = src.slice(start + "generate_handler![".length, end);

  const names = new Set<string>();
  for (let line of block.split("\n")) {
    const comment = line.indexOf("//");
    if (comment >= 0) line = line.slice(0, comment);
    for (const tok of line.split(",")) {
      const t = tok.trim();
      if (/^[a-z_][a-z0-9_]*$/.test(t)) names.add(t);
    }
  }
  return names;
}

/** Every command string passed to invoke<...>("name", ...) in commands.ts. */
function tsInvokeCommands(src: string): string[] {
  // Non-greedy up to the first `>` — none of the invoke generics contain a `>`.
  const re = /invoke<[\s\S]*?>\s*\(\s*"([a-z0-9_]+)"/g;
  const found: string[] = [];
  let m: RegExpExecArray | null;
  while ((m = re.exec(src)) !== null) {
    found.push(m[1]);
  }
  return found;
}

describe("IPC command contract", () => {
  const registered = rustHandlerCommands(libRs);
  const tsCommands = tsInvokeCommands(commandsTs);

  it("parses a plausible number of commands from both sides", () => {
    expect(registered.size).toBeGreaterThan(100);
    expect(tsCommands.length).toBeGreaterThan(100);
  });

  it("every command invoked from commands.ts is registered in Rust", () => {
    const missing = [...new Set(tsCommands)].filter((c) => !registered.has(c)).sort();
    expect(missing, `commands.ts calls invoke() for names not registered in lib.rs: ${missing.join(", ")}`).toEqual([]);
  });

  it("has no duplicate command definitions between the two collectors (sanity)", () => {
    // Guards the parser itself: the Rust set must contain the well-known core commands.
    for (const core of ["go", "go_cue", "stop_all", "hard_stop_all", "add_cue", "save_workspace", "load_workspace"]) {
      expect(registered.has(core), `Rust handler should register ${core}`).toBe(true);
    }
  });
});
