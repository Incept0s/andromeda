// SPDX-License-Identifier: GPL-3.0-or-later
//! `andromedad`: the small always-running part of Andromeda.
//!
//! Owns all data collection. Once per second it samples the metric registry
//! (`andromeda_core::sensors` plus NVML for NVIDIA) and publishes the values
//! as the D-Bus property `Values`. Clients subscribe to its
//! `PropertiesChanged` signal instead of polling. A short history per metric
//! lets the panel draw graphs the moment it opens.

mod nvidia;

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use andromeda_core::dbus;
use andromeda_core::metrics::MetricInfo;
use andromeda_core::sensors::Sensors;
use tokio::sync::Notify;
use zbus::{connection, interface};

/// Samples kept per metric: one minute at one sample per second.
const HISTORY: usize = 60;

struct Daemon {
    metrics: Vec<MetricInfo>,
    values: HashMap<String, f64>,
    history: HashMap<String, VecDeque<f64>>,
    quit: Arc<Notify>,
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

    /// Exits the daemon. The panel's Quit button calls this.
    fn quit(&self) {
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

    let quit = Arc::new(Notify::new());
    let daemon = Daemon { metrics, values: HashMap::new(), history: HashMap::new(), quit: quit.clone() };
    let conn = connection::Builder::session()
        .serve_at(dbus::DAEMON_PATH, daemon)
        .name(dbus::DAEMON_NAME)
        .build()
        .await?;
    let iface = conn.object_server().interface::<_, Daemon>(dbus::DAEMON_PATH).await?;

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
        }
    };

    tokio::select! {
        _ = sampler => {}
        _ = quit.notified() => {}
    }
    Ok(())
}
