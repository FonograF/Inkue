# RENAME.md — Project rename runbook

The product has already been renamed once (**WinCue → Inkue**, 2026-06-29). The name
may change again, so this file is the ground-truth checklist to do it mechanically.
Everything below is parametric: replace **`OLD`** with the current name and **`NEW`**
with the target name.

Current values: `OLD = WinCue / wincue / WINCUE`, `NEW = Inkue / inkue / INKUE`.

---

## 0. Pre-flight

- **Close the running app first.** The Tauri build script copies `libmpv-2.dll` into
  `target/`; a running `*.exe` keeps the DLL open and the rebuild fails with
  `os error 32` ("file is used by another process").
- Work on a clean working tree (commit or stash unrelated changes).

## 1. Find every occurrence

```bash
git grep -lI -e 'wincue' -e 'WinCue' -e 'WINCUE' -e 'Wincue' -- . ':!src-tauri/gen/'
```

Exclude `src-tauri/gen/` — those files are **generated** (see step 4). At the last
rename this matched **65 tracked files / 310 occurrences**.

## 2. Bulk replace (case-sensitive, order matters)

Run all-caps first, then PascalCase, then lowercase, so the substitutions never
collide:

```bash
git grep -lI -e 'wincue' -e 'WinCue' -e 'WINCUE' -e 'Wincue' -- . ':!src-tauri/gen/' \
| while IFS= read -r f; do
  perl -i -pe 's/WINCUE/INKUE/g; s/WinCue/Inkue/g; s/Wincue/Inkue/g; s/wincue/inkue/g;' "$f"
done
# Verify nothing is left:
git grep -nI -i 'wincue' -- . ':!src-tauri/gen/'   # expect: no output
```

### Token-form map (and where each form lives semantically)

| Form    | Mapping | What it is |
|---------|---------|------------|
| `WinCue` | `Inkue` | **Display name** — window titles & `productName` (`tauri.conf.json`), `<title>` (`index.html`), About dialog, menu "About …", titlebar label (`App.tsx`), `releaseName` (`release.yml`), network source names, doc prose. |
| `wincue` | `inkue` | Crate / lib / bin name (`Cargo.toml` + `main.rs` `inkue_lib::run()`), bundle id `com.inkue.app`, **file extension** `.inkue`, OSC namespace `/inkue/...`, OSC thread name `inkue-osc-server`, per-OS **config dir** name, `recovery.inkue`, `package.json` `"name"`, localStorage keys `inkue_recent_files` / `inkue_ui_layout`, custom DOM event `inkue:cue-drag-start`. |
| `WINCUE` | `INKUE` | **Env vars** `INKUE_OUTPUT_BACKEND`, `INKUE_OUTPUT_FPS` (`render.rs`, `Cargo.toml` comment, docs). |

### Display-form capitalization decision

`WinCue` was styled Win+Cue (internal capital). The current name is **`Inkue`** —
single leading capital, read as one word. If you pick a name with an internal
capital again, the bulk `s/WinCue/.../` line covers only the exact PascalCase form;
double-check display strings by eye.

## 3. Manual fixups the bulk replace can't get right

1. **Backward-compatible Open dialog** — `src/App.tsx`, `handleOpen`: keep the old
   extension in the filter so existing shows still appear:
   ```ts
   filters: [{ name: "Inkue Workspace", extensions: ["inkue", "wincue"] }]
   ```
   The **Save** dialog stays new-extension-only (`["inkue"]`). The loader does not
   check the extension, so old files open fine once the filter lists them.

2. **sACN layout test** — `src-tauri/src/engine/dmx_sink.rs`. The sACN source name is
   a fixed 64-byte field; the test slices *exactly the name length*. A name-length
   change breaks the assert. `WinCue` (6 bytes) → `Inkue` (5 bytes) needed
   `&p[44..50]` → `&p[44..49]`. **Re-check this slice whenever the name length
   changes.**

3. **`.gitignore`** — the bulk replace rewrites the *ignored user-data filenames*
   too. Keep BOTH old and new so existing on-disk user files stay ignored:
   ```
   *.inkue
   *.wincue
   ...
   /Inkue.config
   /Wincue.config
   /inkue interface mobile.json
   /inkue-monitor.json
   /wincue interface mobile.json
   /wincue-monitor.json
   ```
   (Loose user files on disk keep their old names — we don't rename the user's data.)

4. **Renamed doc file** — `git mv docs/archive/wincue-prompt.md docs/archive/inkue-prompt.md`.

## 4. Generated files — do NOT hand-edit

- `src-tauri/gen/schemas/capabilities.json` and `acl-manifests.json` are regenerated
  from `src-tauri/capabilities/*.json` + `tauri.conf.json` on the next build. Edit the
  **source** `capabilities/*.json` (descriptions) and let a build refresh `gen/`.
- **Lockfiles** regenerate on build/install: `Cargo.lock` (gitignored here) on
  `cargo build`, `pnpm-lock.yaml` on `pnpm install`.

## 5. Verify

```bash
pnpm exec tsc --noEmit                                   # frontend typecheck
cargo test   --manifest-path src-tauri/Cargo.toml        # 143 tests at last rename
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
```
(Pre-existing `identity_op` warnings in `timecode_receiver.rs` are unrelated to a rename.)

## 6. Out of scope — separate manual steps (NOT done by the bulk replace)

- **Repo folder** `C:\Wincue` — renaming it mid-session breaks the working dir and
  `target/`. Do it with the app and editor closed.
- **GitHub repo** `FonograF/wincue` — rename in GitHub settings. CI uses
  `$GITHUB_REPOSITORY`, so `release.yml` adapts automatically; README URLs are updated
  by the bulk replace.
- **Orphaned config dir** — a "clean switch" leaves the old `%APPDATA%\WinCue\`
  (audio/OSC/timecode config, logs, recovery) behind; the app re-creates
  `%APPDATA%\Inkue\` empty. Migrate by hand only if you want to preserve local config.
- **CSS token prefix `--wc-*`** and localStorage `wc_theme` are NOT literal `wincue`
  and were intentionally left untouched.
- **Tauri updater signing key** filename in `release.yml` comments is documentation
  only.
