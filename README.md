# Andromeda

A system monitor, per-app volume mixer and everyday tools behind one
top-bar icon in GNOME, with an Alt+Tab switcher that shows large window
previews. Built in Rust for Fedora GNOME on Wayland.

**Inspired by Vorssaint Utils.**
Not affiliated with or endorsed by Vorssaint.

## What it does

- **Top bar:** plain numbers next to the icon (default: CPU temperature,
  memory %, battery watts). Choose and reorder them in Settings → Top bar.
- **Panel** (click the icon, or Super+A once shortcuts land): tabs for
  - **Keep awake:** stops suspend and screen blanking for a while, ends by
    itself on a timer or when the battery runs low.
  - **Volume mixer:** volume per app (boost above 100 %), mute, output
    device per app, system output and microphone.
  - **System:** CPU, GPU (AMD, NVIDIA; Intel untested), memory and
    pressure, temperatures, fans, battery health, uptime, graphs.
  - **Network:** live speed, session totals, interfaces and addresses, apps
    with open connections, an on-demand speed test (Cloudflare).
  - **Disks:** space, read/write speed, drive health (SMART), eject.
  - **Energy:** battery and power flow, power mode, accessory batteries,
    the apps using the most CPU.
  - **Tools:** clipboard history, copy text from screen (OCR, QR codes),
    color picker, file converter (images, audio, video, documents), cleaner,
    packages (RPM and Flatpak), and a shelf for files.
- **Alerts:** notifications when any reading stays past a limit you set.
- **App switcher:** Alt+Tab with one large live preview per window,
  minimized windows included, grouped by app.

Every feature can be switched off in Settings → Features. Missing hardware
or tools only hide what needs them; Settings → System check lists what was
found.

## Install

1. Download the latest `andromeda-*.rpm` from
   [Releases](https://github.com/incept0s/andromeda/releases).
2. Install it:

   ```sh
   sudo dnf install ./andromeda-*.rpm
   ```

3. Log out and back in. On Wayland GNOME only loads new extensions at
   login.
4. Open the **Extensions** app and switch on **Andromeda**.

The Andromeda icon appears in the top bar. The background service starts
at login on its own (Settings → General → Start at login).

Updates: download the new RPM from Releases and install it the same way.

### Build from source

```sh
git clone https://github.com/incept0s/andromeda
cd andromeda
scripts/install.sh
```

The script installs the build tools it needs (Rust, the WebAssembly
target, Trunk, WebKitGTK headers), builds an RPM and installs it.

## Uninstall

```sh
sudo dnf remove andromeda
```

Settings and clipboard history stay in
`~/.config/io.github.incept0s.Andromeda` and
`~/.local/share/io.github.incept0s.Andromeda`; delete those folders to
remove everything.

## If the desktop misbehaves

The extension runs inside GNOME Shell, so a bug there can affect the
whole desktop. To recover:

1. Press **Ctrl+Alt+F3** and log in on the text console.
2. Stop the background service. Alt+Tab immediately goes back to GNOME's
   own switcher (the Andromeda switcher only runs while the service does):

   ```sh
   systemctl --user stop andromedad
   ```

3. If the desktop is still not usable, turn the extension off:

   ```sh
   gnome-extensions disable andromeda@incept0s.github.io
   ```

   If that command hangs, remove the package instead:
   `sudo dnf remove andromeda`.
4. Press **Ctrl+Alt+F2** (or F1) to go back, then log out and in.

Please [open an issue](https://github.com/incept0s/andromeda/issues) with
the output of `journalctl --user -b | grep -i andromeda`.

## Privacy

Everything stays on your computer. The network is used only for things
you start yourself: the speed test (Cloudflare) and package searches,
installs and updates (your configured repositories and Flathub). Clipboard
history is stored only on disk, readable by you alone, and never includes
entries password managers mark as secret.

## How it is built

- `andromedad`: a small Rust service that reads `/proc`, `/sys`, NVML
  (NVIDIA), PipeWire, UDisks2 and other D-Bus services, and does all the
  work. It runs all the time and uses about 30 MB.
- `andromeda`: the panel, a [Tauri](https://tauri.app) app whose
  interface is written in Rust ([Leptos](https://leptos.dev), compiled to
  WebAssembly). It only runs while you use it and quits a few minutes
  after it is hidden.
- A GNOME Shell extension (JavaScript, the only language GNOME Shell
  accepts) that shows the icon and numbers, places the panel, forwards
  clipboard copies and provides the switcher. It collects no data itself.

Development notes and the full feature specification are in
[docs/SPEC.md](docs/SPEC.md). Run `cargo run -p andromeda-core --example
probe` to see every sensor Andromeda finds; please include it in hardware
bug reports.

## Known limits

- Intel GPUs are untested; reports welcome.
- Linux has no per-app energy figure, so the Energy tab ranks apps by CPU
  use.
- Per-app network traffic needs root on Linux, so the Network tab counts
  open connections instead.
- Files can be dropped onto the shelf but not yet dragged back out; use
  Copy and paste them in Files.
- Volume boost above 100 % is software amplification and can distort loud
  audio (Settings → Volume mixer → Maximum volume).

## License and credits

GPL-3.0-or-later. See [LICENSE](LICENSE).

Parts adapted from other free software, kept under their licenses:

- [Vorssaint Utils](https://github.com/vorssaint/vorssaint-utils) by
  Vorssaint (GPL-3.0-or-later): the sustained-alert logic
  (`crates/daemon/src/alerts.rs`) and the cleaner's pre-selection policy
  (`crates/daemon/src/cleaner.rs`) are adapted and modified. The rest is a
  new implementation that follows its features and layout. The Vorssaint
  name, logo and look belong to Vorssaint and are not used.
- [GNOME Shell](https://gitlab.gnome.org/GNOME/gnome-shell)
  (GPL-2.0-or-later): the switcher builds on its window switcher, and the
  window-clone helper in `extension/.../switcher.js` is adapted from it.
- [Lucide](https://lucide.dev) icons (ISC license).
