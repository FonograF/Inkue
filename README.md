# WinCue

A professional show-control application for Windows, inspired by QLab (macOS). WinCue manages cue lists for live events — theatre, concerts, corporate shows — with a focus on reliability, low latency, and an extensible cue architecture.

Built with **Rust** (backend) and **React + TypeScript** (frontend) via [Tauri v2](https://tauri.app/).

---

## Features

- **Audio Cues** — play WAV, MP3, FLAC, OGG, AAC files with sample-accurate playback
- **Stop Cues** — instantly stop all running cues when triggered; drag the ■ STOP button or `+ Stop` directly into the cue list to insert at any position
- **WASAPI & ASIO support** — low-latency audio on Windows via [cpal](https://github.com/RustAudio/cpal) + Steinberg ASIO SDK
- **Resizable, reorderable, hideable columns** — customisable cue list layout persisted to localStorage
- **Drag-and-drop cue reordering** — rearrange cues with a live drop indicator
- **Toolbar button drag** — drag `+ Audio` or `+ Stop` from the toolbar to insert a new cue at any position in the list
- **File drag-and-drop** — drop audio files onto the cue list; hover the centre of a row to assign the file, or the edge to insert a new cue between existing ones
- **Continue modes** — Do Not Continue, Auto-Continue (overlap with Post-Wait), Auto-Follow (trigger next on completion)
- **Waveform editor** — visual trim of Start/End points with real-time preview playhead
- **Inspector panel** — edit cue properties (name, number, volume, fade, trim, output patch, continue mode)
- **Workspace save / load** — `.wincue` JSON format with relative file paths
- **Output Patches** — named mappings to audio devices and channel pairs
- **Level meters** — real-time master VU meters at 30 fps
- **Dark theme** — purpose-built UI with custom scrollbars

---

## Architecture

```
src-tauri/src/
├── cue/            # Cue trait, CueRegistry, AudioCue, MemoCue
├── engine/         # AudioEngine (cpal), DeviceManager, lock-free ring buffers
├── show/           # CueList, Transport (GO/STOP/PAUSE), 30fps event loop
├── state/          # AppState (Tauri state)
├── commands/       # Tauri IPC commands (transport, cues, workspace, devices, prefs)
└── preferences.rs  # Persistent user preferences

src/
├── components/
│   ├── CueList/    # CueListView, CueRow, columns config
│   ├── Inspector/  # InspectorPanel (4 tabs)
│   ├── Transport/  # TransportBar
│   ├── Preferences/# PreferencesModal
│   └── WaveformModal.tsx
├── stores/         # Zustand stores (workspace, transport, timing)
├── hooks/          # useTauriEvents, useKeyboardShortcuts
└── lib/            # types.ts, commands.ts (typed Tauri wrappers)
```

The audio engine runs in a dedicated high-priority thread. All communication with the rest of the app uses lock-free ring buffers — zero allocations, zero locks, zero I/O inside the audio callback.

---

## Stack

| Layer | Technology |
|---|---|
| UI | React 18 + TypeScript + Vite |
| State | Zustand |
| Desktop shell | Tauri v2 |
| Backend | Rust 2021 edition |
| Audio I/O | cpal (WASAPI / ASIO) |
| Audio decoding | Symphonia (WAV, MP3, FLAC, OGG, AAC) |
| Lock-free comms | ringbuf + crossbeam-channel |

---

## Getting Started

### Prerequisites

- [Rust](https://rustup.rs/) (stable)
- [Node.js](https://nodejs.org/) + [pnpm](https://pnpm.io/)
- Windows 10/11
- *(Optional)* [Steinberg ASIO SDK](https://www.steinberg.net/developers/) for ASIO support

### Development

```bash
pnpm install
pnpm tauri dev          # without ASIO
pnpm tauri:dev          # with ASIO (requires CPAL_ASIO_DIR env var)
```

### Production build

```bash
pnpm tauri build        # without ASIO
pnpm tauri:build        # with ASIO
```

Generates an `.msi` installer and a setup `.exe` in `src-tauri/target/release/bundle/`.

---

## QLab Terminology

WinCue uses QLab's vocabulary throughout:

- **Workspace** — the project file (`.wincue`)
- **Cue List** — ordered sequence of cues
- **Playhead** — the marker indicating the next cue triggered by GO
- **GO** — trigger the cue at the playhead
- **Pre-Wait / Post-Wait** — delays before/after the cue action
- **Auto-Continue** — start the next cue after Post-Wait elapses (overlap)
- **Auto-Follow** — start the next cue when this one finishes
- **Output Patch** — named mapping to an audio device + channel pair

---

## License

Private — all rights reserved.
