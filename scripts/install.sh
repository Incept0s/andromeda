#!/usr/bin/env bash
# Builds Andromeda from source and installs it as an RPM (Fedora).
# Installs missing build tools first. Asks for your password for dnf.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> Installing build dependencies"
sudo dnf install -y gcc rpm-build systemd-rpm-macros \
    webkit2gtk4.1-devel gtk3-devel libsoup3-devel openssl-devel pulseaudio-libs-devel

# Rust: keep a rustup install if there is one, otherwise use Fedora's.
if command -v rustup >/dev/null; then
    echo "==> Using rustup"
    rustup target add wasm32-unknown-unknown
else
    if ! command -v cargo >/dev/null; then
        echo "==> Installing Rust from Fedora"
        sudo dnf install -y rust cargo
    fi
    sudo dnf install -y rust-std-static-wasm32-unknown-unknown
fi

if ! command -v trunk >/dev/null; then
    echo "==> Installing Trunk (builds the WebAssembly interface)"
    cargo install trunk --locked
    export PATH="$HOME/.cargo/bin:$PATH"
fi

echo "==> Building"
scripts/build-rpm.sh

echo "==> Installing"
sudo dnf install -y "$ROOT"/target/rpm/RPMS/x86_64/andromeda-*.rpm

cat <<'EOF'

Done. Next:
  1. Log out and back in (GNOME loads new extensions at login).
  2. Open the Extensions app and switch on "Andromeda".
EOF
