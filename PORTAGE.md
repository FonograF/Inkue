# WinCue — Plan de portage Mac + Linux

## Décisions architecturales

| Sujet | Décision |
|---|---|
| Cibles | macOS + Linux simultanément |
| Abstraction fenêtre output | Trait `OutputWindow` avec implémentations séparées (Phase B) |
| Fenêtre output Windows | Win32 HWND existant — inchangé |
| Fenêtre output Mac/Linux | mpv crée et gère sa propre NSWindow / X11 / Wayland |
| Fade overlay Windows | Fenêtre `WS_EX_LAYERED` Win32 — inchangé |
| Fade overlay Mac/Linux | OSD libmpv natif (`overlay-add` bitmap RGBA) |
| Floating timer | Tauri WebView pré-définie (`tauri.conf.json`, `visible: false`) — unifié sur les 3 plateformes |
| Backend GPU | `gpu-api=auto` partout ; `d3d11-sync-interval=0` derrière `#[cfg(target_os="windows")]` |
| Audio Mac | CoreAudio via cpal — aucun changement de code |
| Audio Linux | ALSA + PipeWire via cpal ; JACK en feature flag plus tard |
| libmpv Mac | Bundlé dans le `.app` (comme le DLL Windows) |
| libmpv Linux | Dépendance système (`apt install libmpv2`) |
| CI | GitHub Actions — runners `macos-latest` + `ubuntu-latest` sur chaque push |

## Séquence d'implémentation

### Phase A — compilable sur toutes les plateformes (master reste stable)

**Objectif** : `cargo check` vert sur Windows, macOS et Linux. Aucune régression fonctionnelle.

- [x] Ajouter CI GitHub Actions (`cargo check` Mac + Linux)
- [x] Isoler `windows-sys` derrière `#[cfg(target_os = "windows")]` dans `output_engine/`
- [x] `gpu-api=auto` + isoler `d3d11-sync-interval` et `wid`
- [x] `symphonia` en dépendance régulière (audio cross-platform)
- [x] `mpv_sys::open_dll()` cross-platform (noms de lib par plateforme)

**Gate** : une fois CI vert en `cargo check`, une régression Mac/Linux est impossible à merger accidentellement.

### Phase B — features cross-platform une par une

4. **Floating timer → Tauri WebView** : remplace ~300 lignes GDI Win32, unifie les 3 plateformes
5. **Trait `OutputWindow`** + `Win32OutputWindow` (refactor pur, aucun changement de comportement)
6. **`MpvOutputWindow` pour Mac/Linux** : mpv autonome, `gpu-api=auto`, show/hide via mpv properties
7. **Fade overlay → OSD libmpv** : `overlay-add` bitmap RGBA, supprime ~200 lignes layered-window Win32
8. **Screen enumeration cross-platform** : mpv `--screen=N` + enumération via crate dédiée
9. **Bundle libmpv `.dylib` Mac** + pipeline de signing Tauri
10. **Trait `OutputWindow` extracté** : `Win32OutputWindow`, `MpvOutputWindow` dans fichiers séparés

## Détails par composant

### libmpv — chargement dynamique

| Plateforme | Nom de bibliothèque | Source |
|---|---|---|
| Windows | `libmpv-2.dll` | `vendor/mpv/` (bundlé) |
| macOS | `libmpv.dylib` | `Contents/Frameworks/` dans le `.app` |
| Linux | `libmpv.so.2` | Dépendance système |

### Audio

- **Windows** : WASAPI via cpal (inchangé) ; ASIO via feature flag `asio-support`
- **macOS** : CoreAudio via cpal — aucun changement de code
- **Linux** : ALSA/PipeWire via cpal ; JACK en feature flag dans un second temps

### GPU backend mpv

```rust
// Tous
opt_str(&lib, ctx, "vo", "gpu");

// Windows uniquement
#[cfg(target_os = "windows")]
{
    opt_str(&lib, ctx, "gpu-api", "d3d11");
    opt_str(&lib, ctx, "d3d11-sync-interval", "0");
    opt_str(&lib, ctx, "force-window", "immediate");
}

// Mac + Linux : mpv choisit Metal / Vulkan / OpenGL via auto-detection
#[cfg(not(target_os = "windows"))]
opt_str(&lib, ctx, "force-window", "yes");
```

### Floating timer (Phase B)

Remplacer `FLOAT_TIMER_HWND` (Win32 GDI) par une fenêtre Tauri WebView :

```json
// tauri.conf.json
{
  "label": "float-timer",
  "title": "WinCue Timer",
  "visible": false,
  "decorations": false,
  "alwaysOnTop": true,
  "width": 420,
  "height": 110
}
```

Le backend émet un event `float-timer-update` au lieu de `InvalidateRect`.

### Fade overlay Mac/Linux (Phase B)

Utiliser l'API `overlay-add` de libmpv :

```rust
// Allouer un buffer RGBA noir (shared memory via mmap)
// overlay-add 0 0 0 <shm_path> 0 <W> <H> <stride>
// Mettre à jour le canal alpha du buffer pour animer le fade
// Re-poster overlay-add à chaque frame (~16 ms)
```

Avantages : cross-platform (mpv composite dans son pipeline Metal/GL/Vulkan),
fonctionne identiquement pour vidéos et images (même pipeline mpv).
