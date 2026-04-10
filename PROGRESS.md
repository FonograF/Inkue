# WinCue — État du projet au 2026-04-10

## Résultat de cargo build

**Compile sans erreur, zéro warning.** (au 2026-04-10)

## Résultat de cargo test

**20 tests passent, 0 échec.** (au 2026-04-06 — à re-vérifier après les ajouts récents)

---

## Ce qui est implémenté et compile

### Backend Rust

| Module | Fichier | Statut |
|---|---|---|
| Types cue | `cue/types.rs` | ✅ Complet |
| Trait Cue | `cue/traits.rs` | ✅ Complet |
| CueRegistry | `cue/registry.rs` | ✅ Complet |
| CueContext | `cue/context.rs` | ✅ Complet |
| AudioCue | `cue/audio_cue.rs` | ✅ Complet — rate corrigé pour mismatch sample rate device/fichier |
| MemoCue | `cue/memo_cue.rs` | ✅ Complet |
| VoiceState / FadeState | `engine/voice.rs` | ✅ Complet |
| AudioCommand / AudioStatus | `engine/ring_command.rs` | ✅ Complet |
| DeviceManager / OutputPatch | `engine/device_manager.rs` | ✅ Complet |
| AudioEngine | `engine/audio_engine.rs` | ✅ Complet — ASIO device selection par nom, `BufferSize::Default` pour ASIO |
| CueList | `show/cue_list.rs` | ✅ Complet |
| Workspace | `show/workspace.rs` | ✅ Complet |
| Transport | `show/transport.rs` | ✅ Complet |
| Event Loop 30fps | `show/event_loop.rs` | ✅ Complet |
| AppState | `state/app_state.rs` | ✅ Complet |
| Preferences | `preferences.rs` | ✅ Complet |
| Commands transport | `commands/transport_cmds.rs` | ✅ Complet |
| Commands cues | `commands/cue_cmds.rs` | ✅ Complet — `preview_cue` corrigé (rate mismatch + timing IPC) |
| Commands workspace | `commands/workspace_cmds.rs` | ✅ Complet |
| Commands devices | `commands/device_cmds.rs` | ✅ Complet |
| Commands preferences | `commands/preferences_cmds.rs` | ✅ Complet — apply immédiat du output pair ASIO |

### Frontend React / TypeScript

| Fichier | Statut |
|---|---|
| `lib/types.ts` | ✅ Complet |
| `lib/commands.ts` | ✅ Complet |
| `stores/workspaceStore.ts` | ✅ Complet |
| `stores/transportStore.ts` | ✅ Complet |
| `stores/timingStore.ts` | ✅ Complet |
| `hooks/useTauriEvents.ts` | ✅ Complet |
| `hooks/useKeyboardShortcuts.ts` | ⚠️ Partiel |
| `App.tsx` | ✅ Complet |
| `components/CueList/columns.ts` | ✅ Complet — colonnes pixel uniquement (plus de `1fr`), resize/hide/reorder, persistance localStorage |
| `components/CueList/CueListView.tsx` | ✅ Complet — scroll-sync header/rows, `min-width: max-content`, menu clic-droit, drag-reorder, resize |
| `components/CueList/CueRow.tsx` | ✅ Complet — compatible avec `gridStyle` partagé |
| `components/CueList/PlayheadIndicator.tsx` | ✅ Complet |
| `components/Inspector/InspectorPanel.tsx` | ✅ 4 onglets |
| `components/Transport/TransportBar.tsx` | ⚠️ Partiel |
| `components/common/TimeDisplay.tsx` | ✅ Complet |
| `components/Preferences/PreferencesModal.tsx` | ✅ Complet — apply immédiat du output pair sans fermer la modale |
| `components/WaveformModal.tsx` | ✅ Complet — timing playhead corrigé (midpoint IPC) |

---

## Travail accompli lors de cette session (2026-04-10)

### 🔧 Corrections audio

**Fix critique — mismatch sample rate (bug systémique)**
- `Voice::new` initialisait toujours `rate = 1.0`, supposant `file_sr == device_sr`
- Un fichier 44100 Hz sur un device 48000 Hz jouait à 108,8% de vitesse (mauvaise tonalité + vitesse)
- Fix dans `AudioCue::start_audio_action` : `rate = self.rate × (file_sr / device_sr)`
- Fix dans `preview_cue` : `rate = file_sr / device_sr`
- Corrige à la fois la lecture normale et l'indicateur de progression du waveform

**Fix timing indicateur waveform preview**
- `wallStart` était capturé APRÈS `await previewCue()` → indicateur systématiquement en retard de la latence IPC complète
- Fix : `wallStart = t_before + roundTrip / 2` (point médian du round-trip = estimation du moment où l'audio démarre réellement)

**Fix ASIO**
- Sélection du device par nom avant fallback sur le device par défaut
- `BufferSize::Fixed` → `BufferSize::Default` pour ASIO (le driver gère sa propre taille)
- Apply immédiat du output pair ASIO sans fermer la modale préférences

**Optimisation décodage audio**
- Pré-allocation de `Vec<f32>` via `n_frames` des codec params (évite les réallocations)
- `shrink_to_fit()` après décodage
- Thread de préchargement en priorité `BELOW_NORMAL` (Windows) pour éviter les pics CPU/ventilateurs

### 🎨 Refonte du tableau de cues

**Colonnes redimensionnables, réarrangeables, masquables**
- Suppression des colonnes `color` et `trailing`
- Toutes les colonnes en pixels (fini le `1fr` qui causait l'écrasement des colonnes)
- `min-width: max-content` sur le grid → les colonnes ne se compriment plus quand la fenêtre est trop petite
- Scroll horizontal : header avec scrollbar cachée (`.no-scrollbar`) synchronisé via `onScroll` avec le container des rows
- Clic-droit sur le header → menu de visibilité des colonnes
- Drag sur le header → réordonnancement en live (seuil 6px, Escape pour annuler)
- Handle de resize à 8px centré sur la bordure de colonne (lecture depuis `colConfig.widths`, pas le DOM, pour éviter la dérive subpixel)
- `borderLeft` sur chaque cellule header non-première → séparateur visuel NAME↔TARGET
- Padding horizontal sur les labels pour ne pas coller aux séparateurs

**Fix bug drag direction**
- `order.splice(from < to ? to - 1 : to, 0, drag.id)` → `order.splice(to, 0, drag.id)`
- Après `splice(from, 1)`, l'index d'insertion est toujours `to` quelle que soit la direction

### 🎨 UI / Style

**Scrollbar dark theme**
- Scrollbar système Windows (gris clair sur fond sombre) → 6px, fond transparent, pouce `#334155`, hover `#475569`
- Déclaré globalement dans `index.html` pour couvrir toute l'app

### 📦 Build & distribution

**Installeur production**
- Build release : `opt-level = 3`, `lto = true`, `codegen-units = 1`
- Génère `WinCue_0.1.0_x64_en-US.msi` et `WinCue_0.1.0_x64-setup.exe`
- Frontend embarqué via `custom-protocol` (pas de fichiers externes)

**Scripts npm avec ASIO**
- `pnpm tauri:dev` → `tauri dev -- --features asio-support`
- `pnpm tauri:build` → `tauri build -- --features asio-support`
- `pnpm tauri dev` / `pnpm tauri build` restent disponibles sans ASIO

---

## Ce qui est partiellement implémenté ou manquant

### Backend — fonctionnalités manquantes

#### 1. Pre-Wait non respecté
`AudioCue::go()` démarre la lecture immédiatement. `pre_wait` est stocké mais ignoré.

#### 2. Fade-in non appliqué
`AudioCue.fade_in` est sérialisé mais n'est pas injecté dans le `Voice` lors du `go()`.

#### 3. Routing par Output Patch non implémenté
Tout l'audio sort sur le device par défaut. `OutputPatch` est stocké mais l'`AudioEngine` ne le consulte pas.

#### 4. `engine/mixer.rs` absent
La logique de mixage est intégrée dans `fill_buffer()`. Acceptable architecturalement.

### Frontend — fonctionnalités manquantes

| Manquant | Détail |
|---|---|
| Drag-drop reorder cues | CueListView sans DnD pour réordonner les cues |
| Ctrl+S / Ctrl+I (shortcuts) | Absents de useKeyboardShortcuts |
| G (goto cue number) | Non implémenté |
| Ctrl+Arrow Up/Down | Non implémenté |
| Ctrl+Z / Ctrl+Y (undo/redo) | Non implémenté |
| Ctrl+C / Ctrl+V (copy/paste) | Non implémenté |
| LevelMeter par cue | VU-mètres master OK, par cue absent |
| Sous-composants Inspector séparés | Tous inlinés dans InspectorPanel.tsx |

---

## État des étapes de développement (CLAUDE.md)

| Étape | Statut |
|---|---|
| 1. Scaffold Tauri + fenêtre | ✅ |
| 2. Cue trait + CueRegistry + MemoCue | ✅ |
| 3. AudioEngine WAV (cpal + symphonia) | ✅ |
| 4. AudioCue connectée à l'engine | ✅ — rate mismatch corrigé |
| 5. Frontend CueList + GO | ✅ |
| 6. Playhead + transport | ✅ |
| 7. Output Patches + DeviceManager | ⚠️ Modèle présent, routing audio non branché |
| 8. Inspector panel | ✅ 4 onglets |
| 9. Workspace save/load | ✅ |
| 10. Keyboard shortcuts | ⚠️ Partiel |
| 11. Fades, waveform, level meters | ⚠️ Fade-out OK ; fade-in absent ; waveform ✅ ; VU-mètres master ✅ |
| 12. Drag-drop, undo/redo, color tags | ❌ |

---

## Prochaines priorités

1. **Pre-Wait réel** dans `AudioCue::go()` (timer dans une tâche dédiée)
2. **Fade-in** appliqué à la construction du `Voice`
3. **Routing Output Patch** dans `AudioEngine`
4. **Drag-drop reorder** des cues dans CueListView
5. **Shortcuts manquants** : Ctrl+S, Ctrl+I, G, Ctrl+Arrow, Ctrl+Z/Y
6. **LevelMeter par cue** autonome
7. **Refactoring** : extraire sous-composants Inspector en fichiers séparés
