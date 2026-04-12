# WinCue — État du projet au 2026-04-12

## Version courante : 0.1.2

## Résultat de cargo build

**Compile sans erreur, zéro warning.** (au 2026-04-12)

## Résultat de cargo test

**20 tests passent, 0 échec.** (au 2026-04-11)

---

## Ce qui est implémenté et compile

### Backend Rust

| Module | Fichier | Statut |
|---|---|---|
| Types cue | `cue/types.rs` | ✅ Complet |
| Trait Cue | `cue/traits.rs` | ✅ Complet |
| CueRegistry | `cue/registry.rs` | ✅ Complet |
| CueContext | `cue/context.rs` | ✅ Complet — `stop_fade_ms` lu depuis les préférences |
| AudioCue | `cue/audio_cue.rs` | ✅ Complet — pre-wait, fade-in/out, loop, rate mismatch |
| MemoCue | `cue/memo_cue.rs` | ✅ Complet |
| StopCue | `cue/stop_cue.rs` | ✅ Complet — stoppe tous les cues en cours au GO |
| VoiceState / FadeState | `engine/voice.rs` | ✅ Complet |
| AudioCommand / AudioStatus | `engine/ring_command.rs` | ✅ Complet |
| DeviceManager / OutputPatch | `engine/device_manager.rs` | ✅ Complet |
| AudioEngine | `engine/audio_engine.rs` | ✅ Complet — WASAPI/ASIO, `set_master_gain()` |
| CueList | `show/cue_list.rs` | ✅ Complet — `renumber_all()` après chaque mutation |
| Workspace | `show/workspace.rs` | ✅ Complet |
| Transport | `show/transport.rs` | ✅ Complet — chaînes Auto-Continue résolues synchronement |
| Event Loop 30fps | `show/event_loop.rs` | ✅ Complet |
| UndoStack | `show/undo_stack.rs` | ✅ Complet — pile 50 niveaux, snapshots JSON + Arc clones |
| AppState | `state/app_state.rs` | ✅ Complet |
| Preferences | `preferences.rs` | ✅ Complet — `GeneralPreferences` avec 4 champs, `update_general_preferences` |
| Commands transport | `commands/transport_cmds.rs` | ✅ Complet — `set_master_volume`, gestion `StopAll` |
| Commands cues | `commands/cue_cmds.rs` | ✅ Complet |
| Commands workspace | `commands/workspace_cmds.rs` | ✅ Complet |
| Commands devices | `commands/device_cmds.rs` | ✅ Complet |
| Commands preferences | `commands/preferences_cmds.rs` | ✅ Complet |
| Commands undo | `commands/undo_cmds.rs` | ✅ Complet — undo, redo, copy_cue, paste_cue (avec transfert audio) |

### Frontend React / TypeScript

| Fichier | Statut |
|---|---|
| `lib/types.ts` | ✅ Complet |
| `lib/commands.ts` | ✅ Complet |
| `stores/workspaceStore.ts` | ✅ Complet |
| `stores/transportStore.ts` | ✅ Complet |
| `stores/timingStore.ts` | ✅ Complet |
| `hooks/useTauriEvents.ts` | ✅ Complet |
| `hooks/useKeyboardShortcuts.ts` | ✅ Complet — Space/Esc/S/P/[/]/Ctrl+S/O/N/D/I/Z/Y/C/V/G/Delete/↑↓ ; double GO protection ; confirm delete |
| `App.tsx` | ✅ Complet — boutons `+ Audio` / `+ Stop` draggables, insertion après sélection |
| `components/CueList/columns.ts` | ✅ Complet |
| `components/CueList/CueListView.tsx` | ✅ Complet — drag custom (CustomEvent), insert-between fichiers, fix DPI |
| `components/CueList/CueRow.tsx` | ✅ Complet — icône par type, bande colorée gauche (color tags) |
| `components/CueList/PlayheadIndicator.tsx` | ✅ Complet |
| `components/Inspector/InspectorPanel.tsx` | ✅ Complet — shell 135 lignes, tabs, état, browse |
| `components/Inspector/BasicsTab.tsx` | ✅ Extrait |
| `components/Inspector/TimeTab.tsx` | ✅ Extrait |
| `components/Inspector/LevelsTab.tsx` | ✅ Extrait |
| `components/Inspector/FadeTab.tsx` | ✅ Extrait |
| `components/Inspector/WaveformViewer.tsx` | ✅ Extrait |
| `components/Inspector/ColorPicker.tsx` | ✅ Extrait |
| `components/Inspector/Field.tsx` | ✅ Extrait — primitif partagé (Field + inputStyle) |
| `components/Transport/TransportBar.tsx` | ✅ Complet — VU-mètre master, slider volume, STOP draggable |
| `components/common/TimeDisplay.tsx` | ✅ Complet |
| `components/common/CurveSelect.tsx` | ✅ Complet — aperçu SVG de chaque courbe |
| `components/Preferences/PreferencesModal.tsx` | ✅ Complet — onglets Audio + General |
| `components/WaveformModal.tsx` | ✅ Complet |

---

## Historique des changements

### Post-0.1.2 — correctifs et fonctionnalités (2026-04-12)

#### ⚙️ Préférences Générales

**4 options dans l'onglet General des Préférences**

- **Double GO Protection** — délai minimum entre deux GO consécutifs (défaut 500 ms, 0 = désactivé)
  - `preferences.rs` — champ `double_go_protection_ms: u32` avec `serde(default)`
  - `useKeyboardShortcuts.ts` — `useRef<number>` pour horodatage, protection vérifiée avant chaque GO (Space)

- **Confirm Before Delete** — fenêtre de confirmation native avant suppression d'un cue (défaut false)
  - `preferences.rs` — champ `confirm_before_delete: bool`
  - `useKeyboardShortcuts.ts` — `await confirm(...)` via `@tauri-apps/plugin-dialog` si actif (Delete/Backspace)

- **Auto-Scroll to Playhead** — défilement automatique vers le cue à la tête de lecture quand le playhead bouge (défaut true)
  - `preferences.rs` — champ `auto_scroll_to_playhead: bool`
  - `CueListView.tsx` — `useEffect` sur `playheadCueId` → `scrollIntoView({ block: "nearest" })`

- **Cue Row Height** — hauteur des rangées : Compact (22 px) / Normal (26 px) / Tall (32 px), défaut Normal
  - `preferences.rs` — enum `CueRowHeight` + champ dans `GeneralPreferences`
  - `CueListView.tsx` — calcule `rowHeight` depuis la préf, le passe à chaque `<CueRow>`
  - `CueRow.tsx` — prop `rowHeight?: number` utilisé comme `minHeight`

**Infrastructure commune**
- `preferences_cmds.rs` — nouvelle commande `update_general_preferences`
- `lib.rs` — commande enregistrée dans `invoke_handler`
- `lib/types.ts` — `CueRowHeight`, `GeneralPreferences`, `DEFAULT_GENERAL_PREFS`, `AppPreferences.general` typé
- `lib/commands.ts` — `updateGeneralPreferences`
- `stores/workspaceStore.ts` — `generalPrefs`, `loadGeneralPrefs()`, `setGeneralPrefs()`
- `App.tsx` — `loadGeneralPrefs()` appelé au bootstrap
- `PreferencesModal.tsx` — composant `GeneralContent` (4 contrôles), `handleApply` appelle `updateGeneralPreferences` + sync store

---

#### 🐛 Correctifs Backend

**Copy/paste de Audio Cue — son absent**
- `commands/undo_cmds.rs` — `paste_cue` reconstruisait le cue depuis JSON uniquement, sans transférer les samples décodés ; le cue collé était muet et bloquait le GO
- Fix : extraction de l'`Arc` samples depuis le cue original via `extract_decoded_audio` + `accept_preloaded_audio` (même stratégie que `duplicate_cue`)
- Fallback : si le cue original a été supprimé avant le coller, un thread de décodage en arrière-plan est déclenché

**Préférence durée fade STOP ignorée**
- `cue/context.rs` — nouveau champ `stop_fade_ms: u32` dans `CueContext`
- `cue/audio_cue.rs` — `stop()` utilisait la constante `DEFAULT_FADE_OUT_MS = 500` au lieu de la préférence
- `commands/transport_cmds.rs` + `show/event_loop.rs` — tous les contextes lisent `ws.preferences.audio.default_fade_out_ms`

#### 🎨 Fonctionnalités Frontend

**Refactoring Inspector — extraction en sous-composants**
- `InspectorPanel.tsx` réduit de 875 → 135 lignes (shell uniquement)
- 7 fichiers extraits : `BasicsTab`, `TimeTab`, `LevelsTab`, `FadeTab`, `WaveformViewer`, `ColorPicker`, `Field`
- Comportement identique, zéro régression TypeScript

**Color tags**
- `components/CueList/CueRow.tsx` — bande colorée 4px sur le bord gauche (transparent si `"none"`), style QLab
- `components/Inspector/InspectorPanel.tsx` — composant `ColorPicker` : 10 swatches (none + 9 couleurs), ajouté dans le Basics tab sous Continue
- Backend et sérialisation déjà complets — aucune modification Rust nécessaire

---

### 0.1.2 — correctifs post-release (2026-04-12, commit 89b8271)

**Loop playback — statut idle prématuré**
- `cue/audio_cue.rs` — `duration()` retournait la durée d'une seule itération au lieu de `base × (loop_count + 1)` ; l'event loop détectait une fin prématurée dès la 2ème boucle
- Loops infinis (`loop_count == u32::MAX`) → `duration()` retourne `None`

**Auto-Continue bloqué après Stop All**
- `auto_continue_fired` n'était jamais remis à zéro lors d'un Stop externe ; les chaînes ne refireaient plus à la relecture
- Fix : `auto_continue_fired` remis à zéro à chaque `go()` ; purge via `retain()` à chaque tick de l'event loop

**Auto-Continue — délai ~33 ms par maillon**
- Les chaînes immédiates (`post_wait == 0`) étaient déclenchées dans la boucle 30 fps (+1 tick par maillon)
- Fix : `Transport::go()` résout toute la chaîne immédiate synchronement et retourne tous les IDs ; l'event loop ne gère que les chaînes avec délai (`post_wait > 0`)

**Duplicate Cue sans audio**
- `commands/cue_cmds.rs` — `duplicate_cue` reconstruisait depuis JSON sans transférer les samples ; `cached_duration == None` → garde GO sautait silencieusement
- Fix : `extract_decoded_audio` + `accept_preloaded_audio`

**Nouveaux éléments de trait**
- `cue/traits.rs` — `extract_decoded_audio`, `accept_preloaded_audio`, `auto_continue_fired` / reset

---

### 0.1.2 (2026-04-11)

#### 🔧 Backend

**Nouveau type de cue : Stop Cue**
- `cue/stop_cue.rs` — `StopCue` + `StopCueFactory` : au GO, émet `CueEvent::StopAll` et se complète immédiatement
- `cue/context.rs` — variante `CueEvent::StopAll`
- `cue/types.rs` — `CueType::Stop` (sérialisé `"stop"`)
- `commands/transport_cmds.rs` — `go` draine le channel et gère `StopAll` synchronement

#### 🎨 Frontend

**Stop Cue dans l'UI**
- `lib/types.ts` — `"stop"` ajouté à `CueType`
- `components/CueList/CueRow.tsx` — icône `⬛` pour les Stop Cues
- `App.tsx` — bouton `+ Stop` dans la toolbar

**Drag & drop repensé (system custom mouse events)**
- Abandon HTML5 DnD API (interceptée par Tauri comme drag fichier OS)
- `CustomEvent("wincue:cue-drag-start")` au `mousedown` ; `CueListView` l'écoute globalement
- STOP draggable (TransportBar), `+ Audio` / `+ Stop` draggables (toolbar)
- Insertion après le cue sélectionné (au lieu de fin de liste)

**Calcul de position robuste**
- Remplacement de `document.elementFromPoint()` par scan linéaire des rangées `[data-cue-id]` par midpoint
- Fix décalage ~2 rangées sur Windows HiDPI : division par `window.devicePixelRatio`

**File drag-and-drop amélioré**
- Mode insérer (bords ±8px) / mode assigner (centre de rangée)
- Fix coordonnées HiDPI

---

### 0.1.1 (2026-04-11)

#### 🔧 Backend

- `CueList::renumber_all()` après chaque mutation (push, insert, remove, move_cue)
- Commande `set_master_volume` (dB → gain linéaire → `set_master_gain()`)

#### 🎨 Frontend

- Shortcuts manquants : `Ctrl+S/O/I`, `G` (GotoDialog), `Ctrl+↑/↓`
- `CurveSelect` — composant partagé avec aperçu SVG (smooth-step, exponentiel)
- Refonte TransportBar : VU-mètre gradué en dB, gradient couleur, slider volume

---

## Ce qui est partiellement implémenté ou manquant

### Backend

#### Routing par Output Patch non implémenté
Tout l'audio sort sur le device par défaut. `OutputPatch` est stocké mais l'`AudioEngine` ne le consulte pas.

### Frontend

| Manquant | Détail |
|---|---|

---

## État des étapes de développement (CLAUDE.md)

| Étape | Statut |
|---|---|
| 1. Scaffold Tauri + fenêtre | ✅ |
| 2. Cue trait + CueRegistry + MemoCue | ✅ |
| 3. AudioEngine WAV (cpal + symphonia) | ✅ |
| 4. AudioCue connectée à l'engine | ✅ |
| 5. Frontend CueList + GO | ✅ |
| 6. Playhead + transport | ✅ |
| 7. Output Patches + DeviceManager | ⚠️ Modèle présent, routing audio non branché |
| 8. Inspector panel | ✅ |
| 9. Workspace save/load | ✅ |
| 10. Keyboard shortcuts | ✅ |
| 11. Fades, waveform, level meters | ✅ |
| 12. Drag-drop, undo/redo, color tags | ✅ |

---

## Prochaines priorités

1. **Routing Output Patch** dans `AudioEngine`
