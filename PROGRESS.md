# WinCue — Project state as of 2026-06-20

## Current version: 0.9.2

## cargo build result

**Compiles without errors, zero warnings** — default (winit/GL) **and**
`--features legacy-win32-output`.

## cargo test result

**65 tests pass, 0 failures.**

---

## Cue type status

| Cue type | Status | Details |
|---|---|---|
| Audio | ✅ **100% functional** | Pre/post-wait, fade-in/out, loop (finite + infinite), rate, Output Patch routing, pan, master volume, waveform, VU meter, scrub/seek; pause/resume with correct elapsed tracking; SR conversion in `fill_buffer` (44.1k/48k/96k all correct) |
| Stop  | ✅ **Functional** | UUID-based targeting; multi-target (stop any subset of cues); target All Cues or specific cues; Soft (fade) or Hard (cut) |
| Memo  | ✅ **Functional** | Read-only, no audio action |
| Video | ✅ **Functional** | Unified GL Render API path (Windows); paused-load start (no frame-0 freeze), dip-to-black fades (GL quad), scrub/seek; pause/resume; loop (finite + infinite) |
| Image | ✅ **Functional** | Same GL output window as Video via libmpv Render API; dip-to-black fades; stop-on-next-cue only fires for visual GOs (audio GO leaves image running); loop support |
| Group | ✅ **Functional** | Sequential and parallel modes; holds playhead in sequential mode; GO absorption for mid-sequence resume; drag-into-group |
| Wait  | ✅ **Functional** | Fixed duration delay cue; registered in CueRegistry |
| Fade  | ✅ **Functional** | UUID-based multi-target (any subset of cues); audio fade (gain interpolation at 30 fps); visual fade for Video/Image (overlay alpha interpolation at 30 fps, `set_overlay_alpha_direct`); configurable curve; optional Stop at End; context-aware inspector (volume dB for audio/video, brightness % for image, both for video) |
| OSC   | ✅ **Functional** | Sends UDP OSC messages on GO; multiple messages per cue; inspector Messages tab + Test send button; workspace-level patches; receive server with IP allowlist + dedup cache; /wincue/pause_toggle; /wincue/select/next\|previous |
| MIDI  | ✅ **Functional** | Sends Note On/Off, CC, Program Change on GO; multiple messages per cue; dynamic port enumeration (midir); inspector Messages tab + Test send button; cross-platform (WinMM/CoreMIDI) |

---

## What is implemented and compiles

### Rust backend

| Module | File | Status |
|---|---|---|
| Cue types | `cue/types.rs` | ✅ Complete |
| Cue trait | `cue/traits.rs` | ✅ Complete — `stop_on_next_go()`, `stop_specification()` (Vec), `set_fade_voices()`, `resolve_fade_targets()` |
| CueRegistry | `cue/registry.rs` | ✅ Complete |
| CueContext | `cue/context.rs` | ✅ Complete — `audio_engine`, `output_engine`, `stop_fade_ms`, `output_patches`, `output_screen` |
| AudioCue | `cue/audio_cue.rs` | ✅ 100% functional — pre-wait, fade-in/out, loop (finite + infinite, `u32::MAX`), rate, Output Patch routing, pan; pause freezes elapsed; seek while paused; SR correction in `fill_buffer` |
| VideoCue | `cue/video_cue.rs` | ✅ Uses `output_engine.show_content()` / `stop_voice()` / `pause_voice()` / `resume_voice()`; loop support; `file_duration()` override returns raw `cached_duration` |
| ImageCue | `cue/image_cue.rs` | ✅ `display_duration_ms: Option<u64>` — None = hold, Some = timed auto-complete |
| MemoCue | `cue/memo_cue.rs` | ✅ Complete |
| StopCue | `cue/stop_cue.rs` | ✅ UUID-based multi-target (`target_cue_ids: Vec<CueId>`); empty = stop all; backward-compat with old single-UUID format; `resolve_stop_target` handles number→UUID migration |
| FadeCue | `cue/fade_cue.rs` | ✅ UUID-based multi-target (`target_cue_ids: Vec<CueId>`); audio fade via `audio_engine.set_voice_gain()` at 30 fps; visual fade via `output_engine.set_overlay_alpha_direct()` at 30 fps; `has_visual_target` + `visual_start/target_alpha`; `stop_at_end` for audio + visual; backward-compat with old `target_cue_number` |
| VoiceState / FadeState | `engine/voice.rs` | ✅ Complete — `out_l`, `out_r` for channel routing |
| AudioCommand / AudioStatus | `engine/ring_command.rs` | ✅ Complete |
| DeviceManager / OutputPatch | `engine/device_manager.rs` | ✅ Complete |
| AudioEngine | `engine/audio_engine.rs` | ✅ Complete — WASAPI/ASIO; SR conversion in `fill_buffer`; infinite loop (`loops_remaining = u32::MAX`) never sends Completed; 5 unit tests |
| OutputEngine | `engine/output_engine/` | ✅ Complete — unified GL Render API (Stage 1); `vo=libmpv`; winit GL window (Windows; macOS/Linux Stage 2 TODOs); mpv_render_context; GL fade quad; OSD + floating timer; `get_overlay_alpha()`, `set_overlay_alpha_direct()`; legacy Win32+D3D11 behind `legacy-win32-output` feature flag |
| OscPatch | `engine/osc_patch.rs` | ✅ Complete |
| OscServer | `engine/osc_server.rs` | ✅ Complete — UDP listener, IP allowlist, 50ms hash dedup cache |
| mpv_sys (FFI) | `engine/mpv_sys.rs` | ✅ libmpv bindings compile |
| CueList | `show/cue_list.rs` | ✅ Complete — `resolve_fade_targets` called alongside `resolve_stop_target` on load |
| Workspace | `show/workspace.rs` | ✅ Complete |
| Transport | `show/transport.rs` | ✅ Complete — stop spec handles `Vec<CueId>` (empty = all); fade spec resolves audio voices + triggers visual fade via `set_overlay_alpha_direct` |
| Event loop | `show/event_loop.rs` | ✅ Complete — per-loop progress bar uses `file_duration_ms` modulo |
| UndoStack | `show/undo_stack.rs` | ✅ Complete |
| AppState | `state/app_state.rs` | ✅ Complete |
| Preferences | `preferences.rs` | ✅ Complete |
| Transport commands | `commands/transport_cmds.rs` | ✅ Complete — infinite-loop GO fix: uses `file_duration().is_none()` instead of `duration().is_none()` for loading guard |
| Cue commands | `commands/cue_cmds.rs` | ✅ Complete — `CueSummary` now includes `notes`, `file_duration_ms` |
| Cue List commands | `commands/cue_list_cmds.rs` | ✅ Complete |
| OSC commands | `commands/osc_cmds.rs` | ✅ Complete |
| Workspace commands | `commands/workspace_cmds.rs` | ✅ Complete |
| Device commands | `commands/device_cmds.rs` | ✅ Complete |
| Preferences commands | `commands/preferences_cmds.rs` | ✅ Complete |
| Undo commands | `commands/undo_cmds.rs` | ✅ Complete |

### React / TypeScript frontend

| File | Status |
|---|---|
| `lib/types.ts` | ✅ Complete — `CueSummary` + `notes`, `file_duration_ms`; `StopCueData` / `FadeCueData` use `target_cue_ids[]` |
| `lib/commands.ts` | ✅ Complete |
| `stores/workspaceStore.ts` | ✅ Complete |
| `stores/transportStore.ts` | ✅ Complete |
| `stores/timingStore.ts` | ✅ Complete |
| `hooks/useTauriEvents.ts` | ✅ Complete |
| `components/CueList/columns.ts` | ✅ Complete — `notes` + `stop_btn` columns added |
| `components/CueList/CueListTabs.tsx` | ✅ Complete |
| `components/CueList/CueRow.tsx` | ✅ Complete — `notes` cell (truncated + tooltip); `stop_btn` cell (`StopButton` component, visible only when running/paused); per-loop progress bar via `file_duration_ms % loop`; `onStop` prop |
| `hooks/useKeyboardShortcuts.ts` | ✅ Complete |
| `App.tsx` | ✅ Complete |
| `components/CueList/CueListView.tsx` | ✅ Complete — passes `onStop` to CueRow |
| `components/Inspector/InspectorPanel.tsx` | ✅ Complete |
| `components/Inspector/OscTab.tsx` | ✅ Complete |
| `components/OscPatches/OscPatchesPanel.tsx` | ✅ Complete |
| `components/Inspector/BasicsTab.tsx` | ✅ Complete — Stop/Fade: `CueCheckboxList` multi-select; Fade: context-aware UI (volume dB / brightness % / both) |
| `components/Inspector/TimeTab.tsx` | ✅ Complete — Loop control (checkbox + count + ∞ toggle); scrubber shows for infinite loop using `file_duration_ms` |
| `components/Inspector/ScrubBar.tsx` | ✅ Complete — `loopDurationMs` prop for per-loop modulo display |
| `components/Inspector/LevelsTab.tsx` | ✅ Complete |
| `components/Inspector/FadeTab.tsx` | ✅ Complete |
| `components/Transport/TransportBar.tsx` | ✅ Complete |
| `components/Osc/OscMonitor.tsx` | ✅ Complete |
| `components/Preferences/PreferencesModal.tsx` | ✅ Complete |
| `components/WaveformModal.tsx` | ✅ Complete |
| `main.tsx` | ✅ Complete |

---

## Known issues

### Long-video A/V drift (minor, future tuning)

Video frames are timed by mpv's display clock; the video's audio voice plays on
the cpal device clock. These are independent oscillators, so over a long video
(several minutes) audio and video can drift by a few ms. For typical event clips
this is imperceptible. Future refinement: periodically nudge the audio voice rate
to track mpv's `time-pos`. Looping videos re-align at each loop only to within
this drift.

---

## Change history

Condensed log — what each version changed and the key files. Bug entries keep the
fix, not the full investigation.

### 0.9.2 (2026-06-20)

- **Transport-bar Pause/Resume button** — light-blue PAUSE toggle next to GO/STOP; same semantics as OSC `/wincue/pause_toggle` (pause all running, else resume all paused; disabled when idle). `TransportBar.tsx`.
- **Floating timer drag + counter fixed** — the `float-timer` window had no Tauri v2 capability, so `startDragging` and `listen("float-timer-text")` were silently denied. Added `capabilities/float-timer.json` (`core:default` + `core:window:allow-start-dragging`); needs a rebuild. *(A separate Linux crash when showing the timer is still under investigation.)*
- **Windows output → winit/GL by default** — the GL Render API path (`render.rs`) is now the Windows default; the old Win32+D3D11+`wid`+layered-overlay path is gated behind `legacy-win32-output` (off). `build.rs` emits `output_winit` / `output_win32` cfg aliases. `build.rs`, `output_engine/{mod,fade,render,mpv_events,types}.rs`.
- **Hard-cut stop clears to black (GL)** — a no-fade stop now forces overlay alpha 255 after `mpv stop`, so the render loop paints opaque black over the frozen last frame instead of leaving it on screen. `output_engine/mod.rs`.

### 0.9.1 (2026-06-20)

- **Fade-in "frame-black at ~1 s" fixed (legacy path)** — the old separate `WS_EX_LAYERED` overlay over mpv's d3d11 flip-model swapchain forced DWM to drop DirectFlip mid-fade, flashing one black frame. Fix: `d3d11-flip=no` (blit model). Only relevant under `legacy-win32-output`; the default GL path draws the fade in mpv's own framebuffer and is immune. `output_engine/mod.rs`.
- **GL output window startup/handling fixes** — render-context ready handshake (one-shot channel) so the first GO waits for the GL context; `WglThenEgl(None)` to avoid a double `SetPixelFormat`; real init error surfaced in the startup dialog; drag/resize/double-click-fullscreen in `gl_wnd_proc`; arrow cursor. Dead `RenderCtx` struct removed.

### 0.9.0 (2026-06-17) — Unified GL Render API output path (Stage 1)

- `vo=libmpv` + `mpv_render_context` (OpenGL 3.3 Core via glutin) on all 3 OS; fade is a GL quad; OSD timer composites in the FBO. Legacy Win32+D3D11 kept behind `legacy-win32-output`. macOS/Linux window creation marked TODO (Stage 2). `Cargo.toml`, `mpv_sys.rs`, `output_engine/{mod,render(new),fade,types,mpv_events}.rs`. *(Tauri `unstable`/`WindowBuilder` avoided — it imports comctl32 v6 and crashes the test binary.)*

### 0.8.1 (2026-06-16) — Mac/Linux output + floating timer

- Mac/Linux output via mpv properties (`hidden`, `fullscreen`, `screen`); cross-platform fade overlay (Win32 layered on Windows, ASS rectangle via `osd-overlay` elsewhere).
- Floating timer moved to a Tauri WebView window (`float-timer`, defined in `tauri.conf.json`); old Win32 GDI float timer removed. `FloatTimer.tsx` (new).
- Win32 cleanup: removed the never-fed GDI timer overlay (`win32_window.rs` shrank ~900 → ~300 lines).

### 0.8.0 (2026-06-16)

- **Audio/Video loop (finite + infinite)** — `loop_count = u32::MAX` loops forever (RT callback never sends `Completed`); video uses `loop-file`. Transport loading guard switched to `file_duration().is_none()` so infinite loops aren't blocked. Per-loop progress bar via `file_duration_ms` modulo; Inspector Time-tab loop control (count + ∞).
- **Fade/Stop multi-target + visual fade** — Stop Cue: `target_cue_ids: Vec<CueId>` (empty = all), backward-compatible migration from the old single-UUID/number format. Fade Cue: UUID multi-target; audio fade interpolates voice gain at 30 fps; visual fade drives `set_overlay_alpha_direct()` for Video/Image; context-aware inspector (volume dB / brightness %). New `CueCheckboxList`.
- **Cue List Notes column + per-cue Stop button** — `notes` column (ellipsis + tooltip) and a `StopButton` column shown only while a cue is running/paused; both columns toggleable.

### 0.7.4 (2026-06-15)

- **Cue List tab bar no longer disappears on overflow** — `CueListView` root `height:100%` → `flex:1; minHeight:0` (+ `minWidth/minHeight:0` on the left column) so the inner row list scrolls instead of pushing the tabs off-screen. View menu gained Cue List Tabs / Inspector / Output Surface visibility toggles, persisted to `localStorage`.
- **Output window z-order/visibility fixed** — `OutputEngine::new()` starts `visible=false`; `show_output()` uses one atomic `SetWindowPos(HWND_TOPMOST, SWP_SHOWWINDOW|…)`; the parent window is created with `WS_EX_TOPMOST`.

### 0.7.3 (2026-06-14)

- **Normalize to 0 dBFS** button in the Audio Levels tab — reads the decoded peak and sets `volume_db = 20·log10(1/peak)`, clamped to [-60, +12]. New `get_normalize_db` command.

### 0.7.2 (2026-06-14)

- **Image fade-in/out made visible** — overlay created with `WS_EX_LAYERED` only (dropping `WS_EX_TRANSPARENT`, which had let the composite show mpv underneath); `overlay_wnd_proc` returns `HTTRANSPARENT` so mouse events still pass through. (Legacy path.)
- **Cue List tab bar refreshed on project load** — `load_workspace`/`new_workspace` now call `emit_cue_lists_changed`; `App.tsx` bootstrap uses `refreshCueLists()`.

### 0.7.1 (2026-06-13)

- **Cue warnings split from broken** — yellow ⚠ (no file assigned, zero-duration Wait, empty Group) vs red ! (assigned file missing on disk); `warning_message` in `CueSummary`.
- **Image display duration** — `display_duration_ms: Option<u64>`: `None` holds until Stop, `Some(ms)` auto-completes via mpv `image-display-duration`.
- **Audio SR conversion refactor** — `voice.inner.rate_bits` is now a pure user-rate multiplier; the SR ratio lives only in `fill_buffer(output_sample_rate)`. 5 unit tests cover 44.1/48/96 k. *(Down-sampling has no anti-alias filter — imperceptible for band-limited sources.)*

### 0.6.2 (2026-06-13) — Stop Cue redesign (QLab semantics)

- Stop Cue now executes inline inside `transport.go()` via `stop_specification()` (before the Auto-Follow chain), fixing Auto-Follow killing the chained cue; targets all or a specific cue; soft/hard mode. The fragile `CueEvent::StopAll` channel was removed; `go()` returns `GoResult { triggered, stopped }`.
- Image cue: an audio GO no longer cuts a displayed image — `stop_on_next_go` only fires for visual GOs.

### 0.6.1 (2026-06-09) — Pause/Resume + OSC

- Elapsed time freezes on pause (`elapsed_before_pause` accumulators); progress bar freezes orange; seek allowed while paused.
- OSC: `/wincue/pause_toggle`, `/wincue/select/next|previous`; 50 ms dedup cache; OSC Monitor; per-message Test-send; double-GO protection (`double_go_protection_ms`, default 500 ms).

### 0.6.0 (2026-06-09) — OSC Send Cue + receive server

- OSC Send Cue (multiple messages per cue, workspace-level patches, inspector Messages tab) and a UDP receive server (IP allowlist, `/wincue/*` address scheme, activity dot). Design/implementation detail archived in `docs/archive/OSCPLAN.md`.

### 0.5.1 — Group Cue polish

- Drag cue into group (cue-drag and OS file-drop); child color-strip indent by depth; Sequential Group GO absorption to advance the inner sequence. New `absorbs_go()` trait method.

### 0.4.2 (2026-05-30) — Video freeze fixed

- Root fix: mpv plays video muted (`ao=null` / `audio=no`); the video's audio track is decoded by symphonia and played as a normal AudioEngine voice (Output Patch, VU, fades). Lockstep start: the audio voice is submitted paused and released with the video on the first `MPV_EVENT_PLAYBACK_RESTART`. The whole `ao=pcm` named-pipe path (the A/V-desync and replay-deadlock source) was deleted; a 2.5 s watchdog guards against a missed restart. New shared decoder `cue/media_decode.rs`.

### 0.4.1 (2026-05-28) — Persistent PCM pipe *(superseded by 0.4.2)*

- Single `pcm_pipe_manager` thread + `OUTPUT_PCM_DISCARD` flag fixed "no audio on 2nd+ video". Entirely removed in 0.4.2 in favour of the muted-mpv design above.

### 0.4.0 (2026-05-28) — Unified OutputEngine (Win32 + libmpv)

- One persistent `WS_POPUP` window for all visual cues replaced the old two-window approach (Tauri WebviewWindow for images + Win32 for video) that caused windows to disappear/reposition between cues. libmpv renders both video and images; per-cue fade overlay; Hard Stop always cuts; first-GO freeze removed (mpv created at engine init); F9 toggles visibility. Old `.wincue` fields (`ImageStopMode`, per-cue `screen_index`) load silently via serde.

### 0.3.2 (2026-04-28) — Unified output surface *(Tauri WebviewWindow era, superseded by 0.4.0)*

- `DisplayPreferences::output_screen`; single fixed `"output-surface"` window; per-cue screen selector removed in favour of a global Display preference.

### 0.3.1 (2026-04-22) — Image Cue functional

- Persistent `WebviewWindow` per screen, hidden between cues; `stop_on_next_go()` trait method; direct-DOM fade under React 18 batching; draggable floating window.

### 0.3.0 (2026-04-19) — Image Cue added (non-functional)

- `cue/image_cue.rs` skeleton; serialization OK; GO froze the app (fixed in 0.3.1).

### 0.2.0 (2026-04-14) — Audio/video architecture overhaul

- ASIO SDK + `CPAL_ASIO_DIR` build fix; `Voice.out_l/out_r` + `OutputPatch` routing; VU meter (rAF decay, peak hold); Video Cue playback (D3D11, loop, fullscreen, drag).

### 0.1.2 (2026-04-11)

- Stop Cue; drag & drop rework; immediate Auto-Continue fix; loop fix; duplicate/paste fix.

### 0.1.1 (2026-04-11)

- `CueList::renumber_all()`, `set_master_volume`, shortcuts, CurveSelect, TransportBar rework.

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
| 23. MIDI Cue | ✅ Note On/Off, CC, Program Change on GO; multiple messages per cue; dynamic port enumeration (midir) |
| 24. Unified GL output | ✅ winit + mpv Render API default on Windows; macOS/Linux Stage 2 TODO |

---

## Next priorities

See `WHATSNEXT.md` for the full roadmap; macOS/Linux porting detail is in `PORTAGE.md`.

1. **Stage 2 GL output** — macOS (NSWindow/CGL) and Linux (GDK/EGL) window creation for the unified Render API path.
2. **Active A/V resync** (optional) — nudge the video's audio-voice rate to track mpv `time-pos` for drift-free long videos / tight loops (see Known issues).
3. **ASIO → WASAPI Output Patch validation** — routing is wired; needs a hardware test.
