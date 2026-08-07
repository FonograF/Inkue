# Inkue — Project state as of 2026-07-11

## Current version: 1.3.2 released — fade-out now lands on a cue's natural end; automatic fallback to software decoding when hwdec init fails

## cargo build result

**Compiles without errors, zero warnings** on all three OS in CI (Windows, Linux,
macOS). The macOS job runs `cargo clippy` + `cargo test`; Windows/Linux run
`cargo check`. The legacy Win32 output path (`legacy-win32-output` feature,
`win32_window.rs`, the `cfg(output_win32)` / `cfg(output_gl)` split) was
**removed 2026-07-11** — the GL Render API path is the only output path on all
three OS.

## cargo test result

**`cargo test --lib` → 370 pass, 0 failures** (verified 2026-08-06; run the full
`cargo test` from `src-tauri/` after closing the dev server, which holds `inkue.exe` /
`libmpv-2.dll`. Never force-kill `cargo` mid-build — corrupts the incremental cache
→ `LNK anon.*.llvm.*`; if it happens, delete `target/debug/incremental`).
DMX engine + sink, fixtures, groups, Light Cue; live input resampler + Mic Cue; TC
types/DF/display/RT, MTC receiver QF+SysEx+flywheel, LTC encoder/decoder, TC generator
QF round-trip; network interface resolution rules; bounded device enumeration
(`run_bounded` timeout guard); **Group modes (Playlist exclusivity/loop, StartRandom
shuffle-bag, per-mode is_complete), stable cue numbering, Fade pan + fade-on-group +
Stop-at-End**; **VideoGeometry (serde roundtrip/defaults, log2 zoom, fit props,
pixel-crop math + clamping), EOF-fade window math (audio + picture), hwdec
failure detection + software-decode latch, geometry/hold serialization
roundtrips, output-screen resolution fallback, OutputTransform composition +
TestPattern URLs**; **MIDI file tempo map (mid-file Set Tempo, conductor track,
SMPTE timing, format 2) + the playback scheduler driven against a recording
sink (order, timing, pause, notes released on stop, mid-file start)**. Plus
integration suites
(`cue_behavior_tests`, `transport_go_tests`, …): group completion end-to-end, fade
drives every group child voice, logger flood guard.

---

## Cue type status

| Cue type | Status | Details |
|---|---|---|
| Audio | ✅ **Functional** | Pre/post-wait, fade-in/out, loop (finite + infinite), rate, pan, master volume, waveform, VU meter, scrub/seek; pause/resume with correct elapsed tracking; SR conversion in `fill_buffer` (44.1k/48k/96k all correct); formats via symphonia incl. **AIFF/PCM** (decoded natively, no MP3-demuxer log flood); **Output Patch routing (device + channels)** — see note below the table |
| Stop  | ✅ **Functional** | UUID-based targeting; multi-target (stop any subset of cues); target All Cues or specific cues; Soft (fade) or Hard (cut) |
| Memo  | ✅ **Functional** | A note in the stack; no action, completes instantly so chains pass through. `memo_text` shows in the cue list's Target column and is edited in the inspector's Memo tab. Creatable from the toolbar and the Add Cue menu (2026-08-06 — before that it existed only in the backend and could only arrive via import) |
| Video | ✅ **Functional** | Unified GL Render API path (Windows); paused-load start (no frame-0 freeze), dip-to-black fades (GL quad), scrub/seek; pause/resume; loop (finite + infinite); **Fade tab in the inspector (video fade in/out was engine-only before), fade-out at natural EOF (was hard cut), Hold Last Frame at EOF (`keep-open`), per-cue Geometry (fit/fill/stretch, position, scale, rotation, crop) with live-apply** |
| Image | ✅ **Functional** | Same GL output window as Video via libmpv Render API; dip-to-black fades incl. **fade-out landing on the end of a timed display duration**; layers with other visual cues (never auto-stopped — `stop_on_next_visual` removed 2026-07-11); loop support; **per-cue Geometry (same system as Video)** |
| Group | ✅ **Functional** | Four QLab-parity modes: **Simultaneous** (all at once, incl. Timeline via child pre-waits), **Sequential** (start-first), **Playlist** (exclusive one-at-a-time + optional loop), **Start Random** (one random child per GO, shuffle-bag); holds playhead + GO absorption for the ordered modes; drag-into-group |
| Wait  | ✅ **Functional** | Fixed duration delay cue; registered in CueRegistry |
| Fade  | ✅ **Functional** | UUID-based multi-target (any subset of cues, incl. **Groups** and cues nested in a group — voices collected via `all_voice_ids()` recursively); audio fade of **volume and/or pan** (gain/pan interpolation at 30 fps); visual fade for Video/Image (overlay alpha at 30 fps, `set_overlay_alpha_direct`); configurable curve; **Stop at End** now hard-stops the target *cues* (not just their voices) via the event loop + emits state/refresh so the UI clears; sectioned inspector (Targets / Fade / Audio / Visual / On Complete) + searchable target picker with chips |
| OSC   | ✅ **Functional** | Sends UDP OSC messages on GO; multiple messages per cue; inspector Messages tab + Test send button; workspace-level patches; receive server with IP allowlist + dedup cache; /inkue/pause_toggle; /inkue/select/next\|previous |
| MIDI  | ✅ **Functional** | Sends Note On/Off, CC, Program Change on GO; multiple messages per cue; dynamic port enumeration (midir); inspector Messages tab + Test send button; cross-platform (WinMM/CoreMIDI) |
| MIDI File | ✅ **Functional** | Plays a `.mid` to one MIDI port (QLab parity: destination + playback-rate multiplier). Tempo-map-aware parsing (`midly`) so a mid-file Set Tempo moves everything after it; real duration → completes and Auto-Follows on its own; pause/resume and seek; 1 ms timer resolution on Windows; stop releases every note the player started and lifts the sustain pedal |
| Light | ✅ **Functional** | DMX-over-IP (sACN + Art-Net); fixture patch in the workspace (6 built-in types, embedded layout, address-clash warnings, identify); Light Cue fades fixture params to a target look (tracking + LTP via DmxEngine); inspector Light tab (targets + fade time/curve); DMX panel Fixtures section |
| Mic      | ✅ **Functional** | (see 0.9.5) |
| Timecode | ✅ **Functional** | SMPTE timecode generation (MTC out via `TimecodeCue`) + receive (MTC in via `TimecodeReceiver`); per-cue TC triggers + CueList sync toggle; LTC encoder/decoder (`ltc.rs`); TC status indicator in TransportBar; Triggers inspector tab on every cue; TC Preferences (Network tab). LTC out = planned v2; drop-frame 29.97 fully tested. | Routes a live audio input (QLab Mic Cue) through the engine: persistent cpal input stream (instant GO), separate in/out devices + adaptive drift resampler, multichannel Input Patch routed to an Output Patch via a live `Voice` (gain/pan/fade/VU); runs until stopped; inspector Mic tab; Input Patches panel in Preferences → Audio |
| Text     | ✅ **Functional** | Renders styled text on the mpv output surface via the `osd-overlay` command (`format=ass-events`) + ASS inline tags; independent of OSD timer. Font, size, hex colour, 9-point position grid, optional auto-complete duration. Stop-on-next-go. |
| Camera   | ✅ **Functional** | Live feed (webcam / USB camera / HDMI capture via DirectShow-V4L2-AVFoundation, or any network stream — RTSP/HTTP/UDP, covers IP cams + phone apps) shown on the output like any visual cue; low-latency load opts (`cache=no`, `video-latency-hacks`); video fade in/out (overlay), per-cue Geometry, warp applies; runs until stopped (layers with other visual cues, never auto-stopped); device picker (per-OS enumeration) or URL in the Camera inspector tab |

> ✅ **Output Patch routing is now a real feature (2026-07-04, was broken/inert before).**
> Patches live in the workspace (single source of truth — the old unpersisted
> `DeviceManager` patch table is gone), are created/edited in Preferences → Audio →
> Output Patches (device dropdown, 1-based channels, ★ default), and are assigned per
> cue in the inspector Levels tab (Audio + Video). The engine honours `device_id`:
> voices whose patch targets a non-main device play on a dedicated **aux cpal stream**
> (opened lazily at first GO, kept open for zero-latency later GOs, per-stream voice
> pool, commands broadcast to all streams, panic stop covers every stream). If the
> patch device is missing the voice **falls back to the main output** + health banner —
> a mis-patched cue stays audible. Cross-platform: aux streams use the generic default
> host (WASAPI shared / CoreAudio / ALSA-PipeWire). Known limitation: **Mic/live
> voices** stay on the main device (their input feeds are drained by the main callback
> only) — patch channels apply, patch device does not.

---

## What is implemented and compiles

### Rust backend

| Module | File | Status |
|---|---|---|
| Cue types | `cue/types.rs` | ✅ Complete |
| Cue trait | `cue/traits.rs` | ✅ Complete — `stop_on_next_go()`, `stop_specification()` (Vec), `set_fade_voices()`, `resolve_fade_targets()` |
| CueRegistry | `cue/registry.rs` | ✅ Complete |
| CueContext | `cue/context.rs` | ✅ Complete — `audio_engine`, `output_engine`, `stop_fade_ms`, `output_patches`, `output_screen` |
| AudioCue | `cue/audio_cue.rs` | ✅ Functional — pre-wait, fade-in/out, loop (finite + infinite, `u32::MAX`), rate, pan; pause freezes elapsed; seek while paused; SR correction in `fill_buffer`. ⚠️ Output Patch routing is inert (no UI; engine ignores `device_id`) — see correction note above |
| VideoCue | `cue/video_cue.rs` | ✅ Uses `output_engine.show_content()` / `stop_voice()` / `pause_voice()` / `resume_voice()`; loop support; `file_duration()` override returns raw `cached_duration` |
| ImageCue | `cue/image_cue.rs` | ✅ `display_duration_ms: Option<u64>` — None = hold, Some = timed auto-complete |
| MemoCue | `cue/memo_cue.rs` | ✅ Complete — `memo_text()` trait override feeds the Target column; 5 unit tests |
| MidiFileCue | `cue/midi_file_cue.rs` | ✅ Plays a `.mid` via `engine/midi_file.rs`; parsed in `from_json` so the row has a real duration; `restore_runtime_state` restarts the player at the position reached, so an inspector edit does not silence a playing cue |
| MIDI file engine | `engine/midi_file.rs` | ✅ Tempo-map-aware SMF parser (pure, byte-driven) + `MidiFilePlayer` thread; sends through a `MidiSink` trait so the scheduler is testable without a port; 1 ms timer resolution on Windows |
| StopCue | `cue/stop_cue.rs` | ✅ UUID-based multi-target (`target_cue_ids: Vec<CueId>`); empty = stop all; backward-compat with old single-UUID format; `resolve_stop_target` handles number→UUID migration |
| FadeCue | `cue/fade_cue.rs` | ✅ UUID-based multi-target (`target_cue_ids: Vec<CueId>`); audio fade via `audio_engine.set_voice_gain()` at 30 fps; visual fade via `output_engine.set_overlay_alpha_direct()` at 30 fps; `has_visual_target` + `visual_start/target_alpha`; `stop_at_end` for audio + visual; backward-compat with old `target_cue_number` |
| VoiceState / FadeState | `engine/voice.rs` | ✅ Complete — `out_l`, `out_r` for channel routing |
| AudioCommand / AudioStatus | `engine/ring_command.rs` | ✅ Complete |
| DeviceManager / OutputPatch | `engine/device_manager.rs` | ✅ Complete |
| AudioEngine | `engine/audio_engine.rs` | ✅ Complete — WASAPI/ASIO; SR conversion in `fill_buffer`; infinite loop (`loops_remaining = u32::MAX`) never sends Completed; 5 unit tests |
| OutputEngine | `engine/output_engine/` | ✅ Complete — unified GL Render API on all 3 OS; `vo=libmpv`; native GL window — winit (Windows/Linux) or AppKit `NSWindow` via objc2 (macOS, `macos_window.rs`); mpv_render_context; GL fade quad; OSD + floating timer; `get_overlay_alpha()`, `set_overlay_alpha_direct()` |
| OscPatch | `engine/osc_patch.rs` | ✅ Complete |
| OscServer | `engine/osc_server.rs` | ✅ Complete — UDP listener, IP allowlist, 50ms hash dedup cache |
| mpv_sys (FFI) | `engine/mpv_sys.rs` | ✅ libmpv bindings compile |
| CueList | `show/cue_list.rs` | ✅ Complete — `resolve_fade_targets` called alongside `resolve_stop_target` on load |
| Workspace | `show/workspace.rs` | ✅ Complete |
| Transport | `show/transport.rs` | ✅ Complete — stop spec handles `Vec<CueId>` (empty = all); fade spec resolves audio voices + triggers visual fade via `set_overlay_alpha_direct` |
| Event loop | `show/event_loop.rs` | ✅ Complete — per-loop progress bar uses `file_duration_ms` modulo |
| UndoStack | `show/undo_stack.rs` | ✅ Complete |
| AppState | `state/app_state.rs` | ✅ Complete |
| Preferences | `preferences.rs` | ✅ Complete — incl. Personalization (`cue_color_style`) + timer fields |
| Bundled fonts | `bundled_fonts.rs` | ✅ Installs DSEG7 Classic (default timer font) per-user at startup; cross-platform resolution |
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
| `components/CueList/columns.ts` | ✅ Complete — `notes` + `stop_btn` + `led` columns; `led` always follows `playhead` (migration in `loadColumnConfig`); LS key v2 |
| `components/CueList/CueListTabs.tsx` | ✅ Complete |
| `components/CueList/CueRow.tsx` | ✅ Complete — `notes` cell; `stop_btn`; per-loop progress bar; `RunningLed` (sync via negative `animation-delay`); playhead left-aligned |
| `components/ShowMode/ShowModeView.tsx` | ✅ Complete — read-only bubble-card list; `flattenAll` (groups → children); `computeArmedIds` (sequential/simultaneous groups); status: Completed/Armed/Ready/Running/Paused/Loading; progress bar; auto-scroll |
| `hooks/useKeyboardShortcuts.ts` | ✅ Complete — `F5` → `onToggleShowMode` |
| `App.tsx` | ✅ Complete — Show Mode state; View menu with F5 shortcut; toolbar hidden in Show Mode; ShowModeView replaces CueList+Inspector |
| `components/CueList/CueListView.tsx` | ✅ Complete — passes `onStop` to CueRow |
| `components/Inspector/InspectorPanel.tsx` | ✅ Complete |
| `components/Inspector/OscTab.tsx` | ✅ Complete |
| `components/OscPatches/OscPatchesPanel.tsx` | ✅ Complete |
| `components/Inspector/BasicsTab.tsx` | ✅ Complete — Stop/Fade: `CueCheckboxList` multi-select; Fade: context-aware UI (volume dB / brightness % / both) |
| `components/Inspector/TimeTab.tsx` | ✅ Complete — Loop control (checkbox + count + ∞ toggle); scrubber shows for infinite loop using `file_duration_ms` |
| `components/Inspector/ScrubBar.tsx` | ✅ Complete — `loopDurationMs` prop for per-loop modulo display |
| `components/Inspector/LevelsTab.tsx` | ✅ Complete |
| `components/Inspector/FadeTab.tsx` | ✅ Complete |
| `components/Inspector/TextTab.tsx` | ✅ Complete — textarea, font picker, size, colour picker + hex input, 9-button position grid, auto-complete duration toggle |
| `components/Transport/TransportBar.tsx` | ✅ Complete |
| `components/Osc/OscMonitor.tsx` | ✅ Complete |
| `components/Preferences/PreferencesModal.tsx` | ✅ Complete |
| `components/WaveformModal.tsx` | ✅ Complete |
| `components/common/Select.tsx` | ✅ Themed dropdown replacing native `<select>` (15 call sites; readable dark theme on Linux/WebKitGTK) |
| `main.tsx` | ✅ Complete |

---

## Known issues

### ✅ RESOLVED (0.9.26): Linux UI froze while a video cue plays — continuous UI animation

**Symptom.** On the operator's Linux box (`pnpm tauri dev`), Inkue's WebKitGTK **UI** froze
to ~0 fps *while a video cue plays* — GO/Stop only registered after ~5 s. The **video itself
stayed fluid**. Audio cues never caused it; only video. A **production build was fluid** with
the same clip, which is what finally localised the cause.

**Root cause (measured directly on the UI thread).** The lag is **GPU/compositor contention**,
not CPU and not the output render path. Measured with an in-UI `requestAnimationFrame` meter:
during the freeze the GTK main loop stayed responsive (closures dispatched in ~150 µs) but rAF
— WebKitGTK's *paint* clock — sat at 0 fps. So the UI thread wasn't busy; WebKitGTK simply
could not get a frame **composited**. The trigger: UI elements that animate **continuously**
force WebKitGTK to commit a fresh frame for the whole UI surface every display refresh
(~60 fps) for the animation's entire lifetime. On a weak shared-memory iGPU that permanent
recompositing can't coexist with a Video Cue's output window also presenting → the UI starves
to ~0 fps. The culprits, both shown only while a cue runs:

- the **running-cue LED** (`RunningLed`) — a CSS `@keyframes ... infinite` pulse (animating
  `box-shadow`, then even `opacity`);
- the **progress bars** (cue list, Active Cues, Show Mode) — a `transition: width …` retriggered
  on every 30 fps timing update, i.e. effectively continuous.

Audio cues put no load on the GPU, so the same continuous repaints were free → only video
lagged. Dev React (StrictMode double-render, unminified) widened the gap; a production build
had just enough compositor headroom to stay fluid.

**Confirmation.** Capping the output present rate (`INKUE_OUTPUT_FPS=10`) lifted the UI from
0 → ~20 fps (proves the output window was starving it); disabling the LED lifted it further
(0 → 6 fps interactive). With both UI fixes below, **dev mode + a video cue went from 0 fps
(frozen, GO/Stop after ~5 s) to 30+ fps (responsive)**.

**Fix (frontend, cross-platform).** Stop any UI element from driving continuous compositing:

- `components/common/RunningLed.tsx` (new, shared by `CueRow` + `CartView`) — the running
  indicator now blinks via a **discrete JS `setInterval` (~1.4 Hz)** instead of a CSS keyframe,
  so the UI surface is idle between toggles. Removed the `wc-led-pulse` keyframe (`index.html`).
- Progress bars (`CueRow`, `ActiveCuesView`, `ShowModeView`) — animate `transform: scaleX()` on
  a `will-change: transform` layer (compositor-only, no layout/paint) and **dropped the
  continuous `transition`**, so each timing update is one discrete cheap commit.

No backend change was needed. The output render path is already cheap (mpv render call ~3 ms,
swap ~0.6 ms). On Linux, `INKUE_OUTPUT_BACKEND=wayland` additionally renders a correct-size
(smaller) output FBO instead of the XWayland-scaled one, for extra headroom on weak iGPUs if
ever needed.

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

### Unreleased (2026-08-06) — Memo Cue reachable, and its text finally visible

Two defects, found because the Memo Cue could not be created from the UI at all.

- **No way to make one.** `MemoCue` has been implemented and registered since
  early on, but `memo` was absent from both the toolbar and the cue-list "Add
  Cue" menu — the only way to get one into a workspace was to import a QLab
  show, where every unconverted cue becomes a Memo placeholder. Added to both,
  with the 📝 icon it already had everywhere else.

- **`memo_text` was written and then never shown.** The field round-tripped
  through the `.inkue` file and its doc comment said "text displayed in the cue
  list target column", but a `grep` for it found hits in `memo_cue.rs` and
  nowhere else: not in `CueSummary`, not in any component. So the note was
  unreadable and uneditable in the app. That matters most for the importer,
  whose `map_memo` puts the `[Unconverted QLab …]` explanation there — the one
  place telling the operator what needs rebuilding was invisible.

  New `Cue::memo_text()` (default `None`, so no other cue type is touched and
  the extensibility rule holds) → `CueSummary.memo_text` → the Target column,
  which shows the note where a media cue shows its filename. New `MemoTab.tsx`
  makes it editable. A fresh Memo returns `Some("")` rather than `None`: the
  cue type always owns that column, so an empty Memo shows nothing rather than
  falling through to a filename it does not have.

**Tests** — 365 → **370**. `memo_cue.rs` had no tests at all; it now covers the
type, the note exposed through the trait, `Some("")` on a fresh cue, a
serialize roundtrip carrying an importer-style placeholder, and no duration.


### Unreleased (2026-08-06) — MIDI File Cue

Closes the last gap in QLab cue-type parity (`WHATSNEXT.md` bloc 1d): every
QLab cue type now has an Inkue equivalent, so the importer no longer has a
reason to emit a Memo placeholder for a type it recognises.

QLab's MIDI File cue has exactly two settings and so does this one: a
destination port and a playback-rate multiplier. Everything else about the
performance is written into the file.

- **`engine/midi_file.rs`** (new) — parsing and playback, split so the parser
  is pure and testable from bytes.
  - `parse_midi_bytes` flattens an SMF into absolutely-timed events. **The
    tempo map is the whole reason this module exists**: a Set Tempo meta can
    appear anywhere, so a tick has no fixed duration. Tracks are merged onto
    one timeline first (stable sort by tick, then track order) and time is
    accumulated as the merged stream is walked — a tempo change in the
    conductor track therefore moves the events of every other track that
    follows it. SMPTE-timed files (`Timing::Timecode`) ignore tempo entirely.
    Format 2 tracks are concatenated, each with its own tempo map. Duration
    includes the silent tail before End of Track. New dep `midly` 0.5, with
    `parallel` (rayon) off — show files are kilobytes.
  - `MidiFilePlayer` owns the sending thread: pause/resume, start-at-offset,
    and a **1 ms timer resolution on Windows** (`timeBeginPeriod`, winmm is
    already linked by midir) because the default 15.6 ms scheduler tick
    audibly quantises notes. Sleeps stop 1 ms short of each event and yield
    the rest.
  - **Stopping never leaves a note hanging.** The player tracks which notes it
    turned on (`[u128; 16]`) and releases exactly those, lifts the sustain
    pedal, then sends All Notes Off — but only on channels the file actually
    used, so it does not disturb other gear on a shared port.
  - Sends go through a `MidiSink` trait, so the scheduler is tested against a
    recording sink. No CI machine has a MIDI port and not every dev machine
    has one that loops back (checked: none of this one's virtual ports do).

- **`cue/midi_file_cue.rs`** (new) — the cue. Parsed in `from_json`, so a row
  has its real duration the moment the workspace opens and completes /
  Auto-Follows on its own like an Audio Cue. `file_path` uses that exact key,
  which is what makes the path relative in the `.inkue` for free. Soft and
  hard stop are the same cut (MIDI has no fade).
  **Editing a playing cue no longer silences it**: every `update_cue` rebuild
  drops the player thread, so `restore_runtime_state` restarts it at the
  position reached, replaying the channel state (program, controllers, bend)
  the file had established and skipping notes already sounding rather than
  re-attacking them. `seek()` uses the same machinery.

- **Plumbing** — `CueType::MidiFile`; registry; `set_midi_file` command;
  `media_file_path` (so preflight's missing-file check and relink cover it
  with no new code); broken/warning row states; Collect & Save copies `.mid`
  into a new `midi/` subfolder. Preflight also reports a file that exists but
  will not parse, an absent port, and an out-of-range rate.

- **Frontend** — `MidiFileTab.tsx` (port, rate, and a read-only summary of
  what was parsed: length, length at rate, track count, channels — a MIDI file
  gives no other confirmation you picked the right one). Toolbar button, cue
  list menu entry, 🎼 icon, `.mid`/`.midi`/`.smf`/`.kar` drag-and-drop onto the
  cue list and Cart, and "Assign MIDI File…" in the context menu.

**Tests** — 329 → **365** lib tests. The parser tests build SMF bytes inline
(no fixture files): 120 BPM default, mid-file tempo change, conductor-track
tempo applying across tracks, SMPTE ignoring tempo, format 2 concatenation,
silent tail, merge ordering, Note On velocity 0 as a release, SysEx reframing,
channel mask. The scheduler tests drive the real play loop: event order and
timing, rate compression, pause holding the file, stop releasing held notes and
the pedal, end-of-file release, a mid-file start replaying channel state without
re-attacking notes, and — caught by working the units through rather than by a
failing test — that the start offset is *played* time, so the rate is applied
once and not twice (at 2×, seeking to 250 ms had landed 1000 ms into the file). `cargo clippy --all-targets` and `tsc --noEmit`
clean; frontend vitest 20 pass (the IPC contract test covers `set_midi_file`).

### 1.3.2 (2026-08-05) — GitHub issues #4 and #5

- **Fade Out was ignored when a cue reached its own end** (issue #4,
  user-reported): the sound of an Audio Cue — and of a Video Cue — hard-cut at
  EOF; the fade only ever applied to a *manual* Stop (or a Stop Cue), which is
  where `fade_out` was read. `VideoCue` already fell its **picture** out at the
  natural end (`tick_eof_fade`), so the same treatment now covers sound:
  `AudioCue::tick` and the audio half of `VideoCue::tick_eof_fade` arm the
  fade once the remaining action time drops inside the fade-out window, so it
  lands exactly on the end of the media. Picture and sound arm from their own
  spec, independently. Skipped where there is no natural end to land on
  (infinite loops, vamping slices). The shared window math
  (`eof_fade_remaining_ms`) moved from `video_cue.rs` to `cue/types.rs` — three
  cue types use it now. Tests 297 → **300** + 5 behavioural
  (`cue_behavior_tests`: audio fades at EOF and only once, no fade configured =
  no stop, infinite loop never arms, video sound fades at EOF, picture/sound
  windows tracked independently).

- **Green tint + torn picture on some H.264 files** (issue #5, user-reported,
  Windows 11): libmpv failed to initialise d3d11 hardware decoding for one
  file (`Failed setup for format d3d11: hwaccel initialisation returned
  error.`) and did not recover — it retried per frame and handed the
  compositor half-decoded frames. On a hwdec-init failure Inkue now latches
  the whole session to software decoding: `hwdec=no` is applied live to every
  existing slot — which reinitialises the decoder in place, normally before
  the first frame is even revealed, since video loads start paused — and new
  slots are created with it.

  **Where the detection lives matters.** libavcodec's logging is
  process-global and mpv routes it to the **first core created** — the overlay
  context — so a slot's `h264: Failed setup for format …` never reaches the
  slot that loaded the file (measured: a second core loading the failing file
  receives none of them; the first core, which loaded nothing, receives them
  all). The detection therefore sits in `mpv_events` (overlay), with the slot
  handler kept as a backstop should that routing ever change. Slots do now
  request log messages, which they never did before — their `LOG_MESSAGE` arm
  was dead code.

  `INKUE_HWDEC` pins the mode (`no`, `auto-copy`, `d3d11va-copy`, …) and
  disables the automatic fallback, so an explicit choice — reproducing a
  decoder bug, or working around a known-bad GPU path in the field — is never
  silently undone. The overlay context also stopped asking for hardware
  decoding altogether: it only ever renders the timer OSD, Text Cues and
  lavfi test patterns, all software sources, so a d3d11 device there was pure
  failure surface. Tests: hwdec log-line detection (verbatim from the report,
  plus the vaapi wording), no false positive on ordinary mpv warnings, the
  latch, and the pin outranking the latch.

  Repro assets: an H.264 file the GPU refuses to decode in hardware — 4:2:2,
  4:4:4 or 10-bit, all outside NVDEC's 8-bit 4:2:0-only H.264 support — makes
  any machine emit the failure. The *visual* artefact is narrower: it needs
  the d3d11 fallback path specifically (on NVIDIA, mpv tries vulkan/cuda and
  falls back cleanly), which is what `INKUE_HWDEC=d3d11va-copy` is for.

### 1.3.2 (2026-07-14) — OSC monitor matched flag comes from the parser

- **OSC monitor mislabeled valid addresses as "unknown"** (user-reported: seek
  worked but showed red): `OscMonitor.tsx` kept its own copy of the known
  addresses, which had drifted from the Rust parser. The classification now
  lives in one place — `osc_server::resolve_action` (pure enum: Command /
  CueListRequest / PlayheadRequest / Unmatched) computes a `matched` bool
  emitted with each `osc-debug` event; the monitor just displays it. Tests
  295 → **297** (matched for every command/seek address, unmatched for
  strays like `/jog_wheel`).

### 1.3.1 (2026-07-14) — OSC media-progress feedback + OSC seek

- **OSC seek** (user request — navigate inside a playing audio/video cue):
  `/inkue/cue/{n}/seek <s>` (absolute, clip-relative like the scrub bar),
  `/inkue/cue/{n}/seek/relative <±s>` (from the current position),
  `/inkue/cue/{n}/seek/percent <0..1>` (fader-friendly fraction of the clip —
  one loop iteration when looping, else total duration). Parsed with the
  first numeric OSC arg (`numeric_arg`); dispatched through the existing
  `osc-command` → frontend `seekCue` path; clamped to the clip, no-op on
  standby cues (backend guard). Address reference updated in Preferences →
  Network. Tests 292 → **295** (seek absolute/relative/percent parsing,
  missing-value + bad-mode rejection).

- **OSC feedback now streams media progress** (user request): per running
  cue, `/inkue/cue/{i}/progress` (float 0..1), `/elapsed` (s), `/remaining`
  (s, −1 = unknown) and `/duration` (s, −1 = unknown) — slot `i` = 0..7,
  same ordering as the existing `/inkue/cue/{i}/number|name` (slot 0 =
  topmost running cue), so an Open Stage Control "now playing" strip maps
  once and works for any simultaneous set. Vamps/∞ loops: progress follows
  the position within one file pass (media position for sliced cues via
  `media_elapsed_ms`, shared with the UI bars); remaining/duration report −1.
  Freed slots get one zeroing pulse so client gauges don't freeze.
  Rate is configurable (Preferences → OSC → "Progress rate", default 10 Hz,
  0 = off; `feedback_progress_hz` in `osc.json`); the rate gate
  (`osc_feedback::progress_due`) sits before payload construction so
  off-pulse ticks cost nothing. Tests 288 → **292** (`progress_values`:
  known duration, clamp past end, vamp fallback to file position, live feed).

### 1.3.1 (2026-07-13) — sliced cues: progress follows the media position

- **Progress bars swept and looped during a vamp** (user-reported): the time
  display was wall-clock (`cue.action_elapsed()`), which keeps advancing while
  a vamping segment holds its file position — the ActiveCues bar and the
  inspector scrub kept sweeping 0→end in a loop, and stayed wrong after a
  devamp (only the engine knows when the loop released). `collect_time_snapshots`
  now reports the **engine media position** as `action_elapsed_ms` for cues
  with active slices: new `AudioEngine::voice_position_ms` (RT frame cursor —
  reflects slice jumps) and `OutputEngine::voice_position_ms` (mpv `time-pos` —
  reflects ab-loop jumps), gated by the `Cue::uses_sliced_playback()` hook
  (default false; Audio/Video override). Unsliced cues are untouched.

### 1.3.0 (2026-07-13) — part 4: QLab slices + Devamp Cue; clip editor dock replaces the waveform modal

Full vamp/devamp workflow on Audio **and** Video cues (user request).

- **Slices data model** (`SliceList` in `cue/types.rs`, `PLAY_COUNT_INFINITE =
  u32::MAX`): markers split the clip into segments, each with a play count
  (∞ = *vamp*). Serde-defaulted field `slices` on AudioCue + VideoCue (legacy
  workspaces load unchanged); `segments(clip_start, clip_end)` clamps markers
  to the trim window. When slices are present, `loop_count` is ignored.
- **Audio engine slices** (`voice.rs` `SliceProgram`/`SliceSegment`,
  `audio_engine.rs`): the RT callback's boundary check walks the program —
  repeat the segment while `remaining > 0` (∞ stays ∞), then advance;
  the whole thing allocation-free (program built at GO, `UnsafeCell` written
  before submission like `end_frame`). `AudioCommand::Devamp { stop_at_end }`
  sets an `AtomicU8` request consumed at the next boundary: release the loop
  and continue, or stop with `Completed` at the slice edge.
- **Video engine slices** (`slot.rs` `SlicePlan`; `ContentRequest.slices` as
  `(start_s, end_s, count)`): segments loop via **mpv ab-loop** (a/b/count set
  as loadfile options for segment 0 — active before the first frame); the slot
  event thread observes `time-pos` (`mpv_observe_property`, new symbol +
  `MpvEventProperty` in `mpv_sys`) and programs the next segment's ab-loop
  when playback crosses a boundary. Devamp sets `ab-loop-count=0` (finish the
  pass, then continue past B); stop-at-end hard-unloads at the boundary
  (normal `Completed` path). A video's **paired audio voice gets the same
  sample-resolved program**, so picture and sound release together.
  `VideoCue::duration()`: ∞ while any segment vamps, else the counted sum.
- **Devamp Cue** (`cue/devamp_cue.rs`, `CueType::Devamp`, registered in
  `AppState` + test registry): targets audio/video/group cues; completes
  synchronously; new trait hook `devamp_specification()` resolved by the
  transport after `go()` (same pattern as Stop/Fade — recursive target lookup,
  `all_voice_ids()` for groups, output voice + paired audio voice for videos).
  Options: **Continue** (into the next slice) or **Stop at end of current
  slice**. Toolbar "+ Devamp", 🔁 icon, inspector Devamp tab (target picker +
  segmented mode).
- **Clip editor dock** (`Editor/ClipEditorDock.tsx` + `SliceTimeline.tsx`):
  the ⤢ button on the inline waveform/filmstrip now opens a **second
  inspector under the cue list** (the WaveformModal is deleted): a large
  DPR-crisp timeline — waveform (2000 bins) for audio, 16-tile filmstrip +
  scrub drag-preview for video — with trim handles, and slice editing:
  double-click adds a marker, drag moves it (counts follow by permutation),
  right-click removes it, per-segment **play-count badges** (click → inline
  input, `inf`/`∞`/`0` = vamp, vamp badges highlighted). Audio cues get an
  **▶ Audition** button (previewCue). Dock saves via the generic `update_cue`
  merge; the inspector re-fetches through a `reloadToken` bump.
- **Timeline zoom** in the clip editor dock: mouse wheel zooms centered on the
  cursor (Shift+wheel pans, −/+/Fit buttons, visible-range readout), down to a
  200 ms window. All mapping (markers, trim handles, badges, hit-testing,
  double-click) is view-window aware (`TrimPainter` gained a `view` param; the
  inline inspector strips keep passing the full clip). Zooming past 2× swaps
  in high-resolution data once per file: 16 000-bin waveform / 48-tile
  filmstrip (both disk-cached). Post-ship fixes: dock `slices` identity was
  recreated every render, retriggering the timeline's reset effect (glitchy
  marker drags, badge editor closing instantly — now memoized + content-keyed
  reset); video duration read from `cached_duration_ms` (the serialized form —
  `file_duration_ms` only exists on summaries, so the dock showed "Waiting
  for media duration" forever).
- **Zoom polish** (user feedback): (1) inspector ↔ dock two-way sync — the
  dock re-fetches its cue on every inspector save (`onCueSaved` → dock
  `reloadToken`; media only reloads when the *file* changes); (2) zoomed video
  now streams **window-matched frames**: new `video_filmstrip_range`
  (`get_video_filmstrip_range`, ½-second-grid disk cache) fetched debounced
  (300 ms) for the visible window and composited over the stretched
  whole-file tiles; (3) all strip canvases repaint on element resize
  (`useCanvasWidth` ResizeObserver hook — narrow windows squashed the bitmap
  before); (4) the inline inspector strips draw the cue's **slice markers**
  as read-only dashed yellow lines (`sliceMarkersMs` prop on TrimStrip).
- **Changing a media file resets the clip window** (user-reported): the file
  setters kept the old start/end times and slice markers — on a shorter file
  the start landed past EOF (silent audio) and ab-loop segments pointed past
  the end (video looping with Loop unchecked). `set_audio_file` /
  `set_video_file` now go through `set_file_path_resetting_clip`: when the
  path actually changed, start/end/slices/cached-duration reset; re-picking
  the same file keeps everything. Tests: reset-on-change, keep-on-same-file
  (288 lib tests). The inspector re-fetches the cue after Browse… (it used to
  patch `file_path` locally, showing the stale clip window until re-select)
  and bumps the dock's reload token.
- Tests 274 → **286** + 2 integration: SliceList (normalize, tiling,
  clamped/out-of-window markers), audio RT slices (vamp holds position,
  devamp-continue plays through, devamp-stop ends at the boundary, finite
  counts replay then advance), AudioCue slices serde roundtrip + legacy JSON,
  DevampCue serde roundtrip, transport devamp fan-out (per-voice with mode,
  no-op on idle targets). `ALL_CUE_TYPES` contract now 15 types.

### 1.3.0 (2026-07-13) — part 3 (2026-07-12): DPI-proof output placement; screen goes live on load; shortcuts work with output focused

Driven by a theatre-user video report: with the laptop display above 100 %
scaling, GO on a visual cue shifted/resized the output window off the selected
projector (their workaround was Floating Window + manual placement).

- **Output placement was DPI-broken** (`render.rs`): `set_outer_rect` passed
  the **physical** monitor rect from `list_screens()` as a winit
  `LogicalPosition`/`LogicalSize`, so winit re-multiplied it by the current
  monitor's scale factor — with any display above 100 % the window landed
  shifted and oversized on every GO / Preferences apply. Both machines at
  100 % masked the bug (repro: primary display at 150 %). Replaced by
  `set_fullscreen_on_rect`: match the target `MonitorHandle` by physical
  origin and apply `Fullscreen::Borderless(monitor)` — DPI-proof, covers the
  taskbar, pins the window to the monitor, and works on Wayland (where
  `set_outer_position` is a no-op); physical-rect + `Borderless(None)`
  fallback when no monitor origin matches. macOS path unchanged.
- **Configured screen goes live on workspace load**
  (`OutputEngine::apply_output_screen_on_load`, called from
  `install_workspace`): the output now shows as a black fullscreen surface on
  the selected screen as soon as the show opens — not only on the first visual
  GO. Unlike the GO path, a configured-but-missing screen does **not** fall
  back to fullscreen-on-primary (that would black out the operator's main
  display when opening the show at home); it keeps the window hidden and
  raises the `output-screen` health banner.
- **Keyboard shortcuts now work while the output window has focus**
  (`render.rs` → `useTauriEvents.ts`): the winit window swallowed every key —
  Space/GO and Escape/panic went dead if the operator clicked the output. The
  winit event loop now forwards `KeyboardInput` presses (`dom_key` maps winit
  logical keys to DOM `KeyboardEvent.key` values; modifiers tracked via
  `ModifiersChanged`) as an `output-keydown` Tauri event; the frontend replays
  them into the window-level shortcut handler via `window.dispatchEvent`.
  macOS needs no forwarding: the borderless NSWindow can't become key, so keys
  already stay on the main window.
- Tests 259 → **264**: `dom_key` mapping (DOM `" "` for Space, NamedKey names
  = DOM values, character passthrough, dead keys dropped).
- **Every backend event was handled twice in dev** — surfaced as F9 (pressed
  with the output window focused) hiding and instantly re-showing the window.
  `useTauriEvents` registered its listeners in an async `setup()`; under React
  StrictMode's dev double-mount, the first mount's cleanup runs while the
  `listen()` promises are still pending, so those listeners were never
  unsubscribed → two handlers per event (also double-fired `osc-command` GO in
  dev). Fixed with a `cancelled` flag + post-setup sweep; the same racy
  "save unlisten in `.then`, cleanup with `unlisten?.()`" pattern was fixed in
  `App.tsx`, `LogViewerModal`, `LightingPanel` and `ImageSurface` (cleanup now
  resolves the listen promise, the codebase's safe idiom). Release builds
  (no StrictMode) were unaffected.
- **F9 (show output) instantly re-hid the window**: winit on Windows
  fabricates `KeyboardInput` *Pressed* events (`is_synthetic: true`) for every
  key physically held when a window gains focus — showing the output window
  activates it while F9 is still down, and the ghost press was forwarded and
  toggled the window straight back to hidden. Key forwarding now skips
  `is_synthetic` events.
- **Inspector redesign** (readability at high parameter counts): the panel is
  now **resizable** (left-edge drag handle, 360 px default, 320–560 clamp,
  width persisted in the `inkue_ui_layout` localStorage blob) and every cue
  type gets a **dedicated main tab** instead of piling into Basics —
  `FadeCueTab` (targets / fade / audio / visual / on-complete), `StopTab`
  (targets + Soft/Hard segmented), `GroupTab` (mode + hint + playlist loop);
  `GeometryTab` split into `LayerTab` (compositing: layer order / opacity /
  blend) and a pure Geometry tab. Basics is identity-only (n° + color grid,
  name, notes, media file, flow). New shared primitives in
  `Inspector/Field.tsx` — `Section` (card with uppercase micro-title), `Grid2`
  + `MiniField` (side-by-side numerics: waits, position, crop 2×2, clip
  start/end), `SliderRow` (opacity/scale/rotation/brightness/pan), `Segmented`
  (Fit/Fill/Stretch, Soft/Hard), `NumberInput` (clamped commit-on-blur),
  `ToggleRow`; the cue target picker moved to `CueTargetPicker.tsx` (shared by
  Fade/Stop). Tab order is uniform: Basics | type tab | Time | Levels | Fade |
  Layer | Geometry | Triggers. TimeTab/FadeTab re-laid on the same system.
  Fade Cues can now target **Camera** cues (the target picker excluded them;
  the engine already faded any `is_visual()` target's layer opacity).
- **Media previews in the inspector** (user request): Video and Image cues
  show a thumbnail in Basics → Media — first representative frame for videos
  (`start=15%`, frame 0 is often black), the image itself for images. Audio
  already had its inline waveform (Time tab). Generation is headless libmpv
  with `vo=image` + `frames=1` + `vf=scale=400:-2` in a throwaway context
  (`engine/thumbnails.rs`) — one code path covers every video/image format
  the engine plays; raw-file fallback for browser-native formats mpv can't
  rasterise (SVG), capped at 10 MB. JPEGs cached in
  `<config>/Inkue/thumbnails/` keyed by path+size+mtime hash, plus a
  session-lifetime frontend cache (`MediaThumbnail.tsx`). Command
  `get_media_thumbnail` (async + `spawn_blocking` — decode takes ~100-300 ms).
- Tests 264 → **270**: thumbnail cache-key stability/invalidations, data-URL
  encoding, raw-fallback extension gate.
- **Detailed DAW-style waveform** (user request): `get_waveform_peaks` now
  also returns per-bin **RMS** (`WaveformData.rms`, helper
  `compute_waveform_bins`); the inline viewer draws a dim peak envelope with a
  brighter RMS body, one column per CSS pixel at devicePixelRatio (1600 bins
  requested), center line — replaces the blocky 400-bin bars.
- **Video trim strip with filmstrip preview** (user request): the video Time
  tab gets the same draggable start/end markers as audio, over a **filmstrip**
  of 8 frames spread across the file. Backend `video_filmstrip`
  (`thumbnails.rs`): one `vo=image` pass with `sstep=duration/tiles` +
  `frames=tiles` (duration via `probe_duration`), per-tile disk cache
  (`<hash>-strip8-<i>.jpg`); command `get_video_filmstrip`. The shared trim
  shell (markers, drag, labels, DPR-aware canvas) is extracted into
  `TrimStrip.tsx`; `WaveformViewer` and the new `VideoTrimmer` are thin
  painters on top of it.
- Tests 270 → **274**: waveform peak+RMS bins (empty input, RMS ≤ peak, sine
  RMS = 1/√2, DC signal RMS = peak).
- **Scrub preview while trimming video**: dragging a start/end marker shows a
  popup above the cursor with the video frame at that position + timestamp.
  Instant during the drag: a denser 32-tile × 320 px strip is prefetched in
  the background after the 8-tile strip loads (same disk cache, keys now
  include the tile width: `-strip{N}w{W}-{i}.jpg`; `video_filmstrip` takes a
  `tile_width` param, clamps 2–48 tiles / 80–640 px) and the popup snaps to
  the nearest tile (falls back to the coarse strip until ready). Generic
  `dragPreview` slot on `TrimStrip` (cursor-tracked, clamped, above-strip).
- **Two simultaneous videos stuttered** — even with the top layer fully
  transparent (slot renders are gated on `has_new_frame`, not opacity).
  Cause: `mpv_render_context_render()` was called without
  `MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME` (default **1**), so every call
  slept until *that* context's frame display time — with N contexts sharing
  the single render thread the waits serialise (up to ~16 ms each per pass)
  and every video drops frames. One video never showed it: a single wait
  aligns with its own cadence. Fix: pass `block_for_target_time=0` (constant
  added to `mpv_sys.rs`, id 12) on the overlay + every slot render — the loop
  is paced by the update callbacks and `video-sync=desync` owns each clock,
  exactly mpv's recommended compositor pattern. Plan B if lag persists on
  weak iGPUs: slots decode with `hwdec=auto-copy` (GPU→RAM→GPU roundtrip per
  frame, ×N videos) — switching to direct-interop `auto` is untested across
  multiple contexts on one GL thread.

### 1.3.0 (2026-07-13) — part 2 (2026-07-11): compositor black-screen fix; legacy Win32 path removed; visual cues always stack

The layer-compositor build showed a **fully black output window on Windows**
for every Video/Image/Camera GO (slots decoded and revealed fine per the log).
Root-caused with a standalone GL+mpv probe that read back every FBO stage:

- **Black screen — the overlay context masked the stage.** On libmpv 0.41-dev
  (Windows) the overlay context's **idle** render clears its FBO to **opaque
  black** — `background=none` is ignored in idle (`alpha=yes` no longer exists
  ≥ 0.38) — and it was composited above all layers at opacity 1 every frame.
  Fix (`render.rs`, `mod.rs`): the overlay is only rendered + composited while
  it actually shows something (`overlay_active()` = timer OSD / Text Cue /
  test pattern), with a one-shot `OVERLAY_DIRTY` repaint when it deactivates.
  Additionally mpv renders **no OSD at all in idle**, so timer/Text now load a
  tiny fully-transparent lavfi dummy
  (`av://lavfi:color=c=black@0.0:s=64x64:r=10,format=rgba`,
  `ensure_overlay_surface`/`release_overlay_surface_if_idle`) — probe-verified:
  with the dummy, OSD text composites with correct per-pixel alpha (97 % of
  pixels alpha=0). Known limitation: letterbox bars of a video layer are
  opaque (mpv fills them black), so they occlude layers below.
- **Zero-fade stop never unloaded the slot** — camera relaunch failed with
  `dshow: device already in use` (-17): `begin_stop(0)` parks the opacity
  animation at 0 already *resting*, and `tick_slot` required the tick to
  report "just completed" before unloading — which a resting animation never
  does.  The mpv `stop` was never issued, the slot stayed occupied and the
  capture device stayed open, so re-GOing the cue (or any cue on the same
  camera) allocated a new slot and hit the busy device.  Unload now fires
  whenever `pending_unload && opacity <= 0 && !animating` (`slot.rs`; stops
  *with* a visual fade were unaffected).  Regression test
  `resting_anim_never_reports_completed`.
- **GO intermittently erroring `mpv error (code -16)`** (NOTHING_TO_PLAY):
  `loadfile` raced the slot's `mpv_render_context` creation (render thread) —
  with `vo=libmpv` and no render context, the video track can't init, and with
  `audio=no` nothing is left to play. `create_slot` now blocks (≤ 2 s, usually
  a few ms) until the render context exists before the first load, same
  discipline as `render::init` for the overlay context.
- **Legacy Win32 output path removed**: `legacy-win32-output` feature,
  `win32_window.rs`, `cfg(output_win32)`/`cfg(output_gl)` and every dual-arm
  method in `output_engine/{mod,fade,mpv_events,types}.rs`; `build.rs` no
  longer emits output-path cfgs. The GL Render API path is the only output
  path on all three OS.
- **`stop_on_next_visual` removed** (Video/Image/Camera + `types.ts` +
  GeometryTab checkbox): visual cues always **stack** as layers — launching a
  visual cue never stops another; only Stop/Fade cues (or EOF) remove one.
  Old workspaces carrying the field load fine (ignored). The Text Cue keeps
  its own stop-on-next-GO behaviour. `Cue::stop_on_next_go()` trait mechanism
  in `transport.rs` unchanged.
- Tests: 259 pass (stop-on-next assertions updated; legacy-JSON compat tests
  now assert the field is ignored).

### 1.3.0 (2026-07-13) — part 1 (2026-07-10): Layer compositor: N simultaneous visual cues, blend modes, crossfades

The QLab video model, closing the image/video chapter: every Video / Image /
Camera cue is now an independent **layer** on the output stage — multiple cues
play at once, composited in layer order with per-cue opacity and blend mode.
Crossfades emerge naturally: GO video B (fade-in) while video A fades out —
both decode simultaneously, no black flash possible.

- **Slot pool** (`output_engine/slot.rs`, new): one mpv context per
  simultaneously-visible cue (lazy, cap 8, never destroyed; pool exhaustion
  steals the oldest content with a `Completed` so its cue resets).  Each slot:
  own event thread (PLAYBACK_RESTART reveal handshake + watchdog, END_FILE
  EOF/ERROR per voice, VIDEO_RECONFIG per-slot crop, duration reporting), own
  per-slot **opacity animation** (reveal fade-in, stop fade-out → unload, EOF
  fade, Fade Cue), per-slot geometry/hold/live-latency options,
  `background=none` + `alpha=yes` so idle/letterbox pixels are transparent.
  Layer sort key: explicit layers 1–1000 band below automatic (newest on top),
  GO order breaks ties.
- **Compositor** (`render.rs`): every slot renders into its own RGBA FBO; the
  stack is accumulated **ping-pong** with a blend shader (W3C separable modes,
  backdrop sampled as texture — GL 3.3 can't read the framebuffer), the
  overlay context (timer OSD / Text Cue / test patterns) composites topmost,
  then the global **warp** (unchanged) and the master fade quad (now purely a
  blackout curtain: startup idle, panic, cleared pattern).  Idle = engine
  clear to black, no reliance on mpv idle frames.
- **`BlendMode`** (`blend.rs`, new): 14 modes (Normal, Add, Multiply, Screen,
  Overlay, Soft/Hard Light, Darken, Lighten, Dodge, Burn, Difference,
  Exclusion, Subtract).  Pure-Rust reference formulas are the executable spec
  of the GLSL (same discipline as `warp.rs`), 12 unit tests incl. duals,
  edge-division cases and alpha compositing.
- **`LayerStyle` per cue** (`layer` auto/1–1000, `opacity`, `blend_mode`) on
  Video/Image/Camera + `stop_on_next_visual` (default **true** = pre-1.3
  replace behaviour preserved for existing workspaces; unchecked, the cue
  stays and layers).  Serialized with serde defaults; live-applied from the
  inspector via `set_layer_props` (new Compositing section in the Geometry
  tab: layer, opacity slider, blend dropdown, stop-on-next checkbox).
- **Per-voice engine API** (breaking, internal): `stop/pause/resume/seek`,
  `video_audio_voice`, `resync_audio_to_video`, `current_video_position_ms`
  all take the voice now; `get/set_voice_opacity` replace the global overlay
  alpha in the Fade path.  **Fade Cues fade each visual target's own layer**
  (multi-target visual fades finally meaningful — `set_fade_voices` carries
  `(voice, start_opacity)` pairs); Stop-at-End stops the visual voices too.
  Per-cue video pause/resume/seek now truly per-cue (used to hit the single
  global mpv).
- **Legacy Win32 path**: intact — single-context replace semantics behind
  `cfg(output_win32)` (show_content/stop_content split, mpv-props transform
  composition, global overlay fades).  Compiles warning-free.
- Text Cue on GL: the black lavfi dummy is gone (the overlay context idles
  transparent, so text floats **over** video layers; the compositor provides
  the black stage).
- Tests 238 → **259**: blend formulas/serde/shader-ids, layer-key ordering,
  opacity-anim math, LayerStyle serde + defaults, VideoCue layer roundtrip +
  legacy-JSON compat (old workspaces keep replace behaviour).

### 1.2.0 (2026-07-10) — part 4: Camera Cue (live input as a visual cue)

The theatre-user request's item 5.  A live feed is now a first-class visual
cue — fades, Geometry, the global warp and stop-on-next-GO all apply.

- **`CameraCue`** (`cue/camera_cue.rs`, `CueType::Camera`): source =
  `CameraSource::Device { id, name }` (platform capture device) or
  `CameraSource::Url { url }` (RTSP / HTTP / UDP — IP cameras, phone-camera
  apps).  mpv URL per OS: `av://dshow:video=NAME` (Windows),
  `av://v4l2:/dev/videoN` (Linux), `av://avfoundation:NAME` (macOS); network
  URLs pass through.  Modeled on ImageCue: no duration (runs until stopped),
  `stop_on_next_go`, video fade in/out via the overlay, per-cue
  `VideoGeometry`, no pause (a live feed can't).  Unconfigured source →
  instant-complete (Auto-Continue safe).  7 unit tests (serde roundtrip incl.
  tagged source, platform URL building, visual/stop flags).
- **`Cue::is_visual()` trait method** — the transport's stop-on-next-GO rule
  and the Fade-target visual detection matched `CueType::Video | Image` in
  hard-coded lists, which violated "adding a cue type never touches
  `show/transport.rs`".  Both now ask the cue (`is_visual`), so Camera (and
  any future visual type) participates without transport edits.
- **`ContentRequest.live_source`** → low-latency per-load opts (`cache=no`,
  `demuxer-lavf-analyzeduration=0.1`, `video-latency-hacks=yes`); file
  playback untouched.
- **Device enumeration** (`engine/camera_enum.rs`): Windows = DirectShow COM
  (`ICreateDevEnum` over the video-input category, hand-laid vtables —
  windows-sys 0.52 exposes COM interfaces as opaque pointers; friendly names
  are exactly what lavf's dshow demuxer matches); Linux = sysfs
  (`/sys/class/video4linux/*/name`, no ioctl); macOS = `AVCaptureDevice` via
  objc2.  New windows-sys features: Com, Com_StructuredStorage, Ole, Variant.
  `list_camera_devices` command is async + `spawn_blocking` (same
  main-thread-freeze rule as audio devices, 1.1.5).
- **UI**: "+ Cam" toolbar button (drag-insert works), 📷 icon, Camera
  inspector tab (device dropdown + ↺ rescan + missing-device entry, or
  stream-URL input with phone-app hint), Fade tab (visual fades only — no
  audio fields for a feed), Geometry tab shared with Video/Image.
- macOS note: capturing prompts for camera permission; the bundle will need
  `NSCameraUsageDescription` in Info.plist before a store-signed release.
- Tests 231 → **238**.

### 1.2.0 (2026-07-10) — part 3: corner-pin warp (perspective) + fine rotation + visual editor

The "Projector Alignment" transform graduated from mpv-props approximation to a
real **warp render pass** on the GL path.

- **`OutputTransform` v2** (`output_engine/types.rs`): `rotation` is now `f64`
  (fractional degrees — 0.1° steps in the UI; old integer workspaces load
  unchanged) and gains `corners: [[f64;2];4]` (per-corner offsets in fractions
  of the output, storage order TL,TR,BL,BR, applied after scale/rotation/pan).
- **Warp math** (`output_engine/warp.rs`, new, pure/no-GL): destination quad
  from the transform (scale+rotate about centre, pan, corner offsets — y-down
  normalized space), unit-square→quad **homography** (projective fit, affine
  fast-path for parallelograms), 3×3 inversion; `warp_matrix()` returns the
  inverse homography for the shader, `None` for identity or degenerate quads.
  12 unit tests (corner mapping, inverse round-trip, cw rotation orientation,
  fine-rotation, storage-order, singular rejection).
- **Warp render pass** (`render.rs`): when the warp is active mpv renders into
  an offscreen RGBA8 FBO (created/resized lazily to the window size) and a
  fullscreen inverse-homography fragment shader places it on the window —
  pixels outside the quad are black; the fade quad still draws on top. When
  the transform is identity the old direct-to-default-framebuffer path runs
  (zero extra cost). `WARP_DIRTY` forces one redraw on parameter change so
  edits are visible on a **paused** frame or held image, not just the next
  video frame. OSD/timer/Text composite inside mpv's frame → warped with the
  picture (correct for calibration).
- **Path split** (`mod.rs`): GL applies cue geometry *pure* to mpv and routes
  the global transform to `render::set_output_warp` (no double-apply); the
  legacy Win32 path keeps the mpv-props composition (rotation rounded to whole
  degrees, corner pin unsupported — documented in `compose_display_props`).
- **Visual editor** (`Preferences/WarpEditor.tsx`, new): SVG stage mirroring
  the warp math — drag the 4 corner handles to pin (perspective), drag the
  centre cross to pan; dashed base-quad reference, pinned corners highlighted,
  reference grid. `ProjectorToolsSection` now hosts the editor + fine Rotation
  (slider+numeric, 0.1° steps, ±180) + Scale, with "Reset corners" / "Reset
  all"; still debounced-live (40 ms) and saved in the workspace.
- Tests 218 → **231** (warp module + f64-rotation serde/composition updates).

### 1.2.0 (2026-07-10) — part 2: projector alignment + test patterns; UI polish

Second follow-up batch on the theatre-user request.

- **Global output transform — "Projector Alignment"** (`OutputTransform` in
  `output_engine/types.rs`; `DisplayPreferences::output_transform`, serde
  default): venue-level pan/scale/rotation applied to *everything* on the
  output window, **composed** with each cue's own Geometry
  (`compose_display_props`: scales multiply / mpv log2 zooms add, pans add,
  rotations add mod 360; fit/crop stay cue-only). Mirrored into the engine via
  `OUTPUT_TRANSFORM` + `LAST_CUE_GEOMETRY` (a transform edit recomposes against
  the last-applied cue geometry live); the event loop re-asserts the workspace
  value every tick (no-op unless changed) so open/new/recovery all sync without
  per-path hooks. Commands `get/set_output_transform` (set applies live).
  Preferences → Display → "Projector Alignment": sliders + numeric fields
  (pos X/Y ±0.5, scale 0.1–2, rotation) with 40 ms debounce, Reset button.
- **Test patterns** (`TestPattern` enum → lavfi graphs sized to the *target
  screen's* resolution): Grid (drawgrid cells + centre cross + border), SMPTE
  HD bars, RGB chart, Test Card (`testsrc2`, focus), White / Gray 50% / Black,
  and **Custom Image** (operator's colorimetry chart, loaded like an image).
  `OutputEngine::show_test_pattern` hard-stops current content (owning cue
  completes via the normal `Completed` path), positions the window like a GO
  (fallback + banner), applies **neutral cue geometry** so only the global
  transform shows, then `loadfile`s the pattern; `clear_test_pattern` stops +
  black. UI buttons in Preferences → Display (toggle, Clear, Image… picker —
  `dialog:allow-open` added to the preferences-window capability). Leaving
  Preferences auto-clears any active pattern (a grid must never survive into
  the show).
- **Inspector tab bar wraps** (`InspectorPanel.tsx`): the fixed flex row
  cropped trailing tabs (Geometry unreachable at narrow widths) — now
  `flexWrap: wrap` + tighter padding.
- **Output screen applies immediately** (`set_output_screen` →
  `OutputEngine::apply_output_screen`): selecting a screen in Preferences
  forces the output window fullscreen onto it on the spot (same fallback +
  banner as GO); selecting "Floating window" exits fullscreen and restores the
  floating rect (`render::set_windowed_floating`, macOS `set_windowed`, legacy
  Win32 `toggle_fullscreen_impl`). The Preferences dropdown fires on select —
  no Apply needed.
- Tests 210 → **218**: OutputTransform identity/serde-defaults, composition
  (identity / multiply-add / rotation wrap / degenerate scale), TestPattern
  URL generation (sizing, path normalisation, 0×0 guard) + serde tagging.

### 1.2.0 (2026-07-10) — part 1: video/image theatre parity (fades, geometry, hold, screen reliability)

Driven by the theatre-user GitHub feature request (video fades / geometry /
projector reliability — see memory `project_feature_request_theatre_user`).

- **Video Fade tab exposed** (`InspectorPanel.tsx`): the inspector only offered
  Fade for audio/image, so `video_fade_in/out` (fully implemented in the engine
  since 0.9.x) was unreachable — the reported "video cues can't fade" bug. Video
  now gets the four-section Fade tab (Video Fade In/Out + Audio Fade In/Out;
  labels renamed from the misleading "Image Fade In/Out").
- **Fade-out at natural EOF** (`video_cue.rs::eof_fade_remaining_ms`, shared with
  `image_cue.rs`; `OutputEngine::begin_eof_fade_out`): `video_fade_out` (and a
  timed image's `fade_out`) only applied to *manual* stops — at EOF `mpv_events`
  forces the overlay opaque, i.e. a hard cut to black. `tick()` now starts the
  overlay fade `fade_out_ms` before the natural end so it lands exactly on EOF.
  One-shot per play (`eof_fade_started`), skipped for infinite loops and
  hold-last-frame; `begin_eof_fade_out` no-ops when the voice is no longer
  current and leaves `pending=None` (EOF itself ends playback — loop semantics
  untouched).
- **Hold Last Frame** (`VideoCue::hold_last_frame`, mpv `keep-open=yes` set per
  load in `fade.rs`): video freezes on its last frame at EOF instead of cutting
  to black; the frame stays until the next visual cue replaces it. The cue still
  completes via time-based detection. Image loads force `keep-open=no` back
  (image auto-complete relies on END_FILE). Checkbox in TimeTab ("At End").
- **Per-cue Geometry for Video + Image** (`VideoGeometry` in
  `output_engine/types.rs`; `apply_geometry_props`/`try_apply_crop` in `mod.rs`;
  `GeometryTab.tsx`): fit mode (Fit/Fill/Stretch → `keepaspect`+`panscan`),
  position (`video-pan-x/y`, fraction of scaled size), scale (linear, converted
  to mpv's log2 `video-zoom`), rotation (`video-rotate`, 0–359°), fractional
  per-edge crop (converted to pixel `video-crop` once `video-params/w|h` are
  known; parked in `PENDING_CROP` and applied on `VIDEO_RECONFIG` when the load
  hasn't reconfigured yet; edges clamped to 0.45 so the rect never collapses).
  Set on **every** load (a cue without geometry resets the previous cue's
  values — mpv properties persist across loadfile) and **live-applied** from
  `update_cue` when the edited cue is the content on screen
  (`OutputEngine::is_current_voice` + `apply_geometry`, new
  `Cue::visual_geometry()` trait hook). Serialized as a `geometry` object with
  serde defaults (old workspaces load unchanged). New inspector Geometry tab
  with fit buttons, numeric fields and Reset.
- **`show_content` signature collapsed into `ContentRequest`** (types.rs) — 11
  positional args (two unused) became one struct; `OutputEngineApi` trait +
  `RecOutput` test double updated; unused `_this_fade_out_ms` dropped.
- **Output screen reliability** (`resolve_output_screen` + health banner;
  `identify_output_screen` command): a configured-but-disconnected screen index
  used to make `position_window` silently skip fullscreen placement (the window
  showed wherever it last was). Now it falls back to the **primary display** and
  raises an `output-screen` warning banner (cleared when the screen is found).
  New **Identify** button in Preferences → Display: flashes "SCREEN N" (ASS
  osd-overlay, 2.5 s, generation-token cleanup) on the selected output so the
  operator can verify the projector before the show; window is re-hidden if it
  was hidden.
- **Legacy Win32 build fixed**: `render::TEXT_OVERLAY_ACTIVE` (Text Cue, GL-only
  module) was referenced without `#[cfg(output_gl)]` — `--features
  legacy-win32-output` had been failing to compile since the Text Cue landed.
- Tests 189 → **210**: VideoGeometry (roundtrip, empty-object defaults, log2
  zoom incl. degenerate scale, fit props, crop-rect math/clamps/zero-source),
  EOF-fade window (early/inside/none/past/clamp), Video+Image geometry & hold
  serialization roundtrips + legacy-JSON defaults, `resolve_output_screen`
  (floating/found/fallback/no-screens).

### 1.2.0 (2026-07-10) — part 0 (2026-07-09): Group modes, Fade parity, stable numbering, AIFF, perf

On top of 1.1.6 (pushed to `master`, not yet tagged/released).

- **Group cues — full QLab-mode parity** (`cue/types.rs`, `cue/group_cue.rs`): added
  **Playlist** (exclusive one-at-a-time, auto-advances through every child ignoring
  their continue modes, optional group loop) and **StartRandom** (one random child per
  GO; in-house xorshift shuffle-bag → each child once before repeat, no new dep).
  Simultaneous/Sequential unchanged. Per-mode trait methods (absorbs_go/holds_playhead/
  released_playhead/active_child_id/is_complete); `set_playlist_loop` command; UI mode
  selector + Loop toggle; playhead highlighting for the new modes.
- **Group completion fixes**: children never self-complete and the top-level detector
  doesn't descend into groups → Simultaneous groups (and audio children still preloading,
  `duration()==None`) lingered Running forever (lag/stuck). Groups now reap finished
  children in `tick()`; the event loop reaps group children whose voice completed
  (`reap_voice_completed_children`).
- **Fade targeting a group** did nothing (transport read one top-level voice). New
  `Cue::all_voice_ids()` (recursive; GroupCue flattens children) + recursive target
  lookup make a Group a first-class Fade/Stop target (also lets a Fade target a nested
  cue). **Fade pan** added (fade volume and/or pan, or pan-only via `fade_volume=false`).
  **Stop at End** now hard-stops the target *cues* via a pending list the event loop
  drains, and emits `cue-state-changed` + `cue-list-refresh` for target+descendants so
  the UI doesn't freeze on RUNNING.
- **Stable cue numbering** (`show/cue_list.rs`, `preferences.rs`): reordering/add/remove
  no longer rewrites numbers (was destroying imported/blank numbers on the first move).
  Opt-in `general.auto_renumber_on_reorder`; explicit Action → Renumber All Cues
  (`renumber_cues`); one-time non-blocking hint on first reorder.
- **AIFF audio** (`Cargo.toml`, `logger.rs`): enable symphonia `aiff`+`pcm`; cap
  non-inkue `symphonia` logs at Error (a mis-probed stream used to emit millions of WARN
  → 16 MB log + frozen UI). Frontend audio picker accepts `.aif/.aiff/.aifc`.
- **Perf**: dev profile builds deps at opt-level 3 (usable `pnpm tauri dev`);
  `React.memo(CueRow)` + stable props (large lists render incrementally); dedup +
  bounded pool for media preload (58-cue import froze on load); skip the per-tick
  cue-list fingerprint when OSC feedback is off.
- **Converter** (`C:\qlab2inkue`, separate repo): map all 5 QLab group modes (groupMode
  0/1/2→sequential, 3→simultaneous, 4→start_random, 6→playlist) + `playlistLoop`; QLab 5
  media paths (`F53Alias.relativePath`); + tkinter GUI.
- **Community**: Discord server + `discord.gg/3NVGVKfJ7U` in README (only tracked `.md`).

### 1.1.5 (2026-07-08) — Preferences "Loading…" freeze + startup hang (Windows)

**Symptom (reported on 1.1.0):** on Windows with several audio devices (built-in +
external interfaces) the Preferences panel got stuck on "Loading…" forever, and on
some launches the Spacebar/GO hotkey did nothing until the app was restarted.

**Root cause:** every device query went through cpal's WASAPI path
(`host.output_devices()` + `default_output_config()` per device), which is slow-to-
hanging on Windows when one device/driver is unresponsive (cpal #867). That work ran
**on the main thread** — at startup via `DeviceManager::new()` inside `AudioEngine::new`
(`.setup()`), and in Preferences via the **synchronous** `list_audio_devices` command
(Tauri runs sync commands on the main thread). A blocked main thread freezes the whole
UI: the Preferences enumeration never returns ("Loading…" forever) and queued IPC
(including `go()`) stalls, so the hotkey appears dead. (Running as admin was incidental
— the restart is what cleared the flaky device.)

**Fix:**
- `engine/device_manager.rs`: enumeration is now bounded by `run_bounded(ENUM_TIMEOUT,
  …)` — it runs on a scratch thread and gives up after 4 s (detaching a hung WASAPI
  call) instead of blocking. `DeviceManager::new()` no longer enumerates (empty cache);
  `refresh_devices` is replaced by `replace_cache`, so the manager mutex is never held
  across a slow query. Free fns `enumerate_output_devices` / `run_bounded` + unit tests.
- `engine/audio_engine.rs`: the device cache is warmed by a background thread spawned
  from `AudioEngine::new` (startup never blocks on enumeration); the `audio_health`
  fallback check and `restart()` no longer enumerate under the lock on the main thread.
- `commands/preferences_cmds.rs` + `commands/device_cmds.rs`: `list_audio_devices`,
  `list_input_devices`, `refresh_devices` are now `async` and run the cpal work via
  `tauri::async_runtime::spawn_blocking` — off the main thread, so the UI stays
  responsive even while enumeration churns. (First async commands in the codebase.)
- `PreferencesModal.tsx`: `withTimeout` guards both the device row (8 s → inline error +
  ↺ Retry) and the initial `getPreferences()` (12 s → "Failed to load" + Retry), so the
  panel can never spin on "Loading…" indefinitely regardless of backend state.

### 1.2.0 (2026-07-10) — part -1 (2026-07-04): Output Patch routing (multi-device audio) + live-edit fix

**Output Patch routing rebuilt end-to-end** — the headline 1.2.0 feature. See the ✅
note under the cue-type table for the user-facing summary. Implementation:

- **Single source of truth**: `device_cmds.rs` now reads/writes `ws.output_patches`
  (persisted, the table `resolve_patch` uses at GO) instead of the parallel unpersisted
  `DeviceManager` HashMap (deleted — patches created in the UI used to vanish on restart
  and never affect playback). New commands: `remove_output_patch`,
  `set_default_output_patch`; `get_output_patches` returns `{patches, default_patch_id}`.
- **Engine multi-device** (`audio_engine.rs`): `AuxStream` = one extra cpal output
  stream per non-main patch device — own voice pool (RT `try_lock` contention stays
  per-stream, identical to the single-stream design), own cmd/status rings, shared
  master gain. `play_voice_routed` / `play_voice_paused_routed` decide main vs aux via
  `resolves_to_main_device` (pure, unit-tested; treats "patch names the system-default
  device while main *is* the default" as main — no duplicate stream). Voice commands
  (stop/pause/resume/gain/pan/seek/**StopAll panic**) broadcast to every stream; unknown
  voice ids are no-ops. `drain_status` merges aux statuses (aux MasterLevels filtered —
  they'd fight the main VU). `gc_voices` sweeps aux pools and drops failed+empty
  streams so the next GO retries the device. `restart()` closes an aux stream holding
  the device being opened as main (exclusive backends). Open failure at GO → fallback
  to main + `output-patch-device` health banner.
- **Cue layer**: `audio_cue.rs` / `video_cue.rs` pass `patch.device_id` to the routed
  play calls (channel mapping unchanged). Mic voices intentionally stay on main.
- **UI**: `OutputPatches/OutputPatchesPanel.tsx` (Preferences → Audio, above Input
  Patches): add/edit/remove, device dropdown with `(missing)` marker, 1-based channel
  text, ★ default patch. Inspector `LevelsTab`: Output Patch selector (Audio + Video;
  `VideoCueData.output_patch_id` added to types.ts — backend already serialised it).
- Tests: 153 → **157** (routing decision ×3, RT StopAll panic behaviour).

**Routing v2 (same day, after first ASIO field test):**

- **ASIO channel routing actually works now.** The old callback mixed a *stereo*
  scratch and copied it to the selected pair — patch channels could never reach
  outs 3+. Both stream formats (F32 + I32) now mix **full-width** (every device
  channel); `Output Pair` became the default destination for *unpatched* voices,
  applied at submission (`Voice::patched` flag + `default_out_offset`). Patches
  route anywhere: e.g. UMC 404HD ASIO, patch "Main" ch 1-2, patch "Headphones"
  ch 3-4 — both on the single main ASIO stream.
- **ASIO driver listed in the patch device dropdown** (`asio_output_devices()`,
  "(ASIO)" suffix; id matches the main stream's → routes to main, full channel
  count). Patch → WASAPI endpoints of an ASIO-held interface will typically fail
  → fallback-to-main + banner.
- **Output Mixer** (transport bar **MIX** button): one strip per patch — fader
  in dB (`OutputPatch.gain_db`, `#[serde(default)]`, persisted in the workspace,
  double-click = 0 dB) hot-applied to playing voices (`AudioCommand::SetPatchGain`
  → `VoiceInner::patch_gain_bits` multiplier, file + live paths), and stereo
  **per-patch VUs**: RT callback accumulates per `Voice::patch_slot` into a fixed
  `[f32; 16×2]` (no alloc), pushes `AudioStatus::PatchLevels`, event loop emits
  `patch-levels`. Patches beyond 16 are unmetered. Main transport VU unchanged
  (main stream only).
- Inspector: a deleted patch no longer shows its raw UUID — "(deleted patch —
  using default)" option instead.
- Known limitation: mixer fader/VU do not cover Mic/live voices (patch channels
  apply; gain/meters not wired — same scope line as their main-device pinning).

**Routing v4 (same day) — lifecycle hygiene after "it gets messy when I change
outputs" field feedback.** Three real lifecycle bugs fixed:

- **Stale aux streams across a backend/device switch.** `restart()` now closes
  *all* aux streams (old universe; a WASAPI aux holding the interface could make
  the new ASIO open fail, and stale streams kept playing on outputs the operator
  had re-configured away). Patch edits (`set_output_patch`/`remove_output_patch`)
  also `close_all_aux()` — next GO reopens exactly what the new table needs.
- **Banners now always reflect the latest GO.** `output-patch-device` is cleared
  on every *successful* routed GO (main or aux path) — previously it never
  cleared once the fix routed via main, so the UI showed errors contradicting
  reality. `close_all_aux()` clears both routing alerts.
- **Inaudible channel routing is now diagnosed.** New `output-patch-channels`
  warning (`check_channel_bounds`, main + aux submission): "Cue routed to output
  channel N but the device has only M — audio is dropped." Previously bounds
  were silently dropped in the RT callback with zero feedback (the "Headphones
  stopped working" mystery: patch ch 3-4 falling back onto a stereo output).
- Panel UX: device list refetches on `device-changed` (emitted by
  `update_machine_audio_config`, i.e. immediately when the backend/device is
  applied) in addition to window focus; a patch whose device is not in the
  active universe shows an **amber ⚠ "unavailable"** select instead of silently
  keeping the stale id. ASIO device entry is now built from the live engine
  stream state (id/channels/SR) instead of re-enumerating the single-client
  ASIO host — re-enumeration failed while our own stream held the driver, which
  is why the ASIO-only filter didn't kick in on first test.

**Routing v3 (same day, second field-test round):**

- **Backend defines the patch-device universe** (Ableton convention). On the
  ASIO backend `list_output_devices` returns *only* ASIO drivers (their WASAPI
  endpoints are ASIO-held and were misleading); on shared backends, only the
  normal WASAPI/CoreAudio/ALSA devices (no ASIO entries). Falls back to the
  shared list when ASIO is configured but not compiled in (dev builds).
- **The mixer is a real floating window** (label `mixer`, pre-declared in
  `tauri.conf.json` + `capabilities/mixer.json` — dynamic window creation is
  unreliable on WebView2). Mini-DAW vertical strips: custom pointer-driven
  vertical fader (no `slider-vertical` — identical on all 3 OS; double-click =
  0 dB, unity marker), vertical stereo gradient VUs, per-strip dB readout.
  Always-on-top, frameless, draggable title bar, ✕/Escape hides. Reloads on
  `workspace-modified`, themed via `wc_theme` localStorage. `windows/MixerWindow.tsx`;
  MIX button (transport bar) → `open_mixer_window` (show/focus pattern shared
  with the preferences window). The interim in-app modal was removed.

**Fix: changing a cue's media file while it plays orphaned its engine voice.** The
file setters (`set_audio_file` / `set_video_file` / `set_image_file`) rebuild the cue
object from JSON; a live cue's replacement was born Standby and the old voice kept
playing with no owner — unreachable even by Hard Stop (the panic-stop backstop was the
1.1.0 mitigation; this is the root fix). They now soft-stop the cue first
(`stop_if_live`, `transport_cmds::make_context` made `pub(super)`); the engine
completes the fade autonomously after the object swap.

### 1.1.0 (2026-07-03) — Network interface selection, clickable About links, auto-update

Three quality-of-life items for the first post-1.0 release. All three are cross-platform
(Windows / macOS / Linux).

**Network interface selection (Preferences → Network).** One machine-level choice pins
*all* of Inkue's IP traffic — OSC receive, OSC send (cues + Test button), OSC feedback,
sACN multicast and Art-Net — to a specific local interface. Default remains Automatic
(bind `0.0.0.0`, OS routing picks egress), which is byte-identical to the old behaviour.

- `engine/net_interface.rs` (new) — `if-addrs` enumeration (IPv4, loopback flagged),
  global `RwLock<Option<Ipv4Addr>>` selection, `udp_send_socket()` helper with fallback
  to any-interface if the pinned bind fails mid-show, name-first / stored-IP-second
  resolution (survives DHCP renewals *and* interface renames), health banner
  (`network-interface` key) when the configured interface is absent. Unit tests for the
  resolution rules.
- `preferences.rs` `NetworkInterfaceConfig` + `machine_config.rs` `network.json`
  (machine-level, like `osc.json` — never in the workspace).
- Apply points: `osc_server.rs` (bind), `osc_feedback.rs`, `cue/osc_cue.rs`,
  `commands/osc_cmds.rs` (send sockets), `dmx_sink.rs` (bind + `IP_MULTICAST_IF` via
  `socket2::SockRef` for sACN multicast; also now sets `SO_BROADCAST` for Art-Net
  broadcast destinations — previously missing, directed-broadcast sends failed).
- Hot-apply: `set_network_config` rebinds the OSC server (reconfigure) and rebuilds all
  DMX sinks (`DmxCommand::RebindSinks`; the DMX thread now retains its outputs).
- UI: `Preferences/NetworkInterfaceSection.tsx` — dropdown (Automatic + per-interface
  entries, missing-interface warning), applies immediately like the OSC section.
- Commands: `list_network_interfaces` / `get_network_config` / `set_network_config`.

**Clickable About links.** `target="_blank"` does nothing in a WebView (no browser to
hand the navigation to). Links in `AboutDialog` now route through `tauri-plugin-opener`
(`openExternalUrl` wrapper in `lib/commands.ts`) → OS default browser. Capability
`opener:default` added to the main window.

**Auto-update (tauri-plugin-updater).** Startup check (silent, 5 s delay, production
builds only) + manual "Check for Updates" button in About. Update dialog shows the
release notes, download progress, then installs and relaunches (`tauri-plugin-process`).

- `stores/updateStore.ts` (zustand state machine), `components/Update/UpdateDialog.tsx`.
- `tauri.conf.json`: `plugins.updater` (pubkey + GitHub `latest.json` endpoint,
  Windows `installMode: passive`), `bundle.createUpdaterArtifacts: true`.
- `release.yml` passes `TAURI_SIGNING_PRIVATE_KEY(_PASSWORD)` secrets to tauri-action —
  **release builds now fail until those repo secrets are set** (see workflow header;
  private key in `~/.tauri/inkue.key` on the maintainer machine, public key in
  `tauri.conf.json`). tauri-action generates `latest.json` in the release; updates go
  live when the draft release is published. Linux auto-update applies to the AppImage
  (deb/rpm users update via their package manager).
- Capabilities: `updater:default`, `process:default` (main window).

Also: `src/vite-env.d.ts` (new, standard Vite `/// <reference types="vite/client" />`)
so `import.meta.env` typechecks.

**Hard Stop All is now a true panic stop.** Double-Escape previously walked the cue
list (`is_running() || is_paused()`) and called each cue's `hard_stop()` — useless
against a cue whose state desynced from its engine voice (state says Standby but the
voice keeps playing): the filter skips it and its `active_voice_id` may be stale.
Now `Transport::hard_stop_all` adds an engine-level backstop after the per-cue pass:
`AudioEngine::panic_stop_all()` (new `AudioCommand::StopAll` — one ring command, the
RT callback silences the *entire* voice pool without needing voice IDs, can't overflow
the ring like N individual Stops) + `OutputEngine::panic_stop()` (unconditional
`mpv stop` + black quad, no `current_voice` gate), then `reset()` on every cue to
clear stale bookkeeping. Also reached via OSC `/inkue/hardstop` and hard Stop Cues
targeting All.

**Fix: video frozen on last frame at natural end (Windows + Linux, regression).**
Same class as the 0.9.2 hard-cut fix (`67174b1`), different path: at natural EOF
(`keep-open=no` → mpv idle) nothing forced the overlay to black — the GL render loop
skips when `!has_frame && alpha == 0`, so whether the screen went black depended on
mpv emitting a final render update in idle, which changed across libmpv versions
(regressed with the 2026-04 libmpv bump). `mpv_events.rs` END_FILE handler now calls
`fade::set_overlay_alpha(255)` on the EOF **and** ERROR reasons — deterministic black,
identical end state to an operator stop, no reliance on mpv idle behaviour. Looping
videos are unaffected (`loop-file` does not emit END_FILE between iterations).

### 0.9.26 (2026-06-29) — Linux UI froze during video: continuous UI animation (frontend)

The recurring "Inkue UI freezes while a video cue plays" on the weak Linux box (Intel HD 520,
`pnpm tauri dev`) is **fixed**. Root-caused by measuring the UI thread directly (an in-UI
`requestAnimationFrame` meter): during the freeze the GTK main loop stayed responsive (~150 µs)
but WebKitGTK's paint clock sat at 0 fps — i.e. **GPU/compositor contention**, not CPU and not
the output render path. Any UI element that animates **continuously** forces WebKitGTK to
recomposite the whole UI surface ~60 fps for the animation's lifetime; on a shared iGPU that
can't coexist with a Video Cue's output window also presenting → UI starves to ~0 fps. Audio
cues put no GPU load on it, so only video lagged; a production build had enough headroom and
stayed fluid.

- `components/common/RunningLed.tsx` (new) — the running-cue indicator blinks via a **discrete
  JS interval (~1.4 Hz)** instead of a CSS `@keyframes infinite` pulse, so the UI surface is
  idle between toggles. Shared by `CueList/CueRow` and `CueList/CartView`; removed the
  `wc-led-pulse` keyframe (`index.html`).
- `CueList/CueRow`, `ActiveCues/ActiveCuesView`, `ShowMode/ShowModeView` — progress bars animate
  `transform: scaleX()` on a `will-change` layer (compositor-only) and **dropped the continuous
  `transition: width/transform`**, so each 30 fps timing update is one discrete cheap commit
  instead of a permanent re-rasterisation.

Result on the operator's HW: dev mode + a 1080p video cue went from **0 fps (frozen, GO/Stop
after ~5 s) to 30+ fps (responsive)**. No backend change. See the **RESOLVED** entry under
*Known issues* for the measurements and confirmation tests.

### 0.9.25 (2026-06-26) — Linux fixes (redo 2): drag area, fullscreen-at-startup

The 0.9.24 fixes for the three Linux bugs were still reported broken on GNOME/Wayland
(Intel HD 520). Re-verified the working tree carried 0.9.24's render.rs (vsync `Wait(1)`)
and OUTPUT_VISIBLE gate uncommitted — so the running binary likely predated them — and
hardened the two frontend fixes that were genuinely too weak.

**Can't drag the narrow window — real root cause + robust fix.** 0.9.24 only set
`minWidth: 40` on the single-row drag region. With 14 cue-toolbar buttons at
`flexShrink: 0`, the toolbar keeps full width and the flex algorithm collapses the
drag region to that 40px minimum — so only the "Inkue" label is grabbable (exactly
the symptom). Fix: split the custom title bar into **two rows** — Row 1 = window
controls + File/View menus + a full-width draggable title; Row 2 = the cue toolbar
(`flexWrap: wrap`, so every button stays reachable when narrow). The drag area can no
longer be squeezed by the toolbar. No `data-tauri-drag-region` on the row containers
(keeps menus/buttons clickable on WebKitGTK). `src/App.tsx`.

**App starts maximized/fullscreen — hardened.** 0.9.24 called `unmaximize()` once on
mount, which (a) doesn't clear a *fullscreen* state and (b) races Mutter applying the
restored state *after* the window maps. Fix: the mount effect now calls
`setFullscreen(false)` + `unmaximize()` immediately **and again on a 150 ms timeout**,
reliably winning the race and covering both maximized and fullscreen restore.
`src/App.tsx`.

**Video playback froze Inkue's UI to ~1 fps (clicks delayed by seconds) — REGRESSION
REVERTED.** The operator clarified: the *video plays fine*, but Inkue's WebKitGTK UI
drops to ~1 fps while it plays (front **or** behind the output window — so not occlusion),
single screen, and "it worked fine a few days ago." File mtimes confirmed it: the
known-good commit was 11:33, but `render.rs` was edited at 12:16 the same day with the
0.9.23/0.9.24 render-path changes — those are the regression. Root cause: blocking the
render thread inside `eglSwapBuffers` (`SwapInterval::Wait(1)`, the 0.9.24 "fix") holds a
Mesa driver lock for the entire vblank wait and serialises our GL with WebKitGTK's
compositing on the main thread; the native-Wayland backend switch (0.9.23) made the two
surfaces contend directly. Reverted both to the known-good baseline:
- `engine/output_engine/render.rs` — vsync back to `SwapInterval::DontWait` (all OS);
  `build_event_loop` back to the default winit backend (no forced native Wayland). Both
  carry a comment so the regression is not reintroduced.
- `src-tauri/Cargo.toml` — glutin Linux features back to `["egl", "glx", "x11"]`.
- `engine/output_engine/fade.rs` — `hwdec` back to `auto-copy` (the operator confirmed
  `auto` made no difference, because hwdec was never the cause).
`PORTAGE.md` vsync/hwdec rows restored with a "do not re-introduce `Wait(1)`" warning.

A ~30 fps output cap + render-thread `nice` were briefly added on top of the revert, then
**removed**: they over-corrected — dropping output frames freed the UI but made the *video*
judder.

**Actual root cause — native Wayland EGL swap serialisation; fix = force X11/XWayland.**
Even at the plain `DontWait` baseline the UI still lagged during playback. The smoking gun
is in this very 0.9.23 note: *on Mesa/Wayland, `eglSwapBuffers` blocks on the compositor's
frame callback regardless of swap interval*, which serialises the output window's render
thread with WebKitGTK's UI compositing on the single iGPU. The previous sessions diagnosed
this but "fixed" it backwards (forcing **more** Wayland, then `Wait(1)`). The output window
runs on a *native Wayland* EGL surface (winit defaults to Wayland on a Wayland session),
which is the lag. Fix (`engine/output_engine/render.rs`, `build_event_loop`): **force the
X11/XWayland backend** on Linux (`with_x11()`) — XWayland's X11/DRI EGL path honours
`SwapInterval::DontWait`, decoupling the two GL clients so the UI stays fluid during video.
`INKUE_OUTPUT_BACKEND=wayland` is an opt-in escape hatch (logged at startup; glutin keeps
the `wayland` feature so it links). Windows/macOS untouched (`build_event_loop` is
per-`cfg`). `render.rs` otherwise equals the known-good baseline plus the `OUTPUT_VISIBLE`
gate.

**Residual UI lag after decode+XWayland fixed — zero-copy hwdec + opt-in FPS cap.** Logs
confirmed X11/XWayland *and* `vaapi-copy` hardware decode both active, yet the UI still
lagged → the remaining cost is the output window's GPU compositing/bandwidth (consistent
with the earlier cap freeing the UI). Two Linux levers: (1) `hwdec` `auto-copy` → `auto`
(direct VAAPI↔GL interop, zero-copy) so the decoded surface is imported as a DMA-BUF/
EGLImage instead of round-tripping GPU→RAM→GPU — halves memory-bus traffic on the shared
iGPU, no video-smoothness cost (`engine/output_engine/fade.rs`). (2) An **opt-in** output
FPS cap `INKUE_OUTPUT_FPS` (default off/uncapped; `render.rs`) the operator can set to e.g.
30 if the UI still lags — it halves the output's present rate (UI headroom) at the cost of
video smoothness, so it stays off by default. Windows/macOS unaffected (both `cfg(linux)`).

**XWayland needs `libxkbcommon-x11`.** winit's X11 backend hard-requires it and *panics*
(not a recoverable `build()` error) during window creation if absent — which it is on a
Wayland-only install, so the first attempt to force X11 crashed the output engine. Fixed by
probing the lib with `dlopen` up front (`x11_xkb_available()`): X11/XWayland is selected
only when the lib is present, else we fall back to native Wayland (app still runs) and log a
warning naming the package to install. Added `libxkbcommon-x11-0` to the `.deb` `depends`
(`tauri.conf.json`) so packaged builds get the smooth path automatically; `Xwayland` and the
other X11 client libs (libX11/libxcb/libxkbcommon) are already standard on GNOME-Wayland.

**Output window at startup — kept the fix.** The `OUTPUT_VISIBLE` frame-commit gate (added
alongside the reverted changes) is orthogonal to performance and correctly keeps the
output window unmapped until `render::show()`, so it is retained.

### 0.9.24 (2026-06-26) — Linux fixes (redo): vsync, fullscreen, toolbar

Correct fixes for the three Linux bugs reported in 0.9.23; the 0.9.23 CSS
approach introduced two regressions (dropdown menus clipped, toolbar buttons
squished) and the fullscreen fix was incomplete.

**Video lag — root cause corrected.** On Linux with Mesa/Wayland EGL,
`eglSwapInterval(0)` (DontWait) is ignored: `eglSwapBuffers` still blocks on the
compositor's frame callback. But when it blocks _without_ yielding the GL context
ownership, it serialises our render calls with WebKitGTK's GL commands — causing
visible UI lag. Fix: use `SwapInterval::Wait(1)` on Linux so `swap_buffers()` does
a proper vblank wait that yields the GPU to other contexts between frames. The
render thread now blocks at ~60 fps, giving WebKitGTK uncontested GPU time in
between. `engine/output_engine/render.rs`.

**Output window visible at startup** (0.9.23, retained): `OUTPUT_VISIBLE: AtomicBool`
gates all frame commits. Already correct in 0.9.23.

**Wayland-native backend** (0.9.23, retained): `build_event_loop()` prefers Wayland
over X11 on Wayland sessions. Already correct in 0.9.23.

**Main app window starts maximized/fullscreen — correct fix.** `"maximized": false` in
`tauri.conf.json` only sets the initial Tauri default; GNOME's session manager
overrides it by restoring the previous WM state on every launch. Fix: call
`getCurrentWindow().unmaximize()` from a `useEffect` on mount — runs after the WM
has positioned the window and reliably overrides the restored state. `src/App.tsx`.

**Can't drag narrow window — correct fix without regressions.** The 0.9.23 fix put
`overflow: hidden` on the 36px title bar container, which clipped absolutely-
positioned dropdown menus (File/View menu) that extend below the container. It also
put `flexShrink: 1` on the toolbar, which caused button text (e.g. "+ Audio") to
wrap when the buttons compressed. Correct fix: remove `overflow: hidden` from the
container; revert toolbar to `flexShrink: 0` (natural width); keep `minWidth: 40`
on the drag region. When the window is narrow, the toolbar overflows right and is
clipped by the root `overflow: hidden` on `<html>`, not by any ancestor flex
container — so dropdowns are unaffected and button labels never wrap. `src/App.tsx`.

### 0.9.23 (2026-06-26) — Linux UI lag + output window at startup + title bar drag

Three Linux-specific bugs fixed.

**Video lag when playing a cue.** The output window's winit event loop forced X11 via
`EventLoopBuilderExtX11::with_any_thread(true)` even on Wayland sessions, pushing all
rendering through XWayland. On modern Linux distros (Wayland by default), this creates
a translation layer that competes with WebKitGTK/Wayland for GPU time, causing visible
UI lag while video decodes. Fix: `build_event_loop()` now detects `WAYLAND_DISPLAY` and
builds the Wayland-native backend first (`EventLoopBuilderExtWayland::with_any_thread`);
X11 is the fallback for pure X11 sessions. Also added `wayland` to glutin's Linux
features so the EGL display initialises from the Wayland display handle instead of going
through XWayland. `engine/output_engine/render.rs`, `src-tauri/Cargo.toml`.

**Output window visible at startup.** On Wayland, a `wl_surface.commit()` with a buffer
permanently maps (shows) the surface, even before any explicit `set_visible(true)` call.
The render loop was always rendering (alpha=255 from `FadeAnimState::idle()`) and
committing frames regardless of window visibility, so the output appeared at startup.
Fix: new `OUTPUT_VISIBLE: AtomicBool` (false at init) in `render.rs`. The render loop
skips all work while `OUTPUT_VISIBLE==false`. `render::show()` sets the flag and calls
`wake()` so the first frame is committed immediately when the operator opens the window
(or the first visual cue fires); `render::hide()` clears the flag. The
`FadeAnimState::idle()` alpha=255 is retained — it is still the correct idle state for
_when the window is visible_.  `engine/output_engine/render.rs`.

**Main app window starts maximized/fullscreen.** Added `"maximized": false` to the main
window config in `tauri.conf.json` to prevent Linux window managers (especially GNOME)
from auto-maximizing client-side-decorated windows on first launch or session restore.

**Can't drag the main window when narrow.** The toolbar (`flexShrink: 0`) would push the
drag-region div to zero width when the window was made narrow, leaving no grabbable area.
Fix: title bar container gets `overflow: hidden`; toolbar gets `flexShrink: 1, overflow:
hidden, minWidth: 0` so it clips its right-side buttons when space is tight; drag region
gets `minWidth: 40` so it always has a grabbable strip. `src/App.tsx`.
_(Note: this 0.9.23 CSS fix introduced regressions — see 0.9.24 for the correct approach.)_

### 0.9.22 (2026-06-26) — Precise A/V re-sync (mpv time-pos)

Tightens 0.9.21. The re-sync seeked both clocks to the cue's *wall-clock* `action_elapsed`, an approximation of mpv's real position, leaving a small residual offset. Now `OutputEngine::resync_audio_to_video()` reads mpv's actual `time-pos` (new `current_video_position_ms`) and seeks **only the paired audio voice** to it — mpv (the picture) is the master and is left untouched. The event-loop freeze-guard calls it while the cue is still paused, so audio and video are aligned to mpv's true position before playback resumes. Residual is now just the inherent, fixed render/output-buffer latency. `engine/output_engine/mod.rs`, `show/event_loop.rs`.

### 0.9.21 (2026-06-26) — Re-sync video A/V after an audio outage

Follow-up to 0.9.20. mpv runs on its own display clock, independent of the cpal audio device, so during the ~250 ms freeze-detection window the picture kept advancing while the paired audio voice was frozen — leaving a constant lip-sync offset after the freeze guard paused/resumed the video cue. Fix in `show/event_loop.rs`: when the freeze guard resumes an auto-paused **Video** cue, it first re-seeks (`output_engine.seek`, which repositions mpv *and* the paired audio voice together) to the cue's frozen `action_elapsed`, so audio catches up to the picture before playback resumes. Audio cues are unaffected (single clock, already in sync).

### 0.9.20 (2026-06-25) — Freeze the cue timeline during an audio outage

Follow-up to 0.9.19. With voices preserved across a device loss, the audio froze but the cue's **wall-clock timeline kept advancing** — so `time_done` (`action_elapsed >= duration`, event_loop.rs:396) eventually completed the cue while its (still-queued) audio kept playing, leaving an unstoppable voice. Fix in `show/event_loop.rs`: an **audio-freeze guard**. The 30 fps tick watches `AudioEngine::callback_count()`; if it stops advancing for `AUDIO_FREEZE_MS` (250 ms), every running audio cue (`playing_voice_id().is_some()`) is **paused** — which freezes its `action_elapsed` in sync with the frozen audio and makes the completion loop skip it (Paused ≠ Running). When callbacks resume, the cues we auto-paused are resumed. Detection latency caps the drift at ~250 ms; a planned switch's shorter gap never trips it (no pause flicker). `cue-state-changed` (running↔paused) events keep the UI in sync.

### 0.9.19 (2026-06-25) — Seamless audio across a device switch

A device switch (planned change in Preferences, or an auto-fallback after a loss) no longer stops the running cue. `AudioEngine::restart` now **preserves the voice pool** instead of killing it: the `voices` Vec is shared with the new stream's callback, so each voice resumes from its current `frame_pos` on the new device. This is safe cross-device because the cursor is in source frames (output-rate-independent — `fill_buffer` resamples per output rate) and channel routing is already bounds-checked (`if voice.out_l < channels`). On an unplanned loss the voices simply freeze during the gap (the engine produces no callbacks) and resume when the watchdog opens the fallback ~2 s later. Cross-platform: all via generic cpal, no per-OS code.

- `engine/audio_engine.rs` — `restart` drops the stream + re-opens without clearing/stopping voices.
- `commands/health_cmds.rs` (`restore_audio_device`) and `preferences_cmds.rs` (`update_machine_audio_config`) — removed the running-cue reset; the cue keeps playing.

Completion stays correct: an AudioCue completes on the engine's `AudioStatus::Completed` (voice reaching its end), not on a wall-clock timer, so a frozen voice finishes after it resumes — no premature cutoff. Known minor: on an *unplanned* loss the cue's displayed elapsed/remaining drifts by the ~2 s detection latency (audio resumes at the correct position; only the clock is ahead). A planned change has a ~tens-of-ms gap, negligible.

### 0.9.18 (2026-06-25) — Reliable mid-show device-loss detection

The 0.9.14 watchdog never fired on a real unplug: the cpal error callback only set `stream_failed` after **50** `DeviceNotAvailable` errors, but a WASAPI device removal fires it once or twice — so the flag never tripped and no banner appeared. Fixed in `engine/audio_engine.rs`:
- `stream_failed` is now set on the **first** `DeviceNotAvailable`.
- Added a kind-agnostic **heartbeat**: a monotonic `output_callbacks` counter incremented in every output callback (shared across restarts). The `inkue-device-watchdog` (`lib.rs`) treats a count that stops advancing for one ~2 s tick as a dead stream — so device loss is detected even if cpal surfaces no error or a different error kind.
- The cpal error log now includes `err.kind()` for diagnosis via the in-app log viewer.

### 0.9.17 (2026-06-25) — Dismissible health banner

The health banner's left glyph was the severity icon (`✕` for error), which read as a non-working close button. Changed the error icon to a dot (`●`) and added a real dismiss (`×`) button on the right. Dismissal is client-side and keyed on the alert's content (`key|message`), so a changed/recurring fault — e.g. the device's "is back" alert — re-appears; stale dismissals are pruned when their alert clears. `components/Health/HealthBanner.tsx`.

### 0.9.16 (2026-06-25) — Friendly audio device name in alerts

`MachineAudioConfig` gains `device_name: Option<String>`, captured at selection time in Preferences → Audio. The device watchdog banner now shows the human-readable name ("Focusrite Scarlett…") instead of the raw WASAPI endpoint id, even when the device is absent (`audio_health()` resolves presence by id but reports the friendly label, falling back to the id for devices saved before this field existed). `engine/audio_engine.rs`, `preferences.rs`, `PreferencesModal.tsx`, `lib/types.ts`.

### 0.9.15 (2026-06-25) — Startup-fallback fix + English-only UI

- **Bugfix (`engine/audio_engine.rs`)** — `AudioEngine::new` panicked at startup when the saved audio device was absent (e.g. an interface unplugged since it was configured), taking the whole app down. It now falls back to the system default on that failure (sets `in_fallback`, keeps the operator's choice as `desired_config`), so the app always starts; the device watchdog then raises the banner and offers a restore when the device returns.
- **i18n** — all user-facing strings introduced in 0.9.12–0.9.14 (health banner, preflight panel, log viewer, recovery prompt, validation messages) were mistakenly in French; converted to English to match the rest of the UI.

### 0.9.14 (2026-06-25) — Hardware/network resilience (audio + MIDI)

A device that drops mid-show no longer silently kills the show — it is detected, worked around, and surfaced to the operator. Professional-readiness item toward 1.0.

- **`health.rs`** (new) — cross-cutting runtime-health registry (keyed `HealthAlert`s + `SEQ`), same pattern as `logger`. Idempotent `set`/`clear` so the watchdog re-asserts every tick for free; only real changes bump `SEQ`.
- **`engine/audio_engine.rs`** — the per-stream `stream_failed` flag is now stored (replaced on each restart) along with the operator's `desired_config`, the `current_device_id`, and an `in_fallback` flag. New methods: `audio_health()` (enumerates devices **only** while in fallback, so the steady state is just an atomic read), `apply_user_config()` (explicit device change → records desired + clears fallback), `fall_back_to_default()` (auto-switch to default on loss), `restore_desired()` (manual re-switch). The one-shot 500 ms startup watchdog is removed (subsumed by the continuous one).
- **`lib.rs`** — `inkue-device-watchdog` thread (2 s): on output-device loss falls back to the default device to keep audio alive and raises an error banner; when the desired device returns it switches the banner to a "Rebasculer" action (no automatic re-switch — re-opening the stream glitches audio, never forced onto a critical cue). Emits a throttled `health-changed` event.
- **`cue/midi_cue.rs`** — `send_midi_messages` raises a keyed health alert on a missing/unreachable port and clears it on the next successful send to that port (self-healing).
- **`commands/health_cmds.rs`** (new) — `get_health_alerts`, `restore_audio_device` (resets running cues since the restart kills voices). `update_machine_audio_config` now routes through `apply_user_config` and clears the audio alert.
- **Frontend** — `HealthBanner` (non-blocking stack under the title bar, per-level colour, action button), `workspaceStore.healthAlerts` + `refreshHealth`, `health-changed` listener in `useTauriEvents`.

Scope: output audio + MIDI. Network UDP (OSC / DMX) detection and input-device (Mic) loss are deliberately out of this v1. Note: an automatic fallback kills currently-playing voices (the device is gone anyway) — the operator re-triggers; seamless voice migration is not attempted.

**Tests** — 143 pass; `cargo clippy --lib` + `tsc --noEmit` clean. Version 0.9.14.

### 0.9.13 (2026-06-25) — Preflight + relink, in-app log viewer

Two professional-readiness items toward 1.0.

**Preflight ("Check Workspace") + media relink.** Surfaces every cue whose external dependency does not resolve, before the show, with inline fixing.
- **`cue/validation.rs`** (new) — `Severity` (error/warning), `CueIssue`, `ValidationContext` (all cue IDs, fixture/group IDs, OSC patch IDs, output patch IDs, available MIDI ports).
- **`cue/traits.rs`** — new `validate(&self, ctx) -> Vec<CueIssue>` (default empty; a new cue type validates itself). Implemented on Audio (dangling Output Patch), Stop/Fade (dangling targets), Light (unpatched fixture/group), Osc (missing patch), Midi (absent/unconfigured port). Media-file existence is checked centrally via `media_file_path()`.
- **`commands/preflight_cmds.rs`** (new) — `check_workspace` walks all lists/nested groups → `Vec<CueValidation>`; `relink_media(cue_id, new_path)` rebuilds the cue with the new file and auto-relinks every other missing file found in the same folder (then re-preloads audio/video). 2 unit tests (Stop dangling target, MIDI absent port).
- **Frontend** — `PreflightModal` (issue list + per-file "Localiser…" relink), title-bar ⚠ badge (error count, opens the panel), `workspaceStore.refreshValidation` + `brokenCueIds`, debounced re-validate on `workspace-modified` (`useTauriEvents`). The existing per-row `is_broken`/`is_warning` indicators (media files) are unchanged. File menu → "Check Workspace…".

**In-app log viewer.** Logs are now visible to the operator without a terminal.
- **`logger.rs`** (new) — custom `log` backend fanning out to stderr + a size-rotated file (`%APPDATA%/Inkue/logs/inkue.log`, one backup) + a 2000-line in-memory ring buffer. Replaces `env_logger` (removed; `log` now carries the `std` feature). `RUST_LOG=debug/trace` still bumps the level.
- **`commands/log_cmds.rs`** (new) — `get_recent_logs`, `clear_logs`, `open_logs_folder` (per-OS reveal). `lib.rs` spawns a `inkue-log-emitter` thread emitting a throttled `logs-updated` event (event-driven live tail, no frontend polling).
- **Frontend** — `LogViewerModal` (level filter, follow/auto-scroll, copy, open folder, clear). File menu → "Logs…".

**Tests** — 143 pass (141 + 2 validation). `cargo clippy --lib` + `tsc --noEmit` clean. Version 0.9.13.

### 0.9.12 (2026-06-25) — Crash recovery (autosave)

Continuous crash-recovery snapshot so an abnormal exit (crash / power loss) loses at most a few seconds of work — the first reliability item on the road to a professional 1.0.

- **`recovery.rs`** (new) — snapshot lives at `%APPDATA%\Inkue\recovery.inkue` (per-OS config dir, reusing `machine_config::config_base_dir`, so dev writes never trip the source-tree file watcher). Atomic write (`.tmp` + rename) so a crash mid-write never corrupts it. `info()` parses the header for the restore prompt; `exists()/read()/delete()`.
- **`show/workspace.rs`** — `revision: u64` field bumped by `mark_modified` (the single mutation chokepoint) so the autosave thread only re-serialises when the show actually changed. `to_recovery_json()` keeps media paths **absolute** (the snapshot is not beside the media) and embeds `recovery_original_path`. `load()` refactored to share `from_json_str(content, base_dir, registry)` — `base_dir: None` parses the absolute-path recovery snapshot.
- **`lib.rs`** — `inkue-autosave` thread (3 s tick): writes the snapshot while `is_modified`, deletes it once the show is saved/pristine. The `WindowEvent::Destroyed` handler deletes the snapshot on any deliberate close, so presence at startup ⇒ previous session crashed.
- **`commands/recovery_cmds.rs`** (new) — `check_recovery` (→ `RecoveryInfo`), `restore_recovery` (loads the snapshot, re-targets the original `.inkue`, marks dirty), `discard_recovery`. `workspace_cmds::install_workspace` extracted from `load_workspace` and shared with restore. `save_workspace` now drops the snapshot on explicit save.
- **Frontend** — `App.tsx` one-time mount prompt via `ask()` (native dialog): restore or discard. `lib/commands.ts` + `lib/types.ts` (`RecoveryInfo`). `capabilities/default.json` gains `dialog:allow-ask`.
- **mpv_sys.rs** unaffected; version bumped to 0.9.12 across `Cargo.toml`, `tauri.conf.json` (was drifting at 0.9.10), `package.json`.

**Tests** — 141 pass (workspace `load` refactor covered by existing serialize/roundtrip tests); `cargo clippy --lib` + `tsc --noEmit` clean.

### 0.9.11 (2026-06-25) — Text Cue

Displays formatted text on the mpv output surface. Uses mpv's `osd-overlay` command (`format=ass-events`) with ASS inline tags — completely separate from the OSD timer (`osd-msg1`), so both can be active simultaneously.

**Bugfix (post-initial):** the first cut wrote the `sub-text` property, which is **read-only** — nothing rendered (output window opened blank). Switched to the `osd-overlay` command, the API-supported way to draw client ASS. This required FFI bindings for `mpv_command_node` + `mpv_free_node_contents` and the `mpv_node`/`mpv_node_list` structs (`engine/mpv_sys.rs`), since `osd-overlay` mandates named arguments (positional order is not guaranteed). The deferred `TEXT_PENDING_ASS` re-application in `PLAYBACK_RESTART` was removed — unlike subtitle state, an `osd-overlay` persists across file loads. The black `av://lavfi` dummy source is kept to give the OSD a compositing surface (and a black background) when no video/image is playing.

- **`cue/text_cue.rs`** (new) — `TextCue` struct + `TextPosition` enum (9-point grid) + `TextCueFactory`. Key fields: `text`, `font`, `font_size`, `text_color` (#RRGGBB), `position`, `screen_index`, `display_duration_ms`. `build_ass_text()` emits `{\an<N>\fn<family>\fs<size>\c&H00BBGGRR&\bord2\shad1\3c&H00000000&\4c&H00000000&}Text` (ASS colour is BGR-reversed from the hex input; `\N` for multiline). Empty text = instant complete. `stop_on_next_go() = true`. 12 unit tests.
- **`cue/types.rs`** — `CueType::Text` variant added.
- **`cue/mod.rs`** — `pub mod text_cue`.
- **`engine/output_engine/mod.rs`** — `show_text_overlay(ass_text, screen_index)` positions the output window + issues `osd-overlay` via helpers `osd_overlay_set` / `osd_overlay_remove` (`command_node_map` builds the `MPV_FORMAT_NODE_MAP`); `clear_text_overlay()` removes the overlay (`format=none`).
- **`state/app_state.rs`** — `TextCueFactory` registered in `CueRegistry`.
- **`lib/types.ts`** — `CueType` union gains `"text"`; `TextPosition` type; `TextCueData` interface.
- **`components/Inspector/TextTab.tsx`** (new) — multiline textarea, font picker (`listSystemFonts`), size input, colour picker + hex input synced, 9-button position grid, auto-complete duration toggle.
- **`components/Inspector/InspectorPanel.tsx`** — `isText` flag, Text tab button, `TextTab` wired.
- **`App.tsx`** — `handleAddText` handler + `+ Text` toolbar button with drag support.

**Tests** — 142 expected (130 prior + 12 new TextCue — run `cargo test` from `src-tauri/` to confirm). `tsc --noEmit` clean.

### 0.9.10 (2026-06-24) — Inline Editing + Active Cues View

#### Inline Editing

Double-click any `pre_wait`, `post_wait`, or `duration` (Wait/Fade only) cell in the cue list to edit it in-place.

- **`components/CueList/CueRow.tsx`** — `editingCell` / `editingValue` state; `inlineInput()` renders a focused `<input>` with accent border; `commitInlineEdit()` parses seconds (supports `"1.5"`, `"1:30"` formats) and calls `updateCue`; `stopPropagation` prevents row drag/double-click from firing. `parseSeconds` helper and `INLINE_INPUT_STYLE` defined at module level.
- **`components/CueList/CueListView.tsx`** — threads `onRefresh` prop through to `CueRow`.

#### Active Cues View

Compact panel that auto-appears above the cue list whenever one or more cues are running or paused.

- **`components/ActiveCues/ActiveCuesView.tsx`** — new component; `flattenActive()` recursively collects running/paused cues from the nested tree; one `ActiveCueRow` per active cue: color stripe, icon, number, name, state badge (RUNNING / PAUSED), remaining time (or elapsed for infinite cues), bottom progress bar, stop button; `maxHeight: 180px` with overflow scroll; sticky "Active [N]" header; auto-hides when no active cues.
- **`App.tsx`** — `<ActiveCuesView />` inserted between CueListTabs and the main view.

**Tests** — 130 total, unchanged (pure frontend). tsc clean.

### 0.9.9 (2026-06-24) — Cart Mode

Per-cue-list mode property: **Sequential** (current behavior, playhead-driven) or **Cart** (QLab-style grid of trigger tiles).

- **`show/cue_list.rs`** — `CueListMode` enum (`sequential` | `cart`, default sequential); `mode` field on `CueList`; serialized in `.inkue` (backward-compat default). `to_json` + `from_json` updated.
- **`show/transport.rs`** — `Transport::go_by_id(cue_list, cue_id)`: parks the Playhead on the given cue and fires via the normal GO path, so Auto-Continue / Auto-Follow still work.
- **`commands/cue_list_cmds.rs`** — `CueListInfo.mode` added; new `set_cue_list_mode(id, mode)` command.
- **`commands/transport_cmds.rs`** — new `go_cue(cue_id)` command (same loading guard as `go`, calls `go_by_id`).
- **`lib.rs`** — both new commands registered in invoke_handler.
- **`lib/types.ts`** — `CueListMode` type; `CueListSummary.mode` field.
- **`lib/commands.ts`** — `goCue()`, `setCueListMode()`.
- **`components/CueList/CartView.tsx`** — new component: responsive CSS grid (`auto-fill, minmax(160px, 1fr)`), one tile per top-level cue. Each tile: color stripe (left edge), cue number (top-left), type icon (top-right), name (bold, 2-line clamp), running LED + remaining time + STOP button (footer). Progress bar (bottom edge, green). Running: green border + tint + pulsing LED. Paused: orange border + tint. Completed: dimmed.
  - **Drag to reorder** — mousedown+threshold activates drag; dragged tile is removed from `displayItems` and replaced by a `DropSlot` (dashed accent border) that moves with the cursor as it crosses tile midpoints — grid CSS reflowing naturally around it. On drop: `moveCue(id, insertIndex)` where `insertIndex` is already the after-removal index (no adjustment needed). Floating **DragGhost** follows cursor; rotation driven by exponentially-smoothed horizontal velocity (`smoothedVel = 0.78*prev + 0.22*dx`) giving inertia up to ±13°. System cursor hidden (`cursor:none`) during drag; ghost fade-in via `wc-ghost-appear` keyframe.
  - **Drag from toolbar** — listens to `inkue:cue-drag-start` CustomEvent (same as sequential mode); inserts `DropSlot` at cursor position; on drop calls `addCue(type, insertIndex)`.
  - **File drag-drop** — Tauri `onDragDropEvent`; inserts `DropSlot` at cursor position; creates cues with file assigned and name from filename.
  - **Insert indicator** — `DropSlot` is a dashed-border placeholder cell that flows in the grid (not injected via box-shadow). Color-stripe overlay uses `zIndex: 10` to always appear above cue color stripe.
- **`components/CueList/CueListTabs.tsx`** — "Switch to Cart Mode / Sequential Mode" in context menu; CART badge on cart-mode tabs.
- **`App.tsx`** — branches on `activeList.mode === "cart"` to render `CartView` (inspector hidden in cart mode).
- **`index.html`** — `@keyframes wc-ghost-appear` (opacity 0→0.93, 100ms) + `.wc-drag-ghost` class.

**Tests** — 130 total, unchanged (cart mode is pure transport reuse). Clippy clean. tsc clean.

### 0.9.8 (2026-06-24) — Show Mode + CueList LED indicator

#### Show Mode (`View > Show Mode` / `F5`)

Read-only, full-window presentation view — replaces the cue list and inspector while keeping the transport bar fully operational.

- **`components/ShowMode/ShowModeView.tsx`** — bubble-card list of all cues, groups flattened to their children. Each card shows: cue number (left, monospace), name (bold), status label (right).
  - Status mapping: **Completed** (opacity 0.45, no border — idle cues before the playhead) · **Armed** (cyan border + tint — next GO target) · **Ready** (subtle border — idle cues after playhead) · **Running MM:SS** (green border + tint + bottom progress bar) · **Paused MM:SS** (orange border) · **Loading…**
  - `computeArmedIds` — mirrors `CueListView`'s inner-playhead logic for sequential groups (`active_child_id`) and simultaneous groups (all children), so the Armed highlight is always correct even inside nested groups.
  - Auto-scroll: smooth scroll to the Armed (or Running) card on every playhead change.
- **`hooks/useKeyboardShortcuts.ts`** — `F5` → `onToggleShowMode` (8th parameter, added to dependency array).
- **`App.tsx`** — `showMode: boolean` state; View menu entry "Show Mode" with `F5` shortcut displayed; toolbar buttons hidden when active; ShowModeView rendered instead of CueList + Inspector.

#### CueList LED indicator

- **`components/CueList/CueRow.tsx`** — `RunningLed` component: 8px green circle, CSS `wc-led-pulse` animation. Sync: `animation-delay` set to `-(Date.now() % 1800) / 1000` seconds at mount (via `useRef`, stable across re-renders) so all concurrent LEDs share the same phase. Playhead triangle left-aligned with `paddingLeft: 6`.
- **`components/CueList/columns.ts`** — new `"led"` column (20px, fixed, non-resizable), inserted right after `"playhead"`; `loadColumnConfig` migration ensures ordering is correct for existing saved configs; LS key bumped to `inkue_column_config_v2` to force a clean default on the first load.
- **`index.html`** — `@keyframes wc-led-pulse` (1.8 s ease-in-out, opacity 0.2 → 1 with a green glow at 50 %).

### 0.9.7 (2026-06-23) — cpal 0.15.3 → 0.18.1 upgrade (Mic Cue crash root-fix)

**Root cause of the Mic Cue "kills all audio" bug (0.9.5/0.9.6 vendor patch)** — cpal 0.15.3's
ALSA backend had three bugs that compounded into a process-wide SIGABRT: `stream_timestamp()`
called `panic!()` when `htstamp < trigger_htstamp` (transient state right after XRun recovery
resets `trigger_htstamp`); `process_input()` underflowed on `callback.sub(delay_duration)` when
`callback == 0`; and `Stream::drop()` called `join().unwrap()`, so a thread that had already
panicked double-panicked on drop → SIGABRT → the whole process (audio, video, OSC) restarted,
not just the audio thread. 0.9.6 carried a vendor-patched `cpal-0.15.3` (`[patch.crates-io]`)
fixing all three. This release replaces that patch with the upstream fix: **cpal 0.18.1**, which
resolves the same bug cluster natively (no more vendored fork to maintain).

- **`Cargo.toml`** — `cpal = "0.15"` → `"0.18"`; `midir = "0.10"` → `"0.11"` (0.10 pulls
  `alsa 0.9`, which conflicts with cpal 0.18's `alsa 0.11` — both `links = "alsa"`, Cargo only
  allows one). `vendor/cpal-0.15.3/` and the `[patch.crates-io]` block removed.
- **API migration** (`engine/{audio_engine,audio_input,device_manager}.rs`,
  `commands/preferences_cmds.rs`): `cpal::StreamError` → `cpal::Error` + `.kind()` /
  `cpal::ErrorKind` in error callbacks; `build_*_stream(&cfg, …)` → `build_*_stream(cfg, …)`
  (`StreamConfig` is now `Copy`, passed by value); `cpal::SampleRate(n)` newtype removed —
  `sample_rate()` now returns a plain `u32`.
- **Device identity pitfall** — cpal 0.18 removed `Device::name()`. The naive replacement,
  `Device::to_string()` (now `Display`), returns the **human-readable label** (e.g. `"PipeWire
  Sound Server"`), not the **stable PCM/host id** (e.g. `"pipewire"`, `"hw:0,0"`) that output
  patches, input patches, and the `pw:<node>` PipeWire routing in `device_manager.rs` store and
  match against. Using `to_string()` for matching broke every device lookup (`"Audio device
  'pipewire' not found"` at startup). Fix: `Device::id()` → `Result<DeviceId, Error>`, and
  `DeviceId::id()` is the stable identifier — used for all storage/matching;
  `Device::to_string()` is reserved for the UI-facing `DeviceInfo.name` field only. See
  `PORTAGE.md` for the general rule.
- **No regressions** — same three-bug class confirmed fixed upstream (no panic/SIGABRT
  observed across repeated Mic Cue GO/Stop cycles on Linux/PipeWire); all 130 tests still pass.

### 0.9.6 (2026-06-23) — Timecode (MTC receive + generate, LTC codec, per-cue triggers)

**Architecture** — trois couches propres, rien dans `transport.rs` / `cue_list.rs` :

- **`engine/timecode_types.rs`** — `TcPosition` / `TcRate` (24/25/29.97/29.97df/30), conversions SMPTE ↔ frames (drop-frame 29.97 inclus), Real-Time (ms) ↔ frames, `TcTrigger`, `TcEvent`, `CueListTcConfig`, `TcOnStop`. 13 tests.
- **`engine/timecode_receiver.rs`** — `TimecodeReceiver` (thread `inkue-tc-mtc`, `midir::MidiInput`), `MtcAssembler` (quarter-frame state machine + full-frame SysEx), `TcFlywheel` (interpolation + freewheel). 4 tests.
- **`engine/ltc.rs`** — `LtcEncoder` / `LtcDecoder` biphase-mark : encode `TcPosition → [f32]`, decode `[f32] → TcPosition`. Sync word vérification. 3 tests.
- **`engine/timecode_generator.rs`** — `MtcGenerator` (thread `inkue-tc-gen` : quarter-frames à 4×fps, full-frame jam-sync au démarrage). 3 tests.
- **`cue/timecode_cue.rs`** — `TimecodeCue` : génère MTC sur GO (`MtcGenerator`), start/end frame (durée calculée), plusieurs flux simultanés, `CueType::Timecode`, registry. 3 tests.
- **`show/cue_list.rs`** — `CueList.tc_config: CueListTcConfig` + `tc_triggers: HashMap<CueId, TcTrigger>` + garde monotone `tc_last_triggered_frame`. Sérialisé dans `.inkue`.
- **Dispatcher** — `event_loop.rs` reçoit `TcEvent` via channel, franchissement monotone + ré-armement sur saut arrière, émet `timecode` event Tauri pour l'UI.
- **`engine/timecode_receiver.rs`** — `TcReceiverConfig`, `TimecodeReceiver.reconfigure()` (comme `OscServer`). `machine_config.rs` : `TcMachineConfig` + `load/save_tc_config`.
- **Commands** — `timecode_cmds.rs` : `get/set_tc_config`, `get_tc_position`, `list_tc_midi_input_ports`, `get/set_cue_tc_trigger`, `get/set_cuelist_tc_config`.
- **Frontend** — `TriggersTab.tsx` (SMPTE ou RealTime, sur chaque cue), `TimecodeTab.tsx` (TimecodeCue inspector), `TcStatusIndicator.tsx` (position live dans TransportBar, flash sur lock), `TcPreferences.tsx` (Network prefs, source + port MIDI), bouton `+ TC`, icône 🕐.

**Caveat** — LTC OUT / LTC IN = v2 (LTC OUT requiert un voice audio dédié ; LTC IN requiert l'encodeur LTC branché sur l'audio input — l'infrastructure existe, mais pas le câblage end-to-end). Les deux sont documentés dans `TIMECODE.md`.

**Tests** — +26 (13 types, 4 receiver, 3 LTC, 3 generator, 3 TimecodeCue) ; **130 total**, clippy clean, `tsc --noEmit` clean.

### 0.9.5 (2026-06-23) — Input Patches + Mic Cue (live audio input)

Inkue can now route a **live audio input** through the engine — QLab's Mic Cue.
Full design in `INPUT.md`.

- **Live input capture** — `engine/audio_input.rs`: `InputPatch` (named device + channels, workspace-stored, mirror of `OutputPatch`), input-device enumeration, and a **persistent** cpal input stream per device (F32/I16/I32) → lock-free ring. The stream stays open so a Mic Cue GO is instant (no cold-start).
- **Adaptive resampler** — `engine/audio_engine.rs`: `InputFeed` (ring + circular staging drained each output block) and `mix_live` — resamples the input device clock to the output clock with drift compensation (read cursor held ~25 ms behind the write head, ratio nudged ±2 %, resync on gross lag). Separate in/out devices supported; same device = unity no-op. `ensure_input_feed` (one feed per device, shared), `play_mic_voice`.
- **Live Voice** — `engine/voice.rs`: `LiveSource` + `Voice::new_live` — a live voice reads the ring instead of a sample buffer and inherits gain/pan/fade/VU/Output-Patch routing for free.
- **MicCue** — `cue/mic_cue.rs`: input patch + channels + output patch + volume/pan/fade; `go()` ensures the feed and submits the live voice; `duration()` = None (runs until stopped); soft-fade stop. Registered in `CueRegistry`; `CueType::Mic`; `CueContext.input_patches` + `resolve_input_patch`; `input_patches` serialized in the workspace; `MachineAudioConfig.input_device_id`.
- **Commands** — `list_input_devices`, `list_input_patches`, `add/update/remove_input_patch`.
- **Frontend** — `lib/{types,commands}.ts` (`InputPatch`, `MicCueData`), inspector **Mic tab** (`MicTab.tsx`), **+ Mic** toolbar button (+ drag), 🎤 row/inspector icon, **Input Patches panel** + default-input selector in Preferences → Audio (`InputPatchesPanel.tsx`).
- **Caveat** — routing + level + fade + pan only; no reverb/EQ (no audio FX rack yet). Unblocks LTC timecode input (`TIMECODE.md`).
- **Tests** — +4 (resampler drain/interp, `mix_live` unity routing, MicCue serde); **103 total**, clippy clean, `tsc --noEmit` clean.

### 0.9.4 (2026-06-23) — macOS GL output port + DMX lighting (Light Cue M1–M4)

#### macOS unified GL output port

macOS now joins the unified mpv OpenGL Render API path (`output_gl`, shared with
Windows/Linux) instead of the previous cocoa-cb mpv-managed window (`vo=gpu`). This
makes the dip-to-black fade work on macOS (it was a silent no-op before) and renders
mpv into a framebuffer Inkue controls — the prerequisite for future video transforms /
projection mapping on all three OS.

- **New `engine/output_engine/macos_window.rs`** — borderless `NSWindow` created on the
  AppKit main thread via `objc2` raw `msg_send!`; its `contentView` is handed to `glutin`
  as the CGL drawable, after which the shared render thread + GL fade quad run identically
  to Windows/Linux. winit cannot be used on macOS (its `EventLoop` must own the AppKit main
  run loop, which Tauri already does), so the window backend is the one piece that differs.
  Output window starts hidden at 960×540 centered on the main screen; double-click toggles
  fullscreen (level 25, above menu bar); window stays at normal level (0) between shows.
- **`render.rs`** — window creation branches by `target_os` (winit on Windows/Linux, AppKit
  on macOS); fade shaders lowered to `#version 150 core`; GL 3.2 core requested on macOS
  (no 3.3 core profile there; 150 is accepted by all three).
- **`mod.rs`** — dropped the cocoa-cb hacks (`vo=gpu`, `force-window`/`window-minimized`,
  `set_mpv_window_visible`, the `dispatch_sync` deadlock workarounds, mpv `fullscreen`/
  `screen` properties); macOS uses `vo=libmpv` like every other OS.
- **`build.rs`** — `output_winit` cfg renamed to `output_gl` (Windows-default + Linux +
  macOS); AppKit framework linked on macOS. **`Cargo.toml`** — `objc2` 0.5 +
  `objc2-foundation` 0.2 + `block2` 0.5 on macOS, pinned to winit's own objc2 stack (no
  duplicate). **CI** — the macOS job now runs `clippy` + `test` instead of bare `check`.

#### DMX lighting: fixture patch + Light Cue (M1–M4)

Full design + status in `LIGHT.md`. Inkue is now a direct DMX-over-IP controller,
not just a console trigger.

- **DMX engine (M1/M2)** — `engine/dmx_sink.rs` (byte-exact sACN E1.31 + Art-Net encoders, UDP sink) and `engine/dmx_engine.rs` (`DmxState`: per-universe buffers, timed fades with **LTP + tracking + 8/16-bit**, blackout; `DmxEngine` handle + `inkue-dmx` thread at ~40 Hz, send-on-change + 800 ms keepalive). Live monitor via the `dmx-monitor` event. `AppState.dmx_engine`.
- **Fixture patch (M3)** — `engine/fixture.rs`: `ParamKind` / `FixtureParam` / `FixtureType` / `PatchedFixture` (type **embedded** in each fixture → portable, self-contained workspace), `builtin_fixture_types()` (Dimmer, RGB, RGBW, RGBA, PAR Dimmer+RGB, 16-bit moving head), `resolve_channel()` (1-based address → 0-based engine channel), `find_conflicts()` (address-clash detection). Stored in the workspace alongside `universe_outputs` (`show/workspace.rs`); both pushed to the engine on load/new. Commands: `add/update/remove/list_fixtures`, `list_builtin_fixture_types`, `get_fixture_conflicts`, `dmx_test_fixture` (identify), `dmx_get/set_outputs`.
- **Light Cue (M4)** — `cue/light_cue.rs`: stores only the params it changes (`targets: [ParamTarget]`) + a `FadeSpec`; `go()` resolves each target's `(universe, channel, width)` from the patch and submits a fade to the engine; `duration()` = fade time (progress bar + Auto-Continue/Follow); stop is tracking (lights hold). A target's `fixture_id` is a `String` (an empty placeholder while configuring must not poison the whole list on the `update_cue` round-trip; resolved/parsed at GO). `CueContext` gained `dmx_engine` + `fixtures` (+ `resolve_fixture`), threaded through `transport_cmds` and `event_loop`. Registered in the `CueRegistry`.
- **Frontend** — `components/Lighting/{LightingPanel,FixturePatch}.tsx` (outputs now workspace-backed; Fixtures section with add/edit/identify/clash warnings), `components/Inspector/LightTab.tsx` (targets + fade), `+ Light` toolbar button (`App.tsx`), 💡 icon (`CueRow.tsx`, `InspectorPanel.tsx`). Types/commands in `lib/{types,commands}.ts`.
- **Live Dashboard + "Capture live state" (QLab-style look building)** — `components/Lighting/FixtureDashboard.tsx`: one row per fixture (intensity slider + RGB colour picker + per-param sliders) that drives the engine live (`dmx_set_fixture_param`), with `↻ Live` / `Clear`. The Light Cue inspector gains **⏺ Capture live state**, which records the current live state of every fixture into the cue's targets via `capture_live_targets` (pure read — applied through the normal `update_cue` path, single write/undo). So you sculpt the look by eye and freeze it, instead of typing values. `dmx_clear_fixtures` too.
- **Light Cue inspector grouped by fixture + fixture groups** — the Light tab now shows one card per fixture (colour picker + intensity + extra-param sliders) instead of one row per channel, with unique default fixture labels. **Fixture groups** (`FixtureGroup` in the workspace, `GroupManager.tsx` in the DMX panel) let one cue control drive several fixtures: a target now addresses **either** a fixture parameter **or** a group parameter-*kind* (`ParamTarget` is a tagged enum `Fixture | Group`, with backward-compat for the old flat form), resolved to all members at GO. So "wash to blue" is 3 targets, not 3×N. `CueContext` gained `fixture_groups` + `resolve_group`; group CRUD commands; shared colour helpers in `lib/fixtureColor.ts`.
- **Tests** — +10 (5 fixtures + 5 Light Cue, incl. group target + legacy-format upgrade) on top of the 4 packet + 7 engine-state tests; **99 total**, clippy clean, `tsc --noEmit` clean.

### 0.9.3 (2026-06-21) — Group Cue fixes + cross-platform polish (Linux/macOS) + UI

**Group Cue:**

- **Edit / delete cues inside a group** — the inspector and delete were top-level only, so a cue nested in a group showed an empty inspector and couldn't be removed. `get_cue`, `remove_cue`/`remove_cues`, `duplicate_cue`/`duplicate_cues`, waveform/normalize/preview now resolve recursively. New `cue_list` helpers `get_recursive`, `remove_anywhere`, `remove_many_anywhere`, `insert_after_anywhere`. `show/cue_list.rs`, `commands/cue_cmds.rs`.
- **Sequential audio overlaps** — a GO that advances a Sequential group no longer stops the current child, so sounds overlap like top-level cues; the group now ticks **all** running children so overlapping ones finish on their own. `cue/group_cue.rs`.
- **Playhead leaves the group on the last child** — firing the last child of a Sequential group now releases the outer Playhead to the cue after the group (previously the next GO stopped the group and then moved on). New trait `released_playhead()`; the transport releases on GO, the event loop on Auto-Continue/Follow reaching the last child. `cue/group_cue.rs`, `show/transport.rs`, `show/event_loop.rs`.
- **Park the Playhead on a specific child** — clicking a child of a Sequential group parks the outer Playhead on the group and points its inner sequence at that child, so GO fires it (Standby starts there, Running fires it next). New trait `set_active_child()`; `set_playhead` routes nested IDs to the top-level ancestor; `active_child_id` is now state-independent. `cue/{traits,group_cue}.rs`, `show/cue_list.rs`, `commands/cue_cmds.rs`, `CueListView.tsx`.

**Cross-platform & UI:**

- **Bundled timer font (DSEG7 Classic)** — `bundled_fonts::ensure_installed()` copies DSEG7 Classic (SIL OFL 1.1) into the per-user font dir at startup (`~/.local/share/fonts` + `fc-cache` on Linux, `~/Library/Fonts` on macOS, per-user Fonts dir + registry on Windows); it then resolves by family name for both the mpv OSD and the floating WebView. New default `timer_font`. `list_system_fonts()` also works on Linux/macOS now via `fc-list` (fontconfig — the backend mpv/libass resolve `osd-font` through). New `bundled_fonts.rs`, `vendor/fonts/`.
- **mpv `loadfile <index>` on all OS** — the Linux branch omitted the `<index>` arg, so mpv parsed the options string as the index and rejected it → video/image silently failed (Linux libmpv 0.41). Now passed on every OS. `output_engine/`.
- **Machine-config path per-OS** — `machine_config::config_path()` read Windows-only `%APPDATA%` and fell back to the CWD elsewhere, writing `audio.json` into `src-tauri/` under `tauri dev` (retriggered rebuilds). Now `~/.config` (Linux), `~/Library/Application Support` (macOS), `%APPDATA%` (Windows). `machine_config.rs`.
- **Wayland: output window now shows** — `FadeAnimState::idle()` started at alpha 0, so the GL loop never committed a buffer while idle → Wayland never mapped the surface (F9/View toggled nothing until a cue forced the first frame). Idle now starts at alpha 255 (opaque black). Also `skipTaskbar` on the hidden `preferences` window. `output_engine/`, `tauri.conf.json`.
- **Themed custom dropdown** — `components/common/Select.tsx` replaces the native `<select>` (WebKitGTK rendered it as an unreadable white GTK popup under the dark theme on Linux) at all 15 call sites.
- **Personalization preferences + cue colours** — new Personalization category (Colour Theme moved there) with a Cue Appearance section: `cue_color_style: stripe | full_row`. New **Cyan** (`#06b6d4`); default colours de-collided (Fade Blue→Pink, MIDI Green→White, OSC Blue→Cyan); toolbar swatches match defaults and `+ Cue` buttons reordered by frequency. Fixed `update_display_preferences` silently dropping `cue_color_style`; column-header drag now `preventDefault`s. `preferences.rs`, `commands/preferences_cmds.rs`, `cue/types.rs`, `PreferencesModal.tsx`, `CueRow.tsx`, `ColorPicker.tsx`, `App.tsx`.
- **No-file video/image cue completes instantly** — a Video/Image cue with no file assigned now goes Running → Completed (like MemoCue) instead of sticking "running", so Auto-Continue/Auto-Follow keeps advancing. `cue/video_cue.rs`, `cue/image_cue.rs`.
- **New app icon** — replaces the placeholder Tauri default (`.ico` / `.icns` / PNG set). `src-tauri/icons/`.

### 0.9.2 (2026-06-20)

- **Transport-bar Pause/Resume button** — light-blue PAUSE toggle next to GO/STOP; same semantics as OSC `/inkue/pause_toggle` (pause all running, else resume all paused; disabled when idle). `TransportBar.tsx`.
- **Floating timer drag + counter fixed** — the `float-timer` window had no Tauri v2 capability, so `startDragging` and `listen("float-timer-text")` were silently denied. Added `capabilities/float-timer.json` (`core:default` + `core:window:allow-start-dragging`); needs a rebuild.
- **Floating timer Linux crash fixed** — `set_floating_timer_visible` called `WebviewWindow::show()/hide()` directly from a Tauri command thread; on Linux that touches GTK off the main thread → crash (it also fired in OSD mode because the prefs-apply path always hides the floating window). Now routed through `app_handle.run_on_main_thread()`, so show/hide is main-thread-safe on all 3 OS. `output_engine/mod.rs`.
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
- OSC: `/inkue/pause_toggle`, `/inkue/select/next|previous`; 50 ms dedup cache; OSC Monitor; per-message Test-send; double-GO protection (`double_go_protection_ms`, default 500 ms).

### 0.6.0 (2026-06-09) — OSC Send Cue + receive server

- OSC Send Cue (multiple messages per cue, workspace-level patches, inspector Messages tab) and a UDP receive server (IP allowlist, `/inkue/*` address scheme, activity dot). Design/implementation detail archived in `docs/archive/OSCPLAN.md`.

### 0.5.1 — Group Cue polish

- Drag cue into group (cue-drag and OS file-drop); child color-strip indent by depth; Sequential Group GO absorption to advance the inner sequence. New `absorbs_go()` trait method.

### 0.4.2 (2026-05-30) — Video freeze fixed

- Root fix: mpv plays video muted (`ao=null` / `audio=no`); the video's audio track is decoded by symphonia and played as a normal AudioEngine voice (Output Patch, VU, fades). Lockstep start: the audio voice is submitted paused and released with the video on the first `MPV_EVENT_PLAYBACK_RESTART`. The whole `ao=pcm` named-pipe path (the A/V-desync and replay-deadlock source) was deleted; a 2.5 s watchdog guards against a missed restart. New shared decoder `cue/media_decode.rs`.

### 0.4.1 (2026-05-28) — Persistent PCM pipe *(superseded by 0.4.2)*

- `pcm_pipe_manager` thread for "no audio on 2nd+ video"; entirely removed in 0.4.2 for the muted-mpv design above.

### 0.4.0 (2026-05-28) — Unified OutputEngine (Win32 + libmpv)

- One persistent `WS_POPUP` window for all visual cues replaced the old two-window approach (Tauri WebviewWindow for images + Win32 for video) that caused windows to disappear/reposition between cues. libmpv renders both video and images; per-cue fade overlay; Hard Stop always cuts; first-GO freeze removed (mpv created at engine init); F9 toggles visibility. Old `.inkue` fields (`ImageStopMode`, per-cue `screen_index`) load silently via serde.

### 0.3.2 (2026-04-28) — Unified output surface *(Tauri WebviewWindow era, superseded by 0.4.0)*

- Single fixed output window + global `DisplayPreferences::output_screen` (replaced the per-cue screen selector); the WebviewWindow approach was dropped in 0.4.0.

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
| 7. Output Patches + DeviceManager | ⚠️ DeviceManager only — Output Patch routing was removed; NOT a working feature (see correction note under the cue-type table) |
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
| 23b. MIDI File Cue | ✅ Plays a `.mid` to one port; tempo-map-aware parsing (`midly`), playback-rate multiplier, pause/resume/seek, notes released on stop; `engine/midi_file.rs` + `cue/midi_file_cue.rs` |
| 24. Unified GL output | ✅ mpv Render API on all 3 OS — winit window (Windows/Linux) + AppKit `NSWindow` via objc2 (macOS); legacy Win32+D3D11 behind a feature flag |
| 25. DMX lighting (Light Cue) | ✅ sACN + Art-Net engine, fixture patch, Light Cue (M1–M4); M5 (NIC machine-config) + effects = next, see `LIGHT.md` |
| 27. Timecode (MTC/LTC) | ✅ `engine/timecode_types.rs` (SMPTE math, DF 29.97), `timecode_receiver.rs` (MTC QF + SysEx + flywheel), `timecode_generator.rs` (MTC OUT thread), `ltc.rs` (biphase encoder/decoder); `TimecodeCue` (MTC gen, start/end frame, multi-stream); per-cue `TcTrigger` + CueList `tc_config`; dispatcher in event loop; `timecode_cmds.rs`; frontend: TriggersTab, TimecodeTab, TcStatusIndicator, TcPreferences, + TC toolbar, 🕐 icon. LTC OUT/IN = v2. |
| 26. Input Patches + Mic Cue | ✅ Live audio input: persistent cpal capture, adaptive drift resampler, multichannel Input Patch → live Voice → Output Patch; see `INPUT.md`. Unblocks LTC timecode |

---

## Next priorities

See `WHATSNEXT.md` for the full roadmap; cross-platform detail is in `PORTAGE.md`.

1. **macOS runtime verification** — the unified GL output port (NSWindow via objc2) compiles clean on CI for all 3 OS; confirm window show/hide, video/image playback, and dip-to-black fades on real Apple hardware. First thing to watch: glutin/CGL surface creation on the render thread (fallback: build the GL stack on the main thread). See the *Unreleased* change-history entry.
2. **Active A/V resync** (optional) — nudge the video's audio-voice rate to track mpv `time-pos` for drift-free long videos / tight loops (see Known issues).
3. ~~Output Patch routing (rebuild)~~ — **done 2026-07-04** (multi-device aux streams +
   UI; see the Unreleased change-history entry). Remaining niche: Mic/live voices still
   play on the main device only.
