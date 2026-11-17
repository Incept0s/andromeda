#!/usr/bin/env bash
# Runs a nested GNOME Shell (in a window) with only the Andromeda extension
# enabled. Safe for your real session: it uses its own D-Bus and its own
# settings database (a temporary XDG_CONFIG_HOME), so enabling or crashing
# the extension here changes nothing in your real login.
#
# Run scripts/dev-setup.sh first. Close the nested window to stop.
set -euo pipefail

export XDG_CONFIG_HOME="$(mktemp -d -t andromeda-nested-XXXX)"
echo "Nested settings in $XDG_CONFIG_HOME"

exec dbus-run-session -- bash -c '
    echo "Nested bus: $DBUS_SESSION_BUS_ADDRESS"
    gsettings set org.gnome.shell enabled-extensions "[\"andromeda@incept0s.github.io\"]"
    gsettings set org.gnome.shell welcome-dialog-last-shown-version "999"
    exec gnome-shell --devkit --wayland
'
