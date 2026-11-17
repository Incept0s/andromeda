// SPDX-License-Identifier: GPL-3.0-or-later
//! Clipboard history and the shelf, stored by the daemon.
//!
//! On Wayland only the compositor sees every copy, so the Shell extension
//! forwards copied text here (it reads nothing else and stores nothing).
//! The history is saved to `~/.local/share/io.github.incept0s.Andromeda/
//! clipboard.json`, readable by the user only.

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

    /// Writes with mode 0600: clipboards hold private things.
    fn save(&self) {
        let Some(dir) = data_dir() else { return };
        let _ = fs::create_dir_all(&dir);
        let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
        let tmp = dir.join("clipboard.json.tmp");
        let ok = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .and_then(|mut f| {
                f.write_all(
                    serde_json::to_string(&self.entries)
                        .unwrap_or_default()
                        .as_bytes(),
                )
            });
        if ok.is_ok() {
            let _ = fs::rename(tmp, dir.join("clipboard.json"));
        }
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
    pub fn add(&mut self, path: &str) -> bool {
        let p = std::path::Path::new(path);
        let Ok(meta) = fs::metadata(p) else {
            return false;
        };
        if !p.is_absolute() || self.items.iter().any(|i| i.path == path) {
            return false;
        }
        self.items.push(ShelfItem {
            path: path.to_owned(),
            name: p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_owned()),
            is_dir: meta.is_dir(),
            size: meta.len() as f64,
        });
        true
    }

    pub fn remove(&mut self, path: &str) {
        self.items.retain(|i| i.path != path);
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
}
