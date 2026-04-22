# WinCue — Project state as of 2026-04-22

## Current version: 0.3.1

## cargo build result

**Compiles without errors, zero warnings.**

## cargo test result

**20 tests pass, 0 failures.**

---

## Cue type status

| Cue type | Status | Details |
|---|---|---|
| Audio | ✅ **100% functional** | Pre/post-wait, fade-in/out, loop, rate mismatch, Output Patch routing, pan, master volume, waveform, VU meter |
| Stop  | ✅ **Functional** | Targeted Stop and Stop All, default 0.5 s fade |
| Memo  | ✅ **Functional** | Read-only, no audio action |
| Video | ⚠️ **Functional with glitch** | Plays correctly, but **freezes ~0.5 s on first GO** (pre-arm mitigates when playhead lands on the cue) |
| Image | ✅ **Functional** | Persistent surface windows, fade-in/out, stop-on-next-go, draggable floating window |

---

## What is implemented and compiles

### Rust backend

| Module | File | Status |
|---|---|---|
| Cue types | `cue/types.rs` | ✅ Complete |
| Cue trait | `cue/traits.rs` | ✅ Complete — includes `stop_on_next_go()` |
| CueRegistry | `cue/registry.rs` | ✅ Complete |
| CueContext | `cue/context.rs` | ✅ Complete — `audio_engine`, `video_engine`, `image_engine`, `stop_fade_ms`, `output_patches` |
| AudioCue | `cue/audio_cue.rs` | ✅ 100% functional — pre-wait, fade-in/out, loop, rate mismatch, `Voice.out_l/r` routing via OutputPatch |
| VideoCue | `cue/video_cue.rs` | ⚠️ Plays correctly, ~0.5 s freeze on first GO (pre-arm on playhead landing) |
| ImageCue | `cue/image_cue.rs` | ✅ Functional — persistent surface windows, stop modes, fade-in/out |
| MemoCue | `cue/memo_cue.rs` | ✅ Complete |
| StopCue | `cue/stop_cue.rs` | ✅ Complete |
| VoiceState / FadeState | `engine/voice.rs` | ✅ Complete — `out_l`, `out_r` for channel routing |
| AudioCommand / AudioStatus | `engine/ring_command.rs` | ✅ Complete |
| DeviceManager / OutputPatch | `engine/device_manager.rs` | ✅ Complete |
| AudioEngine | `engine/audio_engine.rs` | ✅ Complete — WASAPI/ASIO, mixes audio + video PCM in `fill_buffer` |
| VideoEngine | `engine/video_engine.rs` | ⚠️ Playback OK, ~0.5 s startup freeze |
| ImageEngine | `engine/image_engine.rs` | ✅ Functional — persistent `output-surface-{key}` WebviewWindows, one per screen |
| mpv_sys (FFI) | `engine/mpv_sys.rs` | ✅ libmpv bindings compile |
| CueList | `show/cue_list.rs` | ✅ Complete |
| Workspace | `show/workspace.rs` | ✅ Complete |
| Transport | `show/transport.rs` | ✅ Complete — `stop_on_next_go()` called before each GO |
| 30fps event loop | `show/event_loop.rs` | ✅ Complete |
| UndoStack | `show/undo_stack.rs` | ✅ Complete |
| AppState | `state/app_state.rs` | ✅ Complete |
| Preferences | `preferences.rs` | ✅ Complete |
| Transport commands | `commands/transport_cmds.rs` | ✅ Complete |
| Cue commands | `commands/cue_cmds.rs` | ✅ Complete — `get_surface_current_voice`, `report_image_faded_out` |
| Workspace commands | `commands/workspace_cmds.rs` | ✅ Complete |
| Device commands | `commands/device_cmds.rs` | ✅ Complete |
| Preferences commands | `commands/preferences_cmds.rs` | ✅ Complete |
| Undo commands | `commands/undo_cmds.rs` | ✅ Complete |

### React / TypeScript frontend

| File | Status |
|---|---|
| `lib/types.ts` | ✅ Complete — `ImageCueData` with `stop_mode`, `display_duration_ms` |
| `lib/commands.ts` | ✅ Complete — `getSurfaceCurrentVoice` |
| `stores/workspaceStore.ts` | ✅ Complete |
| `stores/transportStore.ts` | ✅ Complete |
| `stores/timingStore.ts` | ✅ Complete |
| `hooks/useTauriEvents.ts` | ✅ Complete |
| `hooks/useKeyboardShortcuts.ts` | ✅ Complete |
| `App.tsx` | ✅ Complete |
| `components/CueList/` | ✅ Complete |
| `components/Inspector/InspectorPanel.tsx` | ✅ Complete — audio, video, image |
| `components/Inspector/BasicsTab.tsx` | ✅ Complete — video + image screen selector |
| `components/Inspector/TimeTab.tsx` | ✅ Complete — Image stop mode selector |
| `components/Inspector/LevelsTab.tsx` | ✅ Complete — Pan conditional on `isAudio` |
| `components/Inspector/FadeTab.tsx` | ✅ Complete — Fade-in/out for audio, video, image |
| `components/Transport/TransportBar.tsx` | ✅ Complete — rAF decay + peak hold |
| `components/Preferences/PreferencesModal.tsx` | ✅ Complete |
| `components/WaveformModal.tsx` | ✅ Complete |
| `components/ImageSurface.tsx` (`OutputSurface`) | ✅ Complete — direct DOM fade, force reflow |

---

## Known bugs — to fix

### ⚠️ VideoCue — ~0.5 s freeze on first GO

**Symptom:** video playback is correct (fullscreen, routed audio, loop OK), but the ~500 ms following the first GO on a video cue block the UI. The freeze disappears once the video starts. Pre-arm (when the playhead lands on the cue) avoids the freeze for cues reached via the playhead, but not for manual GOs on arbitrary cues.

**Investigation leads:**

1. **Synchronous `mpv_create()` + `mpv_initialize()` on GO**
   - libmpv takes ~200–400 ms to initialize on first use
   - Fix: pre-create a pool of mpv instances when the workspace opens, reuse one on GO
2. **Win32 window creation on GO**
   - `CreateWindowExW` + `ShowWindow` + `wid` injection take time
   - Fix: pre-create the window (hidden) when the workspace loads, show it on GO
3. **Blocking `loadfile` on the transport thread**
   - Fix: move `loadfile` to the dedicated `VideoEngine` thread

---

### ⚠️ Video Output Patch routing — needs ASIO hardware validation

The `ao=pcm` → named pipe → `AudioEngine` architecture compiles and works on default WASAPI. Still to verify on an ASIO interface:

1. `PCM pipe: mpv connected` logs appear on video GO
2. VU meter moves during video playback
3. Video audio comes out of the ASIO device (not default WASAPI)

---

## Change history

### 0.3.1 — Image Cue fully functional (2026-04-22)

#### ✅ Image Cue — freeze fixed, full feature set

The Image Cue was completely non-functional in 0.3.0. This release makes it production-ready.

**Architecture: persistent surface windows**
- Replaced per-voice Win32 window approach with persistent Tauri `WebviewWindow` per screen
- One `output-surface-{index}` window created lazily on first use, hidden (not closed) between cues
- Consecutive image cues on the same screen share the window — no close/reopen flicker
- `surface-show-image` / `surface-hide-image` Tauri events drive the React component

**Focus fix**
- Image surface window no longer steals keyboard focus from the main window
- `main.set_focus()` called after every `win.show()` ensures GO/STOP shortcuts keep working

**Stop mode**
- New `ImageStopMode` enum: `stop_on_next_cue` (default) or `display_duration`
- `stop_on_next_go()` trait method (default `false`) — Transport calls it before each GO
- Inspector Time tab shows a stop-mode selector; duration field appears only in `display_duration` mode

**Fade-in / fade-out**
- Rewrote `OutputSurface` to use direct DOM manipulation instead of React state for opacity
- `getBoundingClientRect()` forces a browser reflow before the CSS transition starts, making
  the fade reliable under React 18's automatic batching
- Removed the hardcoded 500 ms default fade-out — no implicit fade; only configured values apply

**Draggable floating window**
- Full-window `onMouseDown` → `win.startDragging()` on the floating surface
- Added `core:window:allow-start-dragging` to `capabilities/image-surface.json`

**Bug fixes**
- TypeScript build error in `InspectorPanel.tsx` — `ImageCueData` cast to `AudioCueData | VideoCueData` for `LevelsTab`
- Capability file updated: `output-surface-*` glob instead of `image-surface-*`

---

### 0.3.0 — Image Cue type added (non-functional) (2026-04-19)

#### 🔴 ImageCue introduced but non-functional
- `cue/image_cue.rs` skeleton in place, registered in the `CueRegistry`
- Workspace serialization/deserialization OK
- **Blocking bug**: GO on an Image cue froze the app — fixed in 0.3.1

#### ⚠️ VideoCue — startup latency
- A ~0.5 s freeze appears on video GO
- Playback itself remains correct
- Pre-arm added in a subsequent commit: video pre-loads when playhead lands on the cue

---

### 0.2.0 — Audio/video architecture overhaul (2026-04-14)

#### 🔧 ASIO build fix
- ASIO SDK copied to `vendor/asiosdk/` (outside WalkDir reach)
- `src-tauri/.cargo/config.toml`: `CPAL_ASIO_DIR = { value = "../vendor/asiosdk", relative = true }`
- `pnpm tauri:dev -- --features asio-support` compiles cleanly

#### 🔌 Output Patch audio routing (AudioCue)
- `Voice.out_l / out_r` (`engine/voice.rs`) — target channels in the WASAPI/ASIO buffer
- `AudioEngine.fill_buffer` — uses `voice.out_l / out_r` instead of hardcoded 0/1
- `CueContext`: `output_patches`, `default_patch_id` to resolve patches on GO
- `AudioCue`: resolves `OutputPatch` on GO, sets `voice.out_l / out_r`

#### 🎚️ VU Meter + Video PCM through AudioEngine (`ao=pcm` + named pipe)

**New architecture:**
- `mpv ao=pcm` writes float32 stereo PCM to `\\.\pipe\wincue-mpv-audio`
- `wincue-mpv-pcm` thread reads the pipe → ring buffer → `AudioEngine.set_video_pcm_consumer()`
- `AudioEngine.fill_buffer` mixes video PCM with audio voices (same WASAPI/ASIO device)
- VU meter reads `AudioStatus::MasterLevels` from AudioEngine's ring buffer — includes audio + video
- `TransportBar.tsx`: rAF-based decay (20 dB/sec), 1.5 s peak hold, red needle > -6 dBFS

#### 🎬 Video Cue — playback operational
- D3D11 backend, `loop-file=no`, `keep-open=no`, `WS_EX_NOACTIVATE`, `HWND_TOPMOST`
- Drag and fullscreen double-click, resize, focus-stealing prevention
- Screen selection in inspector (`list_video_screens` command)

---

### 0.1.2 (2026-04-11)
- Stop Cue
- Drag & drop reworked (CustomEvent, no Tauri conflict)
- Fix immediate Auto-Continue (synchronous resolution in Transport)
- Fix loop playback
- Fix duplicate/paste cue with no audio

---

### 0.1.1 (2026-04-11)
- `CueList::renumber_all()`
- `set_master_volume`
- Missing shortcuts
- `CurveSelect` with SVG preview
- TransportBar rework

---

## Development stage status

| Stage | Status |
|---|---|
| 1. Tauri scaffold + window | ✅ |
| 2. Cue trait + CueRegistry + MemoCue | ✅ |
| 3. WAV AudioEngine (cpal + symphonia) | ✅ |
| 4. AudioCue connected to engine | ✅ |
| 5. Frontend CueList + GO | ✅ |
| 6. Playhead + transport | ✅ |
| 7. Output Patches + DeviceManager | ✅ Routing wired — ASIO→WASAPI to validate on hardware |
| 8. Inspector panel | ✅ Complete for audio, video, image |
| 9. Workspace save/load | ✅ |
| 10. Keyboard shortcuts | ✅ |
| 11. Fades, waveform, level meters | ✅ |
| 12. Drag-drop, undo/redo, color tags | ✅ |
| 13. Video Cue | ⚠️ Functional with 0.5 s freeze on first GO |
| 14. Image Cue | ✅ Functional — persistent windows, fades, stop modes |
| 15. Stop Cue | ✅ Functional |

---

## Next priorities (in order)

1. **⚠️ Eliminate VideoCue's 0.5 s freeze** — pre-create mpv instance + window on workspace load, or a hot pool of mpv handles
2. **⚠️ Validate video routing on ASIO hardware** — verify `PCM pipe: mpv connected` + VU meter + audio output
3. **Future cue types** — Wait, Fade, Group, MIDI, OSC (architecture is ready; add via `CueRegistry` without touching transport)
