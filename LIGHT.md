# WinCue — Light Cue / contrôleur DMX

Objectif : faire de WinCue un **contrôleur lumière complet** (pas seulement un
déclencheur de console externe) — sortie DMX-over-IP directe vers des projecteurs,
avec un modèle de fixtures et des fades temporisés.

État : **M1 + M2 faits et testés** (cœur moteur + câblage app + panneau de test).
Le `LightCue` lui-même (M3/M4) n'est pas encore là. Branche : `light-cue`.

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
cue/light_cue.rs   (M4)   LightCue : look = [ParamTarget] + FadeSpec ; go() soumet
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

- **Patch fixtures** (M3) → sérialisé dans le `.wincue`, comme les patches OSC.
- **`CueContext`** recevra un `dmx_engine: Arc<DmxEngine>` (M4), à côté de `audio_engine` / `output_engine`.
- **Réutilise** `FadeCurve` (`engine::ring_command`) pour les courbes de fade.

---

## Modèle de données

**Déjà implémenté** (`engine/dmx_sink.rs`, `engine/dmx_engine.rs`) :

```rust
enum OutputProtocol { Sacn, ArtNet }
struct UniverseOutput { universe: u16, protocol: OutputProtocol,
                        destination: Option<IpAddr>, enabled: bool } // None = multicast sACN
enum ChannelWidth { Bit8, Bit16 }          // 16-bit = 2 canaux coarse+fine
```

**À venir** (M3, workspace) :

```rust
FixtureType { name, parameters: [FixtureParam{ kind: Intensity|Red|Green|Blue|Pan|Tilt|Generic,
                                               channel_offset, width, default }] }
PatchedFixture { id, label, type_ref, universe, base_address }
```

**À venir** (M4, cue) :

```rust
LightCue { …champs de cue…, targets: [ParamTarget{ fixture_id, param, value:0..1 }], fade: FadeSpec }
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

Émission : thread `wincue-dmx` à **~40 Hz**, send-on-change + **keepalive 800 ms**, n° de séquence par univers.

---

## État d'implémentation

| Jalon | Statut | Contenu |
|---|---|---|
| **M1 — le fil** | ✅ | DmxEngine (buffers + thread 40 Hz) + encodeurs sACN/Art-Net + commande « set channel » + moniteur intégré (event `dmx-monitor`) + panneau DMX dans la transport bar |
| **M2 — moteur de fade** | ✅ | `DmxState` : interpolation, **LTP** (repart de la valeur courante), **tracking**, **16-bit**, blackout — 7 tests unitaires |
| **M3 — patch fixtures** | ⬜ | `FixtureType` (intégrés) + `PatchedFixture` + UI patch + test-fixture + warnings de chevauchement d'adresses |
| **M4 — Light Cue** | ⬜ | `LightCue` + `CueRegistry` + `CueContext.dmx_engine` + inspector `LightTab` + roundtrip sérialisation |
| **M5 — finition** | ⬜ | NIC en machine-config (`socket2`), UI préférences réseau, cadence keepalive, validation visuelle |
| **Phase 2** | ⬜ | Effets (oscillateurs/chases), import bibliothèque de fixtures (OFL / QLC+), fade par-param, groupes/palettes, merge multi-source (HTP/priorité), master dimmer |

### Détail de ce qui est fait

**Backend**
- `engine/dmx_sink.rs` — `encode_sacn` / `encode_artnet` (fonctions pures, golden-testées au niveau octet), `DmxSink` (socket UDP), `UniverseOutput`, `sacn_multicast_ip`.
- `engine/dmx_engine.rs` — `DmxState` (pur, testable) + `DmxEngine` (handle + thread). Commandes : `submit_fade`, `set_channel`, `set_blackout`, `set_outputs`, `snapshot`.
- `engine/mod.rs` — exporte `DmxEngine`.
- `state/app_state.rs` — champ `dmx_engine: Arc<DmxEngine>`.
- `lib.rs` — création au démarrage + thread `wincue-dmx-monitor` qui émet l'event `dmx-monitor` (~20 fps, on-change, jamais de polling).
- `commands/light_cmds.rs` — `dmx_set_outputs`, `dmx_set_channel`, `dmx_set_blackout`, `dmx_get_blackout`, `dmx_get_snapshot`.

**Frontend**
- `lib/types.ts` — `OutputProtocol`, `UniverseOutput`, `DmxUniverseSnapshot`.
- `lib/commands.ts` — wrappers `dmxSetOutputs` / `dmxSetChannel` / `dmxSetBlackout` / `dmxGetBlackout` / `dmxGetSnapshot`.
- `components/Lighting/LightingPanel.tsx` — popover : config sorties (protocole + destination, persisté en localStorage), poke de canal (slider), blackout, moniteur live.
- `components/Transport/TransportBar.tsx` — bouton **DMX** qui ouvre le panneau.

**Tests** : 11 nouveaux (4 encodage paquets + 7 moteur d'état). **89 au total**, clippy clean, `tsc --noEmit` clean.

---

## Tester en soft (aucun matériel)

Tout se valide sur une seule machine en loopback :

1. `pnpm tauri dev`
2. Bouton **DMX** dans la transport bar → ajouter un univers (sACN multicast, ou Art-Net vers `127.0.0.1`).
3. Lancer un récepteur : **QLC+** (entrée Art-Net/sACN + moniteur de fixtures), **sACNView**, **OLA** (`ola_dmxmonitor`), ou **Capture** (démo, prévisualisation 3D).
4. Bouger le slider « Test channel » → la valeur monte dans le récepteur **et** dans le moniteur live du panneau. **Blackout** force tout à zéro.

Le hardware (un node Art-Net USB + un PAR LED) n'est utile que pour la vérif « vrai projecteur » finale — jamais pour développer.

---

## Prochaine étape

**M3 — patch fixtures** : passer du « poke de canal brut » aux projecteurs nommés
(Dimmer / RGB / RGBW / tête mobile 16-bit), avec l'UI de patch et le test-fixture.
