// SPDX-License-Identifier: GPL-3.0-or-later
//! Tauri commands the frontend may call. Each one forwards to the daemon
//! over D-Bus and converts its JSON replies into typed values, so the
//! frontend never sees D-Bus or touches the system.

use std::collections::HashMap;

use andromeda_core::config::Config;
use andromeda_core::info::{Check, Drive, Interface, MixerState, NetApp, SpeedResult};
use andromeda_core::metrics::{Group, MetricInfo, Unit};
use andromeda_core::procs::AppUsage;
use andromeda_core::tools::{CleanItem, ClipEntry, ConvertOption, Package, ShelfItem};
use serde::de::DeserializeOwned;
use tauri::{AppHandle, Manager};
use zbus::proxy;

/// Client side of the daemon's interface. The macro generates `DaemonProxy`
/// with one async method per D-Bus method and `receive_<prop>_changed()`
/// streams for properties.
#[proxy(
    interface = "io.github.incept0s.Andromeda1",
    default_service = "io.github.incept0s.Andromeda",
    default_path = "/io/github/incept0s/Andromeda"
)]
pub trait Daemon {
    #[zbus(property)]
    fn values(&self) -> zbus::Result<HashMap<String, f64>>;
    #[zbus(property)]
    fn config(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn awake_until(&self) -> zbus::Result<i64>;
    #[zbus(property)]
    fn mixer(&self) -> zbus::Result<String>;
    fn metrics(&self) -> zbus::Result<Vec<(String, String, String, String)>>;
    fn history(&self) -> zbus::Result<HashMap<String, Vec<f64>>>;
    fn set_config(&self, json: &str) -> zbus::Result<()>;
    fn awake_start(&self, minutes: u32) -> zbus::Result<()>;
    fn awake_stop(&self) -> zbus::Result<()>;
    fn power_profile(&self) -> zbus::Result<(String, Vec<String>)>;
    fn set_power_profile(&self, profile: &str) -> zbus::Result<()>;
    fn top_apps(&self, limit: u32) -> zbus::Result<Vec<(String, String, f64, f64, u32)>>;
    fn interfaces(&self) -> zbus::Result<String>;
    fn network_apps(&self) -> zbus::Result<String>;
    fn drives(&self) -> zbus::Result<String>;
    fn job(&self, kind: &str, args: &[&str]) -> zbus::Result<u32>;
    #[zbus(signal)]
    fn job_done(&self, id: u32, ok: bool, result: String) -> zbus::Result<()>;
    fn mixer_volume(&self, app: &str, percent: f64) -> zbus::Result<()>;
    fn mixer_mute(&self, app: &str, mute: bool) -> zbus::Result<()>;
    fn mixer_output(&self, app: &str, sink: &str) -> zbus::Result<()>;
    fn mixer_default(&self, kind: &str, name: &str) -> zbus::Result<()>;
    fn system_check(&self) -> zbus::Result<String>;
    fn clipboard_list(&self, query: &str) -> zbus::Result<String>;
    fn clipboard_copy(&self, id: u64) -> zbus::Result<()>;
    fn clipboard_pin(&self, id: u64, pinned: bool) -> zbus::Result<()>;
    fn clipboard_delete(&self, id: u64) -> zbus::Result<()>;
    fn clipboard_clear(&self) -> zbus::Result<()>;
    fn shelf_list(&self) -> zbus::Result<String>;
    fn shelf_add(&self, paths: &[&str]) -> zbus::Result<()>;
    fn shelf_remove(&self, path: &str) -> zbus::Result<()>;
    fn panel_visible(&self, visible: bool) -> zbus::Result<()>;
    fn autostart(&self) -> zbus::Result<bool>;
    fn set_autostart(&self, on: bool) -> zbus::Result<()>;
    fn quit(&self) -> zbus::Result<()>;
}

type R<T> = Result<T, String>;

pub async fn daemon(app: &AppHandle) -> R<DaemonProxy<'static>> {
    let conn = app.try_state::<zbus::Connection>().ok_or("D-Bus not ready")?;
    DaemonProxy::new(&conn).await.map_err(|e| e.to_string())
}

fn err(e: zbus::Error) -> String {
    match e {
        zbus::Error::MethodError(_, Some(msg), _) => msg,
        e => e.to_string(),
    }
}

fn json<T: DeserializeOwned>(s: String) -> R<T> {
    serde_json::from_str(&s).map_err(|e| e.to_string())
}

/// Every metric the daemon knows. Entries with an unknown unit or group
/// (a newer daemon) are skipped rather than failing the whole list.
#[tauri::command]
pub async fn metrics(app: AppHandle) -> R<Vec<MetricInfo>> {
    let list = daemon(&app).await?.metrics().await.map_err(err)?;
    Ok(list
        .into_iter()
        .filter_map(|(id, name, unit, group)| Some(MetricInfo::new(id, name, Unit::parse(&unit)?, Group::parse(&group)?)))
        .collect())
}

#[tauri::command]
pub async fn history(app: AppHandle) -> R<HashMap<String, Vec<f64>>> {
    daemon(&app).await?.history().await.map_err(err)
}

#[tauri::command]
pub async fn config(app: AppHandle) -> R<Config> {
    json(daemon(&app).await?.config().await.map_err(err)?)
}

#[tauri::command]
pub async fn set_config(app: AppHandle, config: Config) -> R<()> {
    let s = serde_json::to_string(&config).map_err(|e| e.to_string())?;
    daemon(&app).await?.set_config(&s).await.map_err(err)
}

#[tauri::command]
pub async fn awake_until(app: AppHandle) -> R<i64> {
    daemon(&app).await?.awake_until().await.map_err(err)
}

#[tauri::command]
pub async fn awake_start(app: AppHandle, minutes: u32) -> R<()> {
    daemon(&app).await?.awake_start(minutes).await.map_err(err)
}

#[tauri::command]
pub async fn awake_stop(app: AppHandle) -> R<()> {
    daemon(&app).await?.awake_stop().await.map_err(err)
}

#[tauri::command]
pub async fn power_profile(app: AppHandle) -> R<(String, Vec<String>)> {
    daemon(&app).await?.power_profile().await.map_err(err)
}

#[tauri::command]
pub async fn set_power_profile(app: AppHandle, profile: String) -> R<()> {
    daemon(&app).await?.set_power_profile(&profile).await.map_err(err)
}

#[tauri::command]
pub async fn top_apps(app: AppHandle, limit: u32) -> R<Vec<AppUsage>> {
    let list = daemon(&app).await?.top_apps(limit).await.map_err(err)?;
    Ok(list
        .into_iter()
        .map(|(key, name, cpu, memory, processes)| AppUsage { key, name, cpu, memory, processes })
        .collect())
}

#[tauri::command]
pub async fn interfaces(app: AppHandle) -> R<Vec<Interface>> {
    json(daemon(&app).await?.interfaces().await.map_err(err)?)
}

#[tauri::command]
pub async fn network_apps(app: AppHandle) -> R<Vec<NetApp>> {
    json(daemon(&app).await?.network_apps().await.map_err(err)?)
}

/// Runs a daemon background job and waits for its `JobDone` signal, so
/// the frontend sees one ordinary call. Subscribing before starting the
/// job means the signal cannot be missed.
pub async fn job(app: &AppHandle, kind: &str, args: &[&str]) -> R<String> {
    use futures_util::StreamExt;
    let d = daemon(app).await?;
    let mut done = d.receive_job_done().await.map_err(err)?;
    let id = d.job(kind, args).await.map_err(err)?;
    while let Some(sig) = done.next().await {
        if let Ok(a) = sig.args()
            && a.id == id
        {
            return if a.ok { Ok(a.result) } else { Err(a.result) };
        }
    }
    Err("The Andromeda service stopped".into())
}

#[tauri::command]
pub async fn speed_test(app: AppHandle) -> R<SpeedResult> {
    json(job(&app, "speed_test", &[]).await?)
}

#[tauri::command]
pub async fn drives(app: AppHandle) -> R<Vec<Drive>> {
    json(daemon(&app).await?.drives().await.map_err(err)?)
}

#[tauri::command]
pub async fn eject(app: AppHandle, drive: String) -> R<()> {
    job(&app, "eject", &[&drive]).await.map(|_| ())
}

#[tauri::command]
pub async fn mixer(app: AppHandle) -> R<MixerState> {
    json(daemon(&app).await?.mixer().await.map_err(err)?)
}

#[tauri::command]
pub async fn mixer_volume(app: AppHandle, key: String, percent: f64) -> R<()> {
    daemon(&app).await?.mixer_volume(&key, percent).await.map_err(err)
}

#[tauri::command]
pub async fn mixer_mute(app: AppHandle, key: String, mute: bool) -> R<()> {
    daemon(&app).await?.mixer_mute(&key, mute).await.map_err(err)
}

#[tauri::command]
pub async fn mixer_output(app: AppHandle, key: String, sink: String) -> R<()> {
    daemon(&app).await?.mixer_output(&key, &sink).await.map_err(err)
}

#[tauri::command]
pub async fn mixer_default(app: AppHandle, kind: String, name: String) -> R<()> {
    daemon(&app).await?.mixer_default(&kind, &name).await.map_err(err)
}

/// Opens a folder (a disk's mount point) in the file manager.
#[tauri::command]
pub fn open_path(path: String) -> R<()> {
    let p = std::path::Path::new(&path);
    // Only existing absolute folders: this must not become a way to run things.
    if !p.is_absolute() || !p.is_dir() {
        return Err("not a folder".into());
    }
    std::process::Command::new("gio").arg("open").arg(p).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn system_check(app: AppHandle) -> R<Vec<Check>> {
    json(daemon(&app).await?.system_check().await.map_err(err)?)
}

/// Opens the project's releases page in the browser (fixed URL only).
#[tauri::command]
pub fn open_releases() -> R<()> {
    std::process::Command::new("gio")
        .args(["open", "https://github.com/incept0s/andromeda/releases"])
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

// ---------- Tools ----------

#[tauri::command]
pub async fn clipboard_list(app: AppHandle, query: String) -> R<Vec<ClipEntry>> {
    json(daemon(&app).await?.clipboard_list(&query).await.map_err(err)?)
}

#[tauri::command]
pub async fn clipboard_copy(app: AppHandle, id: u64) -> R<()> {
    daemon(&app).await?.clipboard_copy(id).await.map_err(err)
}

#[tauri::command]
pub async fn clipboard_pin(app: AppHandle, id: u64, pinned: bool) -> R<()> {
    daemon(&app).await?.clipboard_pin(id, pinned).await.map_err(err)
}

#[tauri::command]
pub async fn clipboard_delete(app: AppHandle, id: u64) -> R<()> {
    daemon(&app).await?.clipboard_delete(id).await.map_err(err)
}

#[tauri::command]
pub async fn clipboard_clear(app: AppHandle) -> R<()> {
    daemon(&app).await?.clipboard_clear().await.map_err(err)
}

#[tauri::command]
pub async fn copy_text(app: AppHandle, text: String) -> R<()> {
    job(&app, "copy_text", &[&text]).await.map(|_| ())
}

#[tauri::command]
pub async fn shelf_list(app: AppHandle) -> R<Vec<ShelfItem>> {
    json(daemon(&app).await?.shelf_list().await.map_err(err)?)
}

#[tauri::command]
pub async fn shelf_add(app: AppHandle, paths: Vec<String>) -> R<()> {
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    daemon(&app).await?.shelf_add(&refs).await.map_err(err)
}

#[tauri::command]
pub async fn shelf_remove(app: AppHandle, path: String) -> R<()> {
    daemon(&app).await?.shelf_remove(&path).await.map_err(err)
}

/// Puts the file on the clipboard so it can be pasted in Files.
#[tauri::command]
pub async fn shelf_copy(app: AppHandle, path: String) -> R<()> {
    job(&app, "shelf_copy", &[&path]).await.map(|_| ())
}

/// The panel hides first: it must not end up in the capture.
#[tauri::command]
pub async fn ocr(app: AppHandle) -> R<String> {
    crate::hide_panel(&app);
    job(&app, "ocr", &[]).await
}

#[tauri::command]
pub async fn pick_color(app: AppHandle) -> R<String> {
    crate::hide_panel(&app);
    job(&app, "pick_color", &[]).await
}

#[tauri::command]
pub async fn convert_options(app: AppHandle, path: String) -> R<Vec<ConvertOption>> {
    json(job(&app, "convert_options", &[&path]).await?)
}

/// Returns the new file's path (empty for "Extract text").
#[tauri::command]
pub async fn convert(app: AppHandle, path: String, target: String) -> R<String> {
    job(&app, "convert", &[&path, &target]).await
}

#[tauri::command]
pub async fn cleaner_scan(app: AppHandle) -> R<Vec<CleanItem>> {
    json(job(&app, "clean_scan", &[]).await?)
}

/// Returns the bytes freed in the home folder.
#[tauri::command]
pub async fn cleaner_run(app: AppHandle, ids: Vec<u32>) -> R<f64> {
    let ids: Vec<String> = ids.iter().map(u32::to_string).collect();
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    Ok(job(&app, "clean_run", &refs).await?.parse().unwrap_or(0.0))
}

#[tauri::command]
pub async fn pkg_search(app: AppHandle, source: String, query: String) -> R<Vec<Package>> {
    json(job(&app, "pkg_search", &[&source, &query]).await?)
}

#[tauri::command]
pub async fn pkg_action(app: AppHandle, source: String, action: String, id: String) -> R<()> {
    job(&app, "pkg_action", &[&source, &action, &id]).await.map(|_| ())
}

#[tauri::command]
pub async fn pkg_updates(app: AppHandle) -> R<Vec<Package>> {
    json(job(&app, "pkg_updates", &[]).await?)
}

#[tauri::command]
pub async fn pkg_update_all(app: AppHandle) -> R<()> {
    job(&app, "pkg_update_all", &[]).await.map(|_| ())
}

/// Opens a file with its default app (a shelf item).
#[tauri::command]
pub fn open_file(path: String) -> R<()> {
    let p = std::path::Path::new(&path);
    if !p.is_absolute() || !p.exists() {
        return Err("not found".into());
    }
    std::process::Command::new("gio").arg("open").arg(p).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn autostart(app: AppHandle) -> R<bool> {
    daemon(&app).await?.autostart().await.map_err(err)
}

#[tauri::command]
pub async fn set_autostart(app: AppHandle, on: bool) -> R<()> {
    daemon(&app).await?.set_autostart(on).await.map_err(err)
}
