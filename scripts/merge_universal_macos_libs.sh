#!/usr/bin/env bash
# merge_universal_macos_libs.sh — fuse an arm64 and an x86_64 libmpv bundle
# (both produced by bundle_macos_libs.sh) into one universal bundle.
#
# Usage: merge_universal_macos_libs.sh <arm64-dir> <x86_64-dir>
#
# The result replaces the contents of <arm64-dir> (the directory Tauri bundles),
# so an Intel Mac can load libmpv instead of silently running without video.
# A dylib present in only one bundle (Homebrew can ship a different soname per
# runner, e.g. libx265.216 vs .217) is kept as-is: each slice of the fat libmpv
# only references its own version, so the other slice never loads the stray file.

set -euo pipefail

ARM_DIR="${1:?usage: $0 <arm64-dir> <x86_64-dir>}"
X86_DIR="${2:?usage: $0 <arm64-dir> <x86_64-dir>}"

for dylib in "$ARM_DIR"/*.dylib; do
  name="$(basename "$dylib")"
  if [[ ! -f "$X86_DIR/$name" ]]; then
    echo "note: $name is arm64-only (kept thin)"
    continue
  fi
  lipo -create "$dylib" "$X86_DIR/$name" -output "$dylib.universal"
  mv "$dylib.universal" "$dylib"
  codesign --force --sign - "$dylib"
done

for dylib in "$X86_DIR"/*.dylib; do
  name="$(basename "$dylib")"
  if [[ ! -f "$ARM_DIR/$name" ]]; then
    echo "note: $name is x86_64-only (copied thin)"
    cp "$dylib" "$ARM_DIR/$name"
  fi
done

echo "Universal libmpv bundle:"
for dylib in "$ARM_DIR"/*.dylib; do
  archs="$(lipo -archs "$dylib")"
  echo "  $(basename "$dylib"): $archs"
done
for required in libmpv.dylib; do
  archs="$(lipo -archs "$ARM_DIR/$required")"
  [[ "$archs" == *x86_64* && "$archs" == *arm64* ]] || { echo "error: $required is not universal" >&2; exit 1; }
done
echo "Deployment target of libmpv:"
vtool -show-build "$ARM_DIR/libmpv.dylib" | grep -E "minos|architecture" || true
