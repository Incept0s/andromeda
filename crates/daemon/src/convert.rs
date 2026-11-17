// SPDX-License-Identifier: GPL-3.0-or-later
//! File converter and media tools, fully local: ffmpeg for images, audio
//! and video, LibreOffice (headless) for documents. Both are tool-style
//! jobs run once per request. Only conversions whose encoder exists are
//! offered: Fedora's `ffmpeg-free` lacks H.264, for example, while RPM
//! Fusion's `ffmpeg` has it.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use andromeda_core::tools::ConvertOption;
use tokio::process::Command;

const IMAGE: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "bmp", "gif", "tif", "tiff", "avif",
];
const AUDIO: &[&str] = &[
    "mp3", "wav", "flac", "ogg", "oga", "m4a", "aac", "opus", "wma",
];
const VIDEO: &[&str] = &[
    "mp4", "mkv", "webm", "mov", "avi", "m4v", "wmv", "flv", "mpg", "mpeg",
];
const DOC: &[&str] = &[
    "docx", "doc", "odt", "rtf", "txt", "pptx", "ppt", "odp", "xlsx", "xls", "ods",
];

#[derive(Default)]
pub struct Tools {
    /// ffmpeg encoder names, read once from `ffmpeg -encoders`.
    encoders: Option<HashSet<String>>,
}

fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
}

fn ext(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// `name.ext`, or `name (1).ext`, ... so nothing is ever overwritten.
fn free_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let mut p = dir.join(format!("{stem}.{ext}"));
    let mut n = 1;
    while p.exists() {
        p = dir.join(format!("{stem} ({n}).{ext}"));
        n += 1;
    }
    p
}

impl Tools {
    async fn encoders(&mut self) -> &HashSet<String> {
        if self.encoders.is_none() {
            let set = match Command::new("ffmpeg")
                .args(["-hide_banner", "-encoders"])
                .output()
                .await
            {
                // Lines look like " V....D libx264   H.264 ...": the name is field 2.
                Ok(o) => String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .filter_map(|l| l.split_whitespace().nth(1).map(str::to_owned))
                    .collect(),
                Err(_) => HashSet::new(),
            };
            self.encoders = Some(set);
        }
        self.encoders.as_ref().expect("set above")
    }

    /// First available encoder of `wanted`, if any.
    async fn pick(&mut self, wanted: &[&'static str]) -> Option<&'static str> {
        let enc = self.encoders().await;
        wanted.iter().copied().find(|w| enc.contains(*w))
    }

    pub async fn options(&mut self, path: &Path) -> Vec<ConvertOption> {
        let e = ext(path);
        let ffmpeg = on_path("ffmpeg");
        let office = on_path("soffice") || on_path("libreoffice");
        let mut v = Vec::new();
        let mut opt = |id: &str, label: &str, note: &str, ok: bool, need: &str| {
            v.push(ConvertOption {
                id: id.into(),
                label: label.into(),
                note: note.into(),
                missing: if ok {
                    String::new()
                } else {
                    format!("Install {need} to enable this")
                },
            });
        };
        if IMAGE.contains(&e.as_str()) {
            let webp = self.pick(&["libwebp"]).await.is_some();
            for (t, label) in [("png", "PNG"), ("jpg", "JPEG")] {
                if e != t {
                    opt(t, &format!("Convert to {label}"), "", ffmpeg, "ffmpeg");
                }
            }
            if e != "webp" {
                opt(
                    "webp",
                    "Convert to WebP",
                    "",
                    ffmpeg && webp,
                    "ffmpeg with WebP",
                );
            }
            opt(
                "compress",
                "Compress",
                "smaller file, slight quality loss",
                ffmpeg,
                "ffmpeg",
            );
            opt(
                "ocr",
                "Extract text",
                "copied to the clipboard",
                on_path("tesseract"),
                "tesseract",
            );
        } else if AUDIO.contains(&e.as_str()) {
            for (t, label, enc) in [
                ("mp3", "MP3", "libmp3lame"),
                ("ogg", "Ogg Vorbis", "libvorbis"),
                ("opus", "Opus", "libopus"),
                ("m4a", "AAC (M4A)", "aac"),
                ("flac", "FLAC", "flac"),
                ("wav", "WAV", "pcm_s16le"),
            ] {
                if e != t {
                    let ok = ffmpeg && self.pick(&[enc]).await.is_some();
                    opt(t, &format!("Convert to {label}"), "", ok, "ffmpeg");
                }
            }
        } else if VIDEO.contains(&e.as_str()) {
            let h264 = self.pick(&["libx264", "libopenh264"]).await.is_some();
            let webm = self
                .pick(&["libvpx-vp9", "libsvtav1", "libaom-av1"])
                .await
                .is_some();
            opt(
                "mp4",
                "Convert to MP4 (H.264)",
                "",
                ffmpeg && h264,
                "ffmpeg with H.264 (RPM Fusion)",
            );
            opt(
                "webm",
                "Convert to WebM",
                "",
                ffmpeg && webm,
                "ffmpeg with VP9 or AV1",
            );
            opt(
                "mkv",
                "Convert to MKV",
                "same streams, no quality loss",
                ffmpeg,
                "ffmpeg",
            );
            opt(
                "compress",
                "Compress video",
                "max 1080p",
                ffmpeg && (h264 || webm),
                "ffmpeg",
            );
            opt("gif", "Make a GIF", "first 15 seconds", ffmpeg, "ffmpeg");
            opt(
                "mp3",
                "Extract audio (MP3)",
                "",
                ffmpeg && self.pick(&["libmp3lame"]).await.is_some(),
                "ffmpeg",
            );
        } else if DOC.contains(&e.as_str()) {
            opt("pdf", "Convert to PDF", "", office, "LibreOffice");
        } else if e == "pdf" {
            opt(
                "docx",
                "Convert to Word (DOCX)",
                "best effort; scanned PDFs need OCR first",
                office,
                "LibreOffice",
            );
        }
        v
    }

    /// Runs one conversion and returns the new file.
    pub async fn convert(&mut self, path: &Path, target: &str) -> Result<PathBuf, String> {
        if !path.is_file() {
            return Err("File not found".into());
        }
        let dir = path.parent().ok_or("no folder")?.to_owned();
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "converted".into());
        let e = ext(path);
        match target {
            "pdf" | "docx" => return office(path, &dir, &stem, target).await,
            "ocr" => return Err("handled by OCR".into()),
            _ => {}
        }

        let (out, args): (PathBuf, Vec<String>) = if IMAGE.contains(&e.as_str()) {
            match target {
                "png" => (free_path(&dir, &stem, "png"), vec![]),
                "jpg" => (free_path(&dir, &stem, "jpg"), s(&["-q:v", "2"])),
                "webp" => (
                    free_path(&dir, &stem, "webp"),
                    s(&["-c:v", "libwebp", "-quality", "90"]),
                ),
                "compress" if self.pick(&["libwebp"]).await.is_some() => (
                    free_path(&dir, &format!("{stem} (compressed)"), "webp"),
                    s(&["-c:v", "libwebp", "-quality", "70"]),
                ),
                "compress" => (
                    free_path(&dir, &format!("{stem} (compressed)"), "jpg"),
                    s(&["-q:v", "8"]),
                ),
                _ => return Err("Unknown conversion".into()),
            }
        } else if AUDIO.contains(&e.as_str()) || target == "mp3" {
            let (codec, extra): (&str, &[&str]) = match target {
                "mp3" => ("libmp3lame", &["-q:a", "2"]),
                "ogg" => ("libvorbis", &["-q:a", "5"]),
                "opus" => ("libopus", &["-b:a", "128k"]),
                "m4a" => ("aac", &["-b:a", "192k"]),
                "flac" => ("flac", &[]),
                "wav" => ("pcm_s16le", &[]),
                _ => return Err("Unknown conversion".into()),
            };
            let mut a = s(&["-vn", "-c:a", codec]);
            a.extend(s(extra));
            (free_path(&dir, &stem, target), a)
        } else {
            match target {
                "mkv" => (free_path(&dir, &stem, "mkv"), s(&["-c", "copy"])),
                "mp4" | "compress" if self.pick(&["libx264", "libopenh264"]).await.is_some() => {
                    let enc = self
                        .pick(&["libx264", "libopenh264"])
                        .await
                        .unwrap_or("libx264");
                    let crf = if target == "compress" { "28" } else { "23" };
                    let mut a = if enc == "libx264" {
                        s(&["-c:v", "libx264", "-crf", crf, "-preset", "medium"])
                    } else {
                        s(&["-c:v", enc, "-b:v", "4M"])
                    };
                    a.extend(s(&[
                        "-c:a",
                        "aac",
                        "-b:a",
                        "160k",
                        "-movflags",
                        "+faststart",
                    ]));
                    if target == "compress" {
                        a.extend(s(&["-vf", "scale='min(1920,iw)':-2"]));
                        (free_path(&dir, &format!("{stem} (compressed)"), "mp4"), a)
                    } else {
                        (free_path(&dir, &stem, "mp4"), a)
                    }
                }
                "webm" | "compress" => {
                    let enc = self
                        .pick(&["libvpx-vp9", "libsvtav1", "libaom-av1"])
                        .await
                        .ok_or("No WebM encoder")?;
                    let mut a = s(&["-c:v", enc, "-crf", "34", "-b:v", "0", "-c:a", "libopus"]);
                    if target == "compress" {
                        a.extend(s(&["-vf", "scale='min(1920,iw)':-2"]));
                        (free_path(&dir, &format!("{stem} (compressed)"), "webm"), a)
                    } else {
                        (free_path(&dir, &stem, "webm"), a)
                    }
                }
                "gif" => (
                    free_path(&dir, &stem, "gif"),
                    // Two-pass palette in one filter graph: much better colors.
                    s(&[
                        "-t",
                        "15",
                        "-vf",
                        "fps=12,scale=480:-1:flags=lanczos,split[a][b];[a]palettegen[p];[b][p]paletteuse",
                        "-loop",
                        "0",
                    ]),
                ),
                _ => return Err("Unknown conversion".into()),
            }
        };

        let status = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
            .arg(path)
            .args(&args)
            .arg(&out)
            .output()
            .await
            .map_err(|_| "Install ffmpeg to convert media.".to_owned())?;
        if !status.status.success() {
            let _ = std::fs::remove_file(&out);
            return Err(String::from_utf8_lossy(&status.stderr)
                .lines()
                .last()
                .unwrap_or("Conversion failed")
                .to_owned());
        }
        Ok(out)
    }
}

fn s(a: &[&str]) -> Vec<String> {
    a.iter().map(|x| (*x).to_owned()).collect()
}

/// LibreOffice in its own temporary profile (so it also works while the
/// user has LibreOffice open) and a temporary output folder (it would
/// silently overwrite files otherwise).
async fn office(path: &Path, dir: &Path, stem: &str, target: &str) -> Result<PathBuf, String> {
    let bin = if on_path("soffice") {
        "soffice"
    } else {
        "libreoffice"
    };
    let tmp = std::env::temp_dir().join(format!("andromeda-office-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp);
    let profile = format!("-env:UserInstallation=file://{}/profile", tmp.display());
    let mut cmd = Command::new(bin);
    cmd.arg(&profile).arg("--headless");
    if target == "docx" {
        cmd.args([
            "--infilter=writer_pdf_import",
            "--convert-to",
            "docx:MS Word 2007 XML",
        ]);
    } else {
        cmd.args(["--convert-to", "pdf"]);
    }
    let out = cmd
        .arg("--outdir")
        .arg(&tmp)
        .arg(path)
        .output()
        .await
        .map_err(|_| "Install LibreOffice to convert documents.".to_owned())?;
    let produced = tmp.join(format!("{stem}.{target}"));
    if !out.status.success() || !produced.exists() {
        return Err("LibreOffice could not convert this file.".into());
    }
    let dest = free_path(dir, stem, target);
    // rename fails across filesystems (/tmp is tmpfs), so copy then delete.
    std::fs::copy(&produced, &dest).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&produced);
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_overwrites() {
        let dir = std::env::temp_dir().join(format!("andromeda-conv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.png"), b"x").unwrap();
        std::fs::write(dir.join("a (1).png"), b"x").unwrap();
        assert_eq!(free_path(&dir, "a", "png"), dir.join("a (2).png"));
        assert_eq!(free_path(&dir, "b", "png"), dir.join("b.png"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
