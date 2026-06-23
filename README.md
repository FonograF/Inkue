# WinCue

A professional, cross-platform show-control application (Windows, macOS, Linux), inspired by QLab (macOS). WinCue manages cue lists for live events — theatre, concerts, corporate shows — with a focus on reliability, low latency, and an extensible cue architecture.

Built with **Rust** (backend) and **React + TypeScript** (frontend) via [Tauri v2](https://tauri.app/).

---

## Features

### Cue types

| Type | Description |
|---|---|
| **Audio** | WAV, MP3, FLAC, OGG, AAC, M4A — sample-accurate WASAPI/ASIO playback with fade-in/out, trim, loop (finite + infinite), rate, pan, Output Patch routing |
| **Video** | Fullscreen or floating output window via libmpv (unified GL Render API); audio decoded as a normal audio voice (shared Output Patch, VU metering, fades); loop |
| **Image** | Any image format via libmpv; dip-to-black fades; optional display duration; stops on next visual GO |
| **Group** | Sequential or Simultaneous; sequential mode holds the outer playhead and absorbs GO presses to advance the internal sequence |
| **Wait** | Fixed-duration delay; integrates with Auto-Continue chains |
| **Stop** | Stops all running cues or a chosen subset (soft fade or hard cut); drag the ■ STOP button or `+ Stop` from the toolbar to insert anywhere |
| **Fade** | Fades volume (dB) and/or image brightness on any running cue(s) to a target; configurable curve; optional Stop at End |
| **OSC** | Sends one or more UDP OSC messages on GO; multiple messages per cue; workspace-level named patches |
| **MIDI** | Sends Note On/Off, Control Change, Program Change on GO; multiple messages per cue; dynamic port enumeration (WinMM/CoreMIDI) |
| **Memo** | Read-only label; no playback action |

### Transport & playback

- **GO / STOP / Hard Stop** — keyboard (Space / Escape / double-Escape), toolbar buttons, or OSC remote
- **Pre-Wait / Post-Wait** — delays before/after the cue action
- **Continue modes** — Do Not Continue, Auto-Continue (overlap), Auto-Follow (chain on finish)
- **Pause / Resume** — individual cues or all running cues
- **Scrub / Seek** — drag the playhead in the Time tab; audio and video both seekable while running or paused
- **Pause / Resume** — progress bar and inspector counter freeze at the exact pause position; seek while paused repositions the cue
- **Double-GO protection** — configurable debounce window (default 500 ms) silently drops duplicate triggers from OSC controllers or accidental rapid presses

### Output

- **Unified output window** — single persistent native window (winit + mpv OpenGL Render API) for all video and image cues; no flicker between cues; supports fullscreen on any monitor or draggable floating window. The legacy Win32 + D3D11 path is kept behind the `legacy-win32-output` feature flag as a regression fallback
- **Output timer** — OSD overlay via mpv; ships with the bundled **DSEG7 Classic** 7-segment font as default; configurable font/size/position/margin/ms display, with system-font autocomplete on Windows, Linux and macOS; live preview in Preferences. An optional always-on-top floating timer window mirrors it on the operator's screen
- **WASAPI & ASIO** — low-latency audio via [cpal](https://github.com/RustAudio/cpal); ASIO requires the Steinberg SDK
- **Output Patches** — named mappings to audio devices and channel pairs, shared across cue lists

### OSC remote control

WinCue listens on UDP port 53001 (configurable). Supported receive addresses:

| Address | Action |
|---|---|
| `/wincue/go` | Advance playhead and fire GO |
| `/wincue/stop` | Stop all (soft fade) |
| `/wincue/hardstop` | Hard stop all |
| `/wincue/pause` | Pause all running cues |
| `/wincue/resume` | Resume all paused cues |
| `/wincue/pause_toggle` | Pause if anything is running, resume if anything is paused |
| `/wincue/select/next` | Move playhead to next cue (no fire) |
| `/wincue/select/previous` | Move playhead to previous cue (no fire) |
| `/wincue/cue/{number}/go` | Jump to cue number and fire |
| `/wincue/cue/{number}/select` | Move playhead to cue number (no fire) |
| `/wincue/cue/{number}/stop` | Stop specific cue |

Configure in **Preferences → Network**. An activity dot in the transport bar flashes on every received packet. The **OSC Monitor** (click the dot) shows all incoming packets in real time with address, arguments, and match status. A built-in dedup cache (50 ms window) eliminates duplicate UDP packets from Windows loopback and OSC controllers that transmit each packet twice.

### Editor

- **Inspector panel** — Basics, Time, Levels, Fade, Messages tabs per cue type
- **Waveform editor** — visual start/end trim with real-time preview playhead
- **Drag-and-drop** — reorder cues, drag into groups, drop media files from Explorer
- **Toolbar drag** — drag `+ Audio`, `+ Video`, etc. from the toolbar to insert at any list position
- **Multi-select** — Ctrl/Shift/Ctrl+A; multi-delete, multi-duplicate, multi-drag, multi-color
- **Undo / redo** — full snapshot-based history
- **Copy / paste** — serialize cues to clipboard, paste anywhere
- **Column config** — resizable, reorderable, hideable columns; layout persisted to localStorage
- **Color tags** — QLab-compatible color labels on cue rows; render as a left-edge stripe or tint the whole row (Personalization preferences)
- **Consistent dark theme** — custom dropdowns and a Personalization preferences category keep the look identical across Windows, Linux and macOS

---

## Architecture

```
src-tauri/src/
├── cue/            # Cue trait + CueRegistry + all cue types
│   ├── audio_cue.rs, video_cue.rs, image_cue.rs
│   ├── group_cue.rs, wait_cue.rs, stop_cue.rs, memo_cue.rs
│   ├── osc_cue.rs, osc_types.rs
│   ├── context.rs  # CueContext — passed to every lifecycle method
│   └── registry.rs # factory registry — add new types without touching transport
├── engine/
│   ├── audio_engine.rs     # cpal real-time thread; zero alloc/lock in callback
│   ├── output_engine/      # libmpv GL Render API output window for video + image
│   ├── osc_patch.rs        # OscPatch (named UDP send target)
│   ├── osc_server.rs       # UDP receive thread + frontend dispatch
│   └── device_manager.rs   # WASAPI/ASIO device enumeration + Output Patches
├── show/
│   ├── transport.rs        # GO / STOP / PAUSE / RESUME logic
│   ├── cue_list.rs         # ordered list + playhead
│   ├── event_loop.rs       # 30 fps tick (completions, auto-continue, time events)
│   │                       # + 60 fps timer-refresh thread for OSD
│   ├── workspace.rs        # save / load .wincue JSON
│   └── undo_stack.rs
├── commands/       # Tauri IPC handlers (transport, cues, workspace, devices, prefs, osc)
├── state/          # AppState — engines, registry, undo, clipboard, last-GO timestamp
├── machine_config.rs  # per-OS config dir (audio.json, osc.json)
└── preferences.rs  # AppPreferences (audio, general, display, network / OSC)

src/
├── components/
│   ├── CueList/        # CueListView, CueRow, column definitions
│   ├── Inspector/      # InspectorPanel + per-type tabs (Basics, Time, Levels, Fade, Messages)
│   ├── Transport/      # TransportBar — GO/STOP, VU meters, master volume, OSC dot
│   ├── Osc/            # OscMonitor — floating real-time packet log
│   ├── OscPatches/     # OscPatchesPanel — workspace OSC patch CRUD
│   ├── Preferences/    # PreferencesModal — Audio, General, Network, Display tabs
│   └── WaveformModal.tsx
├── stores/         # Zustand: workspaceStore, transportStore (incl. OSC log), timingStore
├── hooks/          # useTauriEvents (all backend events), useKeyboardShortcuts
└── lib/            # types.ts, commands.ts — typed Tauri invoke wrappers
```

**Key invariants:**
- The audio callback has zero allocations, zero locks, zero I/O — all comms via lock-free ring buffers.
- The `Cue` trait is the only interface the transport layer uses. Adding a new cue type never requires touching `transport.rs`, `cue_list.rs`, or the CueList UI.
- Machine-specific settings (audio device, OSC config) live in the per-OS config dir (`%APPDATA%\WinCue\` on Windows, `~/.config/WinCue` on Linux, `~/Library/Application Support/WinCue` on macOS); show-specific settings travel in the `.wincue` file.

---

## Stack

| Layer | Technology |
|---|---|
| UI | React 18 + TypeScript + Vite |
| State | Zustand |
| Desktop shell | Tauri v2 |
| Backend | Rust 2021 |
| Audio I/O | cpal (WASAPI / ASIO on Windows, CoreAudio on macOS, ALSA / PipeWire on Linux) |
| Audio decoding | Symphonia (WAV, MP3, FLAC, OGG, AAC, M4A) |
| Video / Image | libmpv (OpenGL Render API, persistent native window via winit + glutin) |
| OSC | rosc 0.10 |
| Lock-free comms | ringbuf + crossbeam-channel |

---

## Getting Started

### Prerequisites

- [Rust](https://rustup.rs/) (stable)
- [Node.js](https://nodejs.org/) + [pnpm](https://pnpm.io/)
- Windows 10 / 11, macOS, or Linux
- libmpv — required for video and image cues (not versioned, ~113 MB): `vendor/mpv/libmpv-2.dll` on Windows; `libmpv` via Homebrew on macOS; the `libmpv2` (or `libmpv1`) system package on Linux. Per-OS resolution detail in `PORTAGE.md`.
- *(Optional, Windows only)* [Steinberg ASIO SDK](https://www.steinberg.net/developers/) in `vendor/asiosdk/` for ASIO support

### Development

```bash
pnpm install
pnpm tauri dev          # WASAPI only
pnpm tauri:dev          # with ASIO (requires vendor/asiosdk/)
```

### Production build

```bash
pnpm tauri build        # WASAPI only
pnpm tauri:build        # with ASIO
```

Generates an `.msi` installer and a standalone `.exe` in `src-tauri/target/release/bundle/`.

### Tests

```bash
cd src-tauri && cargo test   # cue registry, OSC types/server/dedup, SR conversion, stop/fade specs, cue list ops, DMX engine/fixtures/Light Cue
```

---

## QLab Terminology

WinCue uses QLab's vocabulary throughout:

| Term | Meaning |
|---|---|
| **Workspace** | The project file (`.wincue`) |
| **Cue List** | Ordered sequence of cues |
| **Playhead** | Marker for the next cue triggered by GO |
| **GO** | Trigger the cue at the playhead |
| **Pre-Wait** | Delay before the cue action starts |
| **Post-Wait** | Delay after the action starts before continue mode fires |
| **Auto-Continue** | Start the next cue after Post-Wait elapses |
| **Auto-Follow** | Start the next cue when this one finishes |
| **Cue Number** | A string — `"1"`, `"1.5"`, `"Intro"` are all valid |
| **Output Patch** | Named mapping to an audio device + channel pair |

---

## License

Private — all rights reserved.
