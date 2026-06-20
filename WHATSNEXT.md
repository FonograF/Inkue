# WinCue — Roadmap

Mise à jour : 2026-06-20 (v0.9.2)

## Ce qui est fait

| Feature | Version |
|---|---|
| Audio, Video, Image, Group, Wait, Stop, Memo | 0.1–0.4 |
| OSC Send + Receive server | 0.6.0 |
| Fade Cue (volume fade sur cue en cours) | 0.6.3 |
| Cue disable (skip au GO, badge visuel) | 0.6.4 |
| Broken cue detection (fichier manquant) | 0.6.4 |
| MIDI Cue (Note On/Off, CC, Program Change) | 0.6.5 |
| Multiple Cue Lists (tabs, add/rename/delete, playhead indépendant par liste) | 0.7.0 |
| Cue Warnings (badge ⚠ jaune — no file assigned, durée zéro, groupe vide) | 0.7.1 |
| Image Display Duration (durée d'affichage optionnelle, auto-complete via mpv) | 0.7.1 |
| Cue List Notes column + bouton Stop par cue | 0.8.0 |
| Fade/Stop Cue multi-target UUID + fade visuel (Video/Image) | 0.8.0 |
| Audio/Video loop fini + infini (∞) | 0.8.0 |
| Output Mac/Linux + floating timer en WebView Tauri | 0.8.1 |
| Chemin de sortie unifié GL Render API (winit + mpv), legacy Win32 derrière un flag | 0.9.0–0.9.2 |
| Bouton Pause/Resume dans la transport bar | 0.9.2 |

---

## Priorité 1 — Fiabilité show (bloquant en production)

### ~~Multiple Cue Lists~~ ✅ v0.7.0

### ~~Cue Warnings~~ ✅ v0.7.1

### ~~Image Display Duration~~ ✅ v0.7.1

---

## Priorité 2 — Workflow opérateur

### Inline Editing dans la Cue List
**Pourquoi :** QLab permet d'éditer pre-wait, post-wait, durée directement dans la ligne — sans ouvrir l'inspecteur. Très utile pour les ajustements rapides en répétition.  
**Effort :** ~1–2 jours  
**Architecture :** Cellules `pre_wait` / `post_wait` / `duration` dans `CueRow` deviennent des `<input>` en double-clic. `onBlur` appelle `updateCue`. Pas de changement backend.

### Cart Mode
**Pourquoi :** Déclencher n'importe quel cue par clic direct, indépendamment du playhead. Indispensable pour le busking et les sons d'ambiance déclenchés à la demande.  
**Effort :** ~1 jour  
**Architecture :** Bouton toggle "Cart" dans la transport bar. Quand actif, le clic sur un cue dans la liste appelle `go_cue(id)` au lieu de déplacer la sélection. `go_cue` = nouveau Tauri command qui trigger directement un cue par ID sans bouger le playhead.

### Active Cues View
**Pourquoi :** Vue séparée listant tous les cues actuellement en cours. Critique en show complexe avec plusieurs cues parallèles.  
**Effort :** ~1 jour  
**Architecture :** Panel flottant ou section dans le layout. Le frontend filtre `cues.filter(c => c.state === "running" || c.state === "paused")`. Affiche nom, durée restante, barre de progression. Bouton stop individuel.

### Hotkeys par Cue
**Pourquoi :** Assigner une touche clavier (F1–F12, chiffres) à un cue spécifique pour le déclencher directement depuis le clavier.  
**Effort :** ~1 jour  
**Architecture :** Champ `hotkey: Option<String>` sur chaque cue (sérialisé). `useKeyboardShortcuts.ts` scanne les cues et intercepte les touches correspondantes. Appel `go_cue(id)`.

### Recherche / Filtre de Cues
**Pourquoi :** Sur une liste de 200+ cues, trouver rapidement un cue par nom ou numéro.  
**Effort :** ~0.5 jour  
**Architecture :** Champ de recherche en haut de la CueList. Filtre local sur `cues` en mémoire (pas de requête backend). Les cues cachés conservent leur état (ne disparaissent pas de l'exécution).

---

## Priorité 3 — Fonctionnalités avancées

### Input Patches (Mic / Live Audio)
**Pourquoi :** Router un micro ou une entrée ligne live à travers le moteur audio de WinCue, avec routing vers les Output Patches.  
**Effort :** ~3–4 jours  
**Architecture :** `cpal` supporte les entrées audio sur WASAPI et CoreAudio. `InputPatch` struct (miroir de `OutputPatch`). `AudioEngine` ouvre un stream d'entrée supplémentaire et route les samples vers le mix de sortie. Nouveau cue type `MicCue` ou activation via un toggle dans les Preferences.  
**Attention macOS :** Déjà cross-platform via `cpal`.

### Text Cue
**Pourquoi :** Afficher du texte formaté sur la surface de sortie (titres, surtitres, annonces). Courant dans la conférence et le théâtre.  
**Effort :** ~1–2 jours  
**Architecture :** Passer par l'OSD mpv (déjà utilisé pour le timer). `mpv.set_property("osd-msg1", text)` avec le bon `osd-font` et `osd-font-size`. Cross-platform natif. Champs : texte, police, taille, position, couleur, durée.

### Light Cue (DMX via Art-Net / sACN)
**Pourquoi :** Contrôle DMX vers une console lumière ou directement des luminaires. Concurrent direct de QLab dans les petites productions.  
**Effort :** ~3–5 jours  
**Architecture :** UDP Art-Net / sACN — networking pur, 100% cross-platform. Nouveau type `LightCue` avec universes et valeurs DMX. Pas de driver propriétaire — protocole réseau standard.

### MIDI File Cue
**Pourquoi :** Playback d'un fichier .mid via un port MIDI.  
**Effort :** ~2 jours  
**Architecture :** Lire le fichier MIDI avec `midly` ou similaire, reconstruire la timeline d'événements, les envoyer via `midir` dans un thread background avec timing. Le cue a une durée (longueur du fichier MIDI).

### Script Cue
**Pourquoi :** Exécuter une commande shell / script PowerShell au GO.  
**Effort :** ~1 jour  
**Architecture :** `std::process::Command`. Champs : commande, arguments, working directory, timeout. Exécution dans un thread background pour ne pas bloquer. Sortie loggée.

---

## Priorité 4 — Vidéo avancé

### Multiple Video Outputs
**Pourquoi :** Envoyer des vidéos différentes sur deux écrans simultanément.  
**Effort :** ~3–4 jours  
**Architecture :** Créer plusieurs instances `OutputEngine`, une par écran. Chaque `VideoCue` cible un output_engine_id. La fenêtre Win32 actuelle passe de singleton à pool. Complexité principale : la gestion des fades entre cues sur le même output.

### Video Transforms
**Pourquoi :** Redimensionner, positionner, faire pivoter la vidéo sur la surface.  
**Effort :** ~1–2 jours  
**Architecture :** Propriétés mpv : `video-zoom`, `video-pan-x`, `video-pan-y`, `video-rotate`. Champs dans `VideoCue` / `ImageCue`. Sliders dans l'inspecteur.

---

## Note sur la compatibilité macOS

La surface de portage macOS/Linux est **fixée et connue** — aucune des features ci-dessus n'agrandit le périmètre de portage (voir `PORTAGE.md` pour le détail). Depuis 0.9.0 le rendu passe par le chemin GL unifié (`vo=libmpv` + Render API) ; il ne reste que la **création de la fenêtre native** (Stage 2 : NSWindow/CGL sur macOS, GDK/EGL sur Linux). Toutes les features peuvent être développées sans compromis cross-platform.

Seule exception à surveiller : **Input Patches** — si on utilise des API WASAPI spécifiques plutôt que `cpal` générique. Rester sur `cpal` les garde cross-platform.
