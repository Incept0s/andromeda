// SPDX-License-Identifier: GPL-3.0-or-later
//! Clipboard history and the shelf, stored by the daemon.
//!
//! On Wayland only the compositor sees every copy, so the Shell extension
//! forwards copied text here (it reads nothing else and stores nothing).
//! The history is saved to `~/.local/share/io.github.incept0s.Andromeda/
//! clipboard.json`, readable by the user only.
//!
//! The shelf keeps links to files (paths, never copies) in `shelf.json`
//! next to it, so it survives a logout. Items leave after the configured
//! number of hours, or as soon as the file is gone.

use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;

use andromeda_core::tools::{ClipEntry, ShelfItem};

use crate::config_file::APP_ID;

/// Unpinned entries kept; pinned ones are never dropped automatically.
const MAX_ENTRIES: usize = 200;
/// Longer copies (whole files pasted as text) are not worth keeping.
const MAX_TEXT: usize = 100_000;

pub fn data_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join(APP_ID))
}

/// Writes `value` as JSON to `name` in the data dir, atomically and with
/// mode 0600: clipboards and file lists hold private things.
fn write_private<T: serde::Serialize + ?Sized>(name: &str, value: &T) {
    let Some(dir) = data_dir() else { return };
    let _ = fs::create_dir_all(&dir);
    let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
    let tmp = dir.join(format!("{name}.tmp"));
    let ok = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .and_then(|mut f| f.write_all(serde_json::to_string(value).unwrap_or_default().as_bytes()));
    if ok.is_ok() {
        let _ = fs::rename(tmp, dir.join(name));
    }
}

#[derive(Default)]
pub struct Clipboard {
    pub entries: Vec<ClipEntry>,
    next_id: u64,
}

impl Clipboard {
    pub fn load() -> Self {
        let entries: Vec<ClipEntry> = data_dir()
            .and_then(|d| fs::read_to_string(d.join("clipboard.json")).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let next_id = entries.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        Self { entries, next_id }
    }

    fn save(&self) {
        write_private("clipboard.json", &self.entries);
    }

    /// Adds a copy. Copying the same text again moves it to the top.
    pub fn add(&mut self, text: &str, now: i64) -> bool {
        if text.trim().is_empty() || text.len() > MAX_TEXT {
            return false;
        }
        if let Some(pos) = self.entries.iter().position(|e| e.text == text) {
            let mut e = self.entries.remove(pos);
            e.time = now;
            self.entries.insert(0, e);
        } else {
            self.entries.insert(
                0,
                ClipEntry {
                    id: self.next_id,
                    text: text.to_owned(),
                    pinned: false,
                    time: now,
                },
            );
            self.next_id += 1;
        }
        // Drop the oldest unpinned entries beyond the limit.
        let mut unpinned = 0;
        self.entries.retain(|e| {
            if e.pinned {
                return true;
            }
            unpinned += 1;
            unpinned <= MAX_ENTRIES
        });
        self.save();
        true
    }

    pub fn text(&self, id: u64) -> Option<String> {
        self.entries
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.text.clone())
    }

    pub fn pin(&mut self, id: u64, pinned: bool) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.pinned = pinned;
            self.save();
        }
    }

    pub fn delete(&mut self, id: u64) {
        self.entries.retain(|e| e.id != id);
        self.save();
    }

    /// Clears everything except pinned entries.
    pub fn clear(&mut self) {
        self.entries.retain(|e| e.pinned);
        self.save();
    }
}

#[derive(Default)]
pub struct Shelf {
    pub items: Vec<ShelfItem>,
}

impl Shelf {
    pub fn load() -> Self {
        let items = data_dir()
            .and_then(|d| fs::read_to_string(d.join("shelf.json")).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Self { items }
    }

    fn save(&self) {
        write_private("shelf.json", &self.items);
    }

    /// Adds a file or folder by absolute path. Adding one that is already
    /// there moves it to the top and restarts its timer.
    pub fn add(&mut self, path: &str, now: i64) -> bool {
        let p = std::path::Path::new(path);
        if !p.is_absolute() {
            return false;
        }
        let Ok(meta) = fs::metadata(p) else {
            return false;
        };
        self.items.retain(|i| i.path != path);
        self.items.insert(
            0,
            ShelfItem {
                path: path.to_owned(),
                name: p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_owned()),
                is_dir: meta.is_dir(),
                size: meta.len() as f64,
                added: now,
            },
        );
        self.save();
        true
    }

    pub fn remove(&mut self, path: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|i| i.path != path);
        let changed = self.items.len() != before;
        if changed {
            self.save();
        }
        changed
    }

    pub fn clear(&mut self) -> bool {
        let changed = !self.items.is_empty();
        self.items.clear();
        if changed {
            self.save();
        }
        changed
    }

    /// Drops items older than `keep_hours` (`None` = keep) and items whose
    /// file no longer exists. Returns true if anything was dropped.
    pub fn prune(&mut self, now: i64, keep_hours: Option<u32>) -> bool {
        let before = self.items.len();
        self.items.retain(|i| {
            let fresh = keep_hours.is_none_or(|h| now - i.added < i64::from(h) * 3600);
            fresh && std::path::Path::new(&i.path).exists()
        });
        let changed = self.items.len() != before;
        if changed {
            self.save();
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedupes_limits_and_keeps_pins() {
        let mut c = Clipboard::default();
        // Point saving at a temp dir so tests never touch the real history.
        unsafe {
            std::env::set_var(
                "XDG_DATA_HOME",
                std::env::temp_dir().join(format!("andromeda-clip-{}", std::process::id())),
            )
        };
        assert!(c.add("a", 1));
        assert!(c.add("b", 2));
        assert!(c.add("a", 3));
        assert_eq!(
            c.entries
                .iter()
                .map(|e| e.text.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert!(!c.add("   ", 4));
        let id_b = c.entries[1].id;
        c.pin(id_b, true);
        for i in 0..(MAX_ENTRIES + 10) {
            c.add(&format!("x{i}"), 10);
        }
        assert!(
            c.entries.iter().any(|e| e.text == "b"),
            "pinned entry survives the limit"
        );
        assert_eq!(c.entries.iter().filter(|e| !e.pinned).count(), MAX_ENTRIES);
        c.clear();
        assert_eq!(c.entries.len(), 1);
    }

    #[test]
    fn shelf_expires_and_forgets_missing_files() {
        let dir = std::env::temp_dir().join(format!("andromeda-shelf-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("XDG_DATA_HOME", dir.join("data")) };
        let (a, b) = (dir.join("a.txt"), dir.join("b.txt"));
        fs::write(&a, "a").unwrap();
        fs::write(&b, "b").unwrap();
        let (a, b) = (a.to_str().unwrap(), b.to_str().unwrap());

        let mut s = Shelf::default();
        assert!(!s.add("relative/path", 0), "relative paths are refused");
        assert!(!s.add("/does/not/exist", 0));
        assert!(s.add(a, 0));
        assert!(s.add(b, 3600 * 20));
        assert!(s.add(a, 3600 * 10), "re-adding moves to the top");
        assert_eq!(s.items.len(), 2);
        assert_eq!(s.items[0].path, a);

        // 25 h after start: b (added at 20 h) stays, a (re-added at 10 h) stays.
        assert!(!s.prune(3600 * 25, Some(24)));
        // 35 h: a is 25 h old and goes.
        assert!(s.prune(3600 * 35, Some(24)));
        assert_eq!(s.items.len(), 1);
        // Never expire, but a deleted file still goes.
        fs::remove_file(b).unwrap();
        assert!(s.prune(i64::MAX / 2, None));
        assert!(s.items.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
