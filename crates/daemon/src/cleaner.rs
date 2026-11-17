// SPDX-License-Identifier: GPL-3.0-or-later
//
// The pre-check policy (strong evidence + cheap to rebuild = checked;
// guesses = unchecked) follows Vorssaint Utils' CleanerPolicy.swift,
// Copyright (C) 2026 Vorssaint, GPL-3.0-or-later. Modified: rewritten for
// Linux paths and tools.

//! Cleaner: finds caches, Trash, old screenshots, leftovers of removed
//! Flatpak apps and (with the admin password) system caches. Nothing is
//! deleted without the user seeing the list first: `scan()` only reports,
//! `run(ids)` deletes exactly the scanned items the user kept checked.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use andromeda_core::tools::CleanItem;
use tokio::process::Command;

use crate::config_file::APP_ID;

/// What deleting an item means. Built only by `scan`, never from client
/// input, so a client can only pick among these by id.
#[derive(Clone)]
enum Action {
    /// Delete everything inside the folder, keep the folder.
    EmptyDir(PathBuf),
    /// Delete these files (old screenshots).
    Files(Vec<PathBuf>),
    EmptyTrash,
    FlatpakUnused,
    /// A fixed shell command run as root through pkexec.
    System(&'static str),
}

/// ~/.cache entries that must never be offered: losing them breaks or
/// signs things out, or they belong to us.
const NEVER: &[&str] = &[APP_ID, "keyring", "dconf", "ibus", "flatpak", "evolution", "doc", "gnome-desktop-thumbnailer"];
/// Caches that rebuild themselves without anyone noticing: pre-checked.
const SAFE: &[&str] = &[
    "thumbnails", "mesa_shader_cache", "mesa_shader_cache_db", "nvidia", "radv_builtin_shaders", "fontconfig",
    "pip", "go-build", "yarn", "pnpm", "mozilla", "google-chrome", "chromium", "BraveSoftware", "vivaldi",
    "gstreamer-1.0", "babl", "gegl-0.4", "typescript", "node-gyp", "electron",
];

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

/// Total size of a tree, without following symlinks.
pub fn dir_size(p: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(p) else { return 0 };
    if meta.is_dir() {
        fs::read_dir(p).into_iter().flatten().flatten().map(|e| dir_size(&e.path())).sum()
    } else {
        meta.len()
    }
}

#[derive(Default)]
pub struct Cleaner {
    actions: Vec<Action>,
}

impl Cleaner {
    /// Blocking: walks folders. Call from `spawn_blocking`.
    pub fn scan(&mut self, installed_flatpaks: &[String]) -> Vec<CleanItem> {
        let mut items = Vec::new();
        self.actions.clear();
        let h = home();
        let mut add = |items: &mut Vec<CleanItem>, cat: &str, label: String, detail: String, size: u64, checked: bool, system: bool, a: Action| {
            if size == 0 && !matches!(a, Action::FlatpakUnused | Action::System(_)) {
                return;
            }
            items.push(CleanItem { id: self.actions.len() as u32, category: cat.into(), label, detail, size: size as f64, checked, system });
            self.actions.push(a);
        };

        for e in fs::read_dir(h.join(".cache")).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if NEVER.contains(&name.as_str()) || !e.path().is_dir() {
                continue;
            }
            let size = dir_size(&e.path());
            let safe = SAFE.iter().any(|s| name.starts_with(s));
            add(&mut items, "App caches", name.clone(), format!("~/.cache/{name}"), size, safe, false, Action::EmptyDir(e.path()));
        }

        for (rel, label) in [
            (".cargo/registry/cache", "Rust crate downloads"),
            (".npm/_cacache", "npm cache"),
            (".gradle/caches", "Gradle cache"),
        ] {
            let p = h.join(rel);
            add(&mut items, "Developer", label.into(), format!("~/{rel}"), dir_size(&p), true, false, Action::EmptyDir(p));
        }

        // Data of Flatpak apps that are no longer installed: strong
        // evidence, but it is user data, so it waits for the user's eye.
        for e in fs::read_dir(h.join(".var/app")).into_iter().flatten().flatten() {
            let id = e.file_name().to_string_lossy().into_owned();
            if !installed_flatpaks.is_empty() && !installed_flatpaks.contains(&id) {
                let size = dir_size(&e.path());
                add(&mut items, "Leftovers", id.clone(), format!("~/.var/app/{id} (app removed)"), size, false, false, Action::EmptyDir(e.path()));
            }
        }

        let trash = h.join(".local/share/Trash");
        add(&mut items, "Trash", "Empty the Trash".into(), "Files you deleted earlier".into(), dir_size(&trash.join("files")), false, false, Action::EmptyTrash);

        let month = SystemTime::now() - Duration::from_secs(30 * 86_400);
        let shots: Vec<PathBuf> = fs::read_dir(h.join("Pictures/Screenshots"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t < month))
            .map(|e| e.path())
            .collect();
        let size = shots.iter().map(|p| dir_size(p)).sum();
        add(&mut items, "Old screenshots", format!("{} screenshots older than 30 days", shots.len()), "~/Pictures/Screenshots".into(), size, false, false, Action::Files(shots));

        if !installed_flatpaks.is_empty() {
            add(&mut items, "Flatpak", "Unused runtimes".into(), "Runtimes no installed app needs".into(), 0, false, false, Action::FlatpakUnused);
        }

        let sys = [
            ("Package cache (dnf)", "/var/cache/libdnf5", "dnf clean all"),
            ("System logs older than 2 weeks", "/var/log/journal", "journalctl --vacuum-time=2weeks"),
            ("Crash reports", "/var/lib/systemd/coredump", "rm -f /var/lib/systemd/coredump/*"),
        ];
        for (label, path, cmd) in sys {
            add(&mut items, "System (needs password)", label.into(), path.into(), dir_size(Path::new(path)), false, true, Action::System(cmd));
        }
        items
    }

    /// Deletes the chosen items. Returns how many bytes were freed (user
    /// items only; system sizes are not re-measured).
    pub async fn run(&self, ids: &[u32]) -> Result<u64, String> {
        let mut freed = 0;
        let mut system = Vec::new();
        for id in ids {
            let Some(a) = self.actions.get(*id as usize).cloned() else { continue };
            match a {
                Action::EmptyDir(p) => {
                    freed += tokio::task::spawn_blocking(move || empty_dir(&p)).await.unwrap_or(0);
                }
                Action::Files(files) => {
                    for f in files {
                        freed += fs::symlink_metadata(&f).map(|m| m.len()).unwrap_or(0);
                        let _ = Command::new("gio").arg("trash").arg(&f).status().await;
                    }
                }
                Action::EmptyTrash => {
                    freed += dir_size(&home().join(".local/share/Trash/files"));
                    let _ = Command::new("gio").args(["trash", "--empty"]).status().await;
                }
                Action::FlatpakUnused => {
                    let _ = Command::new("flatpak").args(["uninstall", "--unused", "-y", "--noninteractive"]).status().await;
                }
                Action::System(cmd) => system.push(cmd),
            }
        }
        if !system.is_empty() {
            // One password prompt for all system items. Only the fixed
            // commands above are ever joined here.
            let script = system.join("; ");
            let st = Command::new("pkexec").args(["/bin/sh", "-c", &script]).status().await.map_err(|e| e.to_string())?;
            if !st.success() {
                return Err("System cleanup was cancelled or failed.".into());
            }
        }
        Ok(freed)
    }
}

/// Removes the contents of `dir` (not following symlinks out of it).
fn empty_dir(dir: &Path) -> u64 {
    let mut freed = 0;
    for e in fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        let size = dir_size(&p);
        let ok = match fs::symlink_metadata(&p) {
            Ok(m) if m.is_dir() => fs::remove_dir_all(&p).is_ok(),
            Ok(_) => fs::remove_file(&p).is_ok(),
            Err(_) => false,
        };
        if ok {
            freed += size;
        }
    }
    freed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_dir_keeps_the_folder_and_symlink_targets() {
        let base = std::env::temp_dir().join(format!("andromeda-clean-{}", std::process::id()));
        let (dir, outside) = (base.join("cache"), base.join("outside"));
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(dir.join("sub/f"), b"12345").unwrap();
        fs::write(outside.join("keep"), b"x").unwrap();
        assert_eq!(dir_size(&dir), 5);
        std::os::unix::fs::symlink(&outside, dir.join("link")).unwrap();
        empty_dir(&dir);
        assert!(dir.exists() && fs::read_dir(&dir).unwrap().next().is_none());
        assert!(outside.join("keep").exists(), "symlink target must survive");
        fs::remove_dir_all(&base).unwrap();
    }
}
