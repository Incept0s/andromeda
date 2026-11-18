# Changelog

All notable changes to Andromeda are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Shelf icon in the top bar (a box, on by default): drop files on it, click
  it to open a small shelf window and drag files out again. Items leave
  after 24 hours (Settings → Top bar: 1 hour, 24 hours or never), a Clear
  button empties the shelf, and it survives a logout.
- "Choose files…" (GNOME's file chooser) for the shelf.

### Changed

- Top bar: the numbers are their own item and can sit right of the clock
  (before the open apps) or at the far left of the bar (Settings → Top bar →
  Position). The Andromeda icon moves next to the system icons.

### Fixed

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
