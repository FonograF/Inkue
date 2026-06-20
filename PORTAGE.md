# WinCue — Plan de portage Mac + Linux

## Décisions architecturales (révisées 2026-06-17)

| Sujet | Décision |
|---|---|
| Cibles | macOS + Linux simultanément |
| Fenêtre output | OS-native (Win32/AppKit/GTK) par petits helpers `#[cfg]` — PAS Tauri WindowBuilder (requiert feature `unstable` qui importe comctl32 v6 et crashe les tests) |
| Backend GPU | `vo=libmpv` partout + Render API OpenGL — UNE implémentation commune |
| Contexte GL | `glutin 0.32` (rwh 0.6) : WGL sur Windows, CGL sur macOS, EGL sur Linux |
| GL version | OpenGL 3.3 Core (macOS plafonne à 4.1 mais 3.3 suffit) |
| hwdec | `hwdec=auto-copy` cross-platform (interop GL sûre) |
| Fade overlay | Quad GL noir dans le compositeur (PAS `overlay-add`, PAS layered window) |
| OSD timer | `osd-msg1` composite par mpv dans le FBO — inchangé sur les 3 OS |
| vsync | `glutin` SwapInterval::Wait(1) — remplace `d3d11-sync-interval=0` |
| Floating timer | Tauri WebView `float-timer` — unifié 3 OS, inchangé |
| Legacy Windows | Win32+D3D11+wid derrière `#[cfg(feature="legacy-win32-output")]` (éteint) |
| Audio Mac | CoreAudio via cpal — aucun changement |
| Audio Linux | ALSA + PipeWire via cpal |
| libmpv Mac | Bundlé dans le `.app` |
| libmpv Linux | Dépendance système |
| CI | GitHub Actions — runners macos-latest + ubuntu-latest |

**Items 5/6/10 (Win32OutputWindow + MpvOutputWindow comme deux impls) : CADUCS.**
Il n'y a qu'une impl : `render.rs` avec de petits helpers `#[cfg]` pour la création de la fenêtre.

---

## Séquence d'implémentation

### Phase A — compilable sur toutes les plateformes ✅

- [x] Ajouter CI GitHub Actions
- [x] Isoler `windows-sys` derrière `#[cfg(target_os = "windows")]`
- [x] `gpu-api=auto` + isoler options D3D11
- [x] `symphonia` en dépendance régulière
- [x] `mpv_sys::open_dll()` cross-platform

### Phase B — chemin GL unifié ✅ (Stage 1, 2026-06-17)

- [x] **Floating timer → Tauri WebView** (fait en 0.8.1)
- [x] **mpv_sys.rs** : +6 symboles Render API (`mpv_render_context_create/render/free/…`)
- [x] **render.rs** (nouveau) : fenêtre Win32 GL + glutin WGL + mpv RenderContext + boucle rendu + fade quad GL
- [x] **fade.rs** : simplifié — `tick_fade()` + `execute_pending()` driven par le thread render ; legacy derrière feature flag
- [x] **mod.rs** : `vo=libmpv` uniforme ; `render::init()` dans `OutputEngine::new()` ; show/hide/position via `render::GL_WINDOW` (API winit cross-platform ; était `GL_HWND` Win32 en Stage 1, migré vers winit en 0.9.2)
- [x] **Cargo.toml** : `glutin 0.32`, `glow 0.13`, `raw-window-handle 0.6`, feature `legacy-win32-output`
- [ ] **macOS fenêtre GL** : NSWindow via objc2 + `app_handle.run_on_main_thread()` → CGL display
- [ ] **Linux fenêtre GL** : GDK/GTK via `app_handle.run_on_main_thread()` → EGL display

### Stage 2 (future)

- Pool d'instances mpv pré-rollées (TODO marqué dans render.rs)
- hwdec sans copie (DXGI/WGL_NV_DX_interop) optionnel
- Suppression du code legacy Win32 une fois la parité validée

---

## Détails par composant

### Fenêtre output — winit (actuel)

Depuis 0.9.2, `render.rs` crée la fenêtre via **`winit 0.30`** (un seul chemin
cross-platform) au lieu d'appels Win32 bruts :
- Thread `wincue-output-window` : `winit::event_loop::EventLoop` + gestion des events
  (drag, resize, double-clic fullscreen). Thread `wincue-output-render` : contexte GL
  glutin + `mpv_render_context`.
- Fenêtre stockée dans `render::GL_WINDOW: OnceLock<Arc<winit::window::Window>>` —
  `OutputEngine` (show/hide/position/fullscreen) appelle l'API winit cross-platform
  depuis n'importe quel thread.
- **Windows / Linux** : la fenêtre winit est créée depuis le thread de fond.
- **macOS** : nécessite le thread principal AppKit → création via
  `AppHandle::run_on_main_thread()` puis transfert au render thread *(Stage 2 — TODO)*.

### Render API mpv (mpv/render.h)

Symboles ajoutés dans `mpv_sys.rs` :
- `mpv_render_context_create(res, mpv_handle, params)` — init avec backend OpenGL
- `mpv_render_context_render(ctx, params)` — rend video+OSD dans FBO 0 (avec flip_y=1)
- `mpv_render_context_update(ctx)` — flags (MPV_RENDER_UPDATE_FRAME)
- `mpv_render_context_set_update_callback(ctx, fn, ctx_ptr)` — réveille le thread render
- `mpv_render_context_report_swap(ctx)` — signale le swap à mpv
- `mpv_render_context_free(ctx)` — libération

Structures : `MpvRenderParam`, `MpvOpenglInitParams`, `MpvOpenglFbo`

### Fade GL

Shader GLSL 3.30 core dans `render.rs` :
- Vertex : fullscreen triangle via `gl_VertexID` (pas de VBO, un VAO dummy requis par Core profile)
- Fragment : `vec4(0, 0, 0, u_alpha)`
- Dessiné après `mpv_render_context_render`, avant `swap_buffers`
- Alpha : `FADE_STATE.current_alpha` mis à jour par `fade::tick_fade()` à chaque frame

### Note : `tauri = {unstable}` à éviter

La feature `unstable` expose `tauri::window::WindowBuilder` (fenêtre sans WebView) mais importe `TaskDialogIndirect` depuis `comctl32.dll v6`. Le binaire de test Rust n'a pas le manifest de dépendance common controls v6 qu'ajoute Tauri dans le vrai binaire → STATUS_ENTRYPOINT_NOT_FOUND avant le premier test. Solution retenue : créer les fenêtres output avec les APIs OS directement.

### libmpv — chargement dynamique

| Plateforme | Nom | Source |
|---|---|---|
| Windows | `libmpv-2.dll` | `vendor/mpv/` (bundlé) |
| macOS | `libmpv.dylib` | `Contents/Frameworks/` |
| Linux | `libmpv.so.2` | Dépendance système |

### Audio

- **Windows** : WASAPI via cpal ; ASIO via feature `asio-support`
- **macOS** : CoreAudio via cpal
- **Linux** : ALSA/PipeWire via cpal
