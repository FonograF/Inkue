# CLAUDE.md — WinCue Project Instructions

## What is this project?

WinCue is a show control application for Windows, inspired by QLab (macOS). It manages cue lists for live events (theatre, concerts, corporate). The first version implements Audio Cues, but the architecture must support any cue type (MIDI, OSC, Video, Fade, Group, Wait, Memo, Network, Script) without modifying existing code.

The full specification is in `wincue-prompt.md` at the project root. Read it before starting any work.

## Stack

- Backend: Rust (engine, audio, show logic)
- Audio: cpal (WASAPI/ASIO) + symphonia (decoding WAV/MP3/FLAC/OGG)
- Lock-free comms: ringbuf or crossbeam
- UI: Tauri v2 + React + TypeScript
- State: Zustand
- Build: cargo + pnpm

## Build & run commands

```
pnpm install              # install frontend deps (first time)
pnpm tauri dev            # run in dev mode (compiles Rust + starts frontend)
pnpm tauri build          # production build
cargo build               # compile Rust only (from src-tauri/)
cargo test                # run Rust tests (from src-tauri/)
cargo clippy              # lint Rust (from src-tauri/)
```

## Architecture rules — DO NOT VIOLATE

### Cue system extensibility

The entire app revolves around the `Cue` trait in `src-tauri/src/cue/traits.rs`. Every cue type (Audio, Memo, Wait, Group, MIDI, Video...) implements this trait. The `CueRegistry` in `registry.rs` maps `CueType` to `CueFactory` instances.

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

### Separation of concerns

Three distinct layers, never mix them:
1. **Audio Engine** (`engine/`): knows about samples, devices, voices, mixing. Does NOT know about cues or shows.
2. **Cue System** (`cue/`): knows about cue lifecycle, timing, serialization. AudioCue talks to AudioEngine but the trait itself is engine-agnostic.
3. **Show/Transport** (`show/`): knows about cue lists, playhead, GO logic, continue modes. Does NOT know audio internals.

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
- Zustand for all shared state (two stores: workspaceStore, transportStore)
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

## File you must read first

Before any implementation work, read `wincue-prompt.md` in the project root. It contains the complete specification including the exact project file structure, all trait methods, all Tauri commands and events, the save format, and the UI layout.

## Development order

If starting from scratch or resuming, follow this order:
1. Tauri v2 scaffold + basic window opens
2. Cue trait + types + CueRegistry + MemoCue (proves extensibility)
3. AudioEngine with basic WAV playback (cpal + symphonia)
4. AudioCue implementation connected to engine
5. Frontend: static cue list display + GO button that plays audio
6. Playhead + transport (GO/STOP/PAUSE with continue modes)
7. Output Patches + DeviceManager
8. Inspector panel with audio cue editing
9. Workspace save/load (.wincue JSON)
10. Keyboard shortcuts
11. Fades, waveform display, level meters
12. Polish: drag-drop reorder, undo/redo, color tags
