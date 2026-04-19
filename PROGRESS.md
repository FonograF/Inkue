# WinCue — Project state as of 2026-04-19

## Current version: 0.3.0

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
| Video | ⚠️ **Functional with glitch** | Plays correctly, but **freezes ~0.5 s on GO** |
| Image | 🔴 **Broken** | **Freezes the entire app on GO** — highest-priority fix |

---

## What is implemented and compiles

### Rust backend

| Module | File | Status |
|---|---|---|
| Cue types | `cue/types.rs` | ✅ Complete |
| Cue trait | `cue/traits.rs` | ✅ Complete |
| CueRegistry | `cue/registry.rs` | ✅ Complete |
| CueContext | `cue/context.rs` | ✅ Complete — `audio_engine`, `video_engine`, `stop_fade_ms`, `output_patches` |
| AudioCue | `cue/audio_cue.rs` | ✅ 100% functional — pre-wait, fade-in/out, loop, rate mismatch, `Voice.out_l/r` routing via OutputPatch |
| VideoCue | `cue/video_cue.rs` | ⚠️ Plays correctly, ~0.5 s freeze on GO |
| ImageCue | `cue/image_cue.rs` | 🔴 Broken — freezes the app on GO |
| MemoCue | `cue/memo_cue.rs` | ✅ Complete |
| StopCue | `cue/stop_cue.rs` | ✅ Complete |
| VoiceState / FadeState | `engine/voice.rs` | ✅ Complete — `out_l`, `out_r` for channel routing |
| AudioCommand / AudioStatus | `engine/ring_command.rs` | ✅ Complete |
| DeviceManager / OutputPatch | `engine/device_manager.rs` | ✅ Complete |
| AudioEngine | `engine/audio_engine.rs` | ✅ Complete — WASAPI/ASIO, mixes audio + video PCM in `fill_buffer` |
| VideoEngine | `engine/video_engine.rs` | ⚠️ Playback OK, startup freeze to diagnose |
| ImageEngine (if present) | `engine/image_engine.rs` | 🔴 Broken — blocks the UI thread |
| mpv_sys (FFI) | `engine/mpv_sys.rs` | ✅ libmpv bindings compile |
| CueList | `show/cue_list.rs` | ✅ Complete |
| Workspace | `show/workspace.rs` | ✅ Complete |
| Transport | `show/transport.rs` | ✅ Complete |
| 30fps event loop | `show/event_loop.rs` | ✅ Complete |
| UndoStack | `show/undo_stack.rs` | ✅ Complete |
| AppState | `state/app_state.rs` | ✅ Complete |
| Preferences | `preferences.rs` | ✅ Complete |
| Transport commands | `commands/transport_cmds.rs` | ✅ Complete |
| Cue commands | `commands/cue_cmds.rs` | ✅ Complete — `set_video_file`, `list_video_screens` |
| Workspace commands | `commands/workspace_cmds.rs` | ✅ Complete |
| Device commands | `commands/device_cmds.rs` | ✅ Complete |
| Preferences commands | `commands/preferences_cmds.rs` | ✅ Complete |
| Undo commands | `commands/undo_cmds.rs` | ✅ Complete |

### React / TypeScript frontend

| File | Status |
|---|---|
| `lib/types.ts` | ✅ Complete — `VideoCueData.screen_index`, `ScreenInfo`, `ImageCueData` |
| `lib/commands.ts` | ✅ Complete — `listVideoScreens` |
| `stores/workspaceStore.ts` | ✅ Complete |
| `stores/transportStore.ts` | ✅ Complete |
| `stores/timingStore.ts` | ✅ Complete |
| `hooks/useTauriEvents.ts` | ✅ Complete |
| `hooks/useKeyboardShortcuts.ts` | ✅ Complete |
| `App.tsx` | ✅ Complete |
| `components/CueList/` | ✅ Complete |
| `components/Inspector/InspectorPanel.tsx` | ✅ Complete — audio + video (image to validate once the crash is fixed) |
| `components/Inspector/BasicsTab.tsx` | ✅ Complete — video screen selector |
| `components/Inspector/TimeTab.tsx` | ✅ Complete |
| `components/Inspector/LevelsTab.tsx` | ✅ Complete — Pan conditional on `isAudio` |
| `components/Inspector/FadeTab.tsx` | ✅ Complete |
| `components/Transport/TransportBar.tsx` | ✅ Complete — rAF decay + peak hold |
| `components/Preferences/PreferencesModal.tsx` | ✅ Complete |
| `components/WaveformModal.tsx` | ✅ Complete |

---

## Known bugs — to fix

### 🔴 CRITICAL — ImageCue freezes the app on GO

**Symptom:** as soon as an Image cue is triggered (GO or Auto-Continue), the entire application freezes. The transport stops responding, keyboard shortcuts are blocked, the VU meter stops.

**Investigation leads (in order of likelihood):**

1. **Synchronous image decoding on the UI / transport thread**
   - If image loading (PNG/JPG) happens directly in `ImageCue::go()` or inside the Tauri command, a large image will block for several seconds
   - Fix: decode in a worker thread, only hand the ready buffer to the Win32 layer
2. **Blocking Win32 window creation or deadlock**
   - If `ImageEngine` creates its window on the calling thread and waits for an ACK via a channel held by the same thread → classic deadlock
   - Fix: reuse the `VideoEngine` pattern (dedicated thread with Win32 message loop, non-blocking channel communication)
3. **Shared lock with AudioEngine or VideoEngine**
   - Check that `ImageCue::go()` does not grab a `Mutex` already held by another actor (transport, event loop)
4. **Massive allocation / I/O inside the audio callback**
   - Unlikely, but rule it out: if image code touches the audio callback path, it's dead

**Repro test:** create a workspace with a single Image cue (any PNG), GO → the app freezes. Try with a 100×100 image vs a 4000×3000 image: if size changes the behavior, it's synchronous decoding.

**Top priority** — a cue type that freezes the app is a showstopper for live use.

---

### ⚠️ VideoCue — ~0.5 s freeze on GO

**Symptom:** video playback is correct (fullscreen, routed audio, loop OK), but the ~500 ms following a GO on a video cue block the UI and the playhead. The freeze disappears once the video starts.

**Investigation leads:**

1. **Synchronous `mpv_create()` + `mpv_initialize()` on GO**
   - libmpv takes ~200–400 ms to initialize on first use
   - Fix: pre-create a pool of mpv instances when the workspace opens, reuse one on GO
2. **Win32 window creation on GO**
   - `CreateWindowExW` + `ShowWindow` + `wid` injection take time
   - Fix: pre-create the window (hidden) when the workspace loads, show it on GO
3. **Blocking `loadfile` on the transport thread**
   - If `mpv_command(..., "loadfile", path, ...)` runs on the transport-loop thread and mpv parses the file before returning, it blocks
   - Fix: move `loadfile` to the dedicated `VideoEngine` thread, sync via a command ring buffer (same pattern as AudioEngine)
4. **Automatic pre-roll**
   - Elegant alternative: when the playhead lands on a video cue, pre-load the video paused (frame 0 displayed). GO becomes just `set pause no` → zero perceptible latency

**Medium priority** — doesn't prevent usage, but degrades the live experience (a GO should be instantaneous).

---

### ⚠️ Video Output Patch routing — needs ASIO hardware validation

The `ao=pcm` → named pipe → `AudioEngine` architecture compiles and works on default WASAPI. Still to verify on an ASIO interface:

1. `PCM pipe: mpv connected` logs appear on video GO
2. VU meter moves during video playback
3. Video audio comes out of the ASIO device (not default WASAPI)

If mpv can't open `\\.\pipe\wincue-mpv-audio` as an `ao=pcm` file on some versions, fall back to a temp file with polling.

---

## Change history

### 0.3.0 — Image Cue type added (in progress — 2026-04-19)

#### 🔴 ImageCue introduced but non-functional
- `cue/image_cue.rs` skeleton in place, registered in the `CueRegistry`
- Workspace serialization/deserialization OK
- **Blocking bug**: GO on an Image cue freezes the app — see "Known bugs" section

#### ⚠️ VideoCue — startup latency regression
- A ~0.5 s freeze now appears on video GO (not present in 0.2.0)
- To diagnose: mpv init, window creation, or blocking `loadfile`
- Playback itself remains correct

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

**Architectural problems fixed:**
- Removed WASAPI loopback (captured the default system device, unusable with ASIO)
- Removed `wasapi_name_for_asio()` (mpv doesn't support ASIO — wrong approach)
- Removed `audio_device_id` / `audio_backend` from `CueContext` (no longer needed)

**New architecture:**
- `mpv ao=pcm` writes float32 stereo PCM to `\\.\pipe\wincue-mpv-audio`
- `wincue-mpv-pcm` thread reads the pipe → ring buffer → `AudioEngine.set_video_pcm_consumer()`
- `AudioEngine.fill_buffer` mixes video PCM with audio voices (same WASAPI/ASIO device)
- VU meter reads `AudioStatus::MasterLevels` from AudioEngine's ring buffer — includes audio + video
- `TransportBar.tsx`: rAF-based decay (20 dB/sec), 1.5 s peak hold, red needle > -6 dBFS

#### 🎬 Video Cue — playback operational

After several debugging iterations (2026-04-12 → 13), video playback becomes fully functional.

**Issues fixed in `video_engine.rs`:**
- ✅ D3D11 backend — `gpu-api=d3d11` forced; ANGLE/EGL doesn't support `--wid`
- ✅ `loadfile` arguments — order corrected: `url, flags, index(int), options`
- ✅ Infinite loop — `loop-file=no` for `loop_count=0`
- ✅ Second video frozen — `keep-open=no` + `set pause no` before each `loadfile`
- ✅ Windows loading cursor — `WS_EX_TRANSPARENT` applied on the render child at `MPV_EVENT_FILE_LOADED`
- ✅ Drag and fullscreen double-click — `CS_DBLCLKS` + direct handlers
- ✅ Focus stealing — `WS_EX_NOACTIVATE` + `SW_SHOWNA` + `WM_MOUSEACTIVATE → MA_NOACTIVATE`
- ✅ Always-on-top window — `HWND_TOPMOST` + `SWP_NOACTIVATE`
- ✅ Double cue in progress — `VideoStatus::Completed` emitted for the old voice
- ✅ Fullscreen with border — `WS_SIZEBOX` removed in fullscreen, restored in floating

#### 🖥️ Screen selection (inspector)
- `VideoEngine::list_screens()` — `EnumDisplayMonitors` + `GetMonitorInfoW` enumeration
- Serializable `ScreenInfo` struct
- `VideoCue.screen_index: Option<u32>` — serialized in `.wincue`
- `list_video_screens` Tauri command — called by the frontend
- `BasicsTab.tsx` — "Floating window" dropdown + list of detected screens

#### 🪟 Resizable borderless video window
- `WS_SIZEBOX` on the popup style — native OS resize on edges
- `WM_NCHITTEST` — passes edges to `DefWindowProc`, forces `HTCLIENT` for the interior
- `WM_SETCURSOR` — arrow only on `HTCLIENT`, OS resize cursor on edges

#### 🐛 Video Cue inspector fixes
- `LevelsTab.tsx` — `AudioCueData | VideoCueData` type, `isAudio` prop, Pan conditional
- `FadeTab.tsx` — widened type `AudioCueData | VideoCueData`
- `InspectorPanel.tsx` — `isAudio` passed to `LevelsTab`, `as AudioCueData` casts removed

---

### Post-0.1.2 — fixes and features (2026-04-12)

#### ⚙️ General preferences
4 options: Double GO Protection, Confirm Before Delete, Auto-Scroll to Playhead, Cue Row Height.

#### 🐛 Backend fixes
- Audio Cue copy/paste silent — `Arc` samples transferred in `paste_cue`
- STOP fade duration preference ignored — `CueContext.stop_fade_ms` now read from preferences

#### 🎨 Frontend features
- Inspector refactor — 7 sub-components extracted
- Color tags — 4px colored strip, QLab-style

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

## Partially implemented or missing

### Backend
- **ImageCue**: skeleton in place but GO implementation broken (freeze)
- **VideoCue**: works but initialization too slow (~0.5 s freeze)

### Frontend
- Image inspector: to validate once the backend is stable

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
| 8. Inspector panel | ✅ Complete for audio + video |
| 9. Workspace save/load | ✅ |
| 10. Keyboard shortcuts | ✅ |
| 11. Fades, waveform, level meters | ✅ |
| 12. Drag-drop, undo/redo, color tags | ✅ |
| 13. Video Cue | ⚠️ Functional with 0.5 s freeze on GO |
| 14. Image Cue | 🔴 Broken — freezes the app on GO |
| 15. Stop Cue | ✅ Functional |

---

## Next priorities (in order)

1. **🔴 Debug ImageCue** — a cue that freezes the app is a blocker for live use
   - Isolate whether the freeze comes from image decoding, Win32 window creation, or a shared lock
   - Reuse the `VideoEngine` pattern: dedicated thread with message loop, ring-buffer communication
2. **⚠️ Eliminate VideoCue's 0.5 s freeze** — pre-create mpv instance + window on workspace load, or pre-roll when the playhead lands on the cue
3. **⚠️ Validate video routing on ASIO hardware** — verify `PCM pipe: mpv connected` + VU meter + audio output
