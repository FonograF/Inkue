# WinCue — Timecode (design + plan)

Objectif : **parité QLab** sur le timecode — déclenchement de cues à une position (esclave)
**et** génération de timecode pour asservir d'autres machines (maître). Formats **MTC + LTC**.

État : **implémenté (v0.9.6)** — MTC receive (QF + SysEx + flywheel), MTC generate
(`TimecodeCue`), LTC encoder/decoder (`ltc.rs`), per-cue `TcTrigger` + `CueListTcConfig` +
dispatcher, commandes, frontend complet (Triggers tab, TC inspector, TcStatusIndicator,
TcPreferences). Détail dans `PROGRESS.md` (0.9.6).

**Caveat LTC OUT/IN** — infrastructure présente mais pas câblée end-to-end (v2) :
- LTC OUT requiert un voice audio dédié sur un Output Patch.
- LTC IN requiert le décodeur LTC branché sur l'audio input (Input Patches, fait).

Le design verrouillé ci-dessous reste la référence pour v2.

---

## Utilité

Un timecode est une horloge continue `HH:MM:SS:FF` à une cadence (24/25/29.97/30 fps),
diffusée pour que plusieurs machines partagent **une seule ligne de temps**. Il transforme une
liste de cues *pilotée à la main* en **show verrouillé sur une timeline**, répétable à la frame
près, et permet à WinCue de s'insérer dans un écosystème pro.

| Scénario | Sans TC | Avec TC |
|---|---|---|
| Vidéo + son + lumière sur un film/playback | GO « à peu près » | frame-accurate, identique à chaque représentation |
| Concert avec click/bande | dérive humaine | les cues tombent pile sur la musique |
| Corporate avec roll-ins vidéo | risque de décalage | serveur vidéo et WinCue sur la même horloge |
| Broadcast / parc / install permanente | impossible sans opérateur | show déterministe, lancé une fois |
| Piloter console lumière / DAW | déclenchements séparés | WinCue **maître** envoie le TC, tout suit |

---

## Modèle QLab (référence à calquer)

**① Entrée — déclencher des cues (esclave).** *Fire-and-play, PAS de chase/scrub* : quand le
TC entrant franchit l'heure d'une cue, elle part puis joue à son horloge interne.
- Activé **par Cue List** : *« Sync cues in this list from incoming timecode »* + source + format SMPTE.
- **Par cue** : onglet *Triggers* → case *Timecode* + heure, saisie **SMPTE** (`h:m:s:f`) **ou** *Real Time* (`h:m:s:ms`).
- **Freewheel window** (0–2 s) : ignore les micro-coupures du flux.
- Menus **On Start / On Stop** : comportement des cues quand le TC démarre/s'arrête.
- Formats d'entrée : **LTC** (audio) + **MTC** (MIDI). *Pas d'Art-Net.*

**② Sortie — générer du TC (maître).** Un **Timecode Cue** génère **MTC ou LTC**.
- Réglages : routing (patch MIDI pour MTC / patch audio + canal pour LTC), **frame rate**
  (vitesses vidéo & film), **start frame**, **end frame** optionnel (sinon tourne jusqu'au stop).
- **Plusieurs flux simultanés** indépendants. Se comporte comme une cue normale (durée → auto-stop).
- LTC cadencé sur l'horloge du **device audio** (zéro drift) ; MTC sur l'horloge **ordinateur** (peut driver).

---

## Décisions verrouillées

| Sujet | Décision |
|---|---|
| Modèle IN | **Déclenchement à la position (fire-and-play)** — pas de chase/lock, comme QLab. |
| Formats | **MTC + LTC** uniquement (pas d'Art-Net TC, pour coller à QLab). |
| Déclenchement | **Champ trigger générique sur le trait `Cue`** (n'importe quel type de cue devient TC-déclenchable sans toucher `transport.rs` — règle d'extensibilité) + toggle *sync to incoming TC* sur la `CueList`. |
| Saisie du trigger | **SMPTE** (`h:m:s:f`) **ou** *Real Time* (`h:m:s:ms`). |
| Génération | **`TimecodeCue`** (nouveau type, registry), MTC ou LTC, multi-flux, tourne jusqu'au stop. |
| Robustesse | **Freewheel** configurable + menus **On Start/Stop** + cadences (24/25/29.97/30) avec **drop-frame** (29.97 DF). |
| Couches | `engine/timecode.rs` ne connaît **rien** aux cues ; la couche show mappe position → triggers. |

---

## Architecture (respecte les 3 couches)

```
engine/timecode.rs (nouveau)           couche engine — ignore tout des cues
  TimecodeGenerator : MTC (midir) / LTC (encodeur → voice audio sur Output Patch)
  TimecodeReceiver  : MTC (midir input) / LTC (décodeur ← entrée audio, via Input Patches)
  thread dédié + flywheel (continue à travers les dropouts), pattern de dmx_engine.rs
        │  TcPosition { h, m, s, f, rate }   (event tauri `timecode` + snapshot UI)
        ▼
show/  dispatcher : à chaque position, fire les cues dont le trigger est franchi
       (garde monotone anti-rejeu ; politique de saut/seek ; On Start/Stop)
config : Timecode sur la CueList + champ trigger sur le trait Cue
cue/   TimecodeCue (génération) — registry, comme une boucle infinie
```

Réutilisations : `midir` (MIDI), Output Patches/voices (LTC out), Input Patches (LTC in),
pattern thread+snapshot+event de `dmx_engine.rs`, serveur UDP existant si besoin.

---

## Formats — coût dans l'archi WinCue

| Brique | Implémentation | Coût |
|---|---|---|
| MTC **OUT** (Timecode Cue) | réutilise `midir` (déjà là) | faible |
| MTC **IN** (trigger) | ajouter `midir::MidiInput` | faible |
| Triggers tab + toggle Cue List | nouveau concept *Triggers* sur le trait `Cue` | moyen |
| Dispatcher + freewheel + On Start/Stop + frame rates/drop-frame | `engine/timecode.rs` + dispatcher show | moyen |
| LTC **OUT** | encodeur biphase-mark → voice audio sur Output Patch (pas besoin d'entrée audio) | moyen |
| LTC **IN** | décodeur biphase-mark ← **entrée audio (Input Patches, inexistant)** | élevé / bloquant |

---

## Chaînage de dépendances

**Input Patches donne la *capture* audio — pas le décodage du timecode.** LTC est un signal
SMPTE encodé en biphase-mark (Manchester) dans l'audio :

```
Input Patches (entrée cpal)  →  décodeur LTC (DSP → frames SMPTE)  →  LTC IN
Output Patches (déjà là)     →  encodeur LTC (DSP → signal audio)  →  LTC OUT
```

- Input Patches (design complet dans **`INPUT.md`**) est **nécessaire mais pas suffisant** pour LTC IN : il faut aussi le décodeur.
- Encodeur/décodeur LTC = DSP **borné** (vérifier une crate Rust type `ltc` avant d'écrire le nôtre).
- LTC **OUT** ne dépend pas de l'entrée audio (génère vers un Output Patch) — pourrait précéder Input Patches.

---

## Ordre de build

1. **Input Patches + Mic Cue** — design verrouillé dans **`INPUT.md`** (stream d'entrée `cpal`
   persistant, `InputPatch` multicanal, devices in/out séparés + resampler adaptatif, buffer
   bas-latence configurable, routage via `Voice` → Output Patch). **Cross-platform via `cpal`
   générique** (règle `CLAUDE.md` / `PORTAGE.md`).
2. **Timecode, tout d'un bloc** → parité QLab totale :
   - `engine/timecode.rs` (gen + recv, thread, flywheel),
   - **MTC** in/out, **LTC** in/out (encodeur/décodeur + Input/Output Patches),
   - **Triggers** par cue + toggle *sync to incoming TC* sur la Cue List,
   - **TimecodeCue** (génération, multi-flux),
   - freewheel, On Start/Stop, frame rates + drop-frame.

Compromis assumé : aucun timecode tant qu'Input Patches n'est pas livré (arbitrage en faveur de
la complétude / parité, pas d'étape intermédiaire « MTC seul »).

---

## Sous-décisions recommandées *(à confirmer)*

1. **Périmètre d'Input Patches** : **feature complète** (routage micro/live + monitoring + VU
   + éventuel `MicCue`) — *reco*, car sur le roadmap et indépendamment utile. Alternative : un
   primitif de capture minimal pour aller plus vite au timecode.
2. **Triggers par cue** : **vrai onglet *Triggers* extensible** (timecode maintenant,
   hotkey/MIDI plus tard) — *reco*, calque QLab et **absorbe l'item *Hotkeys par cue*** du
   roadmap. Alternative : un champ timecode minimal.

---

## Tests (cargo test)

- Parsing/format SMPTE ↔ frames (24/25/29.97 DF/30), conversions Real-Time (ms) ↔ frames.
- **Drop-frame 29.97** : comptage correct (saut des frames 00 et 01 chaque minute sauf multiples de 10).
- Dispatcher : déclenche au franchissement, **ne rejoue pas** (garde monotone), gère un saut arrière (ré-arme) et un saut avant (politique définie).
- **Freewheel** : tient la position N ms après une coupure, puis On Stop.
- Encodeur LTC : layout biphase-mark + sync word `0011111111111101` (golden test sur quelques frames).
- Décodeur LTC : round-trip encodeur→décodeur sur un buffer audio synthétique.
- MTC : quarter-frame (8 messages = 2 frames) + full-frame, sens montant/descendant.
- Roundtrip serde : trigger de cue + `TimecodeCue` dans `.wincue`.

---

## Cross-platform

`midir` (MIDI in/out) et `cpal` (audio in/out) sont cross-platform — rester sur les API
**génériques** (pas de WinMM/WASAPI spécifique). Aucun morceau du timecode n'agrandit le
périmètre de portage. Détail dans `PORTAGE.md`.

---

## Sources

- [Using Timecode with QLab — QLab 5](https://qlab.app/docs/v5/networking/using-timecode/)
- [Timecode Cues — QLab 5](https://qlab.app/docs/v5/networking/timecode-cues/)
- [The Timecode Status Window — QLab 5](https://qlab.app/docs/v5/tools/timecode-status-window/)
