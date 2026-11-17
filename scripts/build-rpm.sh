#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Builds Andromeda and packs it into an RPM under target/rpm/RPMS/.
# Needs: cargo, the wasm32-unknown-unknown target, trunk, rpm-build,
# systemd-rpm-macros and the WebKitGTK/GTK development packages
# (scripts/install.sh installs all of them).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
UUID="andromeda@incept0s.github.io"
APP_ID="io.github.incept0s.Andromeda"

(cd frontend && trunk build --release)
# The UI embeds the frontend at compile time, so it is built after trunk.
cargo build --release --locked -p andromeda -p andromedad

TOP="$ROOT/target/rpm"
STAGE="$TOP/stage/andromeda-$VERSION"
rm -rf "$TOP/stage" && mkdir -p "$STAGE" "$TOP/SOURCES"
u="$STAGE/usr"
install -Dm755 target/release/andromeda "$u/bin/andromeda"
install -Dm755 target/release/andromedad "$u/libexec/andromedad"
install -Dm644 data/systemd/andromedad.service "$u/lib/systemd/user/andromedad.service"
install -Dm644 data/systemd/80-andromeda.preset "$u/lib/systemd/user-preset/80-andromeda.preset"
install -Dm644 "data/dbus/$APP_ID.service" "$u/share/dbus-1/services/$APP_ID.service"
install -Dm644 "data/dbus/$APP_ID.Panel.service" "$u/share/dbus-1/services/$APP_ID.Panel.service"
install -Dm644 "data/$APP_ID.desktop" "$u/share/applications/$APP_ID.desktop"
install -Dm644 data/icons/andromeda.svg "$u/share/icons/hicolor/scalable/apps/$APP_ID.svg"
install -Dm644 data/icons/andromeda-symbolic.svg "$u/share/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg"
mkdir -p "$u/share/gnome-shell/extensions/$UUID"
cp -r "extension/$UUID/." "$u/share/gnome-shell/extensions/$UUID/"
glib-compile-schemas --strict "$u/share/gnome-shell/extensions/$UUID/schemas"
install -Dm644 LICENSE "$u/share/licenses/andromeda/LICENSE"
for f in README.md CHANGELOG.md THIRD-PARTY.md THIRD-PARTY-LICENSES.md; do
    install -Dm644 "$f" "$u/share/doc/andromeda/$f"
done

tar -czf "$TOP/SOURCES/andromeda-$VERSION.tar.gz" -C "$TOP/stage" "andromeda-$VERSION"
rpmbuild -bb packaging/andromeda.spec \
    --define "_topdir $TOP" \
    --define "pkg_version $VERSION"
ls "$TOP"/RPMS/*/andromeda-*.rpm
