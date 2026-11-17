#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Development setup: builds everything and wires it into your user session
# without root. Undo with: scripts/dev-setup.sh --remove
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
UUID="andromeda@incept0s.github.io"
EXT_DIR="$HOME/.local/share/gnome-shell/extensions/$UUID"
DBUS_DIR="$HOME/.local/share/dbus-1/services"

reload_dbus() {
    # Let the session bus notice added/removed service files.
    busctl --user call org.freedesktop.DBus /org/freedesktop/DBus \
        org.freedesktop.DBus ReloadConfig >/dev/null || true
}

if [[ "${1:-}" == "--remove" ]]; then
    rm -f "$EXT_DIR" "$DBUS_DIR/io.github.incept0s.Andromeda.service" \
        "$DBUS_DIR/io.github.incept0s.Andromeda.Panel.service"
    reload_dbus
    echo "Removed dev links. Disable the extension with: gnome-extensions disable $UUID"
    exit 0
fi

(cd "$ROOT/frontend" && trunk build --release)
cargo build --release --manifest-path "$ROOT/Cargo.toml" -p andromeda -p andromedad

# D-Bus activation: calling either name starts the matching binary.
mkdir -p "$DBUS_DIR"
cat > "$DBUS_DIR/io.github.incept0s.Andromeda.service" <<EOF
[D-BUS Service]
Name=io.github.incept0s.Andromeda
Exec=$ROOT/target/release/andromedad
EOF
cat > "$DBUS_DIR/io.github.incept0s.Andromeda.Panel.service" <<EOF
[D-BUS Service]
Name=io.github.incept0s.Andromeda.Panel
Exec=$ROOT/target/release/andromeda --hidden
EOF
reload_dbus

# Symlink, so edits to extension.js need no copy (still need a re-login or a
# nested session to reload on Wayland).
glib-compile-schemas --strict "$ROOT/extension/$UUID/schemas"
mkdir -p "$(dirname "$EXT_DIR")"
ln -sfn "$ROOT/extension/$UUID" "$EXT_DIR"
echo "Done. Extension linked at $EXT_DIR (not enabled)."
