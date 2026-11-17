# Changelog

All notable changes to Andromeda are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

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
