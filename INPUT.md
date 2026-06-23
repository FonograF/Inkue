# WinCue — Input Patches + Mic Cue (design)

Objectif : **entrée audio live** façon QLab Mic Cue — renfort micro, paging/annonces, routing
multicanal de sources live — **et** prérequis de capture pour le **LTC entrant** du timecode
(voir `TIMECODE.md`).

État : **design verrouillé, non implémenté.** WinCue a la **sortie** audio (cpal, Output
Patches, voices, fades, pan, VU) mais **aucune entrée** (pas de stream cpal input). C'est le
seul vrai code neuf ; tout le reste se réutilise.

Cross-platform impératif : **cpal générique** (WASAPI/ASIO Windows, CoreAudio macOS,
ALSA/PipeWire Linux) — pas d'API par-OS spécifique (règle `CLAUDE.md` / `PORTAGE.md`).

---

## Utilité (cas QLab)

Dans QLab, l'entrée audio sert à trois choses : **Mic Cue**, **Camera Cue**, et **LTC in**.
Le cas principal et autonome est la **Mic Cue** :

- **Renfort micro** live (théâtre, événement),
- **Paging / annonces** dans la sono,
- **Traitement live** d'un micro (réservé — voir caveat effets),
- **Routing multicanal** : plusieurs sources → destinations différentes en une cue.

La Mic Cue tourne **jusqu'au stop** (ou durée finie), avec fades in/out et niveaux — « le même
cueing, routing et fades que les Audio Cues ».

---

## Décisions verrouillées

| Sujet | Décision |
|---|---|
| Devices in/out | **Séparés autorisés** (in et out sur des devices/horloges différents) + **resampler adaptatif** qui compense le drift. Le « même device » est le cas dégénéré ratio≈1 → resampler transparent, latence plancher. Un seul chemin de code. |
| Cycle du stream | **Persistant** : le stream d'entrée tourne dès qu'un Input Patch existe → entrée toujours « chaude », **GO instantané**, zéro glitch de démarrage. La Mic Cue ne fait qu'ouvrir/fermer le robinet vers le mix. |
| Latence | **Buffer configurable** (machine-config par-OS) + **backends bas-latence** (ASIO Windows / CoreAudio macOS / PipeWire ou JACK Linux ; WASAPI shared en repli). Latence round-trip **mesurée et affichée**. |
| Canaux | **Multicanal arbitraire** : l'Input Patch expose tous les canaux du device ; une Mic Cue choisit N canaux (mono / paire / multi) → routés via la **matrice Output Patch existante**. |
| Chemin audio | L'entrée passe par une **`Voice`** → mix → Output Patch → hérite **gratuitement** des fades, du pan, du VU et du routing. |
| Type de cue | **`MicCue`** dédié (registry), pas un flag sur AudioCue. |
| Couches | `engine/` gère la capture ; `cue/mic_cue.rs` la cue ; `show/` **inchangé** (règle d'extensibilité). |

---

## Architecture (respecte les 3 couches)

```
engine/audio_engine.rs            couche engine
  + stream cpal INPUT par Input Patch (persistant) → ring buffer lock-free (in→mix)
  + resampler adaptatif : pilote le ratio de fill_buffer selon le niveau de remplissage
    du ring (clocks in/out divergent lentement → on recentre) ; ratio≈1 = no-op
  InputPatch { device_id, channels: Vec<u32> }   (miroir d'OutputPatch)
        │  une MicVoice lit le ring au lieu de samples décodés
        ▼
cue/mic_cue.rs                    couche cue
  MicCue : input_patch + canaux choisis, output_patch, volume/pan, fade in/out ;
           go() ouvre le robinet (crée la/les voice(s) alimentées par le ring) ;
           duration()=None (tourne jusqu'au stop) ; stop()=fade court ; registry.
machine_config.rs : device d'entrée + taille de buffer/latence, par-OS (comme l'audio out)
```

- **Réutilise** : `Voice` + routing Output Patch + `fill_buffer` (sa conversion SR sert de
  resampler — on rend son ratio **adaptatif** au lieu de fixe), fades, pan, VU, contraintes RT
  du callback (zéro alloc/lock/IO, comms par ring buffers).
- **Drift** : le ring entre le callback d'entrée et le callback de sortie absorbe la gigue ;
  son niveau de remplissage pilote un micro-ajustement du ratio de resampling pour rester centré
  (l'algo de sync de QLab). Pas de XRun tant que le buffer-pont ≥ 1–2 périodes.

---

## Latence

Contributions (cpal = 2 streams séparés, pont par ring) :

```
latence_round_trip ≈ buffer_in + pont(1–2 périodes) + buffer_out
```

Leviers : taille de buffer réglable + backend bas-latence ; stream persistant (pas de
cold-start au GO) ; pont minimal. On **mesure et affiche** la latence effective. Sur Windows,
**ASIO** (déjà supporté) est le chemin bas-latence ; CoreAudio (macOS) et PipeWire/JACK (Linux)
le sont nativement.

---

## Caveat : pas de rack d'effets

QLab applique reverb/EQ (AudioUnits) sur le mic. WinCue **n'a pas de rack d'effets DSP audio** —
une Mic Cue fera **routing + niveau + fade + pan**, **pas** reverb/EQ. C'est un chantier séparé
(DSP audio ; sans rapport avec le moteur d'effets DMX de `LIGHT.md` malgré le nom).

---

## Lien avec le timecode

Le **LTC IN** (`TIMECODE.md`) réutilise cette capture d'entrée + un **décodeur LTC** (DSP
biphase-mark, brique séparée). Input Patches est **nécessaire mais pas suffisant** pour LTC IN.

---

## Ordre de build

1. **Input Patches + Mic Cue** (ce document) — livrable autonome et utile (renfort/paging/routing).
2. **Timecode** (`TIMECODE.md`) — réutilise l'entrée pour LTC IN.

---

## Tests (cargo test)

- `InputPatch` : sérialisation roundtrip ; sélection de canaux (mono / paire / multi).
- Resampler adaptatif : convergence du niveau de ring vers la consigne quand in_sr ≠ out_sr
  (drift simulé) ; ratio≈1 quand in_sr = out_sr (no-op).
- MicVoice : routing vers Output Patch, fade in/out, pan, VU — via `fill_buffer` sur un ring
  synthétique (pas de device réel), comme les tests SR existants.
- MicCue : `duration()=None`, stop = fade court, roundtrip serde dans `.wincue`.
- Pas de glitch : robinet ouvert/fermé ne produit ni clic ni discontinuité (fade aux bornes).

---

## Cross-platform

`cpal` input est cross-platform — rester sur l'API **générique** (host par défaut + ASIO opt-in
sur Windows). Aucune partie n'agrandit le périmètre de portage. Détail dans `PORTAGE.md`.

---

## Sources

- [Mic Cues — QLab 5](https://qlab.app/docs/v5/audio/mic-cues/)
- [Introduction to Audio — QLab 5](https://qlab.app/docs/v5/audio/introduction-to-audio/)
- [Audio Output Patch Editor — QLab 5](https://qlab.app/docs/v5/audio/audio-output-patch-editor/)
