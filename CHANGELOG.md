# Changelog

All notable changes to Andromeda are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Keep awake can also cover a closed lid (Keep awake tab or Settings →
  Keep awake → "Also with the lid closed"): while it is on, closing the
  laptop no longer suspends it.
- Shelf icon in the top bar (a box, on by default): drop files on it, click
  it to open a small shelf window and drag files out again. Items leave
  after 24 hours (Settings → Top bar: 1 hour, 24 hours or never), a Clear
  button empties the shelf, and it survives a logout.
- "Choose files…" (GNOME's file chooser) for the shelf and the converter.
- Optional converter icon in the top bar (off by default): drop a file on it
  and a small converter window opens with that file. Converted files can be
  put on the shelf with one click.
- Clipboard history keeps copied images too (PNG, the last 30, up to 20 MB
  each, stored readable only by you), with previews.
- Optional clipboard icon in the top bar (off by default): a small window
  with the history; click an entry to make it the clipboard again, the
  window closes so you can paste.

- Keyboard shortcuts (Settings → Shortcuts, all changeable): copy text from
  screen (Super+Shift+T), pick a color (Super+Shift+C), keep awake on/off
  (Super+Shift+K), microphone on/off (Super+Shift+M), and the panel
  (Super+A). Keep awake and the microphone show GNOME's on-screen display.

### Changed

- Alt+Tab shows every window's title under its preview, not only the
  selected one. Andromeda's shelf, converter and clipboard windows are left
  out of it, like the panel.
- Top bar: the numbers are their own item and can sit right of the clock
  (before the open apps) or at the far left of the bar (Settings → Top bar →
  Position). The Andromeda icon moves next to the system icons.

### Fixed

- The file converter could not receive files: drops on the panel were not
  delivered. Files now arrive from the converter icon, the converter
  window's own drop target, or the file chooser.
- Panel and Settings glass were darker than the dash and notifications:
  the tint was painted twice. Corners now match the shell's menus (20 px).

## [0.1.0] - 2026-11-18

First public release.

### Added

- Background service `andromedad`: metric registry from `/proc`, `/sys` and
  NVML, alerts, keep awake, power profile, settings, clipboard history,
  shelf, background jobs.
- Panel with eight tabs: Keep awake, Volume mixer, System, Network, Disks,
  Energy, Tools and Controls.
- Tools: clipboard history, copy text from screen (OCR and QR codes), color
  picker, file converter and media tools, cleaner, packages (RPM and
  Flatpak), shelf.
- Settings window: general, features, top bar, alerts, volume mixer, keep
  awake, system check, about.
- GNOME Shell extension: top-bar icon and numbers, panel placement under the
  icon, Super+A, clipboard capture, Alt+Tab with large window previews.
- RPM packaging, systemd user service, release workflow, install script.
