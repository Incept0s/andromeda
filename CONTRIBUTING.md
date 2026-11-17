# Contributing

Bug reports, hardware reports and patches are welcome.

## Reporting a bug

Please include:

* What you did, what you expected and what happened.
* Your hardware (laptop or desktop, CPU, GPUs).
* The output of `journalctl --user -b | grep -i andromeda`.
* For missing or wrong sensors: the output of
  `cargo run -p andromeda-core --example probe` from a source checkout.

## Building

```bash
rustup target add wasm32-unknown-unknown
cargo install trunk --locked
sudo dnf install webkit2gtk4.1-devel gtk3-devel libsoup3-devel openssl-devel pulseaudio-libs-devel

scripts/dev-setup.sh      # builds everything, links the extension, sets up D-Bus activation
scripts/run.sh            # starts the service and opens the panel
```

`scripts/dev-setup.sh --remove` undoes the development links.

## Layout

| Path | What |
|------|------|
| `crates/core` | Shared code: metric registry, `/proc` and `/sys` parsers, settings, formatting |
| `crates/daemon` | `andromedad`, the background service and its D-Bus interface |
| `crates/ui` | `andromeda`, the Tauri panel (backend: commands that forward to the service) |
| `frontend` | The panel's interface (Leptos, compiled to WebAssembly by Trunk) |
| `extension` | The GNOME Shell extension (top bar, placement, clipboard, Alt+Tab) |
| `data`, `packaging`, `scripts` | Icons, service files, RPM spec, build scripts |

## Rules of the house

* **Rust wherever possible.** JavaScript only in the Shell extension (GNOME
  only runs JavaScript there), HTML and CSS only for the panel's looks.
* **The panel and the extension only display.** All reading of the system
  happens in `andromedad`.
* **Never crash on unusual hardware.** Every sysfs read is fallible; a missing
  sensor simply does not appear. No `unwrap` on system data.
* **Keep the background service cheap.** Anything that costs more than a
  file read runs only while someone looks at it.
* **Parsers get tests.** Add a fixture-based test for any new `/proc` or
  `/sys` parser (see `crates/core/src/sensors.rs`).
* Run `cargo test --workspace --exclude andromeda-frontend` and
  `cargo clippy --workspace` before sending a patch.

## Testing the extension safely

A broken extension can freeze the desktop, so test it in a nested GNOME
session first:

```bash
scripts/nested.sh
```

It opens GNOME Shell in a window with its own settings, so nothing you do
there affects your real session. Close the window to stop it.

## Releases

Bump `version` in `Cargo.toml`, add an entry to `CHANGELOG.md`, commit, then
push a tag `vX.Y.Z`. GitHub Actions builds the RPM and attaches it to the
release.
