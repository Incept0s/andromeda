// SPDX-License-Identifier: GPL-3.0-or-later
//! Clipboard history and the shelf, stored by the daemon.
//!
//! On Wayland only the compositor sees every copy, so the Shell extension
//! forwards copied text here (it reads nothing else and stores nothing).
//! The history is saved to `~/.local/share/io.github.incept0s.Andromeda/
//! clipboard.json`, readable by the user only.
//!
//! Copied images (PNG only, up to 20 MB, the last 30) are stored as files in
//! `clipboard-images/` next to it, mode 0600, with a small preview kept in
//! the history for the lists.
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
/// Bigger images are skipped (the Shell extension checks too).
pub const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;
/// Unpinned images kept; they count towards MAX_ENTRIES as well.
const MAX_IMAGES: usize = 30;
/// Longest side of an image preview, in pixels.
const THUMB: usize = 160;

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
        if let Some(pos) = self
            .entries
            .iter()
            .position(|e| e.image.is_none() && e.text == text)
        {
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
                    image: None,
                    thumb: String::new(),
                    hash: 0,
                },
            );
            self.next_id += 1;
        }
        self.limit();
        self.save();
        true
    }

    /// Adds a copied image. The same image again moves it to the top.
    pub fn add_image(&mut self, img: ClipImage, now: i64) -> bool {
        if let Some(pos) = self
            .entries
            .iter()
            .position(|e| e.image.is_some() && e.hash == img.hash)
        {
            let mut e = self.entries.remove(pos);
            e.time = now;
            self.entries.insert(0, e);
        } else {
            let Some(dir) = data_dir().map(|d| d.join("clipboard-images")) else {
                return false;
            };
            let path = dir.join(format!("{}.png", self.next_id));
            let written = fs::create_dir_all(&dir)
                .and_then(|_| fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)))
                .and_then(|_| {
                    fs::OpenOptions::new()
                        .write(true)
                        .create(true)
                        .truncate(true)
                        .mode(0o600)
                        .open(&path)
                })
                .and_then(|mut f| f.write_all(&img.png));
            if written.is_err() {
                return false;
            }
            self.entries.insert(
                0,
                ClipEntry {
                    id: self.next_id,
                    text: format!("Image {} × {}", img.width, img.height),
                    pinned: false,
                    time: now,
                    image: Some(path.to_string_lossy().into_owned()),
                    thumb: img.thumb,
                    hash: img.hash,
                },
            );
            self.next_id += 1;
        }
        self.limit();
        self.save();
        true
    }

    /// Drops the oldest unpinned entries beyond the limits, with their files.
    fn limit(&mut self) {
        let (mut unpinned, mut images) = (0, 0);
        let mut dropped = Vec::new();
        self.entries.retain(|e| {
            if e.pinned {
                return true;
            }
            unpinned += 1;
            if e.image.is_some() {
                images += 1;
            }
            let keep = unpinned <= MAX_ENTRIES && (e.image.is_none() || images <= MAX_IMAGES);
            if !keep {
                dropped.push(e.image.clone());
            }
            keep
        });
        dropped.into_iter().flatten().for_each(remove_image);
    }

    pub fn get(&self, id: u64) -> Option<&ClipEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn pin(&mut self, id: u64, pinned: bool) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.pinned = pinned;
            self.save();
        }
    }

    pub fn delete(&mut self, id: u64) {
        if let Some(path) = self.get(id).and_then(|e| e.image.clone()) {
            remove_image(path);
        }
        self.entries.retain(|e| e.id != id);
        self.save();
    }

    /// Clears everything except pinned entries.
    pub fn clear(&mut self) {
        self.entries
            .iter()
            .filter(|e| !e.pinned)
            .filter_map(|e| e.image.clone())
            .for_each(remove_image);
        self.entries.retain(|e| e.pinned);
        self.save();
    }
}

/// Deletes a stored clipboard image, but only inside our own folder.
fn remove_image(path: String) {
    let p = std::path::Path::new(&path);
    if data_dir().is_some_and(|d| p.starts_with(d.join("clipboard-images"))) {
        let _ = fs::remove_file(p);
    }
}

/// A copied PNG, checked and previewed, ready for `Clipboard::add_image`.
pub struct ClipImage {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub thumb: String,
    pub hash: u64,
}

impl ClipImage {
    /// `None` if `png` is not a readable PNG. Reads row by row into a small
    /// box-averaged preview, so even a big screenshot needs little memory.
    pub fn from_png(png: Vec<u8>) -> Option<Self> {
        if png.len() > MAX_IMAGE_BYTES {
            return None;
        }
        let mut dec = png::Decoder::new(std::io::Cursor::new(png.as_slice()));
        dec.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = dec.read_info().ok()?;
        let (width, height) = (reader.info().width, reader.info().height);
        let (w, h) = (width as usize, height as usize);
        let channels = reader.output_color_type().0.samples();
        let scale = (THUMB as f64 / w.max(h) as f64).min(1.0);
        let tw = ((w as f64 * scale).round() as usize).max(1);
        let th = ((h as f64 * scale).round() as usize).max(1);
        // Per preview pixel: summed r, g, b, a and the number of pixels.
        let mut sums = vec![[0u32; 5]; tw * th];
        let mut add_row = |y: usize, row: &[u8]| {
            let ty = y * th / h;
            for (x, px) in row.chunks_exact(channels).take(w).enumerate() {
                let rgba = match channels {
                    1 => [px[0], px[0], px[0], 255],
                    2 => [px[0], px[0], px[0], px[1]],
                    3 => [px[0], px[1], px[2], 255],
                    _ => [px[0], px[1], px[2], px[3]],
                };
                let s = &mut sums[ty * tw + x * tw / w];
                for c in 0..4 {
                    s[c] += u32::from(rgba[c]);
                }
                s[4] += 1;
            }
        };
        if reader.info().interlaced {
            // Rare for copies; decode whole, but not absurd sizes.
            if w * h > 16_000_000 {
                return None;
            }
            let mut buf = vec![0; reader.output_buffer_size()?];
            let info = reader.next_frame(&mut buf).ok()?;
            for y in 0..h {
                add_row(y, buf.get(y * info.line_size..)?);
            }
        } else {
            let mut y = 0;
            while y < h {
                let row = reader.next_row().ok()??;
                add_row(y, row.data());
                y += 1;
            }
        }
        let pixels: Vec<u8> = sums
            .iter()
            .flat_map(|s| {
                let n = s[4].max(1);
                [s[0] / n, s[1] / n, s[2] / n, s[3] / n].map(|v| v as u8)
            })
            .collect();
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, tw as u32, th as u32);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut writer = enc.write_header().ok()?;
            writer.write_image_data(&pixels).ok()?;
            writer.finish().ok()?;
        }
        let hash = {
            use std::hash::{Hash, Hasher};
            let mut h = std::hash::DefaultHasher::new();
            png.hash(&mut h);
            h.finish().max(1)
        };
        Some(Self {
            thumb: format!("data:image/png;base64,{}", base64(&out)),
            png,
            width,
            height,
            hash,
        })
    }
}

/// Standard base64 with padding (for the preview data URLs).
fn base64(data: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ABC[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
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

    /// Tests that point XDG_DATA_HOME somewhere run one at a time.
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn dedupes_limits_and_keeps_pins() {
        let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
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

    /// A 2×2 RGBA PNG.
    fn tiny_png() -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, 2, 2);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().unwrap();
        w.write_image_data(&[255; 16]).unwrap();
        w.finish().unwrap();
        out
    }

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn images_preview_dedupe_and_cleanup() {
        assert!(ClipImage::from_png(b"not a png".to_vec()).is_none());
        let img = ClipImage::from_png(tiny_png()).unwrap();
        assert_eq!((img.width, img.height), (2, 2));
        assert!(img.thumb.starts_with("data:image/png;base64,"));

        let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("andromeda-img-{}", std::process::id()));
        unsafe { std::env::set_var("XDG_DATA_HOME", &dir) };
        let mut c = Clipboard::default();
        assert!(c.add_image(ClipImage::from_png(tiny_png()).unwrap(), 1));
        assert!(
            c.add("Image 2 × 2", 2),
            "same words as text stay a separate entry"
        );
        assert!(c.add_image(ClipImage::from_png(tiny_png()).unwrap(), 3));
        assert_eq!(c.entries.len(), 2, "same image again moves to the top");
        let e = &c.entries[0];
        let path = e.image.clone().unwrap();
        assert!(std::path::Path::new(&path).exists());
        let id = e.id;
        c.delete(id);
        assert!(
            !std::path::Path::new(&path).exists(),
            "file goes with the entry"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn shelf_expires_and_forgets_missing_files() {
        let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
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
