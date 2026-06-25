#!/usr/bin/env bash
# bundle_macos_libs.sh — collect libmpv + all Homebrew dependencies into
# src-tauri/vendor/mpv/macos/ with install names fixed for standalone .app bundling.
#
# Run once before `pnpm tauri build` on macOS:
#   ./scripts/bundle_macos_libs.sh
#
# After this script, run the normal Tauri build — tauri.macos.conf.json will
# bundle everything in vendor/mpv/macos/ into Contents/Resources/ automatically.
#
# Requirements: macOS with Homebrew mpv installed (brew install mpv).
# The output directory is gitignored; re-run whenever you update libmpv.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEST="$SCRIPT_DIR/../src-tauri/vendor/mpv/macos"

# ── Locate Homebrew prefix ───────────────────────────────────────────────────
if [[ -d "/opt/homebrew" ]]; then
  BREW_PREFIX="/opt/homebrew"   # Apple Silicon
elif [[ -d "/usr/local/Cellar" ]]; then
  BREW_PREFIX="/usr/local"      # Intel
else
  echo "error: Homebrew not found" >&2
  exit 1
fi

MPV_LIB="$BREW_PREFIX/lib/libmpv.dylib"
if [[ ! -f "$MPV_LIB" ]]; then
  echo "error: $MPV_LIB not found — install with: brew install mpv" >&2
  exit 1
fi

echo "Source : $MPV_LIB"
echo "Dest   : $DEST"
echo ""

rm -rf "$DEST"
mkdir -p "$DEST"

# ── Step 1: collect libmpv + all non-system Homebrew deps (recursive) ────────
collect() {
  local src="$1"
  local name
  name="$(basename "$src")"

  # Already collected → skip (handles circular / diamond deps)
  [[ -f "$DEST/$name" ]] && return

  echo "  + $name"
  # -L dereferences symlinks so we always get the actual dylib content,
  # named after the path's basename (e.g., libmpv.dylib, not libmpv.2.dylib).
  cp -L "$src" "$DEST/$name"
  chmod 755 "$DEST/$name"

  # Parse `otool -L` — skip first line (the lib's own install name) then filter:
  #   - /usr/lib/*  and  /System/*  → system libs, not bundled
  #   - @*          → already relative (@rpath, @loader_path, …), handled later
  #   - non-existent paths → skip
  while IFS= read -r line; do
    local dep
    dep="${line%%(*}"     # strip trailing "(compatibility version X, current version Y)"
    dep="${dep#$'\t'}"    # strip leading tab (otool indents with \t)
    dep="${dep% }"        # strip trailing space
    [[ -z "$dep" ]] && continue
    [[ "$dep" == /usr/lib/* ]] && continue
    [[ "$dep" == /System/* ]] && continue
    [[ "$dep" == @* ]] && continue
    [[ -f "$dep" ]] || continue
    collect "$dep"
  done < <(otool -L "$src" | tail -n +2)
}

echo "Collecting dependencies..."
collect "$MPV_LIB"

# ── Step 2: fix all install names to use @loader_path/ ───────────────────────
#
# @loader_path in a dylib loaded via dlopen resolves to the directory that
# contains that dylib — i.e., Contents/Resources/ when bundled in a .app.
# So every dep of libmpv.dylib can find its peers via @loader_path/<name>.
echo ""
echo "Fixing install names..."
for dylib in "$DEST"/*.dylib; do
  local_name="$(basename "$dylib")"

  # Fix the dylib's own install name (-id)
  install_name_tool -id "@loader_path/$local_name" "$dylib" 2>/dev/null || true

  # Rewrite every dependency reference that we collected
  while IFS= read -r line; do
    dep="${line%%(*}"
    dep="${dep#$'\t'}"
    dep="${dep% }"
    [[ -z "$dep" ]] && continue
    dep_name="$(basename "$dep")"
    # Only rewrite if we actually collected it
    [[ -f "$DEST/$dep_name" ]] || continue
    # Skip no-op rewrites (install_name_tool errors on unchanged paths)
    [[ "$dep" == "@loader_path/$dep_name" ]] && continue
    install_name_tool -change "$dep" "@loader_path/$dep_name" "$dylib" 2>/dev/null || true
  done < <(otool -L "$dylib" | tail -n +2)
done

# ── Step 3: ad-hoc sign all dylibs ──────────────────────────────────────────
#
# macOS Catalina+ refuses to load dylibs with no code signature at all.
# Ad-hoc signing (--sign -) is sufficient for local dev and for app bundles
# that will be re-signed by Tauri's `tauri build --sign` step at release time.
echo ""
echo "Ad-hoc signing..."
for dylib in "$DEST"/*.dylib; do
  codesign --force --sign - "$dylib"
done

# ── Summary ──────────────────────────────────────────────────────────────────
count=$(ls -1 "$DEST"/*.dylib | wc -l | tr -d ' ')
total=$(du -sh "$DEST" | cut -f1)
echo ""
echo "Done: $count dylibs bundled ($total total)"
echo ""
ls -lh "$DEST"/*.dylib
echo ""
echo "Next: pnpm tauri build"
