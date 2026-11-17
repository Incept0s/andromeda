// SPDX-License-Identifier: GPL-3.0-or-later
//! `andromedad`: the small always-running part of Andromeda.
//!
//! Owns all data collection and every background job: sampling the metric
//! registry once per second, alerts, keep-awake, settings. Everything is
//! exposed on the session bus as `io.github.incept0s.Andromeda1`; the panel
//! UI and the Shell extension are thin clients of this interface.

mod alerts;
mod check;
mod config_file;
mod desktop;
mod drives;
mod icons;
mod mixer;
mod netinfo;
mod nvidia;

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use andromeda_core::config::Config;
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

    /// Runs the Cloudflare speed test (about 20 s), JSON `info::SpeedResult`.
    async fn speed_test(&self) -> fdo::Result<String> {
        let r = netinfo::speed_test().await.map_err(fdo::Error::Failed)?;
        Ok(serde_json::to_string(&r).unwrap_or_default())
    }

    /// Drives with SMART data, as JSON (`Vec<info::Drive>`).
    async fn drives(&self) -> fdo::Result<String> {
        let sys = self.system.as_ref().ok_or_else(|| fdo::Error::Failed("no system bus".into()))?;
        let list = drives::list(sys).await?;
        Ok(serde_json::to_string(&list).unwrap_or_default())
    }

    /// Unmounts and powers off a drive (UDisks2 object path).
    async fn eject(&self, drive: &str) -> fdo::Result<()> {
        let sys = self.system.as_ref().ok_or_else(|| fdo::Error::Failed("no system bus".into()))?;
        drives::eject(sys, drive).await.map_err(fdo::Error::Failed)
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
        system,
        procs: Procs::new(Path::new("/proc")),
        mixer: MixerState::default(),
        mixer_tx,
        max_volume,
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
    let sampler = async {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        // After a suspend, don't fire a burst of missed ticks.
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let mut sample = sensors.sample();
            if let Some(n) = &nvidia {
                sample.extend(n.sample());
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
