# WinCue — Project state as of 2026-06-25

## Current version: 0.9.22

## cargo build result

**Compiles without errors, zero warnings** on all three OS in CI (Windows, Linux,
macOS) — default (GL) **and** `--features legacy-win32-output` on Windows. The
macOS job runs `cargo clippy` + `cargo test`; Windows/Linux run `cargo check`.

## cargo test result

**143 tests pass, 0 failures.** (run `cargo test` from `src-tauri/` after closing dev server to confirm. DMX engine + sink, fixtures, groups, Light Cue; live input resampler + Mic Cue; TC types/DF/display/RT, MTC receiver QF+SysEx+flywheel, LTC encoder/decoder, TC generator QF round-trip.)

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
| Light | ✅ **Functional** | DMX-over-IP (sACN + Art-Net); fixture patch in the workspace (6 built-in types, embedded layout, address-clash warnings, identify); Light Cue fades fixture params to a target look (tracking + LTP via DmxEngine); inspector Light tab (targets + fade time/curve); DMX panel Fixtures section |
| Mic      | ✅ **Functional** | (see 0.9.5) |
| Timecode | ✅ **Functional** | SMPTE timecode generation (MTC out via `TimecodeCue`) + receive (MTC in via `TimecodeReceiver`); per-cue TC triggers + CueList sync toggle; LTC encoder/decoder (`ltc.rs`); TC status indicator in TransportBar; Triggers inspector tab on every cue; TC Preferences (Network tab). LTC out = planned v2; drop-frame 29.97 fully tested. | Routes a live audio input (QLab Mic Cue) through the engine: persistent cpal input stream (instant GO), separate in/out devices + adaptive drift resampler, multichannel Input Patch routed to an Output Patch via a live `Voice` (gain/pan/fade/VU); runs until stopped; inspector Mic tab; Input Patches panel in Preferences → Audio |
| Text     | ✅ **Functional** | Renders styled text on the mpv output surface via the `osd-overlay` command (`format=ass-events`) + ASS inline tags; independent of OSD timer. Font, size, hex colour, 9-point position grid, optional auto-complete duration. Stop-on-next-go. |

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
| OutputEngine | `engine/output_engine/` | ✅ Complete — unified GL Render API on all 3 OS; `vo=libmpv`; native GL window — winit (Windows/Linux) or AppKit `NSWindow` via objc2 (macOS, `macos_window.rs`); mpv_render_context; GL fade quad; OSD + floating timer; `get_overlay_alpha()`, `set_overlay_alpha_direct()`; legacy Win32+D3D11 behind `legacy-win32-output` feature flag |
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
- Added a kind-agnostic **heartbeat**: a monotonic `output_callbacks` counter incremented in every output callback (shared across restarts). The `wincue-device-watchdog` (`lib.rs`) treats a count that stops advancing for one ~2 s tick as a dead stream — so device loss is detected even if cpal surfaces no error or a different error kind.
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
- **`lib.rs`** — `wincue-device-watchdog` thread (2 s): on output-device loss falls back to the default device to keep audio alive and raises an error banner; when the desired device returns it switches the banner to a "Rebasculer" action (no automatic re-switch — re-opening the stream glitches audio, never forced onto a critical cue). Emits a throttled `health-changed` event.
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
- **`logger.rs`** (new) — custom `log` backend fanning out to stderr + a size-rotated file (`%APPDATA%/WinCue/logs/wincue.log`, one backup) + a 2000-line in-memory ring buffer. Replaces `env_logger` (removed; `log` now carries the `std` feature). `RUST_LOG=debug/trace` still bumps the level.
- **`commands/log_cmds.rs`** (new) — `get_recent_logs`, `clear_logs`, `open_logs_folder` (per-OS reveal). `lib.rs` spawns a `wincue-log-emitter` thread emitting a throttled `logs-updated` event (event-driven live tail, no frontend polling).
- **Frontend** — `LogViewerModal` (level filter, follow/auto-scroll, copy, open folder, clear). File menu → "Logs…".

**Tests** — 143 pass (141 + 2 validation). `cargo clippy --lib` + `tsc --noEmit` clean. Version 0.9.13.

### 0.9.12 (2026-06-25) — Crash recovery (autosave)

Continuous crash-recovery snapshot so an abnormal exit (crash / power loss) loses at most a few seconds of work — the first reliability item on the road to a professional 1.0.

- **`recovery.rs`** (new) — snapshot lives at `%APPDATA%\WinCue\recovery.wincue` (per-OS config dir, reusing `machine_config::config_base_dir`, so dev writes never trip the source-tree file watcher). Atomic write (`.tmp` + rename) so a crash mid-write never corrupts it. `info()` parses the header for the restore prompt; `exists()/read()/delete()`.
- **`show/workspace.rs`** — `revision: u64` field bumped by `mark_modified` (the single mutation chokepoint) so the autosave thread only re-serialises when the show actually changed. `to_recovery_json()` keeps media paths **absolute** (the snapshot is not beside the media) and embeds `recovery_original_path`. `load()` refactored to share `from_json_str(content, base_dir, registry)` — `base_dir: None` parses the absolute-path recovery snapshot.
- **`lib.rs`** — `wincue-autosave` thread (3 s tick): writes the snapshot while `is_modified`, deletes it once the show is saved/pristine. The `WindowEvent::Destroyed` handler deletes the snapshot on any deliberate close, so presence at startup ⇒ previous session crashed.
- **`commands/recovery_cmds.rs`** (new) — `check_recovery` (→ `RecoveryInfo`), `restore_recovery` (loads the snapshot, re-targets the original `.wincue`, marks dirty), `discard_recovery`. `workspace_cmds::install_workspace` extracted from `load_workspace` and shared with restore. `save_workspace` now drops the snapshot on explicit save.
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

- **`show/cue_list.rs`** — `CueListMode` enum (`sequential` | `cart`, default sequential); `mode` field on `CueList`; serialized in `.wincue` (backward-compat default). `to_json` + `from_json` updated.
- **`show/transport.rs`** — `Transport::go_by_id(cue_list, cue_id)`: parks the Playhead on the given cue and fires via the normal GO path, so Auto-Continue / Auto-Follow still work.
- **`commands/cue_list_cmds.rs`** — `CueListInfo.mode` added; new `set_cue_list_mode(id, mode)` command.
- **`commands/transport_cmds.rs`** — new `go_cue(cue_id)` command (same loading guard as `go`, calls `go_by_id`).
- **`lib.rs`** — both new commands registered in invoke_handler.
- **`lib/types.ts`** — `CueListMode` type; `CueListSummary.mode` field.
- **`lib/commands.ts`** — `goCue()`, `setCueListMode()`.
- **`components/CueList/CartView.tsx`** — new component: responsive CSS grid (`auto-fill, minmax(160px, 1fr)`), one tile per top-level cue. Each tile: color stripe (left edge), cue number (top-left), type icon (top-right), name (bold, 2-line clamp), running LED + remaining time + STOP button (footer). Progress bar (bottom edge, green). Running: green border + tint + pulsing LED. Paused: orange border + tint. Completed: dimmed.
  - **Drag to reorder** — mousedown+threshold activates drag; dragged tile is removed from `displayItems` and replaced by a `DropSlot` (dashed accent border) that moves with the cursor as it crosses tile midpoints — grid CSS reflowing naturally around it. On drop: `moveCue(id, insertIndex)` where `insertIndex` is already the after-removal index (no adjustment needed). Floating **DragGhost** follows cursor; rotation driven by exponentially-smoothed horizontal velocity (`smoothedVel = 0.78*prev + 0.22*dx`) giving inertia up to ±13°. System cursor hidden (`cursor:none`) during drag; ghost fade-in via `wc-ghost-appear` keyframe.
  - **Drag from toolbar** — listens to `wincue:cue-drag-start` CustomEvent (same as sequential mode); inserts `DropSlot` at cursor position; on drop calls `addCue(type, insertIndex)`.
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
- **`components/CueList/columns.ts`** — new `"led"` column (20px, fixed, non-resizable), inserted right after `"playhead"`; `loadColumnConfig` migration ensures ordering is correct for existing saved configs; LS key bumped to `wincue_column_config_v2` to force a clean default on the first load.
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
- **`engine/timecode_receiver.rs`** — `TimecodeReceiver` (thread `wincue-tc-mtc`, `midir::MidiInput`), `MtcAssembler` (quarter-frame state machine + full-frame SysEx), `TcFlywheel` (interpolation + freewheel). 4 tests.
- **`engine/ltc.rs`** — `LtcEncoder` / `LtcDecoder` biphase-mark : encode `TcPosition → [f32]`, decode `[f32] → TcPosition`. Sync word vérification. 3 tests.
- **`engine/timecode_generator.rs`** — `MtcGenerator` (thread `wincue-tc-gen` : quarter-frames à 4×fps, full-frame jam-sync au démarrage). 3 tests.
- **`cue/timecode_cue.rs`** — `TimecodeCue` : génère MTC sur GO (`MtcGenerator`), start/end frame (durée calculée), plusieurs flux simultanés, `CueType::Timecode`, registry. 3 tests.
- **`show/cue_list.rs`** — `CueList.tc_config: CueListTcConfig` + `tc_triggers: HashMap<CueId, TcTrigger>` + garde monotone `tc_last_triggered_frame`. Sérialisé dans `.wincue`.
- **Dispatcher** — `event_loop.rs` reçoit `TcEvent` via channel, franchissement monotone + ré-armement sur saut arrière, émet `timecode` event Tauri pour l'UI.
- **`engine/timecode_receiver.rs`** — `TcReceiverConfig`, `TimecodeReceiver.reconfigure()` (comme `OscServer`). `machine_config.rs` : `TcMachineConfig` + `load/save_tc_config`.
- **Commands** — `timecode_cmds.rs` : `get/set_tc_config`, `get_tc_position`, `list_tc_midi_input_ports`, `get/set_cue_tc_trigger`, `get/set_cuelist_tc_config`.
- **Frontend** — `TriggersTab.tsx` (SMPTE ou RealTime, sur chaque cue), `TimecodeTab.tsx` (TimecodeCue inspector), `TcStatusIndicator.tsx` (position live dans TransportBar, flash sur lock), `TcPreferences.tsx` (Network prefs, source + port MIDI), bouton `+ TC`, icône 🕐.

**Caveat** — LTC OUT / LTC IN = v2 (LTC OUT requiert un voice audio dédié ; LTC IN requiert l'encodeur LTC branché sur l'audio input — l'infrastructure existe, mais pas le câblage end-to-end). Les deux sont documentés dans `TIMECODE.md`.

**Tests** — +26 (13 types, 4 receiver, 3 LTC, 3 generator, 3 TimecodeCue) ; **130 total**, clippy clean, `tsc --noEmit` clean.

### 0.9.5 (2026-06-23) — Input Patches + Mic Cue (live audio input)

WinCue can now route a **live audio input** through the engine — QLab's Mic Cue.
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
mpv into a framebuffer WinCue controls — the prerequisite for future video transforms /
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

Full design + status in `LIGHT.md`. WinCue is now a direct DMX-over-IP controller,
not just a console trigger.

- **DMX engine (M1/M2)** — `engine/dmx_sink.rs` (byte-exact sACN E1.31 + Art-Net encoders, UDP sink) and `engine/dmx_engine.rs` (`DmxState`: per-universe buffers, timed fades with **LTP + tracking + 8/16-bit**, blackout; `DmxEngine` handle + `wincue-dmx` thread at ~40 Hz, send-on-change + 800 ms keepalive). Live monitor via the `dmx-monitor` event. `AppState.dmx_engine`.
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

- **Transport-bar Pause/Resume button** — light-blue PAUSE toggle next to GO/STOP; same semantics as OSC `/wincue/pause_toggle` (pause all running, else resume all paused; disabled when idle). `TransportBar.tsx`.
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
- OSC: `/wincue/pause_toggle`, `/wincue/select/next|previous`; 50 ms dedup cache; OSC Monitor; per-message Test-send; double-GO protection (`double_go_protection_ms`, default 500 ms).

### 0.6.0 (2026-06-09) — OSC Send Cue + receive server

- OSC Send Cue (multiple messages per cue, workspace-level patches, inspector Messages tab) and a UDP receive server (IP allowlist, `/wincue/*` address scheme, activity dot). Design/implementation detail archived in `docs/archive/OSCPLAN.md`.

### 0.5.1 — Group Cue polish

- Drag cue into group (cue-drag and OS file-drop); child color-strip indent by depth; Sequential Group GO absorption to advance the inner sequence. New `absorbs_go()` trait method.

### 0.4.2 (2026-05-30) — Video freeze fixed

- Root fix: mpv plays video muted (`ao=null` / `audio=no`); the video's audio track is decoded by symphonia and played as a normal AudioEngine voice (Output Patch, VU, fades). Lockstep start: the audio voice is submitted paused and released with the video on the first `MPV_EVENT_PLAYBACK_RESTART`. The whole `ao=pcm` named-pipe path (the A/V-desync and replay-deadlock source) was deleted; a 2.5 s watchdog guards against a missed restart. New shared decoder `cue/media_decode.rs`.

### 0.4.1 (2026-05-28) — Persistent PCM pipe *(superseded by 0.4.2)*

- `pcm_pipe_manager` thread for "no audio on 2nd+ video"; entirely removed in 0.4.2 for the muted-mpv design above.

### 0.4.0 (2026-05-28) — Unified OutputEngine (Win32 + libmpv)

- One persistent `WS_POPUP` window for all visual cues replaced the old two-window approach (Tauri WebviewWindow for images + Win32 for video) that caused windows to disappear/reposition between cues. libmpv renders both video and images; per-cue fade overlay; Hard Stop always cuts; first-GO freeze removed (mpv created at engine init); F9 toggles visibility. Old `.wincue` fields (`ImageStopMode`, per-cue `screen_index`) load silently via serde.

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
| 24. Unified GL output | ✅ mpv Render API on all 3 OS — winit window (Windows/Linux) + AppKit `NSWindow` via objc2 (macOS); legacy Win32+D3D11 behind a feature flag |
| 25. DMX lighting (Light Cue) | ✅ sACN + Art-Net engine, fixture patch, Light Cue (M1–M4); M5 (NIC machine-config) + effects = next, see `LIGHT.md` |
| 27. Timecode (MTC/LTC) | ✅ `engine/timecode_types.rs` (SMPTE math, DF 29.97), `timecode_receiver.rs` (MTC QF + SysEx + flywheel), `timecode_generator.rs` (MTC OUT thread), `ltc.rs` (biphase encoder/decoder); `TimecodeCue` (MTC gen, start/end frame, multi-stream); per-cue `TcTrigger` + CueList `tc_config`; dispatcher in event loop; `timecode_cmds.rs`; frontend: TriggersTab, TimecodeTab, TcStatusIndicator, TcPreferences, + TC toolbar, 🕐 icon. LTC OUT/IN = v2. |
| 26. Input Patches + Mic Cue | ✅ Live audio input: persistent cpal capture, adaptive drift resampler, multichannel Input Patch → live Voice → Output Patch; see `INPUT.md`. Unblocks LTC timecode |

---

## Next priorities

See `WHATSNEXT.md` for the full roadmap; cross-platform detail is in `PORTAGE.md`.

1. **macOS runtime verification** — the unified GL output port (NSWindow via objc2) compiles clean on CI for all 3 OS; confirm window show/hide, video/image playback, and dip-to-black fades on real Apple hardware. First thing to watch: glutin/CGL surface creation on the render thread (fallback: build the GL stack on the main thread). See the *Unreleased* change-history entry.
2. **Active A/V resync** (optional) — nudge the video's audio-voice rate to track mpv `time-pos` for drift-free long videos / tight loops (see Known issues).
3. **ASIO → WASAPI Output Patch validation** — routing is wired; needs a hardware test.
