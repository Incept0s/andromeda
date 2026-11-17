// SPDX-License-Identifier: GPL-3.0-or-later
//! `andromedad`: the small always-running part of Andromeda.
//!
//! Phase 1 spike: samples total CPU usage once per second and publishes it
//! as the D-Bus property `CpuPercent`. Clients (the Tauri UI, later the Shell
//! extension) subscribe to its `PropertiesChanged` signal instead of polling.

use std::sync::Arc;
use std::time::Duration;

use andromeda_core::{cpu, dbus};
use tokio::sync::Notify;
use zbus::{connection, interface};

struct Daemon {
    cpu_percent: f64,
    quit: Arc<Notify>,
}

/// `#[interface]` turns this impl into a D-Bus interface. Properties marked
/// `#[zbus(property)]` get a generated `<name>_changed()` helper that emits
/// `org.freedesktop.DBus.Properties.PropertiesChanged`.
#[interface(name = "io.github.incept0s.Andromeda1")]
impl Daemon {
    #[zbus(property)]
    fn cpu_percent(&self) -> f64 {
        self.cpu_percent
    }

    /// Exits the daemon. The panel's Quit button calls this.
    fn quit(&self) {
        self.quit.notify_one();
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> zbus::Result<()> {
    let quit = Arc::new(Notify::new());
    let conn = connection::Builder::session()
        .serve_at(dbus::DAEMON_PATH, Daemon { cpu_percent: 0.0, quit: quit.clone() })
        .name(dbus::DAEMON_NAME)
        .build()
        .await?;

    let iface = conn
        .object_server()
        .interface::<_, Daemon>(dbus::DAEMON_PATH)
        .await?;

    let sampler = async {
        let mut prev = cpu::read();
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tick.tick().await;
            let cur = cpu::read();
            if let (Some(p), Some(c)) = (prev, cur)
                && let Some(pct) = cpu::usage_percent(p, c)
            {
                let mut d = iface.get_mut().await;
                d.cpu_percent = pct;
                // Ignore send errors: no listeners is not a failure.
                let _ = d.cpu_percent_changed(iface.signal_emitter()).await;
            }
            prev = cur;
        }
    };

    tokio::select! {
        _ = sampler => {}
        _ = quit.notified() => {}
    }
    Ok(())
}
