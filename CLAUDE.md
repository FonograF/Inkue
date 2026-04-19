# CLAUDE.md — WinCue Project Instructions

## What is this project?

WinCue is a show control application for Windows, inspired by QLab (macOS). It manages cue lists for live events (theatre, concerts, corporate). **Current version: 0.3.0** — Audio, Video, Image, Stop, and Memo cue types exist in the codebase. Audio, Stop, and Memo are fully functional; Video has a known glitch; Image is currently broken. The architecture must support any future cue type (MIDI, OSC, Fade, Group, Wait, Network, Script) without modifying existing code.

The full specification is in `wincue-prompt.md` at the project root.
**Before starting any implementation work, read `PROGRESS.md`** — it reflects the current state of the codebase, known bugs, and next priorities. `wincue-prompt.md` is useful for spec details (trait methods, event names, save format), but PROGRESS.md is the ground truth for what is done.

## Stack

- Backend: Rust (engine, audio, show logic)
- Audio: cpal (WASAPI/ASIO) + symphonia (decoding WAV/MP3/FLAC/OGG)
- Video: libmpv (FFI) + Win32 window, `gpu-api=d3d11`, audio via `ao=pcm` through a named pipe into AudioEngine
- Image: (under debug — see PROGRESS.md)
- Lock-free comms: ringbuf or crossbeam
- UI: Tauri v2 + React + TypeScript
- State: Zustand
- Build: cargo + pnpm

## Build & run commands

```
pnpm install                               # install frontend deps (first time)
pnpm tauri dev                             # run in dev mode (compiles Rust + starts frontend)
pnpm tauri:dev -- --features asio-support  # dev mode with ASIO support (needs vendor/asiosdk/)
pnpm tauri build                           # production build
cargo build                                # compile Rust only (from src-tauri/)
cargo test                                 # run Rust tests (from src-tauri/)
cargo clippy                               # lint Rust (from src-tauri/)
```

Runtime dependency: `vendor/mpv/libmpv-2.dll` must be present for video playback (not versioned, ~113 MB).

## Architecture rules — DO NOT VIOLATE

### Cue system extensibility

The entire app revolves around the `Cue` trait in `src-tauri/src/cue/traits.rs`. Every cue type (Audio, Video, Image, Memo, Stop, Wait, Group, MIDI...) implements this trait. The `CueRegistry` in `registry.rs` maps `CueType` to `CueFactory` instances.

**Adding a new cue type must NEVER require modifying:**
- The transport logic (`show/transport.rs`)
- The cue list (`show/cue_list.rs`)
- The main cue list UI (`components/CueList/`)
- Any existing cue implementation

If you find yourself editing these files to support a new cue type, the architecture is wrong. Fix it.

### Audio thread — real-time safety

The cpal audio callback in `engine/audio_engine.rs` runs in a high-priority thread. Inside this callback:
- **ZERO allocations** (no Vec::push, no String, no Box::new, no format!)
- **ZERO locks** (no Mutex, no RwLock, no channel recv that blocks)
- **ZERO I/O** (no file reads, no logging, no println!)
- All communication uses lock-free ring buffers (ringbuf crate)
- Commands TO the audio thread: ring buffer of command enums
- Status FROM the audio thread: ring buffer of status structs
- Video PCM samples are mixed into the same callback via a lock-free consumer (see `AudioEngine.set_video_pcm_consumer`)

### Separation of concerns

Three distinct layers, never mix them:
1. **Engines** (`engine/`): `AudioEngine` knows about samples, devices, voices, mixing. `VideoEngine` knows about libmpv, Win32 windows, screens. Engines do NOT know about cues or shows.
2. **Cue System** (`cue/`): knows about cue lifecycle, timing, serialization. AudioCue/VideoCue/ImageCue talk to their engines, but the `Cue` trait itself is engine-agnostic.
3. **Show/Transport** (`show/`): knows about cue lists, playhead, GO logic, continue modes. Does NOT know engine internals.

### Frontend-backend communication

- Frontend → Backend: Tauri `invoke()` commands only (defined in `commands/`)
- Backend → Frontend: Tauri `emit()` events only
- Time-sensitive events (cue-time-update, level-meter) are throttled to 30fps on the backend side
- Never poll from the frontend; always react to events

## Coding standards

### Rust

- Error types: use `thiserror` for defining error enums, `anyhow` only in main.rs
- No `.unwrap()` in production code unless the safety is documented with a comment explaining why
- All public items (modules, structs, traits, functions) must have `///` doc comments in English
- Use `clippy` with default lints. Fix all warnings before committing.
- Naming: snake_case for functions/variables, PascalCase for types/traits, SCREAMING_SNAKE for constants
- Prefer `impl Into<String>` over `String` in function parameters for ergonomics
- All durations in the public API use `std::time::Duration`. Milliseconds are only used for serialization (JSON).

### TypeScript / React

- Functional components only, no class components
- Zustand for all shared state (three stores: `workspaceStore`, `transportStore`, `timingStore`)
- All Tauri command calls go through typed wrappers in `lib/commands.ts`
- All types shared with the backend are defined in `lib/types.ts`
- Event listeners are set up via the `useTauriEvents` hook, never inline
- Keyboard shortcuts are handled by `useKeyboardShortcuts` hook at the App level
- Use CSS modules or Tailwind (no inline styles except for dynamic values like waveform rendering)

## QLab terminology — use these exact terms

- **Workspace**: the project file (.wincue), not "project" or "session"
- **Cue List**: ordered sequence of cues, not "playlist" or "sequence"
- **Playhead**: the marker indicating the next cue to be triggered by GO, not "cursor" or "pointer"
- **GO**: trigger the cue at the playhead, not "play" or "start"
- **Pre-Wait**: delay before the cue action starts
- **Post-Wait**: delay after the action starts (NOT after it ends) before continue mode kicks in
- **Auto-Continue**: after post-wait, automatically GO the next cue
- **Auto-Follow**: when this cue starts (after pre-wait), automatically GO the next cue
- **Output Patch**: named mapping to an audio device + channels, not "output" or "bus"
- **Cue Number**: a string label (not a numeric index), used for human reference

## Key behavioral details (match QLab)

- Post-Wait starts at the SAME TIME as the Action, not after it
- Cue Number is a STRING, not a number. "1", "1.5", "A", "Intro" are all valid
- Selection (highlighted cue for editing) is INDEPENDENT from Playhead (next cue for GO)
- Stop on a running audio cue applies a short fade out (default 0.5s). Hard Stop cuts immediately.
- Double-Escape = Hard Stop All
- File paths in .wincue are ALWAYS relative to the workspace file location

## Test requirements

At minimum, these must have unit tests:
- CueNumber parsing and comparison
- CueRegistry: register a factory, create a cue from it, lookup unknown type returns error
- AudioCue serialization roundtrip (serialize → deserialize → compare)
- dB to linear gain conversion and back
- FadeSpec curve calculations (linear, s-curve, exponential) at 0%, 50%, 100%
- CueList: add, remove, reorder, playhead advancement

Run tests with `cargo test` from the `src-tauri/` directory.

## Current state & open work

Core development is complete (scaffold, cue system, audio engine, video engine, frontend, transport, inspector, workspace, shortcuts, fades, drag-drop, undo/redo, color tags). The project compiles with zero warnings and 20 tests pass.

### Cue type status

| Cue type | Status |
|---|---|
| Audio | ✅ 100% functional (including Output Patch routing, fades, loops) |
| Stop   | ✅ Functional |
| Memo   | ✅ Functional |
| Video  | ⚠️ Plays correctly, but freezes ~0.5s at launch (first-frame/startup glitch) |
| Image  | 🔴 Broken — freezes the app on GO (critical bug) |

### Known bugs / next priorities

1. **🔴 CRITICAL — Image Cue freezes the app on launch** (`cue/image_cue.rs`, likely an image engine or display path)
   - The app becomes unresponsive the moment an Image cue is triggered
   - Suspect: blocking I/O on the UI thread, a deadlock, or a Win32 window creation issue (similar to early Video Cue issues)
   - Likely causes to investigate first: synchronous file decode on the main thread, a lock held across a Win32 call, or a message-loop starvation
   - Must be fixed before any other work — a cue type that freezes the app is a showstopper for live use

2. **⚠️ Video Cue — 0.5s freeze on GO** (`engine/video_engine.rs`, `cue/video_cue.rs`)
   - Playback itself is correct, but the first ~500 ms after GO block the UI / playhead
   - Suspect: synchronous libmpv initialization, Win32 window creation, or `loadfile` blocking the transport thread
   - Possible mitigations: pre-create the mpv instance and window on workspace load (warm pool), move `loadfile` off the transport thread, or pre-roll videos when the playhead lands on them
   - See PROGRESS.md for the current video architecture details

3. **Output Patch routing — ASIO→WASAPI validation**
   - Audio routing through `Voice.out_l/out_r` is wired, but the ASIO path still needs validation on hardware
   - Verify VU meter moves during video playback (confirms the `ao=pcm` named-pipe path into AudioEngine is active)

See `PROGRESS.md` for the full detailed state of every module.
