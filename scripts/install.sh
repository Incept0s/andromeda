#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
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
RPM=$(ls "$ROOT"/target/rpm/RPMS/x86_64/andromeda-*.rpm)
if rpm -q andromeda >/dev/null 2>&1; then
    # Already installed: replace it even when the version did not change
    # (dnf would skip a rebuild of the same version).
    sudo rpm -Uvh --replacepkgs "$RPM"
else
    sudo dnf install -y "$RPM"
fi

cat <<'EOF'

Done. Next:
  1. Log out and back in (GNOME loads new extensions at login).
  2. Open the Extensions app and switch on "Andromeda".
EOF
