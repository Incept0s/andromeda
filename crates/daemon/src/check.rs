// SPDX-License-Identifier: GPL-3.0-or-later
//! Settings → System check: what was detected, what is missing and how to
//! get it. Replaces the macOS permissions page of the original.

use andromeda_core::info::Check;
use andromeda_core::metrics::{Group, MetricInfo};
use zbus::fdo::DBusProxy;

const EXTENSION: &str = "andromeda@incept0s.github.io";

fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
}

async fn has_name(conn: Option<&zbus::Connection>, name: &str) -> bool {
    let Some(conn) = conn else { return false };
    let Ok(dbus) = DBusProxy::new(conn).await else { return false };
    let Ok(n) = name.try_into() else { return false };
    dbus.name_has_owner(n).await.unwrap_or(false)
}

/// GNOME Shell extension state: 1 = enabled/active.
async fn extension_active(session: &zbus::Connection) -> bool {
    let reply = session
        .call_method(
            Some("org.gnome.Shell.Extensions"),
            "/org/gnome/Shell/Extensions",
            Some("org.gnome.Shell.Extensions"),
            "GetExtensionInfo",
            &(EXTENSION,),
        )
        .await;
    let Ok(reply) = reply else { return false };
    let Ok(info) = reply.body().deserialize::<std::collections::HashMap<String, zbus::OwnedVariant>>() else {
        return false;
    };
    info.get("state").and_then(|v| f64::try_from(v.try_clone().ok()?).ok()) == Some(1.0)
}

pub async fn run(
    session: &zbus::Connection,
    system: Option<&zbus::Connection>,
    metrics: &[MetricInfo],
    mixer_connected: bool,
) -> Vec<Check> {
    let c = |name: &str, ok: bool, yes: &str, no: &str| Check {
        name: name.into(),
        ok,
        detail: if ok { yes } else { no }.into(),
    };
    let count = |g: Group| metrics.iter().filter(|m| m.group == g).count();
    let nvidia = metrics.iter().any(|m| m.id.starts_with("gpu.nvidia"));
    let other_gpu = metrics.iter().any(|m| m.group == Group::Gpu && !m.id.starts_with("gpu.nvidia"));
    vec![
        c(
            "GNOME Shell extension",
            extension_active(session).await,
            "Top-bar icon, panel placement, clipboard and app switcher work.",
            "Enable \"Andromeda\" in the Extensions app, then log out and back in once.",
        ),
        c("Audio (PipeWire)", mixer_connected, "Volume mixer available.", "PipeWire is not reachable; the mixer is hidden."),
        c("AMD / Intel GPU", other_gpu, "Read from the kernel (sysfs).", "No AMD or Intel GPU readings found."),
        c("NVIDIA GPU", nvidia, "Read through NVML (NVIDIA driver).", "No NVIDIA driver (NVML) found."),
        Check {
            name: "Sensors".into(),
            ok: count(Group::Temps) + count(Group::Fans) > 0,
            detail: format!("{} temperatures, {} fans.", count(Group::Temps), count(Group::Fans)),
        },
        c("Battery", count(Group::Battery) > 0, "Battery and power readings available.", "No battery found (desktop computer?)."),
        c(
            "Power profiles",
            has_name(system, "org.freedesktop.UPower.PowerProfiles").await,
            "Power mode can be switched.",
            "Install and enable power-profiles-daemon or tuned-ppd.",
        ),
        c("Drive health", has_name(system, "org.freedesktop.UDisks2").await, "SMART data via UDisks2.", "UDisks2 is not running."),
        c("Packages", has_name(system, "org.freedesktop.PackageKit").await || on_path("pkcon"), "Install and remove RPMs.", "PackageKit is not available."),
        c("Flatpak", on_path("flatpak"), "Install and remove Flatpak apps.", "Install flatpak to manage Flatpak apps."),
        c("FFmpeg", on_path("ffmpeg"), "File converter and media tools.", "Install ffmpeg to convert audio, video and images."),
        c("Tesseract", on_path("tesseract"), "Copy text from screen (OCR).", "Install tesseract and tesseract-langpack-eng for OCR."),
        c("LibreOffice", on_path("soffice") || on_path("libreoffice"), "Office documents to PDF and back.", "Install LibreOffice to convert documents."),
        c("curl", on_path("curl"), "Network speed test.", "Install curl for the speed test."),
    ]
}
