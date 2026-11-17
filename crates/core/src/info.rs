// SPDX-License-Identifier: GPL-3.0-or-later
//! On-demand details the daemon sends as JSON (network interfaces, apps
//! with connections, drives). Unlike metrics these are lists of records,
//! fetched only while the matching tab is open.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkKind {
    Wifi,
    Ethernet,
    Vpn,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Interface {
    pub name: String,
    pub kind: LinkKind,
    pub up: bool,
    /// IPv4 first, then global IPv6 (link-local fe80:: left out).
    pub addresses: Vec<String>,
    /// Ethernet link speed in Mb/s, when known.
    pub speed_mbps: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NetApp {
    pub name: String,
    pub connections: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Drive {
    /// UDisks2 object path, used to eject.
    pub path: String,
    pub model: String,
    pub size: f64,
    pub removable: bool,
    pub can_eject: bool,
    /// "usb", "sdio", ... or empty for internal.
    pub bus: String,
    pub mounts: Vec<String>,
    /// 0..=100; NVMe: 100 - "percent used".
    pub health: Option<f64>,
    pub temp_c: Option<f64>,
    pub power_on_hours: Option<f64>,
    pub power_cycles: Option<f64>,
    pub unsafe_shutdowns: Option<f64>,
    pub media_errors: Option<f64>,
    pub read_bytes: Option<f64>,
    pub written_bytes: Option<f64>,
    /// SMART reports a failing drive.
    pub failing: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct SpeedResult {
    /// Bytes per second.
    pub download: f64,
    pub upload: f64,
}
