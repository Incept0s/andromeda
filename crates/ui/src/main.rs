// SPDX-License-Identifier: GPL-3.0-or-later
//! `andromeda`: the panel UI. A thin client: it shows what the daemon
//! (`andromedad`) publishes over D-Bus and never reads the system itself.
//!
//! It also owns the D-Bus name `io.github.incept0s.Andromeda.Panel` so the
//! Shell extension can toggle the panel. Wayland apps cannot place their own
//! windows, so the extension moves the window under the top-bar icon.

mod commands;

use std::sync::Mutex;
use std::time::{Duration, Instant};

use andromeda_core::dbus;
use commands::{DaemonProxy, daemon};
use futures_util::StreamExt;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use zbus::{connection, interface};

/// When the panel was last shown, and when it last hid because it lost focus.
#[derive(Default)]
struct PanelTimes {
    shown: Mutex<Option<Instant>>,
    blur_hidden: Mutex<Option<Instant>>,
    /// While true (Tools tab open) the panel stays when it loses focus, so
    /// files can be dragged onto it from another window.
    sticky: Mutex<bool>,
}

/// Tells the daemon whether anyone is looking, so it can slow down
/// expensive readings while the panel is hidden.
fn report_visible(app: &AppHandle, visible: bool) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Ok(d) = daemon(&app).await {
            let _ = d.panel_visible(visible).await;
        }
    });
}

pub fn hide_panel(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
    report_visible(app, false);
}

#[tauri::command]
fn hide(app: AppHandle) {
    hide_panel(&app);
}

#[tauri::command]
fn close_settings(app: AppHandle) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.close();
    }
}

#[tauri::command]
fn set_sticky(app: AppHandle, sticky: bool) {
    if let Ok(mut g) = app.state::<PanelTimes>().sticky.lock() {
        *g = sticky;
    }
}

fn show_panel(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        if let Ok(mut g) = app.state::<PanelTimes>().shown.lock() {
            *g = Some(Instant::now());
        }
        let _ = w.show();
        let _ = w.set_focus();
    }
    report_visible(app, true);
}

fn toggle_panel(app: &AppHandle) {
    let Some(w) = app.get_webview_window("main") else {
        return;
    };
    if w.is_visible().unwrap_or(false) {
        hide_panel(app);
        return;
    }
    // A click on the top-bar icon can take focus from the panel first, which
    // hides it via the blur handler. Without this guard, that same click
    // would re-open it immediately.
    let recently = app
        .state::<PanelTimes>()
        .blur_hidden
        .lock()
        .ok()
        .and_then(|g| *g);
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
    fn open_settings(&self) {
        let _ = open_settings(self.app.clone());
    }
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
    WebviewWindowBuilder::new(
        &app,
        "settings",
        WebviewUrl::App("index.html#settings".into()),
    )
    .title("Andromeda Settings")
    .inner_size(820.0, 600.0)
    // Same glass as the panel: no GTK frame, our own title bar.
    .decorations(false)
    .transparent(true)
    .resizable(false)
    .theme(Some(tauri::Theme::Dark))
    .build()
    .map(|_| ())
    .map_err(|e| e.to_string())
}

fn panel_visible(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false)
}

/// Registers the panel D-Bus name, then forwards daemon updates to the
/// frontend as events: live values and mixer changes only while the panel
/// is visible (no drawing when hidden), settings and keep-awake always
/// (the Settings window needs them too).
async fn dbus_main(app: AppHandle) -> zbus::Result<()> {
    let conn = connection::Builder::session()
        .serve_at(dbus::PANEL_PATH, Panel { app: app.clone() })
        .name(dbus::PANEL_NAME)
        .build()
        .await?;
    app.manage(conn.clone());

    let d = DaemonProxy::new(&conn).await?;
    let (mut values, mut mixer) = (
        d.receive_values_changed().await,
        d.receive_mixer_changed().await,
    );
    let (mut config, mut awake) = (
        d.receive_config_changed().await,
        d.receive_awake_until_changed().await,
    );
    loop {
        tokio::select! {
            Some(c) = values.next() => if panel_visible(&app) && let Ok(v) = c.get().await {
                let _ = app.emit_to("main", "values", v);
            },
            Some(c) = mixer.next() => if panel_visible(&app)
                && let Ok(s) = c.get().await
                && let Ok(m) = serde_json::from_str::<andromeda_core::info::MixerState>(&s)
            {
                let _ = app.emit_to("main", "mixer", m);
            },
            Some(c) = config.next() => if let Ok(s) = c.get().await
                && let Ok(cfg) = serde_json::from_str::<andromeda_core::config::Config>(&s)
            {
                let _ = app.emit("config", cfg);
            },
            Some(c) = awake.next() => if let Ok(v) = c.get().await {
                let _ = app.emit("awake", v);
            },
            else => break,
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
            app.get_webview_window(label)
                .and_then(|w| w.is_visible().ok())
                .unwrap_or(false)
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
        .invoke_handler(tauri::generate_handler![
            quit,
            open_settings,
            hide,
            set_sticky,
            close_settings,
            commands::clipboard_list,
            commands::clipboard_copy,
            commands::clipboard_pin,
            commands::clipboard_delete,
            commands::clipboard_clear,
            commands::copy_text,
            commands::shelf_list,
            commands::shelf_add,
            commands::shelf_remove,
            commands::shelf_copy,
            commands::ocr,
            commands::pick_color,
            commands::convert_options,
            commands::convert,
            commands::cleaner_scan,
            commands::cleaner_run,
            commands::pkg_search,
            commands::pkg_action,
            commands::pkg_updates,
            commands::pkg_update_all,
            commands::open_file,
            commands::autostart,
            commands::set_autostart,
            commands::metrics,
            commands::history,
            commands::config,
            commands::set_config,
            commands::awake_until,
            commands::awake_start,
            commands::awake_stop,
            commands::power_profile,
            commands::set_power_profile,
            commands::top_apps,
            commands::interfaces,
            commands::network_apps,
            commands::speed_test,
            commands::drives,
            commands::eject,
            commands::mixer,
            commands::mixer_volume,
            commands::mixer_mute,
            commands::mixer_output,
            commands::mixer_default,
            commands::open_path,
            commands::system_check,
            commands::open_releases,
        ])
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            match event {
                // Closing the panel hides it; only Quit exits.
                WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    hide_panel(window.app_handle());
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
                    let sticky = times.sticky.lock().is_ok_and(|g| *g);
                    if just_shown || sticky || !window.is_visible().unwrap_or(false) {
                        return;
                    }
                    if let Ok(mut g) = times.blur_hidden.lock() {
                        *g = Some(Instant::now());
                    }
                    hide_panel(window.app_handle());
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
