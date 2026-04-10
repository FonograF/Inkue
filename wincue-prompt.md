# Projet : WinCue — Show Control Application (QLab-like) pour Windows

## Objectif

Créer une application de show control fonctionnellement calquée sur QLab (macOS) mais tournant nativement sur Windows. L'application gère des Workspace contenant des Cue Lists, chaque cue list contenant des cues ordonnées. La première version implémente les Audio Cues, mais l'architecture DOIT permettre d'ajouter n'importe quel type de cue (MIDI, OSC, Video, Fade, Group, Wait/Delay, Memo, Network, Script...) sans modifier le code existant.

Le comportement, la terminologie et les raccourcis clavier doivent reproduire ceux de QLab aussi fidèlement que possible.

---

## Stack technique imposée

- Backend / Engine : Rust
- Audio : crate cpal (accès WASAPI/ASIO), crate symphonia (décodage WAV, MP3, FLAC, OGG)
- Communication lock-free avec le thread audio : crate ringbuf ou crossbeam
- UI : Tauri v2 + React + TypeScript
- State management frontend : Zustand
- Erreurs Rust : thiserror pour les types d'erreur, anyhow dans main
- Sérialisation : serde + serde_json
- IDs : uuid
- Build : Cargo + pnpm

---

## Concepts fondamentaux calqués sur QLab

### Workspace

Un Workspace est l'unité de sauvegarde (fichier .wincue). Il contient :
- Des métadonnées (nom, date de création, date de modification)
- Un ou plusieurs Cue Lists
- La configuration des sorties audio (device mapping)
- Les préférences du workspace (niveaux par défaut, comportement du GO, etc.)

### Cue List

Une Cue List est une séquence ordonnée de cues avec un Playhead (curseur de position). Le Playhead indique la prochaine cue qui sera déclenchée par un GO.

Comportement du Playhead (identique à QLab) :
- Après un GO, le Playhead avance automatiquement à la cue suivante
- Si la cue a un mode Auto-Continue, la cue suivante se déclenche automatiquement après le post-wait
- Si la cue a un mode Auto-Follow, la cue suivante se déclenche dès que la cue courante démarre (après le pre-wait)
- L'utilisateur peut repositionner le Playhead manuellement avec les flèches ou en cliquant

### Cue Number

Comme dans QLab, le numéro de cue est un STRING, pas un nombre. Exemples valides : "1", "1.5", "1.5.1", "A", "Intro", "2B". Le tri se fait par ordre d'insertion dans la liste, pas par valeur numérique du cue number. Le cue number est optionnel et sert de repère humain.

### Cue States

Chaque cue possède un état parmi : Standby (prête, pas en cours), Running (en cours d'exécution), Paused (en pause), Completed (terminée). L'état global Idle correspond à une cue en Standby qui n'a pas encore été jouée.

### Continue Modes (comme QLab)

- Do Not Continue : après cette cue, le playhead attend un GO manuel
- Auto-Continue : après le post-wait de cette cue, un GO automatique est envoyé à la cue suivante
- Auto-Follow : dès que cette cue démarre (après son pre-wait), un GO automatique est envoyé à la cue suivante

### Timing d'une cue (identique à QLab)

Séquence d'exécution quand GO est appelé :
1. Pre-Wait : délai avant que la cue ne démarre réellement (la cue est en état "running" mais l'action n'a pas commencé)
2. Action : l'action principale de la cue (pour audio : lecture du fichier)
3. Post-Wait : délai après le démarrage de l'action, après lequel le mode continue s'applique

Important : le Post-Wait commence en même temps que l'Action, pas après. C'est le comportement QLab.

---

## Architecture Core Rust — Le système de Cues

### Trait Cue

Définir un trait Rust qui sera le contrat pour TOUS les types de cues. Ce trait doit être object-safe (utilisable en dyn Cue). Voici les méthodes requises :

Identité :
- id() -> CueId (UUID)
- cue_type() -> CueType (enum : Audio, Memo, Group, Wait, Fade, ... extensible)
- name() -> &str
- set_name(&mut self, name: String)
- number() -> Option<&str> (cue number optionnel, c'est un string)
- set_number(&mut self, number: Option<String>)
- notes() -> &str (champ notes libre, comme dans QLab)
- color() -> CueColor (couleur de la ligne dans la cue list, comme QLab)

État :
- state() -> CueState
- is_running() -> bool
- is_paused() -> bool

Lifecycle :
- load(&mut self, context: &CueContext) -> Result<()> : pré-charge les ressources (chargement fichier audio en mémoire par ex.)
- go(&mut self, context: &CueContext) -> Result<()> : lance l'exécution
- stop(&mut self, context: &CueContext) -> Result<()> : arrête la cue (reset à standby)
- pause(&mut self, context: &CueContext) -> Result<()>
- resume(&mut self, context: &CueContext) -> Result<()>
- hard_stop(&mut self, context: &CueContext) -> Result<()> : stop immédiat sans fade out
- reset(&mut self) -> Result<()> : remet la cue en état initial

Timing :
- pre_wait() -> Duration
- set_pre_wait(&mut self, d: Duration)
- post_wait() -> Duration
- set_post_wait(&mut self, d: Duration)
- duration() -> Option<Duration> : durée de l'action (None si indéterminée)
- elapsed() -> Duration : temps écoulé depuis le GO
- action_elapsed() -> Duration : temps écoulé depuis le début de l'action (après pre-wait)

Continue :
- continue_mode() -> ContinueMode
- set_continue_mode(&mut self, mode: ContinueMode)

Sérialisation :
- serialize(&self) -> serde_json::Value
- Un constructeur associé (via CueFactory) : from_json(value: serde_json::Value) -> Result<Box<dyn Cue>>

### CueContext

Structure partagée passée aux cues lors de leur exécution. Elle contient :
- Une référence (non-mutable) à l'AudioEngine (pour demander une voix audio)
- Un sender de channel (crossbeam ou tokio mpsc) pour envoyer des événements au show engine (CueStarted, CueStopped, CueCompleted, TimeUpdate, etc.)
- Une référence au DeviceManager (pour résoudre les sorties audio)
- L'horloge globale du show

### CueRegistry (pattern factory)

Un registre qui associe chaque CueType à une CueFactory capable de :
- Créer une nouvelle instance vide du type
- Désérialiser une instance depuis du JSON

Cela permet d'ajouter un nouveau type de cue en une seule étape : implémenter le trait Cue et enregistrer la factory dans le registry. Aucune modification du transport, de la cue list, ou de l'UI principale n'est nécessaire.

---

## Audio Engine

### Architecture

L'AudioEngine est un composant autonome qui gère le playback audio. Il est découplé du système de cues : une AudioCue demande une voix à l'engine, l'engine la fournit.

Composants :

AudioEngine (structure principale)
  - DeviceManager : énumère les devices de sortie du système (WASAPI, ASIO), permet de sélectionner le device par défaut et de mapper des sorties nommées (comme les Output Patches de QLab)
  - VoicePool : pool de voix audio réutilisables. Chaque voix représente un flux audio actif (un fichier en lecture). Capacité minimum : 64 voix simultanées
  - Mixer : reçoit les samples de toutes les voix actives, applique les volumes, effectue la sommation, applique le master volume
  - AudioStream : le stream cpal qui tourne dans un thread dédié haute priorité

### Voice (voix audio)

Chaque voix contient :
- Le décodeur audio (symphonia) — les samples décodés sont dans un ring buffer
- Volume en dB (range -60 à +12, avec mute en dessous de -60)
- État de fade (FadeIn, FadeOut, None) avec durée, courbe (linéaire, S-curve, exponentielle), et progression
- Routing de sortie (device + canaux)
- État (Playing, Paused, Stopped, FadingOut)
- Position de lecture (pour seek/scrub et affichage)
- Pan (stéréo : -1.0 à +1.0)

### Contraintes temps réel du callback audio

Le callback cpal tourne dans un thread haute priorité. Les règles absolues :
- ZÉRO allocation mémoire (pas de Vec::push, pas de String, pas de Box::new)
- ZÉRO lock (pas de Mutex, pas de RwLock)
- ZÉRO I/O (pas de lecture disque, pas de log)
- Toute communication avec le thread audio se fait via des ring buffers lock-free (crate ringbuf)
- Les commandes (play, stop, volume change, fade) sont envoyées au thread audio via un ring buffer de commandes
- Les informations de retour (position, niveaux) sont envoyées depuis le thread audio via un ring buffer de status

### Gestion des fichiers audio

Les fichiers audio sont décodés et pré-bufferisés dans un thread de décodage séparé. Le flux est :
1. L'AudioCue demande le chargement d'un fichier
2. Un thread de décodage lit et décode le fichier via symphonia
3. Les samples décodés sont poussés dans un ring buffer
4. Le callback audio tire les samples depuis ce ring buffer

Pour les fichiers courts (< 30 secondes), on peut les charger entièrement en mémoire au moment du load().

### Fades

Reproduire le comportement des fades QLab :
- Fade in au début de la lecture (optionnel)
- Fade out à la fin (optionnel, peut être déclenché par un stop)
- Quand stop est appelé sur une cue audio en cours, appliquer un fade out court (par défaut 0.5s) avant d'arrêter, sauf si hard_stop est appelé
- Courbes disponibles : linéaire, S-curve (default, comme QLab), exponentielle

### Output Patches (comme QLab)

QLab utilise le concept de "Output Patch" : un mapping nommé vers un device audio et ses canaux. Implémenter le même concept :
- Output Patch 1 : "Main Speakers" -> Device WASAPI "Focusrite" canaux 1-2
- Output Patch 2 : "Monitors" -> Device WASAPI "Focusrite" canaux 3-4
- Etc.

Chaque Audio Cue référence un Output Patch, pas un device directement.

---

## AudioCue — Première implémentation de Cue

L'AudioCue est le premier type de cue implémenté. Propriétés spécifiques (en plus du trait Cue) :

- file_path : chemin vers le fichier audio (relatif au workspace)
- volume_db : volume en dB (-60 à +12)
- pan : panoramique stéréo (-1.0 à +1.0)
- fade_in : Option<FadeSpec> (durée + courbe)
- fade_out : Option<FadeSpec> (durée + courbe)
- start_time : Option<Duration> (commencer la lecture à un point précis dans le fichier)
- end_time : Option<Duration> (arrêter la lecture à un point précis)
- loop_count : u32 (0 = pas de boucle, 1+ = nombre de répétitions, u32::MAX = boucle infinie)
- output_patch : OutputPatchId
- rate : f64 (vitesse de lecture, 1.0 = normal, comme QLab)

---

## Show Engine / Transport

Le Show Engine est le chef d'orchestre. Il gère :

### Playhead et GO

Quand l'utilisateur appuie sur GO :
1. Récupérer la cue au Playhead
2. Avancer le Playhead à la cue suivante
3. Exécuter la séquence de timing de la cue (pre-wait -> action -> post-wait -> continue mode)
4. Si la cue a un Auto-Continue ou Auto-Follow, gérer le chaînage automatique

### Panic / Stop All

Comme QLab : un bouton "Stop All" (ou Escape) qui arrête TOUTES les cues en cours avec un fade out court. Un double-Escape fait un hard stop (coupure immédiate).

### Sélection vs Playhead

Comme dans QLab, la sélection (quelle cue est mise en surbrillance pour édition dans l'inspector) est indépendante du Playhead (quelle cue sera la prochaine à être jouée par GO). Le Playhead est affiché par un indicateur visuel distinct (triangle/flèche à gauche de la cue list).

---

## Interface utilisateur

### Layout principal (calqué sur QLab)

La fenêtre principale est divisée en zones :

Zone haute : Barre de titre du workspace avec le nom du show.

Zone principale gauche — CUE LIST :
- Tableau avec colonnes : Playhead indicator | Cue Number | Cue Name | Target (fichier) | Type icon | Pre-Wait | Action Duration | Post-Wait | Continue Mode | Color tag
- Le Playhead est indiqué par un triangle vert à gauche de la cue
- La cue sélectionnée est en surbrillance bleue
- Les cues en cours d'exécution ont un indicateur animé (barre de progression dans la ligne)
- Les cues sont colorables (bandeau de couleur à gauche de la ligne, comme QLab)
- Double-cliquer sur une cue ouvre l'inspector pour cette cue
- Glisser-déposer pour réordonner les cues

Zone principale droite — INSPECTOR :
Panneau contextuel qui affiche les propriétés de la cue sélectionnée. Le contenu change selon le type de cue.

Pour une AudioCue, l'inspector affiche :
- Onglet "Basics" : Cue number, cue name, notes, output patch, continue mode, color
- Onglet "Time & Loops" : Pre-wait, post-wait, start time, end time, loop count, rate
- Onglet "Levels" : Volume slider (dB), pan, waveform display avec marqueurs start/end
- Onglet "Fade" : Fade in/out specs avec visualisation de la courbe

Zone basse — TRANSPORT BAR :
- Boutons : GO (gros, vert, proéminent), STOP, PAUSE
- Affichage : Nom de la cue en cours, temps écoulé / durée totale, barre de progression
- Master volume slider
- Indicateur de device de sortie actif

### Raccourcis clavier (calqués sur QLab)

- Espace : GO (déclenche la cue au Playhead)
- Escape : Stop All (avec fade out). Double-Escape : Hard Stop All
- S : Stop la cue sélectionnée
- P ou [ : Pause la cue sélectionnée
- ] : Resume la cue sélectionnée (reprendre)
- Flèche Haut/Bas : Déplacer la sélection dans la cue list
- Ctrl+Flèche Haut/Bas : Déplacer le Playhead
- Ctrl+N : Ajouter une nouvelle Audio Cue
- Ctrl+S : Sauvegarder le workspace
- Ctrl+Z / Ctrl+Y : Undo / Redo
- Ctrl+C / Ctrl+V : Copier / Coller une cue
- Ctrl+D : Dupliquer la cue sélectionnée
- Delete : Supprimer la cue sélectionnée (avec confirmation)
- Ctrl+I : Ouvrir/fermer l'inspector
- G : Activer le champ GO TO (saisir un cue number pour y envoyer le playhead)

---

## Communication Frontend (React) <-> Backend (Rust/Tauri)

### Commands (Frontend -> Backend via Tauri invoke)

Nommage des commands :

Transport :
- go : déclenche un GO au playhead
- stop_all : arrête toutes les cues
- hard_stop_all : arrêt immédiat sans fade
- stop_cue(cue_id) : arrête une cue spécifique
- pause_cue(cue_id) : met en pause
- resume_cue(cue_id) : reprend

Cue management :
- add_cue(cue_type, position) : ajoute une cue
- remove_cue(cue_id) : supprime une cue
- move_cue(cue_id, new_position) : réordonne
- duplicate_cue(cue_id) : duplique
- update_cue(cue_id, properties_json) : met à jour les propriétés
- get_cue(cue_id) -> CueData : récupère les données complètes d'une cue
- get_all_cues() -> Vec<CueSummary> : récupère la liste résumée

Playhead :
- set_playhead(cue_id) : positionne le playhead
- get_playhead() -> Option<CueId>

Workspace :
- save_workspace(path) : sauvegarde
- load_workspace(path) : charge
- new_workspace() : nouveau workspace vide

Devices :
- list_output_devices() -> Vec<DeviceInfo>
- get_output_patches() -> Vec<OutputPatch>
- set_output_patch(patch_id, device_id, channels)

### Events (Backend -> Frontend via Tauri emit)

Les événements sont émis du backend vers le frontend pour maintenir l'UI à jour :

- cue-state-changed : { cue_id, old_state, new_state } — émis à chaque changement d'état
- cue-time-update : { cue_id, elapsed, action_elapsed, remaining } — émis à ~30fps pour les cues en cours
- playhead-moved : { cue_id } — émis quand le playhead change de position
- level-meter : { cue_id, peak_l, peak_r, rms_l, rms_r } — niveaux audio, ~30fps
- master-level : { peak_l, peak_r } — niveaux master
- workspace-modified : {} — le workspace a été modifié (pour afficher un indicateur "non sauvé")
- device-changed : { devices: Vec<DeviceInfo> } — un device audio a été branché/débranché

Les events time-update et level-meter sont throttlés à 30fps côté backend pour ne pas surcharger le frontend.

---

## Format de sauvegarde (.wincue)

Le fichier .wincue est un fichier JSON avec cette structure :

Racine :
- version : "1.0.0"
- workspace : objet contenant name (string), created_at (ISO 8601), modified_at (ISO 8601)
- output_patches : tableau d'objets, chaque objet contenant id, name, device_id, channels (tableau d'entiers)
- default_output_patch : id du patch par défaut
- cue_lists : tableau de cue lists

Cue List :
- id : UUID
- name : string
- playhead_cue_id : UUID ou null
- cues : tableau de cues

Cue (commun à tous les types) :
- type : string ("audio", "memo", "wait", "group", etc.)
- id : UUID
- number : string ou null
- name : string
- notes : string
- color : string (nom de couleur ou null)
- pre_wait_ms : nombre
- post_wait_ms : nombre
- continue_mode : "do_not_continue" | "auto_continue" | "auto_follow"

Cue Audio (propriétés additionnelles) :
- file_path : string (chemin relatif au dossier du .wincue)
- volume_db : nombre
- pan : nombre
- fade_in_ms : nombre ou null
- fade_in_curve : "linear" | "s_curve" | "exponential" ou null
- fade_out_ms : nombre ou null
- fade_out_curve : "linear" | "s_curve" | "exponential" ou null
- start_time_ms : nombre ou null
- end_time_ms : nombre ou null
- loop_count : nombre (0 = pas de boucle)
- output_patch_id : UUID
- rate : nombre (1.0 = normal)

Les chemins de fichiers audio sont TOUJOURS relatifs au dossier contenant le fichier .wincue.

---

## Structure du projet

Arborescence des fichiers à créer :

Racine : wincue/

Dossier src-tauri/src/ (Backend Rust) :
- main.rs : point d'entrée Tauri
- lib.rs : module racine
- engine/mod.rs : module audio engine
- engine/audio_engine.rs : structure AudioEngine principale
- engine/device_manager.rs : énumération et sélection des devices, output patches
- engine/mixer.rs : sommation des voix, master volume
- engine/voice.rs : structure Voice (décodeur, volume, fade, routing)
- engine/ring_command.rs : types de commandes envoyées au thread audio via ring buffer
- cue/mod.rs : module cues
- cue/traits.rs : trait Cue + trait CueFactory (object-safe)
- cue/registry.rs : CueRegistry (HashMap de factories)
- cue/types.rs : CueId, CueType, CueState, ContinueMode, CueColor, FadeSpec, FadeCurve
- cue/context.rs : CueContext
- cue/audio_cue.rs : implémentation AudioCue
- show/mod.rs : module show
- show/workspace.rs : structure Workspace (metadata + cue lists + output patches)
- show/cue_list.rs : structure CueList (cues ordonnées + playhead)
- show/transport.rs : logique GO, STOP, PAUSE, playhead management, continue mode chaining
- commands/mod.rs : module Tauri commands
- commands/transport_cmds.rs : commands GO, STOP, PAUSE
- commands/cue_cmds.rs : commands CRUD cues
- commands/workspace_cmds.rs : commands save/load/new
- commands/device_cmds.rs : commands devices et output patches
- state/app_state.rs : état global Tauri (Workspace + AudioEngine wrappés dans Arc/Mutex appropriés)

Dossier src-tauri/ :
- Cargo.toml : dépendances (tauri, serde, serde_json, uuid, cpal, symphonia, ringbuf, crossbeam-channel, thiserror, anyhow)
- tauri.conf.json : configuration Tauri v2

Dossier src/ (Frontend React + TypeScript) :
- App.tsx : layout principal
- components/CueList/CueListView.tsx : tableau de cues avec playhead et sélection
- components/CueList/CueRow.tsx : ligne individuelle de cue
- components/CueList/PlayheadIndicator.tsx : triangle vert du playhead
- components/Inspector/InspectorPanel.tsx : panneau inspector contextuel
- components/Inspector/AudioCueInspector.tsx : inspector spécifique audio (onglets Basics, Time, Levels, Fade)
- components/Inspector/BasicsTab.tsx : onglet propriétés de base
- components/Inspector/TimeTab.tsx : onglet timing
- components/Inspector/LevelsTab.tsx : onglet volume/pan avec waveform
- components/Inspector/FadeTab.tsx : onglet fades avec visualisation courbe
- components/Transport/TransportBar.tsx : barre GO/STOP/PAUSE + affichage temps
- components/Transport/GoButton.tsx : gros bouton GO vert
- components/Audio/VolumeSlider.tsx : slider de volume en dB
- components/Audio/LevelMeter.tsx : vu-mètre peak/RMS
- components/Audio/WaveformDisplay.tsx : affichage waveform du fichier
- components/common/TimeDisplay.tsx : affichage formaté MM:SS.ms
- components/common/ColorPicker.tsx : sélecteur de couleur de cue
- stores/workspaceStore.ts : Zustand store pour les données du workspace (cue list, sélection, playhead)
- stores/transportStore.ts : Zustand store pour l'état du transport (cue en cours, temps, niveaux)
- hooks/useTauriEvents.ts : hook pour écouter les events Tauri backend
- hooks/useKeyboardShortcuts.ts : hook pour les raccourcis clavier globaux
- lib/commands.ts : wrappers TypeScript typés autour des Tauri invoke commands
- lib/types.ts : types TypeScript (CueData, CueSummary, DeviceInfo, OutputPatch, etc.)

Racine :
- package.json
- tsconfig.json
- README.md

---

## Contraintes de développement

1. Commencer par le squelette fonctionnel : Tauri app qui démarre, trait Cue défini, AudioCue implémenté, playback d'un fichier WAV fonctionnel avec GO/STOP, cue list basique affichée dans le frontend. Tout doit compiler et fonctionner dès la première itération.

2. Pas de placeholder ni de stub : chaque fichier créé contient du code fonctionnel. Pas de "// TODO: implement later" sans au minimum une implémentation basique qui compile.

3. Gestion d'erreur rigoureuse : utiliser thiserror pour les types d'erreur, anyhow dans main. Interdit de mettre des .unwrap() sauf cas triviaux documentés avec un commentaire expliquant pourquoi c'est safe.

4. Tests unitaires : au minimum tester CueNumber (parsing, comparaison), CueRegistry (enregistrement et lookup), sérialisation/désérialisation d'une AudioCue, et les conversions dB <-> gain linéaire.

5. Documentation : chaque module public, chaque structure publique, et chaque méthode de trait avec des doc comments (///) en anglais.

6. Performance audio : le callback cpal ne doit JAMAIS bloquer. Zéro allocation, zéro mutex, zéro I/O dans ce callback. Toute communication passe par des ring buffers lock-free.

7. Extensibilité : pour vérifier que l'architecture est bien extensible, le type MemoCue (une cue qui ne fait rien d'autre qu'afficher un texte, comme dans QLab) doit également être implémenté. C'est le test minimal que le trait Cue est correct.

---

## Ce qu'il ne faut PAS faire

- Ne PAS utiliser la crate rodio : trop haut niveau, pas assez de contrôle sur le routing multicanal et les fades
- Ne PAS mélanger la logique show/transport avec la logique audio engine : ce sont deux couches distinctes
- Ne PAS mettre tout le backend dans un seul fichier : respecter la structure de fichiers décrite ci-dessus
- Ne PAS implémenter de Video, MIDI, OSC, ou DMX dans cette version : mais l'architecture doit rendre leur ajout trivial (juste un nouveau struct qui implémente Cue + une factory enregistrée)
- Ne PAS utiliser de mutex ou lock dans le thread audio
- Ne PAS hardcoder les devices audio : utiliser le DeviceManager et les Output Patches
- Ne PAS inventer une terminologie custom : utiliser les termes QLab (Workspace, Cue List, Playhead, GO, Pre-Wait, Post-Wait, Auto-Continue, Auto-Follow, Output Patch, etc.)
