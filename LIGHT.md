# Inkue — Light Cue / contrôleur DMX

Objectif : faire de Inkue un **contrôleur lumière complet** (pas seulement un
déclencheur de console externe) — sortie DMX-over-IP directe vers des projecteurs,
avec un modèle de fixtures et des fades temporisés.

État : **M1 → M4 faits et testés** — moteur DMX, patch de fixtures (workspace),
et **Light Cue** complet (look = targets + fade, tracking/LTP). Reste M5
(NIC en machine-config, prefs réseau) + Phase 2. Branche : `light-cue`.

---

## Décisions de conception (verrouillées)

| Sujet | Décision | Pourquoi |
|---|---|---|
| Modèle d'état | **Fixtures + tracking LTP** | Un Light Cue ne stocke que les params qu'il change ; les canaux non cités gardent leur valeur (tracking) ; le dernier fade sur un canal gagne (LTP). Modèle des consoles / QLab. |
| Protocole | **sACN (E1.31) + Art-Net** | Les deux sont du pur UDP ; supporter les deux couvre tout le matériel / visualiseur. sACN primaire (standardisé, multicast, priorité). |
| Périmètre v1 | **Core complet** (patch, looks, fades, blackout, moniteur) ; **effets/chases = phase 2** | Le moteur d'effets est un sous-système séparé, pas requis pour un contrôleur exploitable. |
| Config | **Hybride** — patch fixtures + mapping univers→destination dans le **workspace** ; NIC/interface source en **machine-config** | Le show est portable ; l'adaptateur réseau dépend de la machine (comme le device audio). |

---

## Architecture (respecte les 3 couches)

```
cue/light_cue.rs   (M4 ✅)  LightCue : look = [ParamTarget] + FadeSpec ; go() soumet
                          des fades au DmxEngine via CueContext. Trait Cue → aucune
                          modif de transport.rs / cue_list.rs / components/CueList.
        │ submit_fade(universe, channel, width, target, dur, curve)
        ▼
engine/dmx_engine.rs (M1/M2 ✅)  DmxEngine : buffers d'univers, interpolation des
                          fades (LTP + tracking + 8/16-bit), blackout, thread ~40 Hz.
                          Ignore tout des cues (comme AudioEngine / OutputEngine).
        │ [u8; 512] par univers
        ▼
engine/dmx_sink.rs   (M1 ✅)  Encodeurs sACN + Art-Net + sink UDP.
```

- **Patch fixtures** (M3 ✅) → `fixtures` + `universe_outputs` sérialisés dans le `.inkue`, comme les patches OSC.
- **`CueContext`** (M4 ✅) porte `dmx_engine: Arc<DmxEngine>` + `fixtures`, à côté de `audio_engine` / `output_engine`.
- **Réutilise** `FadeCurve` (`engine::ring_command`) pour les courbes de fade (conversion depuis `cue::types::FadeCurve` au bord, comme `audio_cue`).

---

## Modèle de données

**Déjà implémenté** (`engine/dmx_sink.rs`, `engine/dmx_engine.rs`) :

```rust
enum OutputProtocol { Sacn, ArtNet }
struct UniverseOutput { universe: u16, protocol: OutputProtocol,
                        destination: Option<IpAddr>, enabled: bool } // None = multicast sACN
enum ChannelWidth { Bit8, Bit16 }          // 16-bit = 2 canaux coarse+fine
```

**Implémenté** (M3, `engine/fixture.rs`, sérialisé dans le workspace) — le
`FixtureType` est **embarqué** dans chaque `PatchedFixture` (pas de `type_ref` à
résoudre → workspace auto-suffisant ; les modèles intégrés ne sont qu'un point
de départ à la création) :

```rust
enum ParamKind { Intensity, Red, Green, Blue, White, Amber, Uv, Pan, Tilt, Generic }
FixtureParam { kind: ParamKind, name, channel_offset, width: ChannelWidth, default }
FixtureType  { name, parameters: [FixtureParam] }
PatchedFixture { id, label, universe, base_address, fixture_type: FixtureType }
```

**Implémenté** (M4 + groupes, `cue/light_cue.rs`) — une target vise **soit** un
paramètre d'une fixture (par index, stable/non ambigu), **soit** un groupe par
**type-de-param** (résolu vers tous les membres au GO). Enum tagué `kind` ;
rétro-compat de l'ancien format plat à la désérialisation.

```rust
FixtureGroup { id, label, fixture_ids: [Uuid] }     // workspace
enum ParamTarget {
    Fixture { fixture_id: String, param_index: usize, value: 0..1 },
    Group   { group_id: String, param_kind: ParamKind, value: 0..1 },
}
LightCue { …champs de cue…, targets: [ParamTarget], fade: FadeSpec }
```

---

## Formats sur le fil (implémentés, golden-testés)

| | sACN (E1.31) | Art-Net (ArtDMX) |
|---|---|---|
| Port UDP | 5568 | 6454 |
| Taille paquet | 638 octets (root 38 + framing 77 + DMP 523) | 530 octets (header 18 + 512 data) |
| Destination défaut | multicast `239.255.{hi}.{lo}` (par univers) | unicast/broadcast explicite requis |
| Priorité | champ priorité (défaut 100) | — |
| Identité source | CID 16 octets (UUID au démarrage) | — |

Émission : thread `inkue-dmx` à **~40 Hz**, send-on-change + **keepalive 800 ms**, n° de séquence par univers.

---

## État d'implémentation

| Jalon | Statut | Contenu |
|---|---|---|
| **M1 — le fil** | ✅ | DmxEngine (buffers + thread 40 Hz) + encodeurs sACN/Art-Net + commande « set channel » + moniteur intégré (event `dmx-monitor`) + panneau DMX dans la transport bar |
| **M2 — moteur de fade** | ✅ | `DmxState` : interpolation, **LTP** (repart de la valeur courante), **tracking**, **16-bit**, blackout — 7 tests unitaires |
| **M3 — patch fixtures** | ✅ | `FixtureType` (6 modèles intégrés) + `PatchedFixture` **embarqué** (workspace) + section Fixtures dans le panneau DMX + identify (test-fixture) + warnings de chevauchement d'adresses |
| **M4 — Light Cue** | ✅ | `LightCue` (`targets: [ParamTarget]` + `FadeSpec`) + `CueContext.dmx_engine`/`fixtures` + inspector `LightTab` + bouton `+ Light` + roundtrip sérialisation |
| **M5 — finition** | ⬜ | NIC en machine-config (`socket2`), UI préférences réseau, cadence keepalive, validation visuelle |
| **Dashboard + Record** | ✅ | Grille live (intensité + color picker par fixture) qui pilote le moteur en direct + **« Capture live state »** dans la Light Cue (workflow QLab : sculpter à l'œil → figer en cue) |
| **Groupes de fixtures** | ✅ | `FixtureGroup` (workspace) + gestionnaire de groupes (panneau DMX) ; une target peut viser **un groupe par type-de-param** → un color picker / une intensité pilote tous les membres (résolu au GO). Cue compacte : 1 couleur de groupe = 3 targets, pas 3×N |
| **Phase 2** | ⬜ | **Moteur d'effets (oscillateurs/chases) — design verrouillé, voir §Phase 2 ci-dessous** ; import bibliothèque de fixtures (OFL / QLC+), fade par-param, palettes, merge multi-source (HTP/priorité), master dimmer, DMX-in (capture depuis console externe) |

### Détail de ce qui est fait

**Backend**
- `engine/dmx_sink.rs` — `encode_sacn` / `encode_artnet` (fonctions pures, golden-testées au niveau octet), `DmxSink` (socket UDP), `UniverseOutput`, `sacn_multicast_ip`.
- `engine/dmx_engine.rs` — `DmxState` (pur, testable) + `DmxEngine` (handle + thread). Commandes : `submit_fade`, `set_channel`, `set_blackout`, `set_outputs`, `snapshot`.
- `engine/mod.rs` — exporte `DmxEngine`.
- `state/app_state.rs` — champ `dmx_engine: Arc<DmxEngine>` + `LightCueFactory` enregistrée.
- `lib.rs` — création au démarrage + thread `inkue-dmx-monitor` qui émet l'event `dmx-monitor` (~20 fps, on-change, jamais de polling) + `dmx_engine` passé à l'event loop.
- `engine/fixture.rs` (M3) — `ParamKind`, `FixtureParam`, `FixtureType`, `PatchedFixture` (type **embarqué** → workspace auto-suffisant), `builtin_fixture_types()` (Dimmer, RGB, RGBW, RGBA, PAR Dimmer+RGB, tête mobile 16-bit), `footprint()`, `resolve_channel()` (1-based → 0-based), `find_conflicts()`. 5 tests.
- `cue/light_cue.rs` (M4) — `LightCue` + `ParamTarget` ; `go()` résout `fixture → (universe, canal, width)` et soumet un fade par target au `DmxEngine` ; `duration()` = temps du fade (progress + auto-continue) ; stop = tracking (ne touche pas aux lumières). `fixture_id` stocké en `String` (placeholder vide toléré → une target non configurée ne casse pas la désérialisation de toute la liste). Factory + roundtrip. 3 tests.
- `cue/context.rs` (M4) — `dmx_engine: Arc<DmxEngine>` + `fixtures: Arc<Vec<PatchedFixture>>` + `resolve_fixture()`. Câblé dans `transport_cmds` et `event_loop`.
- `show/workspace.rs` (M3) — `universe_outputs` **et** `fixtures` sérialisés dans le `.inkue` ; `load_workspace`/`new_workspace` repoussent les sorties vers le moteur (source de vérité = workspace, plus localStorage).
- `commands/light_cmds.rs` — DMX : `dmx_set_outputs` (persiste au workspace), `dmx_get_outputs`, `dmx_set_channel`, `dmx_set_blackout`, `dmx_get_blackout`, `dmx_get_snapshot`. Fixtures : `list_builtin_fixture_types`, `list_fixtures`, `add_fixture`, `update_fixture`, `remove_fixture`, `get_fixture_conflicts`, `dmx_test_fixture` (identify). **Dashboard** : `dmx_set_fixture_param` (set live width-aware), `dmx_clear_fixtures`, `capture_live_targets` (lit le snapshot moteur → renvoie les targets ; ne mute pas la cue — le front applique via `update_cue`).

**Frontend**
- `lib/types.ts` — `OutputProtocol`, `UniverseOutput`, `DmxUniverseSnapshot`, `ChannelWidth`, `ParamKind`, `FixtureParam`, `FixtureType`, `PatchedFixture`, `FixtureConflict`, `ParamTarget`, `LightCueData`.
- `lib/commands.ts` — wrappers DMX + fixtures (`dmxGetOutputs`, `listBuiltinFixtureTypes`, `listFixtures`, `addFixture`, `updateFixture`, `removeFixture`, `getFixtureConflicts`, `dmxTestFixture`).
- `components/Lighting/LightingPanel.tsx` — popover : sorties (workspace-backed), **section Fixtures**, poke de canal, blackout, moniteur live.
- `components/Lighting/FixturePatch.tsx` — patch des projecteurs : ajouter (modèle + univers + adresse, auto-incrément), éditer label/univers/adresse, identify (◉), retirer, warnings de chevauchement.
- `components/Lighting/FixtureDashboard.tsx` — **Dashboard live façon QLab** : une ligne par fixture (slider intensité + **color picker** RGB + sliders des autres params) qui pilote le `DmxEngine` en direct ; boutons `↻ Live` (reseed depuis le moteur) / `Clear`. Lit l'état courant via le snapshot `dmx-monitor`, écrit via `dmx_set_fixture_param`.
- `components/Inspector/LightTab.tsx` — temps + courbe du fade, **bouton « ⏺ Capture live state »** (fige l'état live de toutes les fixtures dans la cue via `capture_live_targets` → `update_cue`, un seul write/undo). **Groupé par fixture** : une carte par projecteur (color picker RGB + slider intensité + sliders des autres params), pas une ligne par canal — N spots = N cartes au lieu de 3N lignes. Ajout/retrait de fixture dans la cue. Helpers couleur partagés : `lib/fixtureColor.ts`. Labels de fixtures uniques par défaut (`RGB 1`, `RGB 2`…).
- `components/Inspector/InspectorPanel.tsx` — onglet **Light** + icône 💡. `App.tsx` — bouton **+ Light** (+ drag). `CueRow.tsx` — icône 💡.
- `components/Transport/TransportBar.tsx` — bouton **DMX** qui ouvre le panneau.

**Groupes** : `engine/fixture.rs` `FixtureGroup` ; `show/workspace.rs` `fixture_groups` (sérialisé) ; `cue/context.rs` `fixture_groups` + `resolve_group` (câblé transport + event loop) ; `commands/light_cmds.rs` `list/add/update/remove_fixture_group` ; front `components/Lighting/GroupManager.tsx` (créer/éditer/supprimer, membres par chips) + `LightTab` (cartes groupe : color picker + intensité → tous les membres) + `lib/fixtureColor.ts`.

**Tests** : 10 nouveaux (5 fixtures + 5 Light Cue, dont groupe + rétro-compat). **99 au total**, clippy clean, `tsc --noEmit` clean. Lancé via `pnpm tauri dev` (audio WASAPI, libmpv, GL 3.3, OSC OK).

---

## Tester en soft (aucun matériel)

Tout se valide sur une seule machine en loopback :

1. `pnpm tauri dev`
2. Bouton **DMX** dans la transport bar → ajouter un univers (sACN multicast, ou Art-Net vers `127.0.0.1`).
3. Lancer un récepteur : **QLC+** (entrée Art-Net/sACN + moniteur de fixtures), **sACNView**, **OLA** (`ola_dmxmonitor`), ou **Capture** (démo, prévisualisation 3D).
4. Bouger le slider « Test channel » → la valeur monte dans le récepteur **et** dans le moniteur live du panneau. **Blackout** force tout à zéro.
5. **Patch + Light Cue** : panneau DMX → section *Fixtures* → ajouter un *PAR Dimmer+RGB* @ U1 adr 1 ; **identify (◉)** doit l'allumer.
6. **Dashboard + Capture (workflow QLab)** : panneau DMX → section *Dashboard* → règle l'intensité + la couleur de la fixture à l'œil (visible en direct dans le récepteur). Puis **+ Light** → onglet *Light* → **⏺ Capture live state** : la cue mémorise l'état courant. Règle le fade → **GO** : le fade monte vers le look capturé. (Saisie manuelle des targets toujours possible en dessous.)

Le hardware (un node Art-Net USB + un PAR LED) n'est utile que pour la vérif « vrai projecteur » finale — jamais pour développer.

---

## Phase 2 — Moteur d'effets (oscillateurs / chases) — design verrouillé

Objectif : variation lumineuse **automatique et continue** (sinus d'intensité, vagues,
chases, twinkle, ballyhoo) — ce que les Light Cue de QLab ne savent PAS faire (QLab se
limite aux courbes de fade ondulées + chaînage de cues). C'est là que Inkue dépasse QLab
plutôt que de le copier.

### Insight central : un effet n'est pas un fade

Un fade est une transition one-shot A→B puis la valeur tracke (écrite dans le buffer base).
Un effet est une oscillation **continue** : l'écrire dans le buffer empoisonnerait le
tracking (un Light Cue LTP suivant repartirait d'une valeur qui gigote). Donc un effet est
une **couche non-destructive composée au render** :

```
sortie_fil = clamp( base (fades / Light Cues)  +  Σ offsets_effets(t) ,  0, 1 )
```

`DmxState` gagne `effects: Vec<ActiveEffect>` à côté de `fades`. `tick()` garde la base
inchangée ; **`rendered()`** superpose les effets actifs à l'instant `t`. **La base n'est
jamais mutée** → tracking / LTP intacts.

### Décisions verrouillées

| Sujet | Décision |
|---|---|
| Modèle de cue | **Les deux** : `EffectCue` autonome **et** `EffectSpec` optionnel sur une target de Light Cue. Un seul type `EffectSpec` partagé. |
| Horloge | **Globale libre** (alimentée par le thread DMX 40 Hz) + offset/phase par effet. Deux effets de même BPM restent en phase ; ouvre la porte à un master de vitesse + tap-tempo. |
| Combinaison | **Additive unique** : `out = clamp(base + size·(wave − centre))`. Centre = biais : 0.5 = bipolaire (autour du look), 0 = unipolaire-haut, 1 = unipolaire-bas. L'« absolu » min→max = un look qui pose base=min + effet unipolaire size=max−min. Un seul code path, zéro masquage. |
| Empilement | **Somme** : tous les effets actifs s'additionnent en offsets, puis clamp. Superposition volontaire possible (swell lent + twinkle rapide). Chaque instance keyée par cue propriétaire → stop de l'un sans toucher l'autre. |
| Cible | **Réutilise `FixtureGroup` + le modèle `ParamTarget`** (Fixture ou Group + param kind). Le **spread de phase sur les membres du groupe** transforme l'oscillateur en chase / vague. |
| Démarrage doux | `size` rampe 0→max sur `fade_in` → pas de « pop » même si l'horloge globale est en plein milieu du swing au GO. |

### Modèle de données (`engine/dmx_effect.rs`, nouveau)

```rust
enum Waveform { Sine, Triangle, SawUp, SawDown, Square, Random }  // Square→duty, Random→twinkle
enum Direction { Forward, Backward, Bounce }                      // ordre du chase sur les membres

struct EffectSpec {
    waveform: Waveform,
    rate_bpm: f64,        // cycles/min, lus sur l'horloge globale
    size: f64,            // amplitude 0..1 (pleine échelle)
    center: f64,          // biais 0..1 (0.5 bipolaire, 0 unipolaire-haut, 1 unipolaire-bas)
    phase_offset: f64,    // 0..1 tours
    duty: f64,            // carré seulement
    fade_in: Duration,    // size 0→max au démarrage
    target: EffectTarget, // Fixture{id,param_index} | Group{group_id,param_kind}
    spread: f64,          // tours de phase répartis sur les membres (0 = unisson)
    direction: Direction,
}
```

`wave(phase) → [0,1]` (sin = (sin 2πφ + 1)/2, etc.). `offset = size·(wave − center)` →
centre 0.5 ⇒ ±size/2, centre 0 ⇒ [0,+size], centre 1 ⇒ [−size,0].

### Moteur (`DmxState`)

- `ActiveEffect { owner: CueId, index, channels: Vec<EffChan{universe,channel,width,phase}>, …params…, start, fade_in, release: Option<(t0,dur)> }`.
- La cue résout fixture/groupe → liste de canaux **au GO** (comme les fades), avec la phase
  de chaque membre = `phase_offset + spread·(rang/N)` selon `direction`.
- `rendered(u)` : part du buffer base, puis pour chaque effet touchant `u` :
  `eff_size = size · ramp_in · ramp_release` ; `Σ offset` par canal ;
  `clamp(base_norm + Σ, 0, 1)` ré-encodé. Base jamais mutée.
- API handle : `submit_effect(owner, index, resolved)`, `release_effects(owner, fade)`
  (soft = size→0), `clear_effects(owner)` (hard).

### Cycle de vie

- `EffectCue.go()` → submit ; `duration() = None` (tourne jusqu'au stop) ; `is_action_started = true`.
- **Stop Cue** ciblant l'`EffectCue` / le `LightCue` : soft = `release_effects` (size→0 sur
  `stop_fade_ms`, retour doux à la base) ; hard = `clear_effects`. **Fade Cue** ciblant
  l'effet : fade de la `size` vers une valeur (stretch).
- `LightCue` : une target gagne `effect: Option<EffectSpec>`, submit après les fades du look,
  owner = id du Light Cue.

### UI

- Onglet **Effect** (Effect Cue) : lignes d'`EffectSpec` (cible, forme, BPM + tap, size %,
  biais, phase, spread °, direction, duty si carré, fade-in) avec **preview live** (submit au
  moteur en éditant, comme le Dashboard).
- **Light tab** : bouton « + Effect » par carte fixture → même éditeur (cible héritée).
- Panneau DMX : section « Effets actifs » (liste + stop) via le monitor existant.

### Défauts tranchés (sans fork)

- Vitesse en **BPM** (sync, tap-tempo sur le champ ; master de vitesse global = fast-follow).
- Formes v1 : Sine, Triangle, SawUp, SawDown, Square (duty), Random (twinkle).
- Spread = total en degrés réparti sur les membres dans l'ordre du patch + Forward/Backward/Bounce.

### Tests (cargo test)

Formes d'onde ; math `offset` (centre 0/0.5/1 + clamp) ; somme/empilement ; répartition du
spread sur N canaux ; rampes fade-in/release ; **`rendered()` superpose sans muter la base**
(tracking préservé après retrait) ; cohérence de phase deux effets même BPM ; roundtrip serde
`EffectSpec` (Effect Cue + target Light Cue).

---

## Prochaine étape

**M5 — finition** : interface réseau (NIC source) en machine-config via `socket2`
(comme le device audio), UI préférences réseau, et validation visuelle hardware.
Ensuite **Phase 2** : **moteur d'effets** (design verrouillé ci-dessus), import de
bibliothèque de fixtures (OFL / QLC+), groupes/palettes, master dimmer.
