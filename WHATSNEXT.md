# Inkue — Roadmap

Backlog des features à venir et de leur architecture *cible*. Pas d'estimation de durée.

- **Ce qui est fait** → `PROGRESS.md` (tables de statut, source de vérité).
- **Cross-platform** → `PORTAGE.md`.
- **DMX / Light** (M5 + Phase 2) → `LIGHT.md`.

---

## Priorité 1 — Workflow opérateur

### Inline Editing dans la Cue List
**Pourquoi :** QLab édite pre-wait / post-wait / durée directement dans la ligne — sans ouvrir l'inspecteur. Utile pour les ajustements rapides en répétition.
**Architecture :** les cellules `pre_wait` / `post_wait` / `duration` de `CueRow` deviennent des `<input>` au double-clic. `onBlur` appelle `updateCue`. Pas de changement backend.

### Cart Mode
**Pourquoi :** Déclencher n'importe quel cue par clic direct, indépendamment du playhead. Indispensable pour le busking et les sons d'ambiance déclenchés à la demande.
**Architecture :** Bouton toggle "Cart" dans la transport bar. Quand actif, le clic sur un cue appelle `go_cue(id)` au lieu de déplacer la sélection. `go_cue` = nouveau Tauri command qui trigger un cue par ID sans bouger le playhead.

### Active Cues View
**Pourquoi :** Vue séparée listant tous les cues en cours. Critique en show complexe avec plusieurs cues parallèles.
**Architecture :** Panel flottant ou section dans le layout. Le frontend filtre `cues.filter(c => c.state === "running" || c.state === "paused")`. Affiche nom, durée restante, barre de progression, stop individuel.

### Hotkeys par Cue
**Pourquoi :** Assigner une touche (F1–F12, chiffres) à un cue spécifique pour le déclencher au clavier.
**Architecture :** Champ `hotkey: Option<String>` sur chaque cue (sérialisé). `useKeyboardShortcuts.ts` scanne les cues et intercepte les touches correspondantes → `go_cue(id)`.

### Recherche / Filtre de Cues
**Pourquoi :** Sur une liste de 200+ cues, trouver rapidement par nom ou numéro.
**Architecture :** Champ de recherche en haut de la CueList. Filtre local sur `cues` en mémoire (pas de requête backend). Les cues cachés conservent leur état d'exécution.

---

## Priorité 2 — Fonctionnalités avancées

### Input Patches (Mic / Live Audio)
**Pourquoi :** Router un micro ou une entrée ligne live à travers le moteur audio, avec routing vers les Output Patches.
**Architecture :** `cpal` supporte les entrées (WASAPI / CoreAudio / ALSA). `InputPatch` struct (miroir de `OutputPatch`). `AudioEngine` ouvre un stream d'entrée et route les samples vers le mix de sortie. Nouveau cue type `MicCue` ou toggle dans les Preferences.
**Cross-platform :** rester sur `cpal` générique (pas d'API WASAPI spécifique) — voir `PORTAGE.md`.

### Text Cue
**Pourquoi :** Afficher du texte formaté sur la surface de sortie (titres, surtitres, annonces). Courant en conférence et théâtre.
**Architecture :** Passer par l'OSD mpv (déjà utilisé pour le timer). `mpv.set_property("osd-msg1", text)` avec `osd-font` / `osd-font-size`. Cross-platform natif. Champs : texte, police, taille, position, couleur, durée.

### MIDI File Cue
**Pourquoi :** Playback d'un fichier .mid via un port MIDI.
**Architecture :** Lire le fichier avec `midly`, reconstruire la timeline d'événements, les envoyer via `midir` dans un thread background avec timing. La cue a une durée (longueur du fichier MIDI).

### Script Cue
**Pourquoi :** Exécuter une commande shell / script au GO.
**Architecture :** `std::process::Command`. Champs : commande, arguments, working directory, timeout. Exécution dans un thread background pour ne pas bloquer. Sortie loggée.

---

## Priorité 3 — Vidéo avancé

### Multiple Video Outputs
**Pourquoi :** Envoyer des vidéos différentes sur deux écrans simultanément.
**Architecture :** Plusieurs instances `OutputEngine`, une par écran ; chaque `VideoCue` cible un `output_engine_id`. La fenêtre output passe de singleton à pool — le shim de création par-OS (winit / `NSWindow`) existe déjà. Complexité principale : les fades entre cues sur le même output.

### Video Transforms
**Pourquoi :** Redimensionner, positionner, faire pivoter la vidéo sur la surface.
**Architecture :** Propriétés mpv `video-zoom`, `video-pan-x/y`, `video-rotate`, ou (cible long terme) rendu dans un FBO offscreen + quad warpé dans `render.rs` (cf. note projection mapping de `PORTAGE.md`). Champs dans `VideoCue` / `ImageCue`, sliders dans l'inspecteur.

---

## Note cross-platform

Les 3 OS partagent le **chemin GL unifié** (`vo=libmpv` + Render API) : winit crée la
fenêtre sur Windows/Linux, un `NSWindow` objc2 sur macOS. Aucune feature ci-dessus
n'agrandit le périmètre de portage — tout le pipeline vidéo (décodage, rendu, transforms,
projection mapping, fades, multi-sortie) est un seul corpus GL partagé. Seul point de
vigilance : **Input Patches** doit rester sur `cpal` générique. Détail dans `PORTAGE.md`.
