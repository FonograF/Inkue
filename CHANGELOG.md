# Changelog

All notable changes to Inkue are documented here.
The format is loosely based on [Keep a Changelog](https://keepachangelog.com/),
and the project follows [Semantic Versioning](https://semver.org/).

## [1.0.0] — first public release

The first public, open-source release of Inkue — a cross-platform, QLab-inspired
show-control application. Released under the GNU GPL v3.

### Cue types

- **Audio** — sample-accurate WASAPI / ASIO / CoreAudio / ALSA playback; fade-in/out,
  trim, finite + infinite loop, rate, pan, waveform editor, VU metering.
- **Video** & **Image** — unified libmpv OpenGL Render API output window (no flicker
  between cues); dip-to-black fades; scrub/seek; loop; audio of a video plays as a normal voice.
- **Group** (sequential / simultaneous), **Wait**, **Stop** (multi-target, soft or hard),
  **Fade** (multi-target volume and/or brightness), **Memo**.
- **OSC** and **MIDI** — send multiple messages per cue; named patches; dynamic port enumeration.
- **Light** — DMX-over-IP (sACN E1.31 + Art-Net); workspace fixture patch; fades to a target look.
- **Mic** — routes a live audio input through the engine with an adaptive drift resampler.
- **Timecode** — SMPTE generate + receive (MTC), LTC encode/decode, per-cue triggers + cue-list sync.
- **Text** — styled text on the output surface, independent of the OSD timer.

### Transport & UI

- GO / STOP / Hard Stop (keyboard, toolbar, OSC), Pre/Post-Wait, Auto-Continue, Auto-Follow.
- Pause/Resume and scrub/seek for individual cues or all at once; double-GO debounce protection.
- Show Mode (F5), inline cell editing, Active Cues panel, multi-select, undo/redo, copy/paste,
  configurable columns, QLab-compatible color tags, consistent dark theme on all three OSes.

### Output, I/O & reliability

- Single persistent native output window; fullscreen on any monitor or a floating window.
- Output timer OSD with the bundled DSEG7 font; optional always-on-top floating timer.
- OSC receive server with IP allowlist + 50 ms dedup cache and a live OSC Monitor.
- Crash-recovery autosave; "Check Workspace" preflight with media relink; in-app log viewer.
- Hardware resilience: audio device-loss detection with fallback, self-healing MIDI alerts.

### Platforms

- Windows 10/11, macOS (Apple Silicon + Intel) and Linux (x86-64), from a single codebase.

[1.0.0]: https://github.com/FonograF/Inkue/releases/tag/v1.0.0
