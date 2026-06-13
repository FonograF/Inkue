# WinCue — Project state as of 2026-06-13

## Current version: 0.7.0

## cargo build result

**Compiles without errors, zero warnings.**

## cargo test result

**57 tests pass, 0 failures.**

---

## Cue type status

| Cue type | Status | Details |
|---|---|---|
| Audio | ✅ **100% functional** | Pre/post-wait, fade-in/out, loop, rate, Output Patch routing, pan, master volume, waveform, VU meter, scrub/seek; pause/resume with correct elapsed tracking |
| Stop  | ✅ **Functional** | QLab-style: target All Cues or a specific cue number; Soft (fade) or Hard (cut); Auto-Follow bug fixed |
| Memo  | ✅ **Functional** | Read-only, no audio action |
| Video | ✅ **Functional** | Single persistent Win32 window, paused-load start (no frame-0 freeze), dip-to-black fades, scrub/seek; pause/resume with correct elapsed tracking |
| Image | ✅ **Functional** | Same Win32 window as Video via libmpv, dip-to-black fades; stop-on-next-cue only fires for visual GOs (audio GO leaves image running) |
| Group | ✅ **Functional** | Sequential and parallel modes; holds playhead in sequential mode; GO absorption for mid-sequence resume; drag-into-group |
| Wait  | ✅ **Functional** | Fixed duration delay cue; registered in CueRegistry |
| Fade  | ✅ **Functional** | Targets a running cue by number; interpolates its volume from current level to target dB over a configurable duration; configurable curve (Linear/S-Curve/Exponential); optional Stop at End; pause/resume supported |
| OSC   | ✅ **Functional** | Sends UDP OSC messages on GO; multiple messages per cue; inspector Messages tab + Test send button; workspace-level patches; receive server with IP allowlist + dedup cache; /wincue/pause_toggle; /wincue/select/next|previous |
| MIDI  | ✅ **Functional** | Sends Note On/Off, CC, Program Change on GO; multiple messages per cue; dynamic port enumeration (midir); inspector Messages tab + Test send button; cross-platform (WinMM/CoreMIDI) |

---

## What is implemented and compiles

### Rust backend

| Module | File | Status |
|---|---|---|
| Cue types | `cue/types.rs` | ✅ Complete |
| Cue trait | `cue/traits.rs` | ✅ Complete — `stop_on_next_go()`, `stop_specification()` |
| CueRegistry | `cue/registry.rs` | ✅ Complete |
| CueContext | `cue/context.rs` | ✅ Complete — `audio_engine`, `output_engine`, `stop_fade_ms`, `output_patches`, `output_screen` |
| AudioCue | `cue/audio_cue.rs` | ✅ 100% functional — pre-wait, fade-in/out, loop, rate, `Voice.out_l/r` routing via OutputPatch; pause freezes elapsed (elapsed_before_pause accumulators); seek works while paused |
| VideoCue | `cue/video_cue.rs` | ✅ Uses `output_engine.show_content()` / `stop_voice()` / `pause_voice()` / `resume_voice()`; pause freezes elapsed; seek works while paused |
| ImageCue | `cue/image_cue.rs` | ✅ Uses `output_engine.show_content()` / `stop_content()`. No DisplayDuration mode (StopOnNextCue only). |
| MemoCue | `cue/memo_cue.rs` | ✅ Complete |
| StopCue | `cue/stop_cue.rs` | ✅ Complete — `target_cue_number` (None = all), `hard_stop_mode`; `stop_specification()` drives transport inline |
| VoiceState / FadeState | `engine/voice.rs` | ✅ Complete — `out_l`, `out_r` for channel routing |
| AudioCommand / AudioStatus | `engine/ring_command.rs` | ✅ Complete |
| DeviceManager / OutputPatch | `engine/device_manager.rs` | ✅ Complete |
| AudioEngine | `engine/audio_engine.rs` | ✅ Complete — WASAPI/ASIO, mixes audio + video PCM in `fill_buffer` |
| OutputEngine | `engine/output_engine/` | ✅ Complete — unified libmpv engine for video+image; single persistent Win32 window; dip-to-black fade overlay (`WS_EX_LAYERED`); OSD timer overlay; `toggle_visibility()` |
| OscPatch | `engine/osc_patch.rs` | ✅ Complete — named UDP send target (id, name, ip, port) |
| OscServer | `engine/osc_server.rs` | ✅ Complete — UDP listener, IP allowlist, 50ms hash dedup cache, dispatch to frontend via `osc-command` event, activity via `osc-activity`, debug via `osc-debug` |
| mpv_sys (FFI) | `engine/mpv_sys.rs` | ✅ libmpv bindings compile |
| CueList | `show/cue_list.rs` | ✅ Complete |
| Workspace | `show/workspace.rs` | ✅ Complete — `active_cue_list_id: Uuid`; `cue_list_by_id()` / `cue_list_by_id_mut()` |
| Transport | `show/transport.rs` | ✅ Complete — returns `GoResult { triggered, stopped }`; `stop_on_next_go()` visual-only guard; `stop_specification()` executed before Auto-Follow chain |
| Event loop | `show/event_loop.rs` | ✅ Complete — processes ALL cue lists each tick (completion, tick, auto-continue/follow); per-list `should_go_lists`; OSC feedback from active list only |
| UndoStack | `show/undo_stack.rs` | ✅ Complete |
| AppState | `state/app_state.rs` | ✅ Complete — `osc_server: Arc<OscServer>`, `last_go_at: AtomicU64` for double-GO protection |
| Preferences | `preferences.rs` | ✅ Complete — `DisplayPreferences` + `OscReceiveConfig` (machine-level) |
| Transport commands | `commands/transport_cmds.rs` | ✅ Complete — double-GO protection (500 ms default) |
| Cue commands | `commands/cue_cmds.rs` | ✅ Complete — `toggle_output_window`, `get_output_window_visible` |
| Cue List commands | `commands/cue_list_cmds.rs` | ✅ Complete — `get_cue_lists`, `add_cue_list`, `remove_cue_list`, `rename_cue_list`, `set_active_cue_list`; emits `cue-lists-changed` |
| OSC commands | `commands/osc_cmds.rs` | ✅ Complete — `list/add/update/remove_osc_patch`, `get/set_osc_config`, `send_osc_test` |
| Workspace commands | `commands/workspace_cmds.rs` | ✅ Complete |
| Device commands | `commands/device_cmds.rs` | ✅ Complete |
| Preferences commands | `commands/preferences_cmds.rs` | ✅ Complete — `get/set_output_screen`, `update_display_preferences` (applies timer style to mpv), `list_system_fonts` (GDI enum), `preview_output_timer` |
| Undo commands | `commands/undo_cmds.rs` | ✅ Complete |

### React / TypeScript frontend

| File | Status |
|---|---|
| `lib/types.ts` | ✅ Complete — `CueListSummary`, `CueListsChangedEvent` added |
| `lib/commands.ts` | ✅ Complete — `getCueLists`, `addCueList`, `removeCueList`, `renameCueList`, `setActiveCueList` added |
| `stores/workspaceStore.ts` | ✅ Complete — `cueLists`, `activeCueListId`, `refreshCueLists`, `setCueLists` added |
| `stores/transportStore.ts` | ✅ Complete — `oscActivityAt`, `oscLog`, `markOscActivity`, `addOscLog` |
| `stores/timingStore.ts` | ✅ Complete |
| `hooks/useTauriEvents.ts` | ✅ Complete — handles `cue-lists-changed` → `setCueLists` + `refreshCues` |
| `components/CueList/CueListTabs.tsx` | ✅ Complete — tab bar, add/rename/delete lists, active tab highlight, double-click rename, right-click context menu |
| `hooks/useKeyboardShortcuts.ts` | ✅ Complete — F9 toggles output window |
| `App.tsx` | ✅ Complete — `+ OSC` toolbar button |
| `components/CueList/` | ✅ Complete |
| `components/Inspector/InspectorPanel.tsx` | ✅ Complete — audio, video, image, stop, OSC (Messages tab) |
| `components/Inspector/OscTab.tsx` | ✅ Complete — messages list, patch selector, arg editor, Test send button |
| `components/OscPatches/OscPatchesPanel.tsx` | ✅ Complete — add/edit/remove OSC patches |
| `components/Inspector/BasicsTab.tsx` | ✅ Complete — Stop Cue: Target selector + Cue # field + Stop Mode selector |
| `components/Inspector/TimeTab.tsx` | ✅ Complete |
| `components/Inspector/LevelsTab.tsx` | ✅ Complete |
| `components/Inspector/FadeTab.tsx` | ✅ Complete |
| `components/Transport/TransportBar.tsx` | ✅ Complete — OSC activity dot + monitor toggle |
| `components/Osc/OscMonitor.tsx` | ✅ Complete — real-time packet log, matched/unknown indicator |
| `components/Preferences/PreferencesModal.tsx` | ✅ Complete — Network tab with OSC receive config + patches |
| `components/WaveformModal.tsx` | ✅ Complete |
| `main.tsx` | ✅ Complete |

---

---

## Change history additions (0.6.2)

### Stop Cue redesign — QLab semantics + Auto-Follow bug fix (2026-06-13)

#### ✅ Stop Cue is now QLab-compatible

**Problems fixed:**

1. **Auto-Follow killed the chained cue** — with `Auto-Follow` set on a Stop Cue, the stop action was delivered via `CueEvent::StopAll` through a channel that was drained in `transport_cmds.rs` *after* `transport.go()` had already chained the next cue. The chained cue started, then was immediately killed by `stop_all()`.

2. **Stop All only** — the Stop Cue could only stop every running cue globally. QLab lets you target a specific cue by number.

3. **No stop mode choice** — no option for immediate cut vs. fade out.

**Solution:**

- `StopCue` gains two fields: `target_cue_number: Option<String>` (None = all, Some = specific cue number) and `hard_stop_mode: bool`.
- The new `stop_specification()` method on the `Cue` trait (default: `None`) lets Stop Cue declare its action. Transport reads it and executes the stop **inline inside `transport.go()`**, before the `chain_now` / Auto-Follow evaluation. The chained cue therefore starts on a clean state.
- The fragile `CueEvent::StopAll` channel mechanism is removed entirely.
- `transport.go()` now returns `GoResult { triggered: Vec<CueId>, stopped: Vec<CueId> }` so callers can emit `cue-state-changed` for both sets.
- Inspector Basics tab shows: **Target** (All Cues / Specific Cue…), **Cue #** (when targeting a specific cue), **Stop Mode** (Soft / Hard).

**Image Cue: audio GO no longer cuts the image (2026-06-13)**

- `stop_on_next_go()` returning `true` for Image Cues caused any GO — including audio — to stop the displayed image.
- Fix: `transport.go()` now checks whether the incoming cue is visual (`CueType::Video | CueType::Image`). A running Image or Video Cue with `stop_on_next_go()` is only stopped when the new GO is also visual.

**Files changed:** `cue/stop_cue.rs`, `cue/traits.rs`, `cue/context.rs`, `show/transport.rs`, `show/event_loop.rs`, `commands/transport_cmds.rs`, `src/lib/types.ts`, `src/components/Inspector/InspectorPanel.tsx`, `src/components/Inspector/BasicsTab.tsx`

---

## Change history additions (0.6.1)

### Pause / Resume fixes

- **Elapsed time now freezes on pause** — `AudioCue` and `VideoCue` gained `elapsed_before_pause` / `action_elapsed_before_pause` accumulators. `pause()` snapshots the current elapsed (using `=` not `+=`), `resume()` re-anchors the `Instant` so only actual play-time counts. `elapsed()` / `action_elapsed()` return the frozen values when paused.
- **Progress bar freezes orange** — event loop now emits `cue-time-update` for Paused cues; frontend no longer calls `clearTiming` on pause (only on standby/completed).
- **Seek while paused** — `seek()` now accepts `state == Paused`; updates `action_elapsed_before_pause` directly so the inspector and progress bar update immediately on the next 30 fps tick.

### OSC improvements (0.6.0 → 0.6.1)

- `/wincue/pause_toggle` — single button pauses all running cues or resumes all paused cues.
- `/wincue/select/next` and `/wincue/select/previous` — move playhead without firing GO.
- **Dedup cache** (50 ms hash window) — eliminates Windows UDP loopback duplicates and OSC controllers that send each packet twice.
- **OSC Monitor** — real-time packet log, click the activity dot in the transport bar; matched addresses shown in green, unknown in orange.
- **Test send button** — each message row in the OSC inspector has a `▶ Test send` button that sends the message immediately and shows the result inline.
- **Double-GO protection** — enforced in `go()` using `double_go_protection_ms` (default 500 ms, configurable in Preferences → General).

**Files changed:** `cue/audio_cue.rs`, `cue/video_cue.rs`, `engine/osc_server.rs`, `show/event_loop.rs`, `commands/osc_cmds.rs`, `commands/transport_cmds.rs`, `state/app_state.rs`, `hooks/useTauriEvents.ts`, `stores/transportStore.ts`, `components/Transport/TransportBar.tsx`, `components/Osc/OscMonitor.tsx`, `components/Inspector/OscTab.tsx`

---

## Change history additions (0.5.1)

### Group Cue polish

- **Drag cue into group**: top-level cues can now be dragged and dropped onto the middle of a Group row in the cue list — the cue becomes a child of that group. Works both for cue-to-cue drag (the existing reorder drag) and OS file drag-drop (dropping a media file on a Group creates the new cue as a child).
- **Color indicator indent**: child cues inside a group have their left color strip shifted right by `depth × 4 px` (one indicator width per nesting level), visually distinguishing them from top-level cues without affecting content alignment.
- **Sequential Group GO absorption**: when a Sequential Group is running and the current child has finished with `DoNotContinue` (sequence paused mid-way), pressing Space/GO fires the next sequential child instead of advancing the outer Playhead. When all children are exhausted, Space/GO resumes normal outer-playhead behavior.

**Files changed:**
- `cue/traits.rs` — `absorbs_go()` default trait method
- `cue/group_cue.rs` — `has_next_sequential_child()` helper; `absorbs_go()` impl; `go()` modified to handle mid-sequence absorption
- `show/transport.rs` — checks `absorbs_go()` before advancing the outer Playhead
- `components/CueList/CueRow.tsx` — color strip replaced `borderLeft` with abs-positioned `<div>` at `depth * 4 px`; `isGroupDropTarget` prop (cyan outline); `data-is-group` / `data-cue-depth` data attrs
- `components/CueList/CueListView.tsx` — `calcDropTarget()` replaces `calcInsertIdxFromY()` for cue drag; `flatItemsRef`; `dropTargetGroupId` state; cue-drag `onUp` calls `addCueToGroup`; `resolveFileDragMode` returns `groupId`; file-drop handler creates child cues in group; `dragOverGroupId` state for visual feedback

---

## Known issues

### Long-video A/V drift (minor, future tuning)

Video frames are timed by mpv's display clock; the video's audio voice plays on
the cpal device clock.  These are independent oscillators, so over a long video
(several minutes) audio and video can drift by a few ms.  For typical event
clips this is imperceptible.  Future refinement: periodically nudge the audio
voice rate to track mpv's `time-pos`.  Looping videos re-align at each loop only
to within this drift.

---

## Change history

### 0.4.2 — Video freeze fixed: muted mpv + separate audio voice (2026-05-30)

#### ✅ Root cause fixed: frozen first frame / replay hang from `ao=pcm`

**Problem.** Two layered faults. (1) On GO mpv `loadfile` started **playing
immediately** while the d3d11 decoder was still warming up, so frame 0 sat frozen
while audio ran ahead.  (2) The deeper cause: video audio was piped out of mpv
via `ao=pcm` into the AudioEngine.  `ao=pcm` gives mpv **no real audio clock**, so
pacing it required back-pressuring its audio writes — which stalls mpv's demuxer
and starves the video decoder (mpv itself logged *"Audio/Video desynchronisation
detected"*).  Replaying a cue could deadlock the whole app on the pipe state
machine.  This had been patched every release (pre-arm, discard throttle, PCM
gate, pipe resizing) without ever fixing the clock.

**Solution — mpv plays video muted; the audio track is a normal AudioEngine voice.**
- mpv is initialised with `ao=null` + `audio=no`: it renders **video only**, so
  its display clock is never perturbed by audio sync and never freezes.
- Each video's audio track is decoded with symphonia (shared
  `cue/media_decode::decode_audio_track`, which selects the first audio track so
  it works on `.mp4` containers) and played as an ordinary `Voice` — inheriting
  **Output Patch routing, master volume, VU metering and fades**, exactly like an
  Audio Cue.  This is the unified professional signal path the project wants.
- **Lockstep start:** the audio voice is submitted *paused* at GO
  (`play_voice_paused`).  The video is loaded *paused*; the first
  `MPV_EVENT_PLAYBACK_RESTART` (frame 0 decoded, decoder warm) reveals the
  overlay, unpauses mpv **and** resumes the audio voice — both from frame 0, so
  there is no A/V offset and no warmup freeze.
- Stop / pause / resume / cross-stop / EOF drive the paired audio voice in step
  with the video; a never-revealed paused voice hard-stops (no blip).
- The entire `ao=pcm` named-pipe machinery is deleted, removing the desync source
  **and** the replay deadlock.  A 2.5 s watchdog still guarantees the output can
  never hang on a permanent black screen if `PLAYBACK_RESTART` is ever missing.

**Files changed:**
- `cue/media_decode.rs` — **new** shared audio-track decoder (Option-returning;
  selects the first audio track, so it decodes a video container's audio)
- `cue/audio_cue.rs` — `decode_file` delegates to the shared decoder
- `cue/video_cue.rs` — decodes its audio track (`load` / `accept_preloaded_audio`
  / `extract_decoded_audio`); builds + submits the paused audio voice and hands
  its id to the OutputEngine
- `engine/audio_engine.rs` — `play_voice_paused`; `Stop` hard-stops a paused
  voice; removed all video-PCM plumbing
- `engine/output_engine/mod.rs` — `ao=null`/`audio=no`; `OUTPUT_CURRENT_AUDIO_VOICE`;
  `show_content` takes the audio voice id and cross-stops the previous one;
  pause/resume/stop/volume drive the paired audio; PCM thread removed
- `engine/output_engine/fade.rs` — video loads paused with `audio=no`
- `engine/output_engine/mpv_events.rs` — resumes the audio voice at the first
  frame; stops it on EOF/error; event loop now takes the `AudioEngine`
- `engine/output_engine/pcm_pipe.rs` — **deleted**
- `commands/cue_cmds.rs`, `commands/workspace_cmds.rs` — background-decode video
  audio on file-assign and on workspace load

---

### 0.4.1 — Persistent PCM pipe (2026-05-28)

#### ✅ Root cause fixed: multiple videos broken, no audio on 2nd+ video

**Problem:** `ao=pcm` in mpv keeps the named-pipe connection open across `loadfile` calls. The old code created a new pipe server instance per video, but mpv never reconnected to those new instances — only the first pipe was ever used. Result: every video after the first had no audio and appeared frozen (ring buffer empty, `video_pcm_active` never set).

**Solution:** Single persistent `pcm_pipe_manager` thread (spawned at engine init). It loops: create pipe server → `ConnectNamedPipe` (blocks until mpv connects on first file load) → read samples until pipe closes (mpv exits or goes idle) → repeat.

A global `OUTPUT_PCM_DISCARD: OnceLock<Arc<AtomicBool>>` flag controls routing:
- `true` (idle / image): bytes consumed from OS buffer and discarded so mpv never blocks writing
- `false` (video actively playing): samples pushed to ring buffer for `AudioEngine` mixing

**Files changed:**
- `engine/output_engine.rs` — persistent `pcm_pipe_manager` replaces per-video `handle_pcm_pipe_connection`; `stop_content` now resets `video_pcm_active` and `OUTPUT_PCM_DISCARD`; `MPV_EVENT_END_FILE` EOF resets audio flags

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
- Cross-stop rule preserved: any new cue GO stops the currently playing visual content

**Files changed:**
- `engine/output_engine.rs` — new file (~700 lines)
- `engine/mod.rs` — removed `ImageEngine`/`VideoEngine` exports, added `OutputEngine`
- `cue/context.rs` — `output_engine: Arc<OutputEngine>` replaces `video_engine + image_engine`
- `cue/image_cue.rs` — removed `ImageStopMode`, uses `output_engine.show_content()`/`stop_content()`
- `cue/video_cue.rs` — uses `output_engine.show_content()`/`stop_voice()`/`pause_voice()`/`resume_voice()`
- `show/event_loop.rs` — drains `OutputStatus` instead of `VideoStatus + ImageStatus`
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
- `engine/video_engine.rs` — `position_window` (no `ShowWindow`) + `show_window`
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
| 13. Video Cue | ✅ Freeze fixed, unified OutputEngine, hard-cut stop, scrub/seek |
| 14. Image Cue | ✅ Unified OutputEngine, hard-cut stop, stop-on-next-cue |
| 15. Stop Cue | ✅ Functional |
| 16. Multi-select | ✅ Ctrl/Shift/Ctrl+A; multi-delete, multi-duplicate, multi-drag, multi-color |
| 17. Scrub/seek | ✅ Audio + video; ScrubBar in Inspector Time tab |
| 18. Group Cue | ✅ Sequential + parallel modes; GO absorption; drag-into-group |
| 19. Wait Cue | ✅ Fixed duration delay; registered in CueRegistry |
| 20. Output timer | ✅ OSD via mpv; 60fps thread; font/size/position/margin/ms; live preview |
| 21. OSC Cue | ✅ Send multiple OSC messages on GO; workspace patches; inspector Messages tab; receive server with allowlist; Preferences OSC tab; activity dot in transport bar |
| 22. Fade Cue | ✅ Volume fade to target dB, configurable curve (Linear/S-Curve/Exponential), stop-at-end, pause/resume, pre-wait |

---

## Next priorities

See `WHATSNEXT.md` for the full roadmap with effort estimates.
2. **Optional: active A/V resync** — nudge the video audio voice's rate to track
   mpv `time-pos` for drift-free long videos / tight loops (see Known issues).
3. **ASIO→WASAPI Output Patch validation** — routing wired, needs hardware test.
