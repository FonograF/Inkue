# WinCue — Architecture reference

Companion to `CLAUDE.md`. Read this when modifying the output engine, audio pipeline, event loop, or preferences system.

## Output engine (`engine/output_engine/`)

Single persistent Win32 popup window (`WS_POPUP`) created at startup, always visible (black when idle). libmpv renders both video and images into it via D3D11/gpu.

**Key statics** (all `OnceLock`):
- `OUTPUT_PARENT_HWND` — the Win32 window handle
- `FADE_OVERLAY_HWND` — `WS_EX_LAYERED` child for dip-to-black fades
- `OUTPUT_MPV_CTX` / `OUTPUT_MPV_LIB` — mpv context shared across threads
- `OUTPUT_CURRENT_AUDIO_VOICE` — UUID of the video's paired audio voice
- `OUTPUT_PENDING_VIDEO_START` — set when a video loads paused; consumed by first `MPV_EVENT_PLAYBACK_RESTART`
- `TIMER_PREVIEW` — `Mutex<Option<String>>`: when `Some`, timer thread shows this instead of live cue time

**Video audio**: mpv runs with `ao=null` / `audio=no`. A video's audio track is decoded by symphonia and played as a normal `AudioEngine` Voice (gets Output Patch routing, VU, fades). Both video and audio start paused at GO; `MPV_EVENT_PLAYBACK_RESTART` releases both simultaneously from frame 0. A 2.5 s watchdog force-reveals if the event never fires.

**Fade overlay**: `WS_EX_LAYERED | WS_EX_TRANSPARENT` child window. Alpha 0 = transparent, 255 = opaque black. Animated via 16 ms `WM_TIMER` in the parent window proc. `WM_DO_FADE` (custom message) starts/updates the fade.

**Cross-stop rule**: any `show_content()` call stops the current voice first (applying its stored `fade_out_ms`).

**Screen positioning**: `position_window(screen_index)` moves the window fullscreen onto the chosen monitor at GO time. `DisplayPreferences::output_screen: Option<u32>` is the global setting.

**F9**: `toggle_output_window` / `get_output_window_visible` Tauri commands.

## Output window timer (OSD via mpv)

The timer is rendered via mpv's OSD (`osd-msg1` property), not a Win32 GDI child window. This guarantees it composites above the D3D11 surface regardless of DWM.

**Thread**: `wincue-timer-refresh` in `show/event_loop.rs`, runs every 16 ms. Does `try_lock` on workspace (skips frame if busy), reads `cue.action_elapsed()` directly (always `Instant::now() - start`, no stale data), formats, calls `output_engine.set_output_timer()`. Independent of the 30 fps main tick.

**Format**: `MM:SS` or `MM:SS.mmm` — minutes always zero-padded.

**Style** — applied via `OutputEngine::set_timer_style(font, size, position, margin)` which sets mpv properties: `osd-font`, `osd-font-size`, `osd-align-x/y`, `osd-margin-x/y`. Must be called after loading a workspace (preferences apply calls it automatically).

**`TimerPosition` → mpv alignment**:

| Value | osd-align-x | osd-align-y | margin |
|---|---|---|---|
| Center | center | center | 0 |
| TopLeft | left | top | timer_margin |
| TopRight | right | top | timer_margin |
| BottomLeft | left | bottom | timer_margin |
| BottomRight | right | bottom | timer_margin |

**`DisplayPreferences` timer fields** (`serde(default)` on all — safe to load old files):

| Field | Type | Default |
|---|---|---|
| `show_output_timer` | bool | false |
| `timer_count_down` | bool | false |
| `timer_show_ms` | bool | false |
| `timer_position` | `TimerPosition` | Center |
| `timer_margin` | u32 | 50 |
| `timer_font` | String | "Arial" |
| `timer_font_size` | u32 | 120 |

**Preview mode**: `output_engine.set_timer_preview(Some("00:00.000"))` — timer thread shows this placeholder instead of live cue time. Used by the preferences "Preview" checkbox. `set_timer_preview(None)` returns to live mode.

**Font enumeration**: `list_system_fonts` Tauri command → `EnumFontFamiliesExW` (GDI) → sorted list of installed family names. UI renders them in a `<datalist>` for searchable autocomplete.

**Note**: a legacy `WinCueTimerOverlay` Win32 GDI child window still exists in `win32_window.rs` but is unused (GDI is composited away by DWM when mpv owns the D3D11 surface). Remove in a future cleanup.

## Transport GO (`show/transport.rs`)

`Transport::go()` returns `GoResult { triggered: Vec<CueId>, stopped: Vec<CueId> }`.

**Execution order within a single GO:**

1. `advance_playhead()` — moves the outer playhead forward.
2. `stop_on_next_go()` guard — stops running visual cues whose `stop_on_next_go()` returns `true`, **but only if the incoming cue is also visual** (`CueType::Video | CueType::Image`). An audio GO never cuts a displayed image.
3. `cue.go()` — triggers the cue at the playhead.
4. `stop_specification()` — if the triggered cue declares a stop action (only `StopCue` does), the transport executes it **immediately and synchronously**, before evaluating the Auto-Follow chain. This prevents a Stop Cue + Auto-Follow from killing the chained cue.
5. Chain evaluation — `AutoContinue` (post_wait = 0) or instant `AutoFollow` recurses into `go()`.

`GoResult.stopped` carries the IDs of any cues killed by a Stop Cue action; callers emit `cue-state-changed` for them.

## Stop Cue and `stop_specification()`

`StopCue` has two configurable fields:
- `target_cue_number: Option<String>` — `None` = stop all running cues; `Some(n)` = stop only the cue whose number matches `n`.
- `hard_stop_mode: bool` — `false` = soft fade, `true` = immediate cut.

`StopCue::stop_specification()` returns `Some((hard_stop_mode, target_cue_number))`.

The `Cue` trait has a default `stop_specification() -> Option<(bool, Option<String>)>` returning `None`. Any future cue type that needs to stop other cues as part of its GO action can override it — no transport changes required.

## Event loop (`show/event_loop.rs`)

Two threads:

| Thread | Interval | Responsibility |
|---|---|---|
| `wincue-event-loop` | 33 ms (30 fps) | Drain audio/output status, tick cues, detect completions, Auto-Continue/Follow, emit Tauri events (`cue-time-update`, `cue-state-changed`, `master-level`) |
| `wincue-timer-refresh` | 16 ms (60 fps) | Update OSD timer text only — no Tauri events |

Both use `workspace.try_lock()` — skip the frame rather than block if a command handler holds the lock.

## Preferences system

`AppPreferences` is persisted inside the `.wincue` workspace file under `"preferences"`. Machine-specific audio config (`MachineAudioConfig`) lives separately in `%APPDATA%\WinCue\audio.json`.

`update_display_preferences` (Tauri command) persists fields to workspace **and** immediately applies timer style to mpv via `set_timer_style`. It also clears any active preview (`set_timer_preview(None)`).

`preview_output_timer` (Tauri command) applies style + sets preview text without persisting — used for live preview while the preferences modal is open. The caller (modal cancel/apply) is responsible for restoring committed style.
