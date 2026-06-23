# CLAUDE.md — WinCue

Show control app (QLab-inspired), **cross-platform Windows / macOS / Linux**. Tauri v2 + Rust backend + React/TS frontend.
**Read `PROGRESS.md` before any implementation work** — it is the ground truth for what is done and what is broken. `ARCHITECTURE.md` documents engine internals (output window, timer, audio pipeline); `PORTAGE.md` is the ground truth for the cross-platform architecture and its per-OS pitfalls.

## Stack

- Rust: audio (cpal — WASAPI/ASIO on Windows, CoreAudio on macOS, ALSA/PipeWire on Linux — + symphonia), video/image (libmpv via OpenGL Render API window; legacy Win32+D3D11 behind `legacy-win32-output` feature), show logic
- UI: Tauri v2, React, TypeScript, Zustand
- Build: `pnpm tauri dev` / `cargo test` / `cargo clippy` (from `src-tauri/`)
- Runtime dep: libmpv (~113 MB, not versioned) — `vendor/mpv/libmpv-2.dll` bundled on Windows; Homebrew `libmpv.dylib` (macOS) / system `libmpv.so` (Linux) in dev. Resolution detail in `PORTAGE.md`.

## Architecture rules — DO NOT VIOLATE

**Cross-platform**: WinCue runs on Windows, macOS and Linux. Every feature must compile and work on all three — design for this from the start, never bolt it on. No per-OS API without `#[cfg(target_os)]` and a working path for the other OSes. Known pitfalls (full detail in `PORTAGE.md`): touch WebView/windows only via `AppHandle::run_on_main_thread` (off-main-thread GTK calls crash on Linux); resolve config/data paths via per-OS dirs (`~/.config`, `~/Library/Application Support`, `%APPDATA%`) — never hardcode `%APPDATA%`; audio via generic `cpal` (no direct WASAPI-specific calls); no winit on macOS (its `EventLoop` needs the AppKit main run loop Tauri already owns).

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
- File paths in `.wincue` are always relative to the workspace file.
- Cue Number is a `String`, not an integer.
- Selection (inspector) is independent from Playhead (GO target).

## Tests

Run `cargo test` from `src-tauri/` (current count in `PROGRESS.md`). Must cover: CueNumber parsing, CueRegistry, AudioCue serialization roundtrip, dB↔linear, FadeSpec curves, CueList operations, audio SR conversion, Stop/Fade specs, OSC types/server/dedup, DMX engine/sink/fixtures/Light Cue.
