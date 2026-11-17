// SPDX-License-Identifier: GPL-3.0-or-later
//! `andromedad`: the small always-running part of Andromeda.
//!
//! Owns all data collection and every background job: sampling the metric
//! registry once per second, alerts, keep-awake, settings. Everything is
//! exposed on the session bus as `io.github.incept0s.Andromeda1`; the panel
//! UI and the Shell extension are thin clients of this interface.

mod alerts;
mod check;
mod cleaner;
mod clipboard;
mod config_file;
mod convert;
mod desktop;
mod drives;
mod icons;
mod jobs;
mod mixer;
mod netinfo;
mod nvidia;
mod ocr;
mod packages;
mod portal;

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use andromeda_core::config::{self, Config};
use andromeda_core::topbar;
use andromeda_core::dbus;
use andromeda_core::info::MixerState;
use andromeda_core::metrics::MetricInfo;
use andromeda_core::procs::Procs;
use andromeda_core::sensors::Sensors;
use tokio::sync::Notify;
use zbus::object_server::SignalEmitter;
use zbus::{connection, fdo, interface};

/// Samples kept per metric: one minute at one sample per second.
const HISTORY: usize = 60;

struct Daemon {
    metrics: Vec<MetricInfo>,
    values: HashMap<String, f64>,
    history: HashMap<String, VecDeque<f64>>,
    config: Config,
    /// -1 = off, 0 = until turned off, otherwise the end as Unix seconds.
    awake_until: i64,
    awake_cookie: Option<u32>,
    session: zbus::Connection,
    system: Option<zbus::Connection>,
    procs: Procs,
    mixer: MixerState,
    mixer_tx: Option<mpsc::Sender<mixer::Cmd>>,
    /// Shared with the mixer thread so a settings change applies at once.
    max_volume: Arc<AtomicU32>,
    clipboard: clipboard::Clipboard,
    shelf: clipboard::Shelf,
    jobs: Arc<jobs::Ctx>,
    next_job: u32,
    /// Text shown by the Shell extension next to its icon.
    topbar: String,
    /// The UI reports this so expensive readings (NVML) can slow down
    /// while nobody looks.
    panel_visible: bool,
    quit: Arc<Notify>,
}

fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

impl Daemon {
    fn mixer_cmd(&self, cmd: mixer::Cmd) {
        if let Some(tx) = &self.mixer_tx {
            let _ = tx.send(cmd);
        }
    }

    async fn awake_on(&mut self, minutes: u32) -> fdo::Result<()> {
        if self.awake_cookie.is_none() {
            let sm = desktop::SessionManagerProxy::new(&self.session).await?;
            let cookie = sm
                .inhibit(config_file::APP_ID, 0, "Keep awake is on", desktop::INHIBIT_SUSPEND_AND_IDLE)
                .await?;
            self.awake_cookie = Some(cookie);
        }
        self.awake_until = if minutes == 0 { 0 } else { unix_now() + i64::from(minutes) * 60 };
        Ok(())
    }

    async fn awake_off(&mut self) {
        if let Some(cookie) = self.awake_cookie.take()
            && let Ok(sm) = desktop::SessionManagerProxy::new(&self.session).await
        {
            let _ = sm.uninhibit(cookie).await;
        }
        self.awake_until = -1;
    }

    /// Ends keep-awake when its timer runs out or, unplugged, when the
    /// battery drops below the user's limit (a forgotten session must not
    /// drain the laptop). Returns true if it stopped.
    async fn awake_tick(&mut self) -> bool {
        if self.awake_until < 0 {
            return false;
        }
        let expired = self.awake_until > 0 && unix_now() >= self.awake_until;
        let on_ac = self.values.get("power.ac").is_some_and(|v| *v > 0.0);
        let low = self.config.awake.stop_below_battery.is_some_and(|limit| {
            !on_ac
                && self
                    .values
                    .iter()
                    .any(|(k, v)| k.starts_with("bat.") && k.ends_with(".charge") && self.metrics.iter().any(|m| m.id == k.replace(".charge", ".health")) && *v < f64::from(limit))
        });
        if expired || low {
            self.awake_off().await;
            if low {
                desktop::notify(&self.session, "Keep awake stopped", "The battery is low.").await;
            }
            return true;
        }
        false
    }
}

/// `#[interface]` turns this impl into a D-Bus interface. Properties marked
/// `#[zbus(property)]` get a generated `<name>_changed()` helper that emits
/// `org.freedesktop.DBus.Properties.PropertiesChanged`.
#[interface(name = "io.github.incept0s.Andromeda1")]
impl Daemon {
    /// Latest value per metric id, in base units (bytes, °C, W, Hz, s).
    #[zbus(property)]
    fn values(&self) -> HashMap<String, f64> {
        self.values.clone()
    }

    /// Every metric this machine has: (id, name, unit, group).
    fn metrics(&self) -> Vec<(String, String, String, String)> {
        self.metrics
            .iter()
            .map(|m| (m.id.clone(), m.name.clone(), m.unit.as_str().into(), m.group.as_str().into()))
            .collect()
    }

    /// Recent values per metric, oldest first.
    fn history(&self) -> HashMap<String, Vec<f64>> {
        self.history.iter().map(|(k, v)| (k.clone(), v.iter().copied().collect())).collect()
    }

    /// Top-bar text, e.g. "CPU 42°  RAM 61%  12W"; empty when the feature
    /// is off. Changes at most once per second.
    #[zbus(property)]
    fn top_bar(&self) -> String {
        self.topbar.clone()
    }

    /// Called by the UI when the panel is shown or hidden.
    fn panel_visible(&mut self, visible: bool) {
        self.panel_visible = visible;
    }

    /// Settings as JSON (see `andromeda_core::config::Config`).
    #[zbus(property)]
    fn config(&self) -> String {
        serde_json::to_string(&self.config).unwrap_or_default()
    }

    /// Replaces the settings. Values are sanitized, then saved to disk.
    async fn set_config(&mut self, json: &str, #[zbus(signal_emitter)] e: SignalEmitter<'_>) -> fdo::Result<()> {
        let mut c: Config = serde_json::from_str(json).map_err(|err| fdo::Error::InvalidArgs(err.to_string()))?;
        c.sanitize();
        config_file::save(&c).map_err(|err| fdo::Error::Failed(err.to_string()))?;
        if !c.features.awake && self.awake_until >= 0 {
            self.awake_off().await;
            let _ = self.awake_until_changed(&e).await;
        }
        self.max_volume.store(c.mixer.max_volume, Ordering::Relaxed);
        self.config = c;
        let _ = self.config_changed(&e).await;
        Ok(())
    }

    /// Volume mixer state as JSON (`info::MixerState`).
    #[zbus(property)]
    fn mixer(&self) -> String {
        serde_json::to_string(&self.mixer).unwrap_or_default()
    }

    /// Sets an app's volume in percent (0..=configured maximum).
    fn mixer_volume(&self, app: &str, percent: f64) {
        self.mixer_cmd(mixer::Cmd::Volume(app.into(), percent));
    }

    fn mixer_mute(&self, app: &str, mute: bool) {
        self.mixer_cmd(mixer::Cmd::Mute(app.into(), mute));
    }

    /// Moves all of an app's streams to the output device `sink`.
    fn mixer_output(&self, app: &str, sink: &str) {
        self.mixer_cmd(mixer::Cmd::Output(app.into(), sink.into()));
    }

    /// Sets the system default device; `kind` is "output" or "input".
    fn mixer_default(&self, kind: &str, name: &str) {
        let cmd = if kind == "input" { mixer::Cmd::DefaultInput(name.into()) } else { mixer::Cmd::DefaultOutput(name.into()) };
        self.mixer_cmd(cmd);
    }

    /// -1 = off, 0 = on until turned off, otherwise the end (Unix seconds).
    #[zbus(property)]
    fn awake_until(&self) -> i64 {
        self.awake_until
    }

    /// Keeps the session awake for `minutes` (0 = until turned off).
    async fn awake_start(&mut self, minutes: u32, #[zbus(signal_emitter)] e: SignalEmitter<'_>) -> fdo::Result<()> {
        self.awake_on(minutes).await?;
        let _ = self.awake_until_changed(&e).await;
        Ok(())
    }

    async fn awake_stop(&mut self, #[zbus(signal_emitter)] e: SignalEmitter<'_>) {
        self.awake_off().await;
        let _ = self.awake_until_changed(&e).await;
    }

    /// (active profile, available profiles), e.g. ("balanced",
    /// ["power-saver", "balanced", "performance"]). Empty when
    /// power-profiles-daemon is not running.
    async fn power_profile(&self) -> (String, Vec<String>) {
        let Some(sys) = &self.system else { return Default::default() };
        let Ok(p) = desktop::PowerProfilesProxy::new(sys).await else { return Default::default() };
        let active = p.active_profile().await.unwrap_or_default();
        let list = p
            .profiles()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|m| String::try_from(m.get("Profile")?.try_clone().ok()?).ok())
            .collect();
        (active, list)
    }

    async fn set_power_profile(&self, profile: &str) -> fdo::Result<()> {
        let sys = self.system.as_ref().ok_or_else(|| fdo::Error::Failed("no system bus".into()))?;
        let p = desktop::PowerProfilesProxy::new(sys).await?;
        p.set_active_profile(profile).await?;
        Ok(())
    }

    /// Busiest apps since the previous call: (key, name, cpu %, memory
    /// bytes, process count). Called by the Energy tab while it is visible;
    /// the first call reports 0 % CPU.
    fn top_apps(&mut self, limit: u32) -> Vec<(String, String, f64, f64, u32)> {
        self.procs
            .sample()
            .into_iter()
            .take(limit as usize)
            .map(|a| (a.key, a.name, a.cpu, a.memory, a.processes))
            .collect()
    }

    /// Network interfaces with addresses, as JSON (`Vec<info::Interface>`).
    fn interfaces(&self) -> String {
        serde_json::to_string(&netinfo::interfaces()).unwrap_or_default()
    }

    /// Apps with open connections, as JSON (`Vec<info::NetApp>`).
    async fn network_apps(&self) -> String {
        // Scanning /proc/*/fd takes a few ms; keep it off the async thread.
        let apps = tokio::task::spawn_blocking(netinfo::apps).await.unwrap_or_default();
        serde_json::to_string(&apps).unwrap_or_default()
    }

    /// Drives with SMART data, as JSON (`Vec<info::Drive>`).
    async fn drives(&self) -> fdo::Result<String> {
        let sys = self.system.as_ref().ok_or_else(|| fdo::Error::Failed("no system bus".into()))?;
        let list = drives::list(sys).await?;
        Ok(serde_json::to_string(&list).unwrap_or_default())
    }

    /// Asks the Shell extension to set the clipboard (`mime` is usually
    /// text/plain; files use x-special/gnome-copied-files).
    #[zbus(signal)]
    async fn set_clipboard(e: &SignalEmitter<'_>, text: &str, mime: &str) -> zbus::Result<()>;

    /// A background job finished; `result` is text/JSON or the error.
    #[zbus(signal)]
    async fn job_done(e: &SignalEmitter<'_>, id: u32, ok: bool, result: &str) -> zbus::Result<()>;

    /// Starts long-running work in the background and returns its id at
    /// once (see jobs.rs for the kinds). The end is reported by `JobDone`.
    fn job(&mut self, kind: String, args: Vec<String>) -> u32 {
        self.next_job += 1;
        let id = self.next_job;
        let ctx = self.jobs.clone();
        tokio::spawn(async move {
            let Ok(e) = SignalEmitter::new(&ctx.session, dbus::DAEMON_PATH) else { return };
            let (ok, result) = match jobs::run(ctx.clone(), &e, &kind, &args).await {
                Ok(r) => (true, r),
                Err(err) => (false, err),
            };
            let _ = Daemon::job_done(&e, id, ok, &result).await;
        });
        id
    }

    /// Called by the Shell extension for every text copy.
    fn clipboard_add(&mut self, text: &str) {
        if self.config.features.clipboard {
            self.clipboard.add(text, unix_now());
        }
    }

    /// History as JSON (`Vec<tools::ClipEntry>`), pinned first, filtered
    /// by `query` (case-insensitive; empty = all).
    fn clipboard_list(&self, query: &str) -> String {
        let q = query.to_lowercase();
        let mut v: Vec<_> = self.clipboard.entries.iter().filter(|e| q.is_empty() || e.text.to_lowercase().contains(&q)).cloned().collect();
        v.sort_by_key(|e| !e.pinned);
        serde_json::to_string(&v).unwrap_or_default()
    }

    async fn clipboard_copy(&self, id: u64, #[zbus(signal_emitter)] e: SignalEmitter<'_>) {
        if let Some(t) = self.clipboard.text(id) {
            jobs::set_clipboard(&e, &t, "text/plain;charset=utf-8").await;
        }
    }

    fn clipboard_pin(&mut self, id: u64, pinned: bool) {
        self.clipboard.pin(id, pinned);
    }

    fn clipboard_delete(&mut self, id: u64) {
        self.clipboard.delete(id);
    }

    /// Clears the history except pinned entries.
    fn clipboard_clear(&mut self) {
        self.clipboard.clear();
    }

    /// Shelf contents as JSON (`Vec<tools::ShelfItem>`).
    fn shelf_list(&self) -> String {
        serde_json::to_string(&self.shelf.items).unwrap_or_default()
    }

    fn shelf_add(&mut self, paths: Vec<String>) {
        for p in paths {
            self.shelf.add(&p);
        }
    }

    fn shelf_remove(&mut self, path: &str) {
        self.shelf.remove(path);
    }

    /// Settings → System check, as JSON (`Vec<info::Check>`).
    async fn system_check(&self) -> String {
        let list = check::run(&self.session, self.system.as_ref(), &self.metrics, self.mixer.connected).await;
        serde_json::to_string(&list).unwrap_or_default()
    }

    /// Exits the daemon. The panel's Quit button calls this.
    async fn quit(&mut self) {
        self.awake_off().await;
        self.quit.notify_one();
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> zbus::Result<()> {
    let mut sensors = Sensors::discover(Path::new("/sys"), Path::new("/proc"));
    let nvidia = nvidia::Nvidia::init();
    let mut metrics = sensors.metrics().to_vec();
    if let Some(n) = &nvidia {
        metrics.extend(n.metrics());
    }
    let config = config_file::load();

    let quit = Arc::new(Notify::new());
    let session = connection::Builder::session().build().await?;
    let system = connection::Builder::system().build().await.ok();
    let start_awake = config.features.awake && config.awake.start_with_app;
    let max_volume = Arc::new(AtomicU32::new(config.mixer.max_volume));
    // The mixer thread only exists while the feature is on at startup.
    // shortcut: toggling the feature takes effect after a daemon restart.
    let mixer_parts = config.features.mixer.then(|| {
        let mv = max_volume.clone();
        mixer::spawn(move || f64::from(mv.load(Ordering::Relaxed)))
    });
    let (mixer_tx, mixer_rx) = match mixer_parts {
        Some((tx, rx)) => (Some(tx), Some(rx)),
        None => (None, None),
    };
    let default_minutes = config.awake.default_minutes;
    let daemon = Daemon {
        metrics,
        values: HashMap::new(),
        history: HashMap::new(),
        config,
        awake_until: -1,
        awake_cookie: None,
        session: session.clone(),
        system: system.clone(),
        procs: Procs::new(Path::new("/proc")),
        mixer: MixerState::default(),
        mixer_tx,
        max_volume,
        clipboard: clipboard::Clipboard::load(),
        shelf: clipboard::Shelf::default(),
        jobs: Arc::new(jobs::Ctx {
            session: session.clone(),
            system: system.clone(),
            tools: Default::default(),
            cleaner: Default::default(),
        }),
        next_job: 0,
        topbar: String::new(),
        panel_visible: false,
        quit: quit.clone(),
    };
    session.object_server().at(dbus::DAEMON_PATH, daemon).await?;
    session.request_name(dbus::DAEMON_NAME).await?;
    let iface = session.object_server().interface::<_, Daemon>(dbus::DAEMON_PATH).await?;

    if start_awake {
        let mut d = iface.get_mut().await;
        if d.awake_on(default_minutes).await.is_ok() {
            let _ = d.awake_until_changed(iface.signal_emitter()).await;
        }
    }

    // Publish every mixer snapshot as soon as the mixer thread sends it.
    let mixer_watch = async {
        let Some(mut rx) = mixer_rx else { return std::future::pending().await };
        while rx.changed().await.is_ok() {
            let state = rx.borrow_and_update().clone();
            let mut d = iface.get_mut().await;
            d.mixer = state;
            let _ = d.mixer_changed(iface.signal_emitter()).await;
        }
    };

    let mut alerts = alerts::Alerts::default();
    let mut ticks: u64 = 0;
    let mut last_nvidia: Vec<(String, f64)> = Vec::new();
    let sampler = async {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        // After a suspend, don't fire a burst of missed ticks.
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            ticks += 1;
            let mut sample = sensors.sample();
            if let Some(n) = &nvidia {
                // NVML calls are the costliest reading. Every second only
                // when someone looks at a GPU number (panel open, top bar,
                // an enabled GPU alert); otherwise every 10 s.
                let fast = {
                    let d = iface.get().await;
                    let topbar = config::topbar_or_default(&d.config, &d.metrics);
                    d.panel_visible
                        || (d.config.features.topbar && topbar.iter().any(|id| id.starts_with("gpu.nvidia")))
                        || d.config.alerts.rules.iter().any(|r| r.enabled && r.metric.starts_with("gpu."))
                };
                if fast || ticks % 10 == 1 {
                    last_nvidia = n.sample();
                }
                sample.extend(last_nvidia.iter().cloned());
            }
            let mut d = iface.get_mut().await;
            for (id, v) in &sample {
                let h = d.history.entry(id.clone()).or_default();
                if h.len() == HISTORY {
                    h.pop_front();
                }
                h.push_back(*v);
            }
            d.values = sample.into_iter().collect();
            // Ignore send errors: no listeners is not a failure.
            let _ = d.values_changed(iface.signal_emitter()).await;

            let bar = if d.config.features.topbar {
                topbar::text(&config::topbar_or_default(&d.config, &d.metrics), &d.metrics, &d.values)
            } else {
                String::new()
            };
            if bar != d.topbar {
                d.topbar = bar;
                let _ = d.top_bar_changed(iface.signal_emitter()).await;
            }

            if d.awake_tick().await {
                let _ = d.awake_until_changed(iface.signal_emitter()).await;
            }
            if d.config.features.alerts {
                for n in alerts.check(&d.config.alerts, &d.metrics, &d.values, Instant::now()) {
                    desktop::notify(&d.session, &n.title, &n.body).await;
                }
            }
        }
    };

    tokio::select! {
        _ = sampler => {}
        _ = mixer_watch => {}
        _ = quit.notified() => {}
    }
    // Exit right away instead of returning: NVIDIA's driver library leaves a
    // background thread that otherwise keeps the process alive after main.
    std::process::exit(0)
}
