#!/usr/bin/env bash
# Starts the current build: stops old copies, starts the daemon, opens the panel.
# Without the Shell extension enabled, GNOME places the panel itself.
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
pkill -x andromeda; pkill -x andromedad
sleep 0.5
"$ROOT/target/release/andromedad" >/dev/null 2>&1 &
sleep 0.5
exec "$ROOT/target/release/andromeda" "$@"
