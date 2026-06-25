# WinCue — Portage cross-platform (Windows / Linux / macOS)

État : **macOS, Linux et Windows partagent le même chemin de sortie GL** depuis le
portage macOS (branche `macos-port`, 2026-06-22). Ce document décrit l'architecture
*réalisée* ; l'historique des décisions est conservé en bas.

## Décisions architecturales (réalisées)

| Sujet | Décision |
|---|---|
| Cibles | Windows + Linux + macOS |
| Chemin de rendu | `vo=libmpv` + mpv OpenGL Render API — **une seule** implémentation (`render.rs`), partagée par les 3 OS |
| Création de fenêtre | Par-OS, derrière `#[cfg(target_os)]` : **winit** (Windows/Linux), **AppKit `NSWindow` via objc2** (macOS, `macos_window.rs`). PAS de Tauri `WindowBuilder` (feature `unstable` → comctl32 v6 → crash des tests) |
| Pourquoi pas winit sur macOS | Son `EventLoop` exige la run loop principale AppKit, que la `NSApplication` de Tauri possède déjà. On ne peut pas avoir deux `[NSApp run]`. winit tourne sur un thread de fond sur Win/Linux (`with_any_thread`), interdit sur macOS |
| Contexte GL | `glutin 0.32` (rwh 0.6) : WGL (Windows), EGL/GLX (Linux), **CGL** (macOS) |
| Version GL | OpenGL 3.3 core (Windows/Linux) / **3.2 core (macOS — pas de profil 3.3)** ; shaders en `#version 150 core` (sous-ensemble accepté partout) |
| hwdec | `hwdec=auto-copy` cross-platform (interop GL sûre), posé par-vidéo dans `fade::execute_load_params` |
| Fade overlay | Quad GL noir dans le FBO mpv (PAS `overlay-add`, PAS layered window) — `tick_fade()` piloté par le thread render |
| OSD timer | `osd-msg1` composité par mpv dans le FBO — inchangé sur les 3 OS |
| vsync | `glutin` `SwapInterval::DontWait` (mpv `video-sync=desync` cadence la lecture) |
| Floating timer | Tauri WebView `float-timer` — unifié 3 OS |
| Legacy Windows | Win32+D3D11+wid derrière `#[cfg(feature="legacy-win32-output")]` (éteint) |
| Audio | cpal **0.18** partout : WASAPI/ASIO (Windows), ALSA/PipeWire (Linux), CoreAudio (macOS) — aucun code par-OS |
| libmpv Windows | `libmpv-2.dll` bundlé (`vendor/mpv/`) |
| libmpv macOS | Homebrew en dev ; **bundle `.app` résolu** : `./scripts/bundle_macos_libs.sh` collecte libmpv + toutes les deps Homebrew dans `src-tauri/vendor/mpv/macos/`, fixe les install names en `@loader_path/`, signe ad-hoc ; `tauri.macos.conf.json` bundle le tout dans `Contents/Resources/`. `mpv_sys::open_dll()` cherche `Contents/Resources/libmpv.dylib` en premier. |
| libmpv Linux | Dépendance système (`.deb` dépend de `libmpv2 | libmpv1`) |
| CI | GitHub Actions — `windows-latest` (check+test), `ubuntu-latest` (check), `macos-latest` (clippy+test) |

## Sélection du backend (`build.rs`)

`build.rs` émet les `cfg` :

- **`output_gl`** — le chemin Render API GL est compilé (`render.rs`, render loop, fade
  quad). Vrai pour : Windows (défaut), Linux, **macOS**.
- **`output_win32`** — chemin legacy Win32+D3D11 (`win32_window.rs`), Windows + feature
  `legacy-win32-output` uniquement.

Dans `render.rs`, la création de fenêtre se branche par `target_os` : winit
(`not(macos)`) vs `super::macos_window` (`macos`). Tout ce qui suit `make_current`
(contexte glutin, `mpv_render_context`, render loop, fade quad) est **partagé**.

## Fenêtre output — backends

### winit (Windows + Linux)

`render.rs` crée la fenêtre via **`winit 0.30`** sur un thread de fond
(`wincue-output-window`) : `with_any_thread(true)` (extensions Windows / X11) lève le
garde-fou « EventLoop sur le thread principal ». La fenêtre est stockée dans
`render::GL_WINDOW: OnceLock<Arc<winit::window::Window>>` ; show/hide/position/fullscreen
appellent l'API winit cross-platform depuis n'importe quel thread.

### AppKit / objc2 (macOS — `macos_window.rs`)

winit étant inutilisable (voir décisions), macOS crée un **`NSWindow` borderless**
directement via `objc2` (`msg_send!` brut, sélecteurs Cocoa stables ; AppKit linké par
`build.rs`). La fenêtre est bâtie **inline sur le thread principal** — `OutputEngine::new`
tourne dans le `.setup()` de Tauri, qui *est* le thread principal. Son `contentView`
(un `NSView`) est passé à glutin comme drawable CGL ; le thread render fait ensuite
exactement comme sur les autres OS.

Les helpers de contrôle (`show`/`hide`/`position_on_screen`/`toggle_fullscreen`) sont
appelés depuis des threads workers (commandes Tauri, event loop) → ils marshalent sur le
thread principal via `AppHandle::run_on_main_thread`. Le pointeur `*mut NSWindow` est
conservé (leaké, +1 retain) dans un `AtomicUsize`. Placement écran via `NSScreen`
(coordonnées AppKit natives, pas de conversion), fullscreen = frame plein écran de
l'`NSScreen` courant (style « borderless » comme winit).

**À valider sur hardware Apple :** la création du contexte/surface CGL se fait sur le
thread render (comme Win/Linux). Si glutin exige le thread principal pour CGL, le repli
est de bâtir la stack GL sur le thread principal pendant `.setup()`.

## Render API mpv (`mpv/render.h`)

Symboles dans `mpv_sys.rs` : `mpv_render_context_create/render/update/
set_update_callback/report_swap/free`. Structures `MpvRenderParam`,
`MpvOpenglInitParams`, `MpvOpenglFbo`. La render loop dit à mpv où dessiner via le champ
`MpvOpenglFbo.fbo` (`0` = framebuffer fenêtre). *Pour de futures transformations /
projection mapping : rendre dans un FBO/texture offscreen puis dessiner un quad/mesh
warpé — purement dans `render.rs`, donc identique sur les 3 OS.*

## Fade GL

Shader GLSL `#version 150 core` dans `render.rs` : vertex fullscreen via `gl_VertexID`
(VAO dummy, pas de VBO), fragment `vec4(0,0,0,u_alpha)`. Dessiné après
`mpv_render_context_render`, avant `swap_buffers`. Alpha piloté par `fade::tick_fade()`.

## Note : `tauri = {unstable}` à éviter

La feature `unstable` expose `tauri::window::WindowBuilder` (fenêtre sans WebView) mais
importe `TaskDialogIndirect` depuis `comctl32.dll v6`. Le binaire de test Rust n'a pas le
manifest common-controls v6 → `STATUS_ENTRYPOINT_NOT_FOUND` avant le premier test. D'où
les fenêtres output créées avec les APIs OS directement (winit / objc2).

## cpal : ID stable vs nom d'affichage (piège, 0.9.7)

cpal 0.18 a supprimé `Device::name()` et fait de `Device` un `Display` — `to_string()` renvoie
le **libellé humain** (ce que montre l'OS dans ses réglages son, ex. `"PipeWire Sound Server"`
ou, sur Linux, le `node.description` PipeWire de la carte — `"Built-in Audio Analog Stereo"`
pour le micro intégré, `node.nick` étant l'alias court du codec, ex. `"ALC293 Analog"`). C'est
**différent** de l'identifiant stable utilisé par le host audio (PCM ALSA `"pipewire"`/`"hw:0,0"`,
nom de device WASAPI/CoreAudio) — c'est ce dernier qu'on doit stocker dans `OutputPatch`/
`InputPatch.device_id` et utiliser pour le matching, parce que c'est ce que le host comprend en
entrée de `build_*_stream`. Confondre les deux casse tous les lookups de device (vu en prod :
`"Audio device 'pipewire' not found"` au démarrage après l'upgrade 0.18).

**Règle** : `Device::id()` → `Result<DeviceId, Error>`, puis `DeviceId::id() -> &str` est l'ID
stable — utilisé pour **tout stockage et tout matching** (`device_manager.rs`, `audio_input.rs`,
`preferences_cmds.rs`). `Device::to_string()` (= `Display`) est réservé au champ `DeviceInfo.name`
**affiché à l'utilisateur**, jamais comparé ou persisté comme clé.

## libmpv — chargement dynamique (`mpv_sys::open_dll`)

| Plateforme | Nom | Source recherchée |
|---|---|---|
| Windows | `libmpv-2.dll` | à côté de l'exe, puis `vendor/mpv/` |
| macOS | `libmpv.dylib` | `Contents/Resources` / `Contents/Frameworks` (bundle), à côté de l'exe, puis Homebrew (`/opt/homebrew/lib`, `/usr/local/lib`) |
| Linux | `libmpv.so.2` / `.so.1` / `.so` | à côté de l'exe, puis chemin système (`ld.so`) |

---

## Historique (décisions superseded)

- **Stage 1 (0.9.0)** : chemin GL unifié introduit sur Windows/Linux (winit), macOS
  laissé sur l'ancienne fenêtre mpv cocoa-cb (`vo=gpu`) — fade non fonctionnel sur macOS.
- **0.8.1** : Mac/Linux output via fenêtre gérée par mpv + propriétés (`hidden`,
  `fullscreen`, `screen`) ; fade ASS `osd-overlay`. **Caduc** : remplacé par le chemin GL.
- **Plan initial « NSWindow via run_on_main_thread + transfert au render thread »** :
  l'hypothèse « créer juste une fenêtre winit sur le thread principal » était fausse (une
  `winit::Window` exige sa propre `EventLoop` qui tourne). Remplacé par le `NSWindow`
  objc2 direct ci-dessus.
