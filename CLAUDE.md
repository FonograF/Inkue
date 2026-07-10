# CLAUDE.md — Inkue

Show control app (QLab-inspired), **cross-platform Windows / macOS / Linux**. Tauri v2 + Rust backend + React/TS frontend.
**Read `PROGRESS.md` before any implementation work** — it is the ground truth for what is done and what is broken. `ARCHITECTURE.md` documents engine internals (output window, timer, audio pipeline); `PORTAGE.md` is the ground truth for the cross-platform architecture and its per-OS pitfalls.

## Stack

- Rust: audio (cpal — WASAPI/ASIO on Windows, CoreAudio on macOS, ALSA/PipeWire on Linux — + symphonia), video/image (libmpv via OpenGL Render API window; legacy Win32+D3D11 behind `legacy-win32-output` feature), show logic
- UI: Tauri v2, React, TypeScript, Zustand
- Build: `pnpm tauri dev` / `cargo test` / `cargo clippy` (from `src-tauri/`)
- Runtime dep: libmpv (~113 MB, not versioned) — `vendor/mpv/libmpv-2.dll` bundled on Windows; Homebrew `libmpv.dylib` (macOS) / system `libmpv.so` (Linux) in dev. Resolution detail in `PORTAGE.md`.

## Architecture rules — DO NOT VIOLATE

**Cross-platform**: Inkue runs on Windows, macOS and Linux. Every feature must compile and work on all three — design for this from the start, never bolt it on. No per-OS API without `#[cfg(target_os)]` and a working path for the other OSes. Known pitfalls (full detail in `PORTAGE.md`): touch WebView/windows only via `AppHandle::run_on_main_thread` (off-main-thread GTK calls crash on Linux); resolve config/data paths via per-OS dirs (`~/.config`, `~/Library/Application Support`, `%APPDATA%`) — never hardcode `%APPDATA%`; audio via generic `cpal` (no direct WASAPI-specific calls); no winit on macOS (its `EventLoop` needs the AppKit main run loop Tauri already owns).

**Cue extensibility**: every cue type implements `Cue` trait (`cue/traits.rs`). Adding a new cue type must **never** require touching `show/transport.rs`, `show/cue_list.rs`, or `components/CueList/`.

**Audio thread** (`engine/audio_engine.rs` cpal callback): zero allocations, zero locks, zero I/O. All comms via lock-free ring buffers (ringbuf).

**Three layers, never mix**:
1. `engine/` — AudioEngine, OutputEngine. Know nothing about cues or shows.
2. `cue/` — cue lifecycle, timing, serialization. Talks to engines, not transport.
3. `show/` — cue list, playhead, GO logic. Does not know engine internals.

**Frontend ↔ backend**: `invoke()` commands in → `emit()` events out. Never poll from the frontend.

## Coding standards

**Rust**: `thiserror` for errors, no `.unwrap()` without a safety comment, `///` on all public items, fix all clippy warnings, `Duration` in public API (ms only for JSON serialization).

**TypeScript/React**: functional components, Zustand stores, all commands via `lib/commands.ts`, all shared types in `lib/types.ts`, event listeners via `useTauriEvents` hook.

## QLab terminology (use these exact terms)

**Workspace** (not project) · **Cue List** · **Playhead** (next GO target) · **GO** (not play) · **Pre-Wait** / **Post-Wait** (post-wait starts at the same time as the action) · **Auto-Continue** / **Auto-Follow** · **Output Patch** (not bus/output) · **Cue Number** (string — "1", "1.5", "Intro" are all valid)

## Key behavioral rules

- Stop on audio cue = short fade-out (default 0.5 s). Hard Stop = immediate cut.
- Double-Escape = Hard Stop All.
- File paths in `.inkue` are always relative to the workspace file.
- Cue Number is a `String`, not an integer.
- **Cue numbers are stable** (QLab-style): reordering/add/remove does NOT renumber.
  Auto-renumber is an opt-in preference (`general.auto_renumber_on_reorder`);
  resequencing is otherwise an explicit action (Action → Renumber All Cues). Never
  reintroduce automatic renumbering as the default.
- Selection (inspector) is independent from Playhead (GO target).
- **Group modes** (`GroupMode`, `cue/types.rs`): Simultaneous, Sequential, Playlist
  (exclusive one-at-a-time + optional loop), StartRandom (one random child/GO,
  shuffle-bag). Simultaneous/Sequential must stay behaviour-identical when extending.
- **Visual cues are layers** (GL path, QLab model): every Video/Image/Camera cue
  gets its own mpv slot (`output_engine/slot.rs`, lazy pool cap 8) and is
  composited in layer order with per-cue opacity + blend mode (`LayerStyle`).
  `stop_on_next_visual` (default true) keeps the historic replace behaviour;
  unchecked, cues stack. Fades drive the target's **own layer opacity**
  (`set_voice_opacity`), never a global overlay; the master fade quad is only a
  blackout curtain (idle/panic). The `legacy-win32-output` path keeps
  single-context replace semantics — every output-engine change must compile on
  both paths (`cargo check --features legacy-win32-output`).

## Cross-cutting invariants (learned the hard way)

- **A cue can own more than one voice.** A Fade/Stop targeting a **Group** must
  collect voices via `Cue::all_voice_ids()` (recursive; Group flattens children) and
  look up targets with `cue_list.get_recursive()` — never assume "one cue = one voice
  at the top level". This is the trap that made fade-on-group silently do nothing.
- **Group children complete themselves via the group, not the event loop.** The
  top-level completion detector does not descend into groups; a Group reaps its own
  finished children in `tick()`, and the event loop reaps group children whose *voice*
  completed (`reap_voice_completed_children`). Leaf audio cues never self-complete.
- **State changes outside the normal completion flow must still notify the UI.** The
  frontend only updates a cue's state from `cue-state-changed`; any code that
  stops/resets cues out-of-band (e.g. Fade "Stop at End") must emit it (+ a
  `cue-list-refresh` for nested/green-highlight resync), or the UI freezes on RUNNING.

## Tests

Run `cargo test` from `src-tauri/` (current count in `PROGRESS.md`). Must cover: CueNumber parsing, CueRegistry, AudioCue serialization roundtrip, dB↔linear, FadeSpec curves, CueList operations (incl. stable-numbering: reorder preserves numbers, explicit `renumber_all`), audio SR conversion, Stop/Fade specs, **Fade pan + fade-targeting-a-group + Stop-at-End queueing**, **Group modes (Playlist exclusivity/loop, StartRandom shuffle-bag, completion/is_complete per mode)**, **layer compositor (blend formulas as executable spec of the GLSL, layer-key ordering, opacity-anim math, LayerStyle serde + legacy-JSON compat)**, OSC types/server/dedup, DMX engine/sink/fixtures/Light Cue. Dev server holds `inkue.exe`/`libmpv-2.dll` — close it before `cargo test` (or validate with `CARGO_TARGET_DIR=target-check`), and **never force-kill `cargo` mid-build** (corrupts the incremental cache → `LNK anon.*.llvm.*`).
