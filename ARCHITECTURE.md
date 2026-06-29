# Inkue — Architecture reference

Companion to `CLAUDE.md`. Read this when modifying the output engine, audio pipeline, event loop, or preferences system.

## Output engine (`engine/output_engine/`)

Single persistent native window for all video and image cues, created at startup
and hidden until the first visual GO. **Unified GL path (`output_gl`, all 3 OS):
mpv OpenGL Render API** (`render.rs`): mpv runs with `vo=libmpv` and renders each
frame into the window's default framebuffer via `glutin` (OpenGL 3.3 Core, 3.2 on
macOS) + `mpv_render_context`. Only **native window creation** differs per OS —
**winit** on Windows/Linux, **AppKit `NSWindow` via objc2** on macOS
(`macos_window.rs`), because winit's `EventLoop` cannot coexist with Tauri's AppKit
main run loop. Everything after `make_current` (render loop, GL fade quad) is shared.
The legacy Win32 + D3D11 `--wid` path (`win32_window.rs`) compiles only behind the
`legacy-win32-output` feature flag (off by default) as a regression fallback.

**Threads** (GL path):

| Thread | Role |
|---|---|
| `inkue-output-window` | **(Windows/Linux only)** winit `EventLoop` — window events (drag, resize, double-click fullscreen). macOS has no such thread: the `NSWindow` is built on the main thread during `.setup()` and AppKit handles its events |
| `inkue-output-render` | glutin GL context + `mpv_render_context` + render loop; draws the fade quad |
| `inkue-output-mpv-events` | `mpv_wait_event` (`PLAYBACK_RESTART`, EOF, …) |

**Key statics:**
- `render::GL_WINDOW` — `Arc<winit::window::Window>` (**Windows/Linux only**), shared so `OutputEngine` show/hide/position/fullscreen call winit's cross-platform API from any thread. On macOS the `*mut NSWindow` lives in `macos_window::MAC_WINDOW` and control calls marshal onto the main thread via `run_on_main_thread`
- `render::RENDER_SIGNAL` — condvar woken by mpv's update callback (and `render::wake()` during Fade Cues) so the loop redraws on demand
- `render::GL_WIDTH` / `GL_HEIGHT` — physical window size, written on resize, read by `surface.resize()`
- `OUTPUT_MPV_CTX` / `OUTPUT_MPV_LIB` — mpv context shared across threads
- `OUTPUT_CURRENT_AUDIO_VOICE` — UUID of the video's paired audio voice
- `OUTPUT_PENDING_VIDEO_START` — set when a video loads paused; consumed by the first `MPV_EVENT_PLAYBACK_RESTART`
- `TIMER_PREVIEW` — `Mutex<Option<String>>`: when `Some`, the timer thread shows this instead of live cue time
- *(legacy path only)* `OUTPUT_PARENT_HWND`, `FADE_OVERLAY_HWND` — Win32 handles, `#[cfg(output_win32)]`

**Video audio**: mpv runs with `ao=null` / `audio=no`. A video's audio track is decoded by symphonia and played as a normal `AudioEngine` Voice (gets Output Patch routing, VU, fades). Both video and audio start paused at GO; `MPV_EVENT_PLAYBACK_RESTART` releases both simultaneously from frame 0. A 2.5 s watchdog force-reveals if the event never fires.

**Fade overlay**: a fullscreen black quad (`vec4(0,0,0,alpha)`) drawn in the same GL framebuffer after the mpv render, before `swap_buffers`. Alpha 0 = transparent, 255 = opaque black; animated by `fade::tick_fade()`. Drawing the fade in mpv's own framebuffer is immune to the DWM DirectFlip "frame-black" glitch that affected the old separate layered window. A hard-cut stop forces alpha 255 to paint black over the frozen last frame. The idle alpha also starts at 255, so the GL loop commits at least one buffer even with no content loaded — required for the Wayland compositor to map the window (otherwise F9/View toggles nothing until the first cue). *(Legacy path only: a `WS_EX_LAYERED` child window animated via `WM_TIMER`, with `d3d11-flip=no` to avoid that DirectFlip glitch.)*

**Cross-stop rule**: any `show_content()` call stops the current voice first (applying its stored `fade_out_ms`).

**Screen positioning**: `position_window(screen_index)` moves the window fullscreen onto the chosen monitor at GO time. `DisplayPreferences::output_screen: Option<u32>` is the global setting.

**Visibility**: `toggle_output_window` / `get_output_window_visible` Tauri commands (F9). The backend emits an `output-window-visible` event on every show/hide so the View-menu checkmark stays in sync.

## Output window timer (OSD via mpv)

The timer is rendered via mpv's OSD (`osd-msg1` property), not a separate child window. mpv composites the OSD into the same framebuffer it renders the video into, so the timer always sits above the picture regardless of the compositor (DWM on Windows, etc.).

**Thread**: `inkue-timer-refresh` in `show/event_loop.rs`, runs every 16 ms. Does `try_lock` on workspace (skips frame if busy), reads `cue.action_elapsed()` directly (always `Instant::now() - start`, no stale data), formats, calls `output_engine.set_output_timer()`. Independent of the 30 fps main tick.

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
| `timer_font` | String | "DSEG7 Classic" (bundled) |
| `timer_font_size` | u32 | 120 |

**Preview mode**: `output_engine.set_timer_preview(Some("00:00.000"))` — timer thread shows this placeholder instead of live cue time. Used by the preferences "Preview" checkbox. `set_timer_preview(None)` returns to live mode.

**Font enumeration**: `list_system_fonts` Tauri command → `EnumFontFamiliesExW` (GDI) on Windows, `fc-list` (fontconfig — the same backend mpv/libass resolve `osd-font` through) on Linux/macOS → sorted list of installed family names. UI renders them in a `<datalist>` for searchable autocomplete.

**Bundled default font**: `bundled_fonts::ensure_installed()` (called at startup) copies DSEG7 Classic (SIL OFL 1.1, `vendor/fonts/`) into the per-user font dir — `~/.local/share/fonts` + `fc-cache` (Linux), `~/Library/Fonts` (macOS), per-user Fonts dir + registry (Windows). Once installed it resolves by family name for both the mpv OSD and the floating-timer WebView with no separate embedding path. It is the default `timer_font`.

## Audio pipeline (`engine/audio_engine.rs`, `engine/voice.rs`)

### Stream opening

`open_stream_inner` appelle `device.default_output_config()` pour obtenir le mix format du device WASAPI (ex. 48 000 Hz). Le stream cpal est ouvert à cette fréquence. `sample_rate` est capturé dans chaque closure de callback.

### Voice

Un `Voice` stocke :
- `samples: Arc<Vec<f32>>` — PCM interleaved à la fréquence **source** du fichier
- `sample_rate: u32` — fréquence source (44 100, 48 000, 96 000, etc.)
- `inner.rate_bits` — **pure user rate multiplier** (1.0 = vitesse normale). Ce champ ne contient PAS de correction SR — c'est une propriété utilisateur exposée dans l'inspecteur.

### Correction sample rate dans `fill_buffer`

`fill_buffer` reçoit `output_sample_rate: u32` et calcule le step d'avance par frame output pour chaque voice :

```rust
let step = voice.inner.rate() as f64
    * (voice.sample_rate as f64 / output_sample_rate as f64);
// frame_pos_f += step  (une fois par output frame)
```

Cette formule garantit qu'un fichier de N secondes est consommé en exactement N secondes de temps réel, indépendamment des fréquences source et output.

**Exemples :**

| Source SR | Output SR | step (rate=1.0) | Frames source / seconde |
|---|---|---|---|
| 44 100 Hz | 48 000 Hz | 0.91875 | 44 100 ✓ |
| 48 000 Hz | 48 000 Hz | 1.0 | 48 000 ✓ |
| 96 000 Hz | 48 000 Hz | 2.0 | 96 000 ✓ |
| 48 000 Hz | 44 100 Hz | 1.0884 | 48 000 ✓ |

**Règle d'architecture :** le ratio SR appartient exclusivement à `fill_buffer`. `cue/audio_cue.rs` et `cue/video_cue.rs` ne doivent **jamais** appeler `audio_engine.sample_rate()` pour corriger leur voice — toute tentative reintroduirait la violation de couche et doublerait la correction.

### Durée des fades

Les fades sont calculés en **frames source** : `total_samples = fade_ms * voice.sample_rate / 1000`. Puisque le step d'avance inclut déjà le ratio SR, le fade dure `fade_ms / user_rate` secondes de temps réel — correct quelle que soit la fréquence source ou output.

### Limitation : downsampling sans anti-aliasing

Quand `source_sr > output_sr` (ex. 96k sur 48k, step = 2.0), le callback saute des frames source sans filtre passe-bas préalable. Le contenu au-dessus de la fréquence de Nyquist output peut aliaser dans le signal audible. En pratique imperceptible : les fichiers haute résolution sont déjà band-limités sous 20 kHz. Un filtre polyphase serait nécessaire pour une qualité audiophile stricte.

### Tests unitaires (`engine::audio_engine::tests`)

5 tests vérifient la mécanique SR sans device audio réel en appelant `fill_buffer` directement sur des voices synthétiques :
- `sr_ratio_44100_on_48000` — step = 0.91875
- `sr_ratio_48000_on_48000` — step = 1.0
- `sr_ratio_48000_on_44100` — step = 1.0884
- `sr_ratio_96000_on_48000` — step = 2.0
- `user_rate_2x_on_matching_sr` — rate utilisateur ×2

Tolérance ±1 frame : le ratio 44100/48000 n'est pas exactement représentable en f64, l'accumulation sur N frames introduit un écart sub-frame.

---

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
- `target_cue_ids: Vec<CueId>` — empty = stop **all** running cues; non-empty = stop only that subset. (`target_cue_numbers` mirrors the IDs for display; `from_json` migrates the old single `target_cue_id` / `target_cue_number` format and `resolve_stop_target` resolves numbers → UUIDs on load.)
- `hard_stop_mode: bool` — `false` = soft fade, `true` = immediate cut.

`StopCue::stop_specification()` returns `Some((hard_stop_mode, target_cue_ids))`.

The `Cue` trait has a default `stop_specification() -> Option<(bool, Vec<CueId>)>` returning `None`. Any future cue type that needs to stop other cues as part of its GO action can override it — no transport changes required.

## Event loop (`show/event_loop.rs`)

Two threads:

| Thread | Interval | Responsibility |
|---|---|---|
| `inkue-event-loop` | 33 ms (30 fps) | Drain audio/output status, tick cues, detect completions, Auto-Continue/Follow, emit Tauri events (`cue-time-update`, `cue-state-changed`, `master-level`) |
| `inkue-timer-refresh` | 16 ms (60 fps) | Update OSD timer text only — no Tauri events |

Both use `workspace.try_lock()` — skip the frame rather than block if a command handler holds the lock.

## Preferences system

`AppPreferences` is persisted inside the `.inkue` workspace file under `"preferences"`. Machine-specific audio config (`MachineAudioConfig`) lives separately in a per-OS config dir resolved by `machine_config::config_path()`: `%APPDATA%\Inkue\` (Windows), `~/.config/Inkue/` (Linux), `~/Library/Application Support/Inkue/` (macOS).

`update_display_preferences` (Tauri command) persists fields to workspace **and** immediately applies timer style to mpv via `set_timer_style`. It also clears any active preview (`set_timer_preview(None)`).

`preview_output_timer` (Tauri command) applies style + sets preview text without persisting — used for live preview while the preferences modal is open. The caller (modal cancel/apply) is responsible for restoring committed style.
