# WinCue — Project state as of 2026-06-16

## Current version: 0.8.1

## cargo build result

**Compiles without errors, zero warnings.**

## cargo test result

**62 tests pass, 0 failures.**

---

## Cue type status

| Cue type | Status | Details |
|---|---|---|
| Audio | ✅ **100% functional** | Pre/post-wait, fade-in/out, loop (finite + infinite), rate, Output Patch routing, pan, master volume, waveform, VU meter, scrub/seek; pause/resume with correct elapsed tracking; SR conversion in `fill_buffer` (44.1k/48k/96k all correct) |
| Stop  | ✅ **Functional** | UUID-based targeting; multi-target (stop any subset of cues); target All Cues or specific cues; Soft (fade) or Hard (cut) |
| Memo  | ✅ **Functional** | Read-only, no audio action |
| Video | ✅ **Functional** | Single persistent Win32 window, paused-load start (no frame-0 freeze), dip-to-black fades, scrub/seek; pause/resume; loop (finite + infinite) |
| Image | ✅ **Functional** | Same Win32 window as Video via libmpv, dip-to-black fades; stop-on-next-cue only fires for visual GOs (audio GO leaves image running); loop support |
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
| OutputEngine | `engine/output_engine/` | ✅ Complete — unified libmpv engine; single persistent Win32 window; dip-to-black fade overlay; OSD + floating timer; `get_overlay_alpha()`, `set_overlay_alpha_direct()`, `get_current_audio_voice()`, `start_overlay_fade()` (kept for reference) |
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

---

## Change history additions (0.8.1)

### Portage Phase B — Mac / Linux output + floating timer Tauri WebView (2026-06-16)

#### Mac/Linux output window control

- `show_output()` / `hide_output()` : utilisent maintenant `mpv_set_property_string("hidden", "no/yes")` sur Mac/Linux.
- `toggle_fullscreen()` : lit et bascule la propriété mpv `fullscreen` (entier 0/1 via `MPV_FORMAT_FLAG`).
- `position_window()` : applique `mpv_set_property_string("screen", "N")` avant `show_output()` sur Mac/Linux.

#### Fade overlay cross-platform

- **`fade.rs` refactorisé** :
  - `apply_overlay_alpha(alpha)` : nouveau helper visuel uniquement — Win32 `SetLayeredWindowAttributes` sur Windows, `osd-overlay 1 ass-events` (dessin ASS rectangle noir plein écran, alpha variable) sur Mac/Linux.
  - `set_overlay_alpha(alpha)` : appelle `apply_overlay_alpha` + met à jour `FADE_STATE.current_alpha`.
  - `execute_fade_pending(hwnd)` : reste Windows-only (via `WM_TIMER`).
  - `execute_fade_pending_nw()` : nouveau, non-Windows — même logique sans `SetTimer`, la boucle de fade thread récupère l'état automatiquement.
  - `run_cross_platform_fade_loop()` : thread fond 16 ms (non-Windows) — poll FADE_STATE, interpole alpha, appelle `apply_overlay_alpha`, déclenche `execute_fade_pending_nw()` en fin de transition.

- `mod.rs` : spawn du thread `wincue-output-fade` dans `OutputEngine::new()` sur non-Windows (`#[cfg(not(target_os = "windows"))]`).

#### Floating timer → Tauri WebView (toutes plateformes)

- Ancienne implémentation Win32 GDI (`floating_timer_wnd_proc`, `FLOAT_TIMER_HWND`, `WM_FLOAT_VISIBILITY`) **supprimée**.
- Fenêtre `float-timer` définie dans `tauri.conf.json` (`decorations: false`, `alwaysOnTop: true`, `transparent: true`, `visible: false`).
- `OutputEngine` possède maintenant un `tauri::AppHandle`.
- `set_floating_timer_visible(visible)` : `app_handle.get_webview_window("float-timer").show()/hide()`.
- `update_floating_timer(text)` : `app_handle.emit("float-timer-text", text)` (dédupliqué via `FLOAT_TIMER_TEXT`).
- `src/windows/FloatTimer.tsx` : composant React — écoute `float-timer-text`, affiche l'heure en police monospace, `data-tauri-drag-region` pour le drag.
- `src/main.tsx` : route `tauriLabel === "float-timer"` → `<FloatTimerWindow />`.

#### Nettoyage Win32

- `win32_window.rs` : suppression du timer overlay GDI interne (`TIMER_OVERLAY_HWND`, `TIMER_TEXT`, `timer_wnd_proc`) qui n'était jamais alimenté (code mort depuis l'adoption du OSD mpv). Suppression du float timer Win32 et de `WM_FLOAT_VISIBILITY`. Fichier réduit de ~900 → ~300 lignes.

**Files changed:** `engine/output_engine/fade.rs`, `engine/output_engine/mod.rs`, `engine/output_engine/win32_window.rs`, `src/lib.rs`, `tauri.conf.json`, `src/main.tsx`, `src/windows/FloatTimer.tsx` (new)

---

## Change history additions (0.8.0)

### Audio/Video loop — boucle finie et infinie (2026-06-16)

**Audio Cue** : `loop_count = 0` = lecture unique, `loop_count = N` = N+1 lectures, `loop_count = u32::MAX` = boucle infinie. Le callback RT rebobine sans jamais envoyer `AudioStatus::Completed` pour la boucle infinie.

**Video Cue** : `loop-file=N` ou `loop-file=inf` passé à mpv. La voice audio couplée porte également `loops_remaining = u32::MAX` pour rester synchronisée.

**Fix transport** : le guard "fichier encore en chargement" utilisait `duration().is_none()` — ce qui bloquait aussi les boucles infinies (`duration()` retourne `None` pour `loop_count = u32::MAX`). Corrigé en `file_duration().is_none()` qui retourne `None` uniquement si le fichier n'est pas encore décodé.

**Progress bar per-loop** : `CueSummary` expose `file_duration_ms` (durée d'une passe, sans multiplicateur). `CueRow` et `ScrubBar` utilisent `action_elapsed_ms % file_duration_ms` comme position → la barre se réinitialise à chaque début de boucle. `ScrubBar` accepte un prop `loopDurationMs` et affiche la position dans la boucle courante. Le scrubber s'affiche aussi pour la boucle infinie (utilise `file_duration_ms` comme période).

**Inspector Time tab** : nouveau contrôle Loop — checkbox + champ compteur + bouton ∞ (valeur `LOOP_INFINITE = 4294967295`).

**Files changed:** `cue/audio_cue.rs` (loop UI), `cue/video_cue.rs` (file_duration override), `cue/types.rs`, `commands/cue_cmds.rs` (file_duration_ms dans CueSummary), `commands/transport_cmds.rs` (fix guard), `src/lib/types.ts`, `src/components/Inspector/TimeTab.tsx`, `src/components/Inspector/ScrubBar.tsx`, `src/components/CueList/CueRow.tsx`

---

### Fade/Stop Cue — UUID multi-target + fade visuel (2026-06-16)

#### Stop Cue : multi-target UUID

- `target_cue_id: Option<CueId>` → `target_cue_ids: Vec<CueId>` + `target_cue_numbers: Vec<String>`
- Vec vide = stop all ; Vec non-vide = stop uniquement ces cues
- Rétrocompatibilité : `from_json` lit l'ancien `target_cue_id` singulier et le migre
- `resolve_stop_target` résout les numéros → UUID au chargement
- Inspector : radio "All Cues" + `CueCheckboxList` multi-sélection

#### Fade Cue : UUID multi-target + fade visuel

- Cible par UUID (`target_cue_ids: Vec<CueId>`) au lieu du numéro de cue
- Multi-target : peut fader plusieurs cues simultanément
- `resolve_fade_targets` : résout les anciens `target_cue_number` → UUID au chargement (rétrocompat)
- **Fade audio** : `FadeCue.tick()` interpole le gain de chaque voice audio à 30 fps (inchangé)
- **Fade visuel** : pour Video/Image cues, `tick()` appelle `output_engine.set_overlay_alpha_direct(alpha)` directement (pas de timer Win32 — évite les conflits d'état). `transport.go()` lit `get_overlay_alpha()` comme valeur de départ et injecte `visual_start/target_alpha` via `set_fade_voices()`
- `target_gain_linear` mappe vers l'alpha : `0.0 (−60 dB)` → `alpha 255` (noir), `1.0 (0 dB)` → `alpha 0` (transparent)
- Inspector adaptatif : "Target Volume (dB)" pour cibles audio/vidéo, "Target Brightness (%)" pour cibles image, les deux pour vidéo
- Nouveaux composants : `CueCheckboxList` (list scrollable de checkboxes)
- OutputEngine : `get_overlay_alpha()`, `set_overlay_alpha_direct()`, `get_current_audio_voice()`

**Files changed:** `cue/types.rs`, `cue/traits.rs`, `cue/fade_cue.rs`, `cue/stop_cue.rs`, `show/transport.rs`, `show/cue_list.rs`, `engine/output_engine/mod.rs`, `src/lib/types.ts`, `src/components/Inspector/BasicsTab.tsx`

---

### Cue List — colonne Notes + bouton Stop par cue (2026-06-16)

**Colonne Notes** : `notes: String` ajouté à `CueSummary` (Rust + TypeScript). Affichée dans la liste avec `text-overflow: ellipsis` et tooltip au survol. Largeur 220px par défaut, redimensionnable.

**Colonne Stop** : bouton `StopButton` (carré rouge 22×22px, hover, icône 8×8px) visible uniquement quand le cue est `running` ou `paused`. `stopPropagation` sur `mouseDown` pour ne pas déclencher de drag. Appelle `stopCue(id)` (soft stop). Les deux colonnes sont optionnelles (togglables via clic droit sur le header).

**Files changed:** `commands/cue_cmds.rs`, `src/lib/types.ts`, `src/components/CueList/columns.ts`, `src/components/CueList/CueRow.tsx`, `src/components/CueList/CueListView.tsx`

---

## Change history additions (0.7.4)

### Fix: barre d'onglets Cue List disparaît quand la liste déborde + bascules View (2026-06-15)

**Symptôme** : en chargeant un projet dont la Cue List contient plus de cues que la
hauteur de la fenêtre, la barre des onglets Cue List disparaissait entièrement. Il
fallait agrandir la fenêtre au maximum pour la faire réapparaître.

**Cause racine** : la racine de `CueListView` utilisait `height: 100%`. C'est un enfant
flex de la colonne de gauche (qui contient aussi `CueListTabs`, hauteur fixe 30 px,
`flexShrink: 0`). Sous WebView2/Chromium, un `height: 100%` sur un flex item dont la
hauteur du bloc conteneur est elle-même dérivée du flex se résout mal quand le contenu
déborde : l'item retombe sur sa hauteur de contenu (auto), déborde la colonne et pousse
la barre d'onglets hors de la zone visible. Agrandir la fenêtre fait rentrer le contenu →
plus de débordement → les onglets réapparaissent.

**Fix** :
- `CueListView` racine : `height: 100%` → `flex: 1; minHeight: 0`. L'item remplit
  l'espace restant après les 30 px d'onglets et peut rétrécir ; le scroll interne des
  rangées (`flex: 1; overflow: auto`) prend le relais. Robuste à toutes les tailles.
- Colonne de gauche dans `App.tsx` : ajout de `minWidth: 0; minHeight: 0` pour fiabiliser
  le rétrécissement du flex.

**Feature — bascules de visibilité dans le menu View** :
- `ViewMenu` généralisé pour afficher une liste d'items à cocher.
- Trois entrées : **Cue List Tabs**, **Inspector**, **Output Surface** (existant).
- `showCueListTabs` (nouveau) et `inspectorOpen` sont persistés en `localStorage`
  (clé `wincue_ui_layout`), même pattern que la config des colonnes — la disposition
  est conservée d'un lancement à l'autre. L'`Inspector` reste synchronisé entre le menu
  View, le bouton de la toolbar et Ctrl+I.

**Files changed:** `src/components/CueList/CueListView.tsx`, `src/App.tsx`

### Fix: output window reste en dessous / état visible incohérent au démarrage (2026-06-15)

**Symptôme** : après un redémarrage, la fenêtre de sortie restait invisible ou en dessous des
autres fenêtres ; impossible de la ramener au premier plan.

**Cause 1 — état `visible` incorrect au démarrage** : `OutputEngine::new()` initialisait
`visible = true` mais la fenêtre Win32 était créée et immédiatement cachée (`SW_HIDE`).
Conséquence : `getOutputWindowVisible()` retournait `true` alors que la fenêtre était invisible.
Le premier appel à `toggle_visibility()` (F9 ou menu View) **cachait** une fenêtre déjà cachée
(sans effet visible), et seulement le deuxième appel l'affichait réellement. L'utilisateur,
voyant le ✓ dans le menu View sans fenêtre visible, pensait la fenêtre "bloquée derrière".

**Cause 2 — Z-order fragile dans `show_output()`** : la séquence `ShowWindow(SW_SHOWNA)` puis
`SetWindowPos(HWND_TOPMOST)` en deux appels séparés laissait un instant où la fenêtre était
visible mais non-topmost. Sur certaines configurations (DWM/D3D11 actif), cette fenêtre pouvait
être coincée derrière d'autres fenêtres avant que `SetWindowPos` arrive.

**Fix** :
- `OutputEngine::new()` : `visible = false` (correspond à l'état réel : fenêtre cachée au démarrage).
- `show_output()` : remplace `ShowWindow` + `SetWindowPos(HWND_TOPMOST)` séparés par un seul
  `SetWindowPos(HWND_TOPMOST, SWP_SHOWWINDOW | SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE)` — show
  et topmost sont appliqués atomiquement, éliminant la fenêtre de race condition. L'overlay de
  fondu n'est plus affiché lors du toggle simple (sans contenu actif) ; il sera montré par
  `position_window()` / `show_content()` lors du prochain GO.
- **Fix définitif du z-order** : la fenêtre parent est maintenant créée avec `WS_EX_TOPMOST`
  dans son extended style (`CreateWindowExW`), comme l'overlay de fondu. Un `SetWindowPos
  (HWND_TOPMOST)` après coup peut être silencieusement ignoré par DWM/Windows 11 si la fenêtre
  n'est pas encore active ; le flag dans le extended style est permanent et ne peut pas être
  effacé par l'activation d'une autre fenêtre.

**Files changed:** `src-tauri/src/engine/output_engine/mod.rs`

---

## Change history additions (0.7.3)

### Normalize button — Audio Cue Levels tab (2026-06-14)

Nouveau bouton **Normalize to 0 dBFS** dans l'onglet Levels des Audio Cue, sous le slider Volume.

**Comportement** :
- Lit le peak de l'audio déjà décodé en mémoire (`extract_decoded_audio` → `Arc::clone`, non destructif)
- Calcule `volume_db = 20 × log10(1 / peak)` → le fader Volume est ajusté exactement pour que le sample le plus fort joue à 0 dBFS
- La valeur est arrondie à 0.1 dB et clampée dans [-60, +12] dB (identique à la plage du slider)
- Si l'audio n'est pas encore chargé (pas de fichier assigné ou decode en cours) : message d'erreur inline
- Si le fichier est silencieux (peak < -120 dBFS) : erreur "File is silent — cannot normalize"

**Implémentation** :
- `commands/cue_cmds.rs` — commande `get_normalize_db(cue_id)` : lit les samples décodés du cue actif, retourne le `volume_db` normalisé
- `lib.rs` — `get_normalize_db` enregistré dans `invoke_handler`
- `src/lib/commands.ts` — `getNormalizeDb(cueId)` exposé côté frontend
- `src/components/Inspector/LevelsTab.tsx` — bouton "Normalize to 0 dBFS" + état loading/erreur inline (uniquement pour `isAudio`)

---

## Change history additions (0.7.2)

### Fix: Image Cue fade-in / fade-out visuellement inactifs (2026-06-14)

**Symptômes** : le fade-in affichait l'image instantanément ; le fade-out attendait la durée configurée puis coupait net — sans fondu visible dans les deux cas.

**Cause racine** : la fenêtre overlay de fondu (`WS_EX_LAYERED | WS_EX_TRANSPARENT`) ne rendait pas son propre fond noir. Sous Windows, `WS_EX_TRANSPARENT` sur une fenêtre enfant layered force le composite à afficher le contenu des siblings en dessous (mpv) plutôt que la surface propre de la fenêtre. `SetLayeredWindowAttributes` animait bien la valeur alpha en interne (le timer tournait), mais sans effet visuel — l'overlay restait transparent quel que soit l'alpha.

**Fix** :
- Overlay créé avec `WS_EX_LAYERED` seul (plus `WS_EX_TRANSPARENT`).
- `overlay_wnd_proc` retourne `HTTRANSPARENT` sur `WM_NCHITTEST` → tous les événements souris (drag, double-clic fullscreen) passent au travers vers la fenêtre parente, identique au comportement antérieur.

**Files changed:** `engine/output_engine/win32_window.rs`

---

### Fix: Barre d'onglets Cue List disparaît au chargement d'un projet (2026-06-14)

**Symptôme** : au démarrage, la barre des onglets Cue List s'affichait correctement. Après avoir chargé un projet (File → Open), la barre disparaissait ou restait figée sur l'état de démarrage.

**Cause racine** : `load_workspace` et `new_workspace` n'émettaient que `workspace-modified`. Le handler frontend de cet event ne rafraîchissait que les cues et les infos workspace — jamais les cue lists. L'event `cue-lists-changed` (qui met à jour la barre d'onglets) n'était jamais déclenché lors du chargement.

**Fix** :
- `emit_cue_lists_changed` rendue publique dans `cue_list_cmds.rs`.
- `load_workspace` et `new_workspace` dans `workspace_cmds.rs` appellent `emit_cue_lists_changed` juste après avoir modifié le workspace → la barre se met à jour avec les listes et l'active_cue_list_id corrects du projet chargé.
- `App.tsx` : bootstrap simplifié — utilise `refreshCueLists()` du store au lieu d'un appel ad-hoc. `handleOpen` et `handleNew` ne font plus de gestion manuelle des cue lists, le backend s'en charge via l'event.

**Files changed:** `commands/cue_list_cmds.rs`, `commands/workspace_cmds.rs`, `src/App.tsx`

---

## Change history additions (0.7.1)

### Cue Warnings — badge ⚠ jaune non-bloquant (2026-06-13)

`is_broken` (rouge `!`) et `is_warning` (jaune `⚠`) sont maintenant deux signaux distincts dans `CueSummary` :

| Condition | Avant | Après |
|---|---|---|
| Fichier non assigné (Audio/Video/Image) | rouge `!` | jaune `⚠` |
| Fichier assigné mais introuvable sur disque | rouge `!` | rouge `!` |
| Wait Cue avec durée = 0 | rien | jaune `⚠` |
| Group Cue vide (0 enfants) | rien | jaune `⚠` |

`check_broken` ne flagge plus les fichiers non-assignés. `check_warning` couvre les cas non-critiques. `warning_message` est sérialisé dans le JSON du CueSummary pour affichage en tooltip.

**Files changed:** `commands/cue_cmds.rs`, `src/lib/types.ts`, `src/components/CueList/CueRow.tsx`

---

### Image Display Duration (2026-06-13)

Champ `display_duration_ms: Option<u64>` réintroduit dans `ImageCue` :

- `None` (défaut) : l'image reste affichée jusqu'à un Stop explicite (`stop_on_next_go = true`)
- `Some(ms)` : mpv reçoit `image-display-duration=X.XXX` au lieu de `inf` → l'image auto-complète via `OutputStatus::Completed` exactement comme une vidéo

`duration()` retourne `None` ou `Some(Duration::from_millis(ms))` selon la valeur, ce qui active la barre de progression et l'Auto-Continue dans l'event loop sans changement de code.

Inspector → onglet Time → checkbox "Display Duration" + saisie en secondes.

**Files changed:** `cue/image_cue.rs`, `engine/output_engine/types.rs`, `engine/output_engine/fade.rs`, `engine/output_engine/mod.rs`, `cue/video_cue.rs` (+ `None` pour le nouveau param), `src/lib/types.ts`, `src/components/Inspector/TimeTab.tsx`, `src/components/Inspector/InspectorPanel.tsx`

---

### Audio SR conversion — refactor architectural (2026-06-13)

**Avant :** `audio_cue.rs` et `video_cue.rs` appelaient `context.audio_engine.sample_rate()` et boulaient `source_sr / output_sr` directement dans `voice.inner.rate_bits`. Problèmes : violation de la séparation des couches (`cue/` ne doit pas interroger les internals du moteur), et `rate_bits` contenait un composite opaque au lieu du rate utilisateur pur.

**Après :**
- `voice.inner.rate_bits` = pure user rate multiplier (1.0 par défaut, contrôlé par l'inspecteur)
- `fill_buffer` reçoit `output_sample_rate: u32` capturé dans la closure à l'ouverture du stream
- Step effectif par voice : `user_rate × (voice.sample_rate / output_sample_rate)`

Résultat : 44.1 kHz, 48 kHz, 96 kHz jouent à la bonne vitesse et durée sur n'importe quel device. **5 tests unitaires** vérifient les cas cross-rate sans device audio réel.

**Note 96 kHz downsampling :** quand `source_sr > output_sr` (ex. 96k → 48k), le callback ne fait pas de filtre anti-repliement avant de sauter des frames. Le contenu au-dessus de la fréquence de Nyquist output (24 kHz pour 48k) peut aliaser. En pratique imperceptible : les fichiers 96 kHz sont déjà band-limités sous 20 kHz par l'encodeur.

**Files changed:** `engine/audio_engine.rs` (signature `fill_buffer`, step SR, 5 tests), `cue/audio_cue.rs` (retire `sr_ratio`), `cue/video_cue.rs` (retire `sr_ratio`)

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
