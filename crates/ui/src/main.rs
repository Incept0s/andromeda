// SPDX-License-Identifier: GPL-3.0-or-later
//! `andromeda`: the panel UI. A thin client: it shows what the daemon
//! (`andromedad`) publishes over D-Bus and never reads the system itself.
//!
//! It also owns the D-Bus name `io.github.incept0s.Andromeda.Panel` so the
//! Shell extension can toggle the panel. Wayland apps cannot place their own
//! windows, so the extension moves the window under the top-bar icon.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use andromeda_core::dbus;
use andromeda_core::metrics::{Group, MetricInfo, Unit};
use futures_util::StreamExt;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use zbus::{connection, interface, proxy};

/// Client side of the daemon's interface. The macro generates `DaemonProxy`
/// with typed methods and a `receive_values_changed()` stream.
#[proxy(
    interface = "io.github.incept0s.Andromeda1",
    default_service = "io.github.incept0s.Andromeda",
    default_path = "/io/github/incept0s/Andromeda"
)]
trait Daemon {
    #[zbus(property)]
    fn values(&self) -> zbus::Result<HashMap<String, f64>>;
    fn metrics(&self) -> zbus::Result<Vec<(String, String, String, String)>>;
    fn history(&self) -> zbus::Result<HashMap<String, Vec<f64>>>;
    fn quit(&self) -> zbus::Result<()>;
}

async fn daemon(app: &AppHandle) -> Result<DaemonProxy<'static>, String> {
    let conn = app.try_state::<zbus::Connection>().ok_or("D-Bus not ready")?;
    DaemonProxy::new(&conn).await.map_err(|e| e.to_string())
}

/// When the panel was last shown, and when it last hid because it lost focus.
#[derive(Default)]
struct PanelTimes {
    shown: Mutex<Option<Instant>>,
    blur_hidden: Mutex<Option<Instant>>,
}

fn show_panel(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        if let Ok(mut g) = app.state::<PanelTimes>().shown.lock() {
            *g = Some(Instant::now());
        }
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn toggle_panel(app: &AppHandle) {
    let Some(w) = app.get_webview_window("main") else { return };
    if w.is_visible().unwrap_or(false) {
        let _ = w.hide();
        return;
    }
    // A click on the top-bar icon can take focus from the panel first, which
    // hides it via the blur handler. Without this guard, that same click
    // would re-open it immediately.
    let recently = app.state::<PanelTimes>().blur_hidden.lock().ok().and_then(|g| *g);
    if recently.is_some_and(|t| t.elapsed() < Duration::from_millis(300)) {
        return;
    }
    show_panel(app);
}

/// Server side of the panel interface, called by the Shell extension.
struct Panel {
    app: AppHandle,
}

#[interface(name = "io.github.incept0s.Andromeda.Panel1")]
impl Panel {
    fn toggle(&self) {
        toggle_panel(&self.app);
    }
    fn show(&self) {
        show_panel(&self.app);
    }
}

/// Every metric the daemon knows. Entries with an unknown unit or group
/// (a newer daemon) are skipped rather than failing the whole list.
#[tauri::command]
async fn metrics(app: AppHandle) -> Result<Vec<MetricInfo>, String> {
    let list = daemon(&app).await?.metrics().await.map_err(|e| e.to_string())?;
    Ok(list
        .into_iter()
        .filter_map(|(id, name, unit, group)| {
            Some(MetricInfo::new(id, name, Unit::parse(&unit)?, Group::parse(&group)?))
        })
        .collect())
}

#[tauri::command]
async fn history(app: AppHandle) -> Result<HashMap<String, Vec<f64>>, String> {
    daemon(&app).await?.history().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn quit(app: AppHandle) {
    // Quit means everything stops, daemon included. Errors are ignored: the
    // daemon may already be gone, and we exit either way.
    if let Ok(d) = daemon(&app).await {
        let _ = d.quit().await;
    }
    app.exit(0);
}

#[tauri::command]
fn open_settings(app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("settings") {
        return w.set_focus().map_err(|e| e.to_string());
    }
    WebviewWindowBuilder::new(&app, "settings", WebviewUrl::App("index.html#settings".into()))
        .title("Andromeda Settings")
        .inner_size(720.0, 520.0)
        .build()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Registers the panel D-Bus name, then forwards the daemon's values to the
/// frontend, but only while the panel is visible (no drawing when hidden).
async fn dbus_main(app: AppHandle) -> zbus::Result<()> {
    let conn = connection::Builder::session()
        .serve_at(dbus::PANEL_PATH, Panel { app: app.clone() })
        .name(dbus::PANEL_NAME)
        .build()
        .await?;
    app.manage(conn.clone());

    let daemon = DaemonProxy::new(&conn).await?;
    let mut changes = daemon.receive_values_changed().await;
    while let Some(change) = changes.next().await {
        let visible = app
            .get_webview_window("main")
            .and_then(|w| w.is_visible().ok())
            .unwrap_or(false);
        if visible && let Ok(values) = change.get().await {
            let _ = app.emit_to("main", "values", values);
        }
    }
    Ok(())
}

/// The UI costs ~180 MB even when hidden, while the daemon keeps all data.
/// So after a few minutes hidden, the UI exits; the next click restarts it
/// through D-Bus activation and it fetches history from the daemon.
async fn exit_when_idle(app: AppHandle) {
    const CHECK: Duration = Duration::from_secs(30);
    const LIMIT: u32 = 10; // 10 x 30 s = 5 minutes
    let mut hidden_checks = 0;
    loop {
        tokio::time::sleep(CHECK).await;
        let visible = |label| {
            app.get_webview_window(label).and_then(|w| w.is_visible().ok()).unwrap_or(false)
        };
        if visible("main") || visible("settings") {
            hidden_checks = 0;
        } else {
            hidden_checks += 1;
            if hidden_checks >= LIMIT {
                app.exit(0);
            }
        }
    }
}

fn main() {
    // D-Bus activation starts us with --hidden; a launch from the app grid
    // shows the panel straight away.
    let start_hidden = std::env::args().any(|a| a == "--hidden");

    tauri::Builder::default()
        // Must be the first plugin: a second launch exits early and pings us.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if !argv.iter().any(|a| a == "--hidden") {
                show_panel(app);
            }
        }))
        .manage(PanelTimes::default())
        .invoke_handler(tauri::generate_handler![metrics, history, quit, open_settings])
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            match event {
                // Closing the panel hides it; only Quit exits.
                WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    let _ = window.hide();
                }
                // Behave like a popup: clicking elsewhere hides it.
                WindowEvent::Focused(false) => {
                    let times = window.state::<PanelTimes>();
                    // Right after showing, Mutter can focus the window and
                    // take focus back within a second (seen when launched
                    // from a terminal). Hiding then would make the panel
                    // vanish before the user sees it.
                    let just_shown = times
                        .shown
                        .lock()
                        .ok()
                        .and_then(|g| *g)
                        .is_some_and(|t| t.elapsed() < Duration::from_millis(1500));
                    if just_shown || !window.is_visible().unwrap_or(false) {
                        return;
                    }
                    if let Ok(mut g) = times.blur_hidden.lock() {
                        *g = Some(Instant::now());
                    }
                    let _ = window.hide();
                }
                _ => {}
            }
        })
        .setup(move |app| {
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = dbus_main(handle).await {
                    eprintln!("andromeda: D-Bus failed: {e}");
                }
            });
            tauri::async_runtime::spawn(exit_when_idle(app.handle().clone()));
            if !start_hidden {
                show_panel(app.handle());
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to start Tauri");
}
