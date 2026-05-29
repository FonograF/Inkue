# WinCue — Project state as of 2026-05-28

## Current version: 0.4.1

## cargo build result

**Compiles without errors, zero warnings.**

## cargo test result

**27 tests pass, 0 failures.**

---

## Cue type status

| Cue type | Status | Details |
|---|---|---|
| Audio | ✅ **100% functional** | Pre/post-wait, fade-in/out, loop, rate, Output Patch routing, pan, master volume, waveform, VU meter |
| Stop  | ✅ **Functional** | Targeted Stop and Stop All, default 0.5 s fade |
| Memo  | ✅ **Functional** | Read-only, no audio action |
| Video | ✅ **Functional** | Single persistent Win32 window, no first-GO freeze, dip-to-black fades, loop |
| Image | ✅ **Functional** | Same Win32 window as Video via libmpv, dip-to-black fades, stop-on-next-cue |

---

## What is implemented and compiles

### Rust backend

| Module | File | Status |
|---|---|---|
| Cue types | `cue/types.rs` | ✅ Complete |
| Cue trait | `cue/traits.rs` | ✅ Complete — includes `stop_on_next_go()` |
| CueRegistry | `cue/registry.rs` | ✅ Complete |
| CueContext | `cue/context.rs` | ✅ Complete — `audio_engine`, `output_engine`, `stop_fade_ms`, `output_patches`, `output_screen` |
| AudioCue | `cue/audio_cue.rs` | ✅ 100% functional — pre-wait, fade-in/out, loop, rate, `Voice.out_l/r` routing via OutputPatch |
| VideoCue | `cue/video_cue.rs` | ✅ Uses `output_engine.show_content()` / `stop_voice()` / `pause_voice()` / `resume_voice()` |
| ImageCue | `cue/image_cue.rs` | ✅ Uses `output_engine.show_content()` / `stop_content()`. No DisplayDuration mode (StopOnNextCue only). |
| MemoCue | `cue/memo_cue.rs` | ✅ Complete |
| StopCue | `cue/stop_cue.rs` | ✅ Complete |
| VoiceState / FadeState | `engine/voice.rs` | ✅ Complete — `out_l`, `out_r` for channel routing |
| AudioCommand / AudioStatus | `engine/ring_command.rs` | ✅ Complete |
| DeviceManager / OutputPatch | `engine/device_manager.rs` | ✅ Complete |
| AudioEngine | `engine/audio_engine.rs` | ✅ Complete — WASAPI/ASIO, mixes audio + video PCM in `fill_buffer` |
| OutputEngine | `engine/output_engine.rs` | ✅ Complete — unified libmpv engine for video+image; single persistent Win32 window; dip-to-black fade overlay (`WS_EX_LAYERED`); pre-arm preserved; `toggle_visibility()` |
| mpv_sys (FFI) | `engine/mpv_sys.rs` | ✅ libmpv bindings compile |
| CueList | `show/cue_list.rs` | ✅ Complete |
| Workspace | `show/workspace.rs` | ✅ Complete |
| Transport | `show/transport.rs` | ✅ Complete — `stop_on_next_go()` called before each GO |
| 30fps event loop | `show/event_loop.rs` | ✅ Complete — drains `OutputStatus` |
| Video pre-arm | `show/video_pre_arm.rs` | ✅ Complete — uses `OutputEngine` |
| UndoStack | `show/undo_stack.rs` | ✅ Complete |
| AppState | `state/app_state.rs` | ✅ Complete — `output_engine: Arc<OutputEngine>` |
| Preferences | `preferences.rs` | ✅ Complete — `DisplayPreferences::output_screen: Option<u32>` |
| Transport commands | `commands/transport_cmds.rs` | ✅ Complete |
| Cue commands | `commands/cue_cmds.rs` | ✅ Complete — `toggle_output_window`, `get_output_window_visible` |
| Workspace commands | `commands/workspace_cmds.rs` | ✅ Complete |
| Device commands | `commands/device_cmds.rs` | ✅ Complete |
| Preferences commands | `commands/preferences_cmds.rs` | ✅ Complete — `get_output_screen`, `set_output_screen` |
| Undo commands | `commands/undo_cmds.rs` | ✅ Complete |

### React / TypeScript frontend

| File | Status |
|---|---|
| `lib/types.ts` | ✅ Complete — `ImageCueData` simplified (no `stop_mode`/`display_duration_ms`), `DisplayPreferences` typed |
| `lib/commands.ts` | ✅ Complete — `toggleOutputWindow`, `getOutputWindowVisible`, `getOutputScreen`, `setOutputScreen` |
| `stores/workspaceStore.ts` | ✅ Complete |
| `stores/transportStore.ts` | ✅ Complete |
| `stores/timingStore.ts` | ✅ Complete |
| `hooks/useTauriEvents.ts` | ✅ Complete |
| `hooks/useKeyboardShortcuts.ts` | ✅ Complete — F9 toggles output window |
| `App.tsx` | ✅ Complete — `handleToggleSurface` wired to F9 + View menu |
| `components/CueList/` | ✅ Complete |
| `components/Inspector/InspectorPanel.tsx` | ✅ Complete — audio, video, image |
| `components/Inspector/BasicsTab.tsx` | ✅ Complete — no per-cue screen selector |
| `components/Inspector/TimeTab.tsx` | ✅ Complete — no ImageStopMode UI (StopOnNextCue only, no `isImage` prop) |
| `components/Inspector/LevelsTab.tsx` | ✅ Complete |
| `components/Inspector/FadeTab.tsx` | ✅ Complete — fade-in/out for audio, video, image |
| `components/Transport/TransportBar.tsx` | ✅ Complete |
| `components/Preferences/PreferencesModal.tsx` | ✅ Complete |
| `components/WaveformModal.tsx` | ✅ Complete |
| `main.tsx` | ✅ Simplified — no output-surface branch, renders `<App />` only |

---

## Known issues

### ⚠️ Video/Audio routing — ASIO hardware validation pending (unchanged)

The `ao=pcm` → named pipe → `AudioEngine` architecture compiles and works on default WASAPI.  Still to verify on an ASIO interface:

1. `PCM pipe: mpv connected` logs appear on video GO
2. VU meter moves during video playback
3. Video audio comes out of the ASIO device (not default WASAPI)

---

## Change history

### 0.4.1 — Persistent PCM pipe + pre-arm first-frame fix (2026-05-28)

#### ✅ Root cause fixed: multiple videos broken, no audio on 2nd+ video

**Problem:** `ao=pcm` in mpv keeps the named-pipe connection open across `loadfile` calls. The old code created a new pipe server instance per video, but mpv never reconnected to those new instances — only the first pipe was ever used. Result: every video after the first had no audio and appeared frozen (ring buffer empty, `video_pcm_active` never set).

**Solution:** Single persistent `pcm_pipe_manager` thread (spawned at engine init). It loops: create pipe server → `ConnectNamedPipe` (blocks until mpv connects on first file load) → read samples until pipe closes (mpv exits or goes idle) → repeat.

A global `OUTPUT_PCM_DISCARD: OnceLock<Arc<AtomicBool>>` flag controls routing:
- `true` (idle / pre-arm / image): bytes consumed from OS buffer and discarded so mpv never blocks writing
- `false` (video actively playing): samples pushed to ring buffer for `AudioEngine` mixing

#### ✅ Pre-arm first-frame no longer visible on output

**Problem:** `loadfile pause=yes` during pre-arm still decoded and displayed the first video frame on the output window.

**Solution:** `pre_arm_voice` sets the fade overlay to alpha=255 (fully black) before sending `loadfile`. When GO activates the armed voice, `activate_armed_voice` restores alpha=0 (or starts a fade-in animation if configured).

**Files changed:**
- `engine/output_engine.rs` — persistent `pcm_pipe_manager` replaces per-video `handle_pcm_pipe_connection`; `ArmedVoice` simplified (removed `pipe_handle`/`ready`/`cancelled`); `OutputEngine` struct simplified (removed `armed_pipe_discard`); `mpv_event_loop` signature simplified (removed `pipe_discard_flag` param); `stop_content` now resets `video_pcm_active` and `OUTPUT_PCM_DISCARD`; `MPV_EVENT_END_FILE` EOF resets audio flags; pre-arm sets overlay alpha=255

---

### 0.4.0 — Unified OutputEngine (Win32 + libmpv) (2026-05-28)

#### ✅ Single persistent Win32 window for all visual cues

**Problem solved:** Two separate window technologies (Tauri WebviewWindow for images, Win32 native for video) caused window disappearing between cues, new window appearing at different position — unusable for professional events.

**New architecture:**
- `engine/output_engine.rs` — new unified engine replacing both `VideoEngine` (for display) and `ImageEngine`
- Single persistent `WS_POPUP` Win32 window created at startup, always visible (black when idle), never closed
- libmpv renders both video files (`loadfile video.mp4`) and image files (`loadfile img.jpg audio=no,image-display-duration=inf`)
- Fade overlay: child window with `WS_EX_LAYERED | WS_EX_TRANSPARENT`, alpha animated via 16 ms Win32 timer for dip-to-black transitions
- Per-cue configurable `fade_in_ms` / `fade_out_ms`; default is no fade (cut)
- `Hard Stop` always cuts immediately (bypasses fade_out)
- Fade-bypass bug fixed: `show_content()` correctly applies the previous cue's `fade_out_ms` before loading new content
- First-GO freeze eliminated: mpv instance created at `OutputEngine::new()` (not lazily on first GO)
- F9 shortcut toggles output window visibility; View menu also has the option
- Pre-arm mechanism preserved intact (uses `OutputEngine` instead of `VideoEngine`)
- Cross-stop rule preserved: any new cue GO stops the currently playing visual content

**Files changed:**
- `engine/output_engine.rs` — new file (~700 lines)
- `engine/mod.rs` — removed `ImageEngine`/`VideoEngine` exports, added `OutputEngine`
- `cue/context.rs` — `output_engine: Arc<OutputEngine>` replaces `video_engine + image_engine`
- `cue/image_cue.rs` — removed `ImageStopMode`, uses `output_engine.show_content()`/`stop_content()`
- `cue/video_cue.rs` — uses `output_engine.show_content()`/`stop_voice()`/`pause_voice()`/`resume_voice()`
- `show/event_loop.rs` — drains `OutputStatus` instead of `VideoStatus + ImageStatus`
- `show/video_pre_arm.rs` — uses `OutputEngine`
- `state/app_state.rs` — `output_engine: Arc<OutputEngine>`
- `lib.rs` — constructs `OutputEngine::new(audio_engine)`, removed `ImageEngine`/`surface_pinned`
- `commands/cue_cmds.rs` — removed image surface commands, added `toggle_output_window`/`get_output_window_visible`
- `commands/transport_cmds.rs` — uses `output_engine`
- `src/main.tsx` — simplified (no output-surface branch)
- `src/lib/commands.ts` — removed image surface commands, added `toggleOutputWindow`/`getOutputWindowVisible`
- `src/lib/types.ts` — removed `ImageStopMode`, simplified `ImageCueData`
- `src/App.tsx` — `handleToggleSurface` toggle handler, View menu uses `onToggle`
- `src/hooks/useKeyboardShortcuts.ts` — F9 → `onToggleOutputWindow`
- `src/components/Inspector/TimeTab.tsx` — removed `isImage` prop, removed stop_mode/display_duration controls
- `src/components/Inspector/InspectorPanel.tsx` — removed `isImage` from `<TimeTab>` call

**Backward compatibility:** old `.wincue` files containing `ImageStopMode`, `display_duration_ms`, or per-cue `screen_index` load silently — fields are ignored by serde.

---

### 0.3.2 — Unified output surface (2026-04-28)

#### ✅ Single output surface for all visual cues (Tauri WebviewWindow era)

- `preferences.rs` — `DisplayPreferences::output_screen: Option<u32>` (serde default `None`)
- `preferences_cmds.rs` — `get_output_screen` / `set_output_screen` Tauri commands
- `cue/context.rs` — `output_screen: Option<u32>` snapshot field in `CueContext`
- `engine/video_engine.rs` — `position_window` (no `ShowWindow`) + `show_window`; pre-arm only calls `position_window`
- `engine/image_engine.rs` — `Option<SurfaceInfo>` (was `HashMap`); fixed label `"output-surface"`
- `cue/video_cue.rs` — removed `screen_index`; calls `image_engine.hard_stop_all()` on GO
- `cue/image_cue.rs` — removed `screen_index`; calls `video_engine.stop_current_voice(0)` on GO
- `components/Preferences/PreferencesModal.tsx` — Display tab with screen selector
- `components/Inspector/BasicsTab.tsx` — per-cue screen selector removed
- `components/ImageSurface.tsx` — `isFloating` derived from `get_output_screen` at mount
- `src/main.tsx` — label check `=== "output-surface"`
- `capabilities/image-surface.json` — window pattern `"output-surface"`

---

### 0.3.1 — Image Cue fully functional (2026-04-22)

- Persistent `WebviewWindow` per screen, hidden between cues (no close/reopen flicker)
- `stop_on_next_go()` trait method; `ImageStopMode` enum
- Direct DOM manipulation for reliable fade-in/out under React 18 batching
- Draggable floating window via `win.startDragging()`

---

### 0.3.0 — Image Cue type added (non-functional) (2026-04-19)

- `cue/image_cue.rs` skeleton; workspace serialization OK; GO froze the app (fixed in 0.3.1)

---

### 0.2.0 — Audio/video architecture overhaul (2026-04-14)

- ASIO SDK + `CPAL_ASIO_DIR` build fix
- `Voice.out_l / out_r` + `OutputPatch` routing wired
- `ao=pcm` → named pipe → `AudioEngine` for video audio mixing
- VU meter: rAF-based decay, peak hold, red needle > -6 dBFS
- Video Cue playback: D3D11, loop, fullscreen double-click, drag, focus fix

---

### 0.1.2 (2026-04-11)
- Stop Cue, drag & drop rework, immediate Auto-Continue fix, loop fix, duplicate/paste fix

---

### 0.1.1 (2026-04-11)
- `CueList::renumber_all()`, `set_master_volume`, shortcuts, CurveSelect, TransportBar rework

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
| 13. Video Cue | ✅ Freeze fixed, unified OutputEngine, dip-to-black fades |
| 14. Image Cue | ✅ Unified OutputEngine, dip-to-black fades, stop-on-next-cue |
| 15. Stop Cue | ✅ Functional |

---

## Next priorities

1. **⚠️ Validate video routing on ASIO hardware** — verify `PCM pipe: mpv connected` + VU meter + audio output
2. **Future cue types** — Wait, Fade, Group, MIDI, OSC (architecture is ready; add via `CueRegistry` without touching transport)
