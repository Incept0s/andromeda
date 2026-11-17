// SPDX-License-Identifier: GPL-3.0-or-later
//! Code shared by the daemon, the UI backend and the WASM frontend: metric
//! descriptions and formatting (`metrics`), and the Linux sensor registry
//! (`sensors`, `cpu`), plus D-Bus names. The frontend only uses `metrics`.

pub mod config;
pub mod cpu;
pub mod metrics;
pub mod sensors;

/// Well-known D-Bus names. The daemon owns `DAEMON_NAME`, the Tauri UI owns
/// `PANEL_NAME`. Both follow the app ID `io.github.incept0s.Andromeda`.
pub mod dbus {
    pub const DAEMON_NAME: &str = "io.github.incept0s.Andromeda";
    pub const DAEMON_PATH: &str = "/io/github/incept0s/Andromeda";
    pub const DAEMON_IFACE: &str = "io.github.incept0s.Andromeda1";

    pub const PANEL_NAME: &str = "io.github.incept0s.Andromeda.Panel";
    pub const PANEL_PATH: &str = "/io/github/incept0s/Andromeda/Panel";
    pub const PANEL_IFACE: &str = "io.github.incept0s.Andromeda.Panel1";
}
