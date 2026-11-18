// SPDX-License-Identifier: GPL-3.0-or-later
//! Long-running work (conversions, OCR, package installs, speed test...).
//!
//! zbus holds an interface's lock while one of its methods runs, so a
//! two-minute conversion inside a method would stall sampling and every
//! other call. Instead `Job(kind, args)` returns an id immediately, the work
//! runs in its own task, and `JobDone(id, ok, result)` reports the end.

use std::path::Path;
use std::sync::Arc;

use tokio::sync::Mutex;
use zbus::object_server::SignalEmitter;

use crate::{cleaner, convert, desktop, drives, netinfo, ocr, packages, portal};

/// Everything jobs need, shared outside the interface lock.
pub struct Ctx {
    pub session: zbus::Connection,
    pub system: Option<zbus::Connection>,
    pub tools: Mutex<convert::Tools>,
    pub cleaner: Mutex<cleaner::Cleaner>,
}

/// Asks the Shell extension to put `text` on the clipboard (only it can,
/// on Wayland, without having focus).
pub async fn set_clipboard(e: &SignalEmitter<'_>, text: &str, mime: &str) {
    let _ = crate::Daemon::set_clipboard(e, text, mime).await;
}

fn system(ctx: &Ctx) -> Result<&zbus::Connection, String> {
    ctx.system
        .as_ref()
        .ok_or_else(|| "The system bus is not available.".to_owned())
}

/// Runs one job; Ok(result) is text or JSON depending on the kind.
pub async fn run(
    ctx: Arc<Ctx>,
    e: &SignalEmitter<'_>,
    kind: &str,
    args: &[String],
) -> Result<String, String> {
    let arg = |i: usize| {
        args.get(i)
            .map(String::as_str)
            .ok_or_else(|| format!("{kind}: missing argument"))
    };
    let json = |v: &dyn erased::Json| v.json();
    match kind {
        "speed_test" => json(&netinfo::speed_test().await?),
        "eject" => drives::eject(system(&ctx)?, arg(0)?)
            .await
            .map(|_| String::new()),
        "ocr" => {
            // The panel is hidden by now, so failures are told by notification.
            let notify_err = |e: String| async {
                if e != "Cancelled" {
                    desktop::notify(&ctx.session, "Could not copy text", &e).await;
                }
                e
            };
            let shot = match portal::screenshot(&ctx.session).await {
                Ok(s) => s,
                Err(e) => return Err(notify_err(e).await),
            };
            let result = ocr::recognise(&shot).await;
            // The capture was only made for this; don't leave it behind.
            let _ = std::fs::remove_file(&shot);
            let text = match result {
                Ok(t) => t,
                Err(e) => return Err(notify_err(e).await),
            };
            set_clipboard(e, &text, "text/plain;charset=utf-8").await;
            let preview: String = text.chars().take(80).collect();
            desktop::notify(&ctx.session, "Text copied", &preview).await;
            Ok(text)
        }
        "pick_color" => {
            let hex = portal::pick_color(&ctx.session).await?;
            set_clipboard(e, &hex, "text/plain;charset=utf-8").await;
            desktop::notify(&ctx.session, "Color copied", &hex).await;
            Ok(hex)
        }
        "convert_options" => json(&ctx.tools.lock().await.options(Path::new(arg(0)?)).await),
        "convert" => {
            let (path, target) = (Path::new(arg(0)?), arg(1)?);
            if target == "ocr" {
                let text = ocr::recognise(path).await?;
                set_clipboard(e, &text, "text/plain;charset=utf-8").await;
                return Ok(String::new());
            }
            let out = ctx.tools.lock().await.convert(path, target).await?;
            Ok(out.to_string_lossy().into_owned())
        }
        "clean_scan" => {
            let flatpaks = packages::flatpak_installed().await;
            let c = ctx.clone();
            let items =
                tokio::task::spawn_blocking(move || c.cleaner.blocking_lock().scan(&flatpaks))
                    .await
                    .map_err(|e| e.to_string())?;
            json(&items)
        }
        "clean_run" => {
            let ids: Vec<u32> = args.iter().filter_map(|a| a.parse().ok()).collect();
            let freed = ctx.cleaner.lock().await.run(&ids).await?;
            Ok(freed.to_string())
        }
        "pkg_search" => match arg(0)? {
            "flatpak" => json(&packages::search_flatpak(arg(1)?).await?),
            _ => json(&packages::search_rpm(system(&ctx)?, arg(1)?).await?),
        },
        "pkg_action" => {
            let (source, action, id) = (arg(0)?, arg(1)?, arg(2)?);
            match (source, action) {
                ("flatpak", a) => packages::flatpak_action(a, id).await?,
                ("rpm", "install") => packages::install_rpm(system(&ctx)?, id).await?,
                ("rpm", "remove") => packages::remove_rpm(system(&ctx)?, id).await?,
                _ => return Err("unknown package action".into()),
            }
            Ok(String::new())
        }
        "pkg_updates" => {
            let mut all = match &ctx.system {
                Some(sys) => packages::rpm_updates(sys).await.unwrap_or_default(),
                None => Vec::new(),
            };
            all.extend(packages::flatpak_updates().await.unwrap_or_default());
            json(&all)
        }
        "pkg_update_all" => {
            let rpms: Vec<String> = packages::rpm_updates(system(&ctx)?)
                .await?
                .into_iter()
                .map(|p| p.id)
                .collect();
            if !rpms.is_empty() {
                packages::update_rpms(system(&ctx)?, &rpms).await?;
            }
            let _ = packages::flatpak_action("update", "all").await;
            Ok(String::new())
        }
        "shelf_copy" => {
            // GNOME Files understands this format: "copy" then file URIs.
            let uris: Vec<String> = args.iter().map(|p| format!("file://{p}")).collect();
            set_clipboard(
                e,
                &format!("copy\n{}", uris.join("\n")),
                "x-special/gnome-copied-files",
            )
            .await;
            Ok(String::new())
        }
        // args: title, "multiple" or "single". Result: JSON list of paths.
        "choose_files" => {
            let paths = portal::choose_files(&ctx.session, arg(0)?, arg(1)? == "multiple").await?;
            json(
                &paths
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect::<Vec<_>>(),
            )
        }
        "copy_text" => {
            set_clipboard(e, arg(0)?, "text/plain;charset=utf-8").await;
            Ok(String::new())
        }
        _ => Err(format!("unknown job {kind}")),
    }
}

/// Small helper so every JSON-returning branch reads the same.
mod erased {
    pub trait Json {
        fn json(&self) -> Result<String, String>;
    }
    impl<T: serde::Serialize> Json for T {
        fn json(&self) -> Result<String, String> {
            serde_json::to_string(self).map_err(|e| e.to_string())
        }
    }
}
