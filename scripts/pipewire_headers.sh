#!/usr/bin/env bash
# pipewire_headers.sh — install libpipewire / libspa headers (PW_VERSION below)
# into a private prefix, with pkg-config files that link the system's runtime
# libpipewire-0.3.so.0.
#
# Why: cpal's native PipeWire host (pipewire-sys / libspa-sys bindgen) needs
# 1.x headers, but the Linux release runner is Ubuntu 22.04 (glibc baseline of
# the AppImage/.deb), whose libpipewire-0.3-dev is 0.3.48.  Taking only the
# *headers* from a newer release while linking 22.04's own 0.3.48 library
# makes the link fail if Inkue imports a symbol the oldest libpipewire it can
# meet lacks, so the binary keeps loading there (it is BIND_NOW).  The native
# host itself is gated at runtime to libpipewire >= 0.3.53, and the one newer
# symbol pipewire-rs calls is shimmed (engine/audio_host.rs).  See PORTAGE.md.
#
# Usage (CI):
#   bash scripts/pipewire_headers.sh "$RUNNER_TEMP/pipewire"
#   echo "PKG_CONFIG_PATH=$RUNNER_TEMP/pipewire/lib/pkgconfig" >> "$GITHUB_ENV"
#
# Requirements: curl, tar, sha256sum, and the runtime library
# (libpipewire-0.3-0) installed.  Override the library directory with
# PW_SYSTEM_LIBDIR (default: the one holding libpipewire-0.3.so.0).

set -euo pipefail

PW_VERSION=1.0.5
PW_SHA256=c5a5de26d684a1a84060ad7b6131654fb2835e03fccad85059be92f8e3ffe993
PREFIX="${1:?usage: pipewire_headers.sh <prefix>}"

# ── Runtime library to link against ──────────────────────────────────────────
LDCONFIG="$(command -v ldconfig || echo /sbin/ldconfig)"
LIBDIR="${PW_SYSTEM_LIBDIR:-$(dirname "$("$LDCONFIG" -p | awk '/libpipewire-0\.3\.so\.0 .*x86-64/ {print $NF; exit}')")}"
if [ ! -e "$LIBDIR/libpipewire-0.3.so.0" ]; then
  echo "libpipewire-0.3.so.0 not found (install libpipewire-0.3-0 or set PW_SYSTEM_LIBDIR)" >&2
  exit 1
fi

# ── Source tarball (headers only are used) ───────────────────────────────────
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
curl -fsSL -o "$WORK/pipewire.tar.gz" \
  "https://gitlab.freedesktop.org/pipewire/pipewire/-/archive/$PW_VERSION/pipewire-$PW_VERSION.tar.gz"
echo "$PW_SHA256  $WORK/pipewire.tar.gz" | sha256sum -c -
tar -xzf "$WORK/pipewire.tar.gz" -C "$WORK"
SRC="$WORK/pipewire-$PW_VERSION"

# ── Headers, laid out as `meson install` would ───────────────────────────────
PW_INC="$PREFIX/include/pipewire-0.3/pipewire"
mkdir -p "$PW_INC/extensions/session-manager" "$PREFIX/include/spa-0.2" "$PREFIX/lib/pkgconfig"
cp "$SRC"/src/pipewire/*.h "$PW_INC/"
cp "$SRC"/src/pipewire/extensions/*.h "$PW_INC/extensions/"
cp "$SRC"/src/pipewire/extensions/session-manager/*.h "$PW_INC/extensions/session-manager/"
cp -r "$SRC/spa/include/spa" "$PREFIX/include/spa-0.2/"

IFS=. read -r MAJOR MINOR MICRO <<< "$PW_VERSION"
sed -e "s/@PIPEWIRE_VERSION_MAJOR@/$MAJOR/g" \
    -e "s/@PIPEWIRE_VERSION_MINOR@/$MINOR/g" \
    -e "s/@PIPEWIRE_VERSION_MICRO@/$MICRO/g" \
    -e "s/@PIPEWIRE_VERSION_NANO@/0/g" \
    -e 's/@PIPEWIRE_API_VERSION@/"0.3"/g' \
    "$SRC/src/pipewire/version.h.in" > "$PW_INC/version.h"

# ── Link name for the runtime library (no -dev package needed) ───────────────
ln -sf "$LIBDIR/libpipewire-0.3.so.0" "$PREFIX/lib/libpipewire-0.3.so"

cat > "$PREFIX/lib/pkgconfig/libspa-0.2.pc" <<EOF
prefix=$PREFIX
includedir=\${prefix}/include

Name: libspa
Description: Simple Plugin API (headers $PW_VERSION)
Version: 0.2
Cflags: -I\${includedir}/spa-0.2 -D_REENTRANT
EOF

cat > "$PREFIX/lib/pkgconfig/libpipewire-0.3.pc" <<EOF
prefix=$PREFIX
includedir=\${prefix}/include
libdir=\${prefix}/lib

Name: libpipewire
Description: PipeWire Interface (headers $PW_VERSION, system library)
Version: $PW_VERSION
Requires: libspa-0.2
Libs: -L\${libdir} -lpipewire-0.3
Cflags: -I\${includedir}/pipewire-0.3 -D_REENTRANT
EOF

echo "PipeWire $PW_VERSION headers in $PREFIX (linking $LIBDIR/libpipewire-0.3.so.0)"
