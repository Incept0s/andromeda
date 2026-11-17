# Security

## Reporting a problem

Please report security problems privately through GitHub's
[security advisories](../../security/advisories/new) for this repository,
not in a public issue. Describe what you found and how to reproduce it; you
will get an answer as soon as possible.

## What counts

Andromeda runs as your user and talks to system services, so these matter
most:

* Anything that lets another user, a website or a file you open make
  Andromeda run commands, delete files or read your clipboard history.
* The cleaner deleting something it did not show you first.
* The `pkexec` step (cleaner system items) running anything other than its
  fixed command list.
* Clipboard history being readable by other users, or keeping entries that
  password managers mark as secret.
* The panel loading anything other than its own bundled files.

## Design choices

* The background service and the panel run as your user, never as root.
* Root is only involved through PackageKit and Flatpak (which ask for your
  password themselves) and one `pkexec` call made of fixed commands.
* The cleaner only deletes items from its own last scan, chosen by id; the
  panel cannot send it paths.
* The panel has a strict content security policy and no remote content.
* The GNOME Shell extension stores nothing and runs no commands.
