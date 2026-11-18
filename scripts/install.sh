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

# Retry crates.io downloads more often than cargo's default of 3.
export CARGO_NET_RETRY="${CARGO_NET_RETRY:-10}"
export PATH="$HOME/.cargo/bin:$PATH"

if ! command -v trunk >/dev/null; then
    echo "==> Installing Trunk (builds the WebAssembly interface)"
    # Prebuilt release, so ~400 crates aren't fetched and compiled.
    TRUNK_VERSION=0.22.1
    TRUNK_SHA256=2ab8d548fd5ed9cda50906f61fd8654204e7893bd52ebbfd90533862b7d71ee1
    tmp=$(mktemp -d)
    url="https://github.com/trunk-rs/trunk/releases/download/v$TRUNK_VERSION/trunk-x86_64-unknown-linux-gnu.tar.gz"
    if [ "$(uname -m)" = x86_64 ] \
        && curl -fsSL --retry 5 -o "$tmp/trunk.tar.gz" "$url" \
        && echo "$TRUNK_SHA256  $tmp/trunk.tar.gz" | sha256sum -c --quiet; then
        tar -xzf "$tmp/trunk.tar.gz" -C "$tmp" trunk
        install -Dm755 "$tmp/trunk" "$HOME/.cargo/bin/trunk"
    else
        echo "    Prebuilt Trunk unavailable, building it with cargo"
        cargo install trunk --version "$TRUNK_VERSION" --locked
    fi
    rm -rf "$tmp"
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
