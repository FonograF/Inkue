#!/usr/bin/env bash
# WinCue — Ubuntu setup script
# Installs all build dependencies (Rust, Node, pnpm, system libs) and runs pnpm install.
# Safe to run more than once.
# Tested on Ubuntu 22.04 (Jammy) and 24.04 (Noble).

set -euo pipefail

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BOLD='\033[1m'; NC='\033[0m'

info()    { echo -e "${BOLD}[wincue]${NC} $*"; }
success() { echo -e "${GREEN}[wincue]${NC} $*"; }
warn()    { echo -e "${YELLOW}[wincue]${NC} $*"; }
die()     { echo -e "${RED}[wincue] ERROR:${NC} $*" >&2; exit 1; }

# ---------------------------------------------------------------------------
# 1. Ubuntu version check
# ---------------------------------------------------------------------------
if ! command -v lsb_release &>/dev/null; then
    die "lsb_release not found — this script requires Ubuntu 22.04 or 24.04."
fi
UBUNTU_VER=$(lsb_release -rs)
UBUNTU_ID=$(lsb_release -is)
[[ "$UBUNTU_ID" == "Ubuntu" ]] || die "This script is for Ubuntu only (detected: $UBUNTU_ID)."
case "$UBUNTU_VER" in
    22.*|24.*) ;;
    *) warn "Untested Ubuntu version $UBUNTU_VER — continuing anyway." ;;
esac
info "Ubuntu $UBUNTU_VER detected."

# ---------------------------------------------------------------------------
# 2. System packages
# ---------------------------------------------------------------------------
info "Installing system packages…"
sudo apt-get update -qq
sudo apt-get install -y \
    build-essential curl git pkg-config \
    libssl-dev \
    libgtk-3-dev \
    libwebkit2gtk-4.1-dev \
    librsvg2-dev \
    libasound2-dev \
    libmpv-dev
success "System packages OK."

# ---------------------------------------------------------------------------
# 3. Rust
# ---------------------------------------------------------------------------
if command -v cargo &>/dev/null; then
    RUST_VER=$(cargo --version)
    info "Rust already installed: $RUST_VER"
else
    info "Installing Rust via rustup…"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path
    # shellcheck source=/dev/null
    source "$HOME/.cargo/env"
    success "Rust installed: $(cargo --version)"
fi

# Ensure cargo is on PATH for the rest of this script
export PATH="$HOME/.cargo/bin:$PATH"

# ---------------------------------------------------------------------------
# 4. Node.js
# ---------------------------------------------------------------------------
if command -v node &>/dev/null && node --version | grep -qE '^v(20|22|2[4-9])'; then
    info "Node.js already installed: $(node --version)"
else
    info "Installing Node.js 22…"
    curl -fsSL https://deb.nodesource.com/setup_22.x | sudo -E bash -
    sudo apt-get install -y nodejs
    success "Node.js installed: $(node --version)"
fi

# ---------------------------------------------------------------------------
# 5. pnpm
# ---------------------------------------------------------------------------
if command -v pnpm &>/dev/null; then
    info "pnpm already installed: $(pnpm --version)"
else
    info "Installing pnpm…"
    npm install -g pnpm
    success "pnpm installed: $(pnpm --version)"
fi

# ---------------------------------------------------------------------------
# 6. JS dependencies (only if inside the repo)
# ---------------------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ -f "$SCRIPT_DIR/package.json" ]]; then
    info "Running pnpm install…"
    cd "$SCRIPT_DIR"
    pnpm install
    success "JS dependencies installed."
else
    warn "package.json not found — skipping pnpm install."
    warn "Clone the repo and run 'pnpm install' manually."
fi

# ---------------------------------------------------------------------------
# Done
# ---------------------------------------------------------------------------
echo ""
echo -e "${GREEN}${BOLD}Setup complete!${NC}"
echo ""
echo "  Dev mode  →  pnpm tauri dev"
echo "  Release   →  pnpm tauri build"
echo "              (packages in src-tauri/target/release/bundle/)"
echo ""
if [[ -n "${DISPLAY:-}" ]]; then
    info "Display detected (\$DISPLAY=$DISPLAY) — dev mode will work."
else
    warn "No \$DISPLAY set. For dev mode on a headless VM:"
    echo "    sudo apt install -y xvfb"
    echo "    Xvfb :99 -screen 0 1280x800x24 &"
    echo "    DISPLAY=:99 pnpm tauri dev"
    echo "  pnpm tauri build works without a display."
fi
