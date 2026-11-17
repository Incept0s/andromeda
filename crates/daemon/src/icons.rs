// SPDX-License-Identifier: GPL-3.0-or-later
//! App icons as `data:` URLs. The panel's strict CSP only allows bundled
//! files and data URLs, and the frontend must not read the filesystem, so
//! the daemon finds the icon file and inlines it.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Icon theme folders, most specific first. Only the hicolor fallback theme
/// is searched: every app installs its icon there.
fn roots() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    [
        format!("{home}/.local/share/icons/hicolor"),
        format!("{home}/.local/share/flatpak/exports/share/icons/hicolor"),
        "/var/lib/flatpak/exports/share/icons/hicolor".into(),
        "/usr/share/icons/hicolor".into(),
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect()
}

const SIZES: [&str; 6] = [
    "scalable", "128x128", "64x64", "48x48", "256x256", "512x512",
];

fn find(name: &str) -> Option<PathBuf> {
    if name.starts_with('/') {
        return Some(PathBuf::from(name)).filter(|p| p.exists());
    }
    for root in roots() {
        for size in SIZES {
            for ext in ["svg", "png"] {
                let p = root.join(size).join("apps").join(format!("{name}.{ext}"));
                if p.exists() {
                    return Some(p);
                }
            }
        }
    }
    ["png", "svg"]
        .iter()
        .map(|e| PathBuf::from(format!("/usr/share/pixmaps/{name}.{e}")))
        .find(|p| p.exists())
}

/// Base64 for data URLs (RFC 4648, with padding).
fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= c.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[derive(Default)]
pub struct Icons {
    cache: HashMap<String, String>,
}

impl Icons {
    /// First candidate name that resolves to a file, as a data URL; empty
    /// when none does (the UI then shows a generic glyph).
    pub fn data_url(&mut self, candidates: &[String]) -> String {
        let key = candidates.join("|");
        if let Some(v) = self.cache.get(&key) {
            return v.clone();
        }
        let url = candidates
            .iter()
            .filter(|c| !c.is_empty())
            .find_map(|c| find(c))
            .and_then(|p| {
                let bytes = fs::read(&p).ok()?;
                // Skip absurdly large files; icons are a few KB.
                (bytes.len() < 512 * 1024).then_some((p, bytes))
            })
            .map(|(p, bytes)| {
                let mime = if p.extension().is_some_and(|e| e == "svg") {
                    "image/svg+xml"
                } else {
                    "image/png"
                };
                format!("data:{mime};base64,{}", base64(&bytes))
            })
            .unwrap_or_default();
        self.cache.insert(key, url.clone());
        url
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(super::base64(b""), "");
        assert_eq!(super::base64(b"f"), "Zg==");
        assert_eq!(super::base64(b"fo"), "Zm8=");
        assert_eq!(super::base64(b"foo"), "Zm9v");
        assert_eq!(super::base64(b"foobar"), "Zm9vYmFy");
    }
}
