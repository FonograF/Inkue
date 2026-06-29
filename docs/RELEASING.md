# Releasing Inkue

Inkue ships via an automated GitHub Actions workflow
([`.github/workflows/release.yml`](../.github/workflows/release.yml)). Pushing a
version tag (`vX.Y.Z`) builds signed installers for Windows, macOS and Linux and
creates a **draft** GitHub release with them attached. You review and publish the draft.

---

## One-time setup (required before the first public release)

### 1. Windows libmpv — automatic, nothing to do

`libmpv-2.dll` (~113 MB) is not versioned in git. The Windows build job downloads it
automatically from the upstream shinchiro build on SourceForge — pinned by filename +
SHA-256 in the "Download libmpv (Windows)" step of `release.yml`. SourceForge keeps old
builds for years, so the pin stays resolvable.

The pinned build is byte-identical to the DLL used in local development
(`mpv-dev-x86_64-20260412-git-062f4bf`, baseline x86-64). To bump libmpv later, pick a
newer file from
<https://sourceforge.net/projects/mpv-player-windows/files/libmpv/> and update
`$LIBMPV_URL` and `$LIBMPV_SHA256` in that step (the SHA-256 is the hash of the `.7z`
archive). macOS gets libmpv via `brew install mpv` + `scripts/bundle_macos_libs.sh`;
Linux via the `.deb` dependency — both already handled in the workflow.

### 2. macOS signing + notarization (Apple Developer Program)

Set these as repository secrets (Settings → Secrets and variables → Actions). Without
them the build still succeeds, but the `.app` is unsigned and Gatekeeper blocks it.

| Secret | Value |
|---|---|
| `APPLE_CERTIFICATE` | base64-encoded `.p12` Developer ID Application cert |
| `APPLE_CERTIFICATE_PASSWORD` | `.p12` export password |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: Your Name (TEAMID)` |
| `APPLE_ID` | your Apple ID email |
| `APPLE_PASSWORD` | app-specific password (appleid.apple.com) |
| `APPLE_TEAM_ID` | 10-character Apple Team ID |

### 3. Tauri updater signing (optional — enables in-app update checks)

```bash
pnpm tauri signer generate -w ~/.tauri/inkue.key
```

Add the public key to `tauri.conf.json` under `plugins.updater.pubkey`, then set the
secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

---

## Cutting a release

1. Make sure `version` matches in **`package.json`**, **`src-tauri/Cargo.toml`** and
   **`src-tauri/tauri.conf.json`** (all three must agree).
2. Update [`CHANGELOG.md`](../CHANGELOG.md) with the new version's notes.
3. Commit, then tag and push:
   ```bash
   git tag v1.0.0
   git push origin v1.0.0
   ```
4. Wait for the **Release** workflow to finish (3 platform jobs).
5. Open the generated **draft** release, paste/adjust the notes (see below), verify the
   attached installers, then **Publish**.

A tag containing a hyphen (e.g. `v1.0.0-rc.1`) is published as a pre-release automatically.

---

## v1.0.0 release notes (paste into the draft)

> ## Inkue 1.0.0 — first public release
>
> Inkue is a professional, cross-platform show-control application inspired by QLab.
> It drives the playback side of a live show — audio, video, image, lighting, OSC,
> MIDI, timecode and more — from a single ordered cue list. This is the first public,
> open-source release (GNU GPL v3).
>
> ### Download
> - **Windows 10/11** — `*-setup.exe` or `*.msi` (libmpv bundled).
> - **macOS** (universal) — `*.dmg` (requires `brew install mpv`).
> - **Linux** (x86-64) — `.deb` (pulls in libmpv) or `.AppImage`.
>
> ### Highlights
> - 14 cue types: Audio, Video, Image, Group, Wait, Stop, Fade, OSC, MIDI, Light (DMX
>   over sACN + Art-Net), Mic, Timecode (MTC/LTC), Text, Memo.
> - Sample-accurate low-latency audio (WASAPI / ASIO / CoreAudio / ALSA); unified
>   flicker-free libmpv video/image output; QLab-style transport, groups and continues.
> - Reliability: crash-recovery autosave, workspace preflight + media relink, in-app
>   log viewer, audio device-loss fallback.
> - Windows, macOS and Linux from a single codebase.
>
> See the full [CHANGELOG](https://github.com/FonograF/Inkue/blob/master/CHANGELOG.md).
>
> *ASIO is a trademark and software of Steinberg Media Technologies GmbH. Inkue is not
> affiliated with Figure 53 (QLab) or Steinberg.*
