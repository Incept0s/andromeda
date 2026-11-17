// SPDX-License-Identifier: GPL-3.0-or-later
//! "Copy text from screen": recognises text (Tesseract) or a QR code in an
//! image. Tesseract is a tool-style job: one run per capture, never polled.

use std::path::Path;

use tokio::process::Command;

/// QR code content, if the image contains one.
pub fn read_qr(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = png::Decoder::new(std::io::BufReader::new(file)).read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let channels = info.color_type.samples();
    // shortcut: assumes 8-bit samples, which is what GNOME screenshots use.
    let luma = |x: usize, y: usize| -> u8 {
        let i = (y * w + x) * channels;
        match channels {
            1 | 2 => buf[i],
            _ => ((u16::from(buf[i]) * 3 + u16::from(buf[i + 1]) * 6 + u16::from(buf[i + 2])) / 10) as u8,
        }
    };
    let mut img = rqrr::PreparedImage::prepare_from_greyscale(w, h, luma);
    img.detect_grids().into_iter().find_map(|g| g.decode().ok().map(|(_, text)| text))
}

/// Text recognised by Tesseract, or an error explaining what is missing.
pub async fn read_text(path: &Path) -> Result<String, String> {
    let out = Command::new("tesseract")
        .arg(path)
        .args(["stdout", "--psm", "3"])
        .output()
        .await
        .map_err(|_| "Install tesseract and tesseract-langpack-eng to copy text from the screen.".to_owned())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("OCR failed").to_owned());
    }
    // Tesseract ends with a form feed and blank lines.
    Ok(String::from_utf8_lossy(&out.stdout).trim_end_matches(['\n', '\x0c', ' ']).to_owned())
}

/// QR first (exact content), then text recognition.
pub async fn recognise(path: &Path) -> Result<String, String> {
    let p = path.to_owned();
    if let Ok(Some(qr)) = tokio::task::spawn_blocking(move || read_qr(&p)).await {
        return Ok(qr);
    }
    let text = read_text(path).await?;
    if text.trim().is_empty() {
        return Err("No text found in that area.".into());
    }
    Ok(text)
}
