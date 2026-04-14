# WinCue — État du projet au 2026-04-14

## Version courante : 0.2.0

## Résultat de cargo build

**Compile sans erreur, zéro warning.**

## Résultat de cargo test

**20 tests passent, 0 échec.**

---

## Ce qui est implémenté et compile

### Backend Rust

| Module | Fichier | Statut |
|---|---|---|
| Types cue | `cue/types.rs` | ✅ Complet |
| Trait Cue | `cue/traits.rs` | ✅ Complet |
| CueRegistry | `cue/registry.rs` | ✅ Complet |
| CueContext | `cue/context.rs` | ✅ Complet — `audio_engine`, `video_engine`, `stop_fade_ms`, `output_patches`, `audio_device_id`, `audio_backend` |
| AudioCue | `cue/audio_cue.rs` | ✅ Complet — pre-wait, fade-in/out, loop, rate mismatch, routing `Voice.out_l/r` via OutputPatch |
| VideoCue | `cue/video_cue.rs` | ✅ Complet — lecture fonctionnelle, `screen_index`, `output_patch_id`, routing ASIO→WASAPI |
| MemoCue | `cue/memo_cue.rs` | ✅ Complet |
| StopCue | `cue/stop_cue.rs` | ✅ Complet |
| VoiceState / FadeState | `engine/voice.rs` | ✅ Complet — `out_l`, `out_r` pour routing canaux |
| AudioCommand / AudioStatus | `engine/ring_command.rs` | ✅ Complet |
| DeviceManager / OutputPatch | `engine/device_manager.rs` | ✅ Complet |
| AudioEngine | `engine/audio_engine.rs` | ✅ Complet — WASAPI/ASIO, routing `Voice.out_l/r` dans `fill_buffer` |
| VideoEngine | `engine/video_engine.rs` | ✅ Complet — libmpv/Win32, plein écran, redimensionnable, `list_screens()`, WASAPI loopback VU, routing ASIO→WASAPI |
| mpv_sys (FFI) | `engine/mpv_sys.rs` | ✅ Bindings libmpv compilés — `mpv_free` ajouté |
| CueList | `show/cue_list.rs` | ✅ Complet |
| Workspace | `show/workspace.rs` | ✅ Complet |
| Transport | `show/transport.rs` | ✅ Complet |
| Event Loop 30fps | `show/event_loop.rs` | ✅ Complet |
| UndoStack | `show/undo_stack.rs` | ✅ Complet |
| AppState | `state/app_state.rs` | ✅ Complet |
| Preferences | `preferences.rs` | ✅ Complet |
| Commands transport | `commands/transport_cmds.rs` | ✅ Complet |
| Commands cues | `commands/cue_cmds.rs` | ✅ Complet — `set_video_file`, `list_video_screens` |
| Commands workspace | `commands/workspace_cmds.rs` | ✅ Complet |
| Commands devices | `commands/device_cmds.rs` | ✅ Complet |
| Commands preferences | `commands/preferences_cmds.rs` | ✅ Complet |
| Commands undo | `commands/undo_cmds.rs` | ✅ Complet |

### Frontend React / TypeScript

| Fichier | Statut |
|---|---|
| `lib/types.ts` | ✅ Complet — `VideoCueData.screen_index`, `ScreenInfo` |
| `lib/commands.ts` | ✅ Complet — `listVideoScreens` |
| `stores/workspaceStore.ts` | ✅ Complet |
| `stores/transportStore.ts` | ✅ Complet |
| `stores/timingStore.ts` | ✅ Complet |
| `hooks/useTauriEvents.ts` | ✅ Complet |
| `hooks/useKeyboardShortcuts.ts` | ✅ Complet |
| `App.tsx` | ✅ Complet |
| `components/CueList/` | ✅ Complet |
| `components/Inspector/InspectorPanel.tsx` | ✅ Complet — video fully supported |
| `components/Inspector/BasicsTab.tsx` | ✅ Complet — sélecteur d'écran vidéo |
| `components/Inspector/TimeTab.tsx` | ✅ Complet |
| `components/Inspector/LevelsTab.tsx` | ✅ Complet — `isAudio` conditionnel sur Pan |
| `components/Inspector/FadeTab.tsx` | ✅ Complet — types `AudioCueData \| VideoCueData` |
| `components/Transport/TransportBar.tsx` | ✅ Complet — rAF decay + peak hold, WASAPI loopback pour vidéo |
| `components/Preferences/PreferencesModal.tsx` | ✅ Complet |
| `components/WaveformModal.tsx` | ✅ Complet |

---

## Bugs connus (non résolus)

### ASIO→WASAPI : résolution de nom à vérifier

Le routing vidéo vers ASIO est implémenté via `wasapi_name_for_asio()` dans `VideoEngine` : fuzzy match en deux passes (substring complet, puis mots-clés sans "ASIO"/"Driver"). La résolution compile et s'exécute mais n'a pas encore été vérifiée sur le matériel de l'utilisateur (UMC ASIO Driver → endpoint WASAPI inconnu). Si `None` est retourné, mpv bascule sur le device système par défaut.

---

## Historique des changements

### Post-0.2.0 — VU meter, ASIO build, Output Patch routing (2026-04-14)

#### 🎚️ VU Meter — mesures réelles

- `VideoEngine` : thread WASAPI loopback (`cpal`) — capture audio sortant, atomics `loopback_peak_l/r` lus par le 30fps loop
- `TransportBar.tsx` : rAF-based decay (20 dB/sec), peak hold 1,5 s, aiguille rouge > -6 dBFS
- Suppression du fallback "volume configuré" pour la vidéo — uniquement vraies mesures loopback

#### 🔧 ASIO build fix

- SDK ASIO copié dans `vendor/asiosdk/` (hors portée du WalkDir)
- `src-tauri/.cargo/config.toml` : `CPAL_ASIO_DIR = { value = "../vendor/asiosdk", relative = true }`
- `pnpm tauri:dev -- --features asio-support` compile sans erreur

#### 🔌 Output Patch routing — câblage complet

- `Voice.out_l / out_r` (`engine/voice.rs`) — canaux cibles dans le buffer WASAPI/ASIO
- `AudioEngine.fill_buffer` — utilise `voice.out_l / out_r` au lieu de 0/1 codés en dur
- `CueContext` enrichi : `output_patches`, `default_patch_id`, `audio_device_id`, `audio_backend`
- `AudioCue` : résout l'`OutputPatch` au GO, positionne `voice.out_l / out_r`
- `VideoCue` : champ `output_patch_id`, résolution device + fallback `ws.preferences.audio.device_id`
- `VideoEngine.play_voice()` : accepte `audio_device` + `audio_backend`, set mpv `audio-device`
- `wasapi_name_for_asio()` : résolution ASIO→WASAPI par fuzzy match (deux passes)
- `event_loop.rs` + `transport_cmds.rs` : snapshottent les préférences audio avant construction du `CueContext`

---

### 0.2.0 — Video Cue fonctionnel + correctifs (2026-04-13)

#### 🎬 Video Cue — lecture opérationnelle

Après plusieurs itérations de débogage (session 2026-04-12 → 13), la lecture vidéo est entièrement fonctionnelle.

**Problèmes résolus dans `video_engine.rs` :**

- ✅ **Backend D3D11** — `gpu-api=d3d11` forcé ; le backend ANGLE/EGL par défaut ne supporte pas `--wid`
- ✅ **Argument `loadfile`** — ordre corrigé : `url, flags, index(int), options` ; l'index entier manquant causait `"argument index can't be parsed"`
- ✅ **Boucle infinie** — `loop-file=no` pour `loop_count=0` (N *extra* loops, pas N total)
- ✅ **Deuxième vidéo figée** — `keep-open=no` + `set pause no` avant chaque `loadfile`
- ✅ **Curseur Windows qui charge** — suppression de l'overlay `WS_EX_LAYERED` (échouait à la création) ; nouveau système : `WS_EX_TRANSPARENT` posé sur la render child de mpv au `MPV_EVENT_FILE_LOADED` via `WM_SETUP_MPV_CHILD`
- ✅ **Drag et double-clic plein écran** — `CS_DBLCLKS` sur la classe parent + handlers `WM_LBUTTONDOWN` / `WM_LBUTTONDBLCLK` directs
- ✅ **Vol de focus** — `WS_EX_NOACTIVATE` + `SW_SHOWNA` + `WM_MOUSEACTIVATE → MA_NOACTIVATE` ; WinCue garde le focus clavier, l'espace GO fonctionne pendant la lecture
- ✅ **Fenêtre toujours au premier plan** — `HWND_TOPMOST` + `SWP_NOACTIVATE`
- ✅ **Double cue en cours** — `VideoStatus::Completed` envoyé pour l'ancien voice avant `loadfile replace`
- ✅ **Plein écran avec bordure** — `WS_SIZEBOX` retiré via `SetWindowLongPtrW(GWL_STYLE)` en fullscreen, restauré en floating

#### 🖥️ Sélection d'écran (inspector)

- `VideoEngine::list_screens()` — énumération `EnumDisplayMonitors` + `GetMonitorInfoW`, primaire en index 0
- `ScreenInfo` struct sérialisable (`index`, `width`, `height`, `x`, `y`, `is_primary`)
- `VideoCue.screen_index: Option<u32>` — sérialisé dans `.wincue`
- `list_video_screens` commande Tauri — appelée par le frontend
- `BasicsTab.tsx` — dropdown "Floating window" + liste des écrans détectés au chargement de l'inspector
- En fullscreen : `WS_SIZEBOX` supprimé, `HWND_TOPMOST`, positionnement exact sur le monitor rect
- En floating : `WS_SIZEBOX` restauré, taille sauvegardée restaurée

#### 🪟 Fenêtre vidéo redimensionnable sans bordure

- `WS_SIZEBOX` sur le style de la popup — redimensionnement OS natif sur les bords
- `WM_NCHITTEST` — passe les bords au `DefWindowProc` (resize), force `HTCLIENT` pour l'intérieur
- `WM_SETCURSOR` — arrow uniquement sur `HTCLIENT`, curseur resize OS sur les bords

#### 🐛 Correctifs Inspector Video Cue

- `LevelsTab.tsx` — type `AudioCueData | VideoCueData`, prop `isAudio`, Pan conditionnel
- `FadeTab.tsx` — type élargi `AudioCueData | VideoCueData`
- `InspectorPanel.tsx` — `isAudio` passé à `LevelsTab`, casts `as AudioCueData` supprimés

---

### Post-0.1.2 — correctifs et fonctionnalités (2026-04-12)

#### ⚙️ Préférences Générales

4 options : Double GO Protection, Confirm Before Delete, Auto-Scroll to Playhead, Cue Row Height.

#### 🐛 Correctifs Backend

- Copy/paste Audio Cue sans son — transfert `Arc` samples dans `paste_cue`
- Préférence durée fade STOP ignorée — `CueContext.stop_fade_ms` lu depuis les préfs

#### 🎨 Fonctionnalités Frontend

- Refactoring Inspector — 7 sous-composants extraits
- Color tags — bande colorée 4px QLab-style

---

### 0.1.2 (2026-04-11)

- Stop Cue
- Drag & drop repensé (CustomEvent, sans conflit Tauri)
- Fix Auto-Continue immédiat (résolution synchrone dans Transport)
- Fix loop playback
- Fix duplicate/paste cue sans audio

---

### 0.1.1 (2026-04-11)

- `CueList::renumber_all()`
- `set_master_volume`
- Shortcuts manquants
- `CurveSelect` avec aperçu SVG
- Refonte TransportBar

---

## Ce qui est partiellement implémenté ou manquant

### Backend

#### Routing par Output Patch non implémenté
Tout l'audio sort sur le device par défaut. `OutputPatch` est stocké mais `AudioEngine` ne le consulte pas.

### Frontend

Rien de bloquant.

---

## État des étapes de développement

| Étape | Statut |
|---|---|
| 1. Scaffold Tauri + fenêtre | ✅ |
| 2. Cue trait + CueRegistry + MemoCue | ✅ |
| 3. AudioEngine WAV (cpal + symphonia) | ✅ |
| 4. AudioCue connectée à l'engine | ✅ |
| 5. Frontend CueList + GO | ✅ |
| 6. Playhead + transport | ✅ |
| 7. Output Patches + DeviceManager | ✅ Routing audio + vidéo câblé — ASIO→WASAPI à valider sur hardware |
| 8. Inspector panel | ✅ Complet audio + vidéo |
| 9. Workspace save/load | ✅ |
| 10. Keyboard shortcuts | ✅ |
| 11. Fades, waveform, level meters | ✅ |
| 12. Drag-drop, undo/redo, color tags | ✅ |
| 13. Video Cue | ✅ Fonctionnel — plein écran, sélection écran, redimensionnable |

---

## Prochaine priorité

1. **Valider ASIO→WASAPI** — lancer l'app, vérifier les logs `WASAPI devices for ASIO match` et `ASIO→WASAPI`, confirmer que mpv route bien l'audio vidéo vers le matériel ASIO
2. **Retirer le log `[DIAG-1]`** dans `VideoEngine::new()` une fois le routing confirmé
