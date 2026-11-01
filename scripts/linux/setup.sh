#!/usr/bin/env bash
# Installs what building Audio Graph on Linux needs, skipping what is already there:
# system libraries (WebKitGTK, libpulse, ...), Node.js 22+, Rust, then the npm packages.
# Supports Debian/Ubuntu/Mint, Fedora and Arch; on other systems install the packages listed
# in README.md by hand.
set -euo pipefail
cd "$(dirname "$0")/../.."

SUDO=""
[ "$(id -u)" -ne 0 ] && SUDO="sudo"

. /etc/os-release
family="${ID} ${ID_LIKE:-}"

node_ok() {
    command -v node >/dev/null && node -e "process.exit(+process.versions.node.split('.')[0] >= 22 ? 0 : 1)"
}

case "$family" in
    *debian*|*ubuntu*)
        echo "Installing system packages (apt)..."
        $SUDO apt-get update -q
        $SUDO env DEBIAN_FRONTEND=noninteractive apt-get install -y -q \
            build-essential curl wget file rsync ca-certificates pkg-config \
            libwebkit2gtk-4.1-dev libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev libpulse-dev
        if ! node_ok; then
            echo "Installing Node.js 22 (NodeSource)..."
            curl -fsSL https://deb.nodesource.com/setup_22.x | $SUDO bash -
            $SUDO apt-get install -y -q nodejs
        fi
        ;;
    *fedora*|*rhel*)
        echo "Installing system packages (dnf)..."
        $SUDO dnf install -y @development-tools curl wget file rsync pkgconf-pkg-config \
            webkit2gtk4.1-devel openssl-devel libappindicator-gtk3-devel librsvg2-devel libxdo-devel \
            pulseaudio-libs-devel
        if ! node_ok; then
            echo "Installing Node.js 22 (NodeSource)..."
            curl -fsSL https://rpm.nodesource.com/setup_22.x | $SUDO bash -
            $SUDO dnf install -y nodejs
        fi
        ;;
    *arch*)
        echo "Installing system packages (pacman)..."
        $SUDO pacman -S --needed --noconfirm base-devel curl wget file rsync openssl \
            webkit2gtk-4.1 libappindicator-gtk3 librsvg xdotool libpulse nodejs npm
        ;;
    *)
        echo "Unknown distribution ($PRETTY_NAME): install the packages from README.md by hand." >&2
        exit 1
        ;;
esac
node_ok || { echo "Node.js 22 or later is required, found: $(node --version 2>/dev/null || echo none)" >&2; exit 1; }

if ! command -v cargo >/dev/null && [ ! -x "$HOME/.cargo/bin/cargo" ]; then
    echo "Installing Rust (rustup)..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
fi

echo "Installing npm packages..."
npm install --no-audit --no-fund

echo
echo "Ready. Run scripts/linux/dev.sh to start the app, scripts/linux/build.sh to build it."
