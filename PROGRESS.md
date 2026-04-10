# WinCue — État du projet au 2026-04-11

## Version courante : 0.1.2

## Résultat de cargo build

**Compile sans erreur, zéro warning.** (au 2026-04-11)

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
| CueContext | `cue/context.rs` | ✅ Complet |
| AudioCue | `cue/audio_cue.rs` | ✅ Complet — pre-wait, fade-in/out, rate mismatch corrigé |
| MemoCue | `cue/memo_cue.rs` | ✅ Complet |
| **StopCue** | **`cue/stop_cue.rs`** | **✅ Nouveau — stoppe tous les cues en cours au GO** |
| VoiceState / FadeState | `engine/voice.rs` | ✅ Complet |
| AudioCommand / AudioStatus | `engine/ring_command.rs` | ✅ Complet |
| DeviceManager / OutputPatch | `engine/device_manager.rs` | ✅ Complet |
| AudioEngine | `engine/audio_engine.rs` | ✅ Complet — ASIO, `set_master_gain()` |
| CueList | `show/cue_list.rs` | ✅ Complet — `renumber_all()` appelé après chaque mutation |
| Workspace | `show/workspace.rs` | ✅ Complet |
| Transport | `show/transport.rs` | ✅ Complet |
| Event Loop 30fps | `show/event_loop.rs` | ✅ Complet |
| UndoStack | `show/undo_stack.rs` | ✅ Complet — pile 50 niveaux, snapshots JSON + Arc clones |
| AppState | `state/app_state.rs` | ✅ Complet |
| Preferences | `preferences.rs` | ✅ Complet |
| Commands transport | `commands/transport_cmds.rs` | ✅ Complet — `set_master_volume`, gestion `StopAll` event |
| Commands cues | `commands/cue_cmds.rs` | ✅ Complet |
| Commands workspace | `commands/workspace_cmds.rs` | ✅ Complet |
| Commands devices | `commands/device_cmds.rs` | ✅ Complet |
| Commands preferences | `commands/preferences_cmds.rs` | ✅ Complet |
| Commands undo | `commands/undo_cmds.rs` | ✅ Complet — undo, redo, copy_cue, paste_cue |

### Frontend React / TypeScript

| Fichier | Statut |
|---|---|
| `lib/types.ts` | ✅ Complet — type `"stop"` ajouté à `CueType` |
| `lib/commands.ts` | ✅ Complet — `setMasterVolume` ajouté |
| `stores/workspaceStore.ts` | ✅ Complet |
| `stores/transportStore.ts` | ✅ Complet |
| `stores/timingStore.ts` | ✅ Complet |
| `hooks/useTauriEvents.ts` | ✅ Complet |
| `hooks/useKeyboardShortcuts.ts` | ✅ Complet — Space/Esc/S/P/[/]/Ctrl+S/O/N/D/I/Z/Y/C/V/G/Delete/↑↓ |
| `App.tsx` | ✅ Complet — boutons `+ Audio` / `+ Stop` draggables, insertion après sélection |
| `components/CueList/columns.ts` | ✅ Complet |
| `components/CueList/CueListView.tsx` | ✅ Complet — drag custom (CustomEvent), insert-between fichiers, fix DPI |
| `components/CueList/CueRow.tsx` | ✅ Complet — icône `⬛` pour Stop Cue |
| `components/CueList/PlayheadIndicator.tsx` | ✅ Complet |
| `components/Inspector/InspectorPanel.tsx` | ✅ 4 onglets — CurveSelect avec aperçu SVG |
| `components/Transport/TransportBar.tsx` | ✅ Complet — STOP draggable via mouse events |
| `components/common/TimeDisplay.tsx` | ✅ Complet |
| `components/common/CurveSelect.tsx` | ✅ Complet — composant partagé avec aperçu SVG de chaque courbe |
| `components/Preferences/PreferencesModal.tsx` | ✅ Complet — CurveSelect intégré |
| `components/WaveformModal.tsx` | ✅ Complet |

---

## Travail accompli en 0.1.2 (2026-04-11)

### 🔧 Backend

**Nouveau type de cue : Stop Cue**
- `cue/stop_cue.rs` — `StopCue` + `StopCueFactory` : au GO, émet `CueEvent::StopAll` et se complète immédiatement
- `cue/context.rs` — nouvelle variante `CueEvent::StopAll` dans l'enum des événements
- `cue/types.rs` — `CueType::Stop` ajouté (sérialisé `"stop"`)
- `cue/mod.rs` — module `stop_cue` exposé
- `state/app_state.rs` — `StopCueFactory` enregistré dans le `CueRegistry`
- `commands/transport_cmds.rs` — la commande `go` draine maintenant le channel d'événements après `transport.go()` et gère `StopAll` en appelant `stop_all` sur tous les cues en cours

### 🎨 Frontend

**Stop Cue dans l'UI**
- `lib/types.ts` — `"stop"` ajouté au type `CueType`
- `components/CueList/CueRow.tsx` — icône `⬛` pour les Stop Cues
- `App.tsx` — bouton `+ Stop` (rouge clair) à côté de `+ Audio` dans la barre d'outils

**Drag & drop repensé (système custom mouse events)**
- Abandon de l'HTML5 DnD API : Tauri intercepte les drag internes comme des drags de fichier OS, bloquant le `onDrop` DOM et déclenchant faussement `isDragOver`
- Nouveau mécanisme via `CustomEvent("wincue:cue-drag-start")` dispatché au `mousedown` ; `CueListView` l'écoute dans son `useEffect` global déjà en place pour le réordonnancement
- Bouton `■ STOP` (TransportBar) : glisser dans la liste → ligne rouge à la position d'insertion → relâcher → Stop Cue créé
- Bouton `+ Audio` et `+ Stop` (toolbar App) : même comportement via le même mécanisme
- Clic sur `+ Audio` / `+ Stop` insère désormais **après le cue sélectionné** (au lieu de toujours à la fin)

**Calcul de position robuste (fix ligne qui saute)**
- Remplacement de `document.elementFromPoint()` par un scan linéaire des rangées `[data-cue-id]` par midpoint
- `elementFromPoint` échouait silencieusement sur les gaps inter-rangées, la scrollbar, le header → fallback brutal sur `cues.length` (fin de liste)
- Le nouveau scan retourne toujours la bonne position même entre les rangées

**File drag-and-drop amélioré**
- Mode **insérer** : curseur dans les 8 px du bord haut/bas d'une rangée → ligne bleue d'insertion → crée un nouveau cue à cette position
- Mode **assigner** : curseur au milieu d'une rangée → encadrement bleu → assigne le fichier au cue existant
- Fix décalage ~2 rangées sur Windows HiDPI : les coordonnées Tauri sont en pixels physiques ; division par `window.devicePixelRatio` avant comparaison avec `getBoundingClientRect()` (pixels CSS)

---

## Travail accompli en 0.1.1 (2026-04-11)

### 🔧 Backend

**Numérotation automatique des cues**
- `CueList::renumber_all()` assigne "1", "2", "3"… après chaque `push`, `insert`, `remove`, `move_cue`
- Couvre drag & drop, add, remove, reorder, duplicate, paste
- Undo/redo restaure les numéros depuis le snapshot

**Commande `set_master_volume`**
- Nouveau Tauri command dans `transport_cmds.rs`
- Convertit dB → gain linéaire et appelle `audio_engine.set_master_gain()` atomiquement

**Diagnostic drag & drop fichiers**
- Bug : terminal lancé en administrateur → UAC bloquait le drag depuis l'Explorateur Windows
- Fix : lancer le terminal sans privilèges élevés (pas de modification de code)

**Permission Tauri drag-drop**
- Investigation : `core:window:allow-drag-drop` n'existe pas en Tauri v2
- `fileDropEnabled` n'est pas un champ valide dans `tauri.conf.json` v2
- `onDragDropEvent` fonctionne nativement — problème était uniquement UAC

### 🎨 Frontend

**Shortcuts manquants ajoutés**
- `Ctrl+S` → save workspace
- `Ctrl+O` → open workspace
- `Ctrl+I` → toggle inspector
- `G` → GotoDialog (overlay input, Enter confirme, Escape annule)
- `Ctrl+↑` / `Ctrl+↓` → déplace le playhead vers la cue précédente / suivante

**CurveSelect — aperçu SVG des courbes de fade**
- Composant partagé `components/common/CurveSelect.tsx`
- Mini-SVG généré depuis les formules exactes du moteur Rust (smooth-step, exponentiel)
- Intégré dans l'Inspector (onglet Fade) et dans les Préférences

**Refonte TransportBar**
- VU-mètre horizontal gradué en dB (−60 à 0, ticks à 0/−6/−12/−18/−24/−36)
- Gradient de couleur vert → jaune → orange → rouge sur échelle logarithmique
- Slider volume `<input type="range">` natif aligné pixel-perfect sous les barres L/R
- Initialisé depuis `prefs.audio.default_volume_db` au montage
- Boutons GO (22px) et STOP (18px) agrandis
- Status idle/running agrandi (18px, fontWeight 600)

---

## Ce qui est partiellement implémenté ou manquant

### Backend — fonctionnalités manquantes

#### Routing par Output Patch non implémenté
Tout l'audio sort sur le device par défaut. `OutputPatch` est stocké mais l'`AudioEngine` ne le consulte pas.

### Frontend — fonctionnalités manquantes

| Manquant | Détail |
|---|---|
| LevelMeter par cue | VU-mètres master OK, par cue absent |
| Color tags | Champ `color` présent dans le modèle, UI non implémentée |
| Sous-composants Inspector séparés | Tous inlinés dans InspectorPanel.tsx |

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
| 8. Inspector panel | ✅ 4 onglets |
| 9. Workspace save/load | ✅ |
| 10. Keyboard shortcuts | ✅ |
| 11. Fades, waveform, level meters | ✅ |
| 12. Drag-drop, undo/redo, color tags | ⚠️ Tout ✅ sauf color tags |

---

## Prochaines priorités

1. **Routing Output Patch** dans `AudioEngine`
2. **Color tags** sur les cues (UI + persistance)
3. **LevelMeter par cue** autonome
4. **Refactoring** : extraire sous-composants Inspector en fichiers séparés
