// SPDX-License-Identifier: GPL-3.0-or-later
//! Records for the Tools tab, shared by the daemon and the frontend.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipEntry {
    pub id: u64,
    pub text: String,
    pub pinned: bool,
    /// Unix seconds when it was last copied.
    pub time: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShelfItem {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: f64,
    /// Unix seconds when it was put on the shelf; it leaves after
    /// `config.shelf.keep_hours`.
    #[serde(default)]
    pub added: i64,
}

/// One conversion the converter offers for a given input file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConvertOption {
    /// Passed back to `Convert` (e.g. "webp", "compress", "gif", "pdf").
    pub id: String,
    pub label: String,
    /// Shown next to the option (e.g. "best effort").
    pub note: String,
    /// Empty when available; otherwise "Install X to enable this".
    pub missing: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CleanItem {
    pub id: u32,
    pub category: String,
    pub label: String,
    pub detail: String,
    /// Bytes, when known.
    pub size: f64,
    /// Pre-selected only when deleting is safe and the data rebuilds itself.
    pub checked: bool,
    /// Needs the administrator password.
    pub system: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PkgSource {
    Rpm,
    Flatpak,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Package {
    /// PackageKit package id or Flatpak app id.
    pub id: String,
    pub name: String,
    pub summary: String,
    pub version: String,
    pub installed: bool,
    pub source: PkgSource,
}
