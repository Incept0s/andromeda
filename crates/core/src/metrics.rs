// SPDX-License-Identifier: GPL-3.0-or-later
//! Metric descriptions shared by the daemon, the UI and (later) the top bar.
//! Values travel as plain `f64` in base units (bytes, °C, W, Hz, seconds).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Unit {
    Percent,
    Celsius,
    Bytes,
    BytesPerSecond,
    Watts,
    Hertz,
    Rpm,
    Seconds,
    /// 0 or 1, e.g. "AC adapter connected".
    Flag,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Group {
    Cpu,
    Gpu,
    Memory,
    Battery,
    Network,
    Disk,
    Fans,
    Temps,
    System,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricInfo {
    /// Stable across reboots (never a hwmonN index), so saved top-bar and
    /// alert choices keep pointing at the same sensor.
    pub id: String,
    pub name: String,
    pub unit: Unit,
    pub group: Group,
}

impl MetricInfo {
    pub fn new(id: impl Into<String>, name: impl Into<String>, unit: Unit, group: Group) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            unit,
            group,
        }
    }
}

// D-Bus carries unit and group as strings; serde's names are reused.
macro_rules! str_enum {
    ($t:ty, $($v:ident),+) => {
        impl $t {
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$v => stringify!($v)),+ }
            }
            pub fn parse(s: &str) -> Option<Self> {
                match s { $(stringify!($v) => Some(Self::$v),)+ _ => None }
            }
        }
    };
}
str_enum!(
    Unit,
    Percent,
    Celsius,
    Bytes,
    BytesPerSecond,
    Watts,
    Hertz,
    Rpm,
    Seconds,
    Flag
);
str_enum!(
    Group, Cpu, Gpu, Memory, Battery, Network, Disk, Fans, Temps, System
);

/// Human-readable value, e.g. `42%`, `61 °C`, `11.4 GiB`, `4.2 GHz`.
pub fn format(value: f64, unit: Unit) -> String {
    match unit {
        Unit::Percent => format!("{value:.0}%"),
        Unit::Celsius => format!("{value:.0} °C"),
        Unit::Bytes => format_bytes(value),
        Unit::BytesPerSecond => format!("{}/s", format_bytes(value)),
        Unit::Watts => format!("{value:.1} W"),
        Unit::Hertz if value >= 1e9 => format!("{:.2} GHz", value / 1e9),
        Unit::Hertz => format!("{:.0} MHz", value / 1e6),
        Unit::Rpm => format!("{value:.0} RPM"),
        Unit::Seconds => format_duration(value),
        Unit::Flag => (if value != 0.0 { "yes" } else { "no" }).into(),
    }
}

/// Binary units, labelled honestly (GiB), like GNOME System Monitor.
pub fn format_bytes(b: f64) -> String {
    const K: f64 = 1024.0;
    if b >= K * K * K {
        format!("{:.1} GiB", b / (K * K * K))
    } else if b >= K * K {
        format!("{:.0} MiB", b / (K * K))
    } else if b >= K {
        format!("{:.0} KiB", b / K)
    } else {
        format!("{b:.0} B")
    }
}

/// `3d 4h`, `1h 11min`, `5min`.
pub fn format_duration(secs: f64) -> String {
    let m = (secs / 60.0) as u64;
    let (d, h, m) = (m / 1440, m / 60 % 24, m % 60);
    match (d, h) {
        (0, 0) => format!("{m}min"),
        (0, _) => format!("{h}h {m}min"),
        _ => format!("{d}d {h}h"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pressure {
    Normal,
    Warning,
    Critical,
}

/// Memory pressure from PSI `some avg10`: the share of the last 10 s in
/// which at least one task waited for memory.
/// shortcut: thresholds 10/30 are a first guess, tune with real use.
pub fn pressure_level(some_avg10: f64) -> Pressure {
    if some_avg10 >= 30.0 {
        Pressure::Critical
    } else if some_avg10 >= 10.0 {
        Pressure::Warning
    } else {
        Pressure::Normal
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(format(41.6, Unit::Percent), "42%");
        assert_eq!(format(12.0 * 1024f64.powi(3), Unit::Bytes), "12.0 GiB");
        assert_eq!(format(600e6, Unit::Hertz), "600 MHz");
        assert_eq!(format(4.2e9, Unit::Hertz), "4.20 GHz");
        assert_eq!(format_duration(4260.0), "1h 11min");
        assert_eq!(format_duration(90000.0), "1d 1h");
    }

    #[test]
    fn str_round_trip() {
        assert_eq!(Unit::parse(Unit::Rpm.as_str()), Some(Unit::Rpm));
        assert_eq!(Group::parse("Gpu"), Some(Group::Gpu));
        assert_eq!(Group::parse("nope"), None);
    }

    #[test]
    fn pressure() {
        assert_eq!(pressure_level(0.0), Pressure::Normal);
        assert_eq!(pressure_level(12.0), Pressure::Warning);
        assert_eq!(pressure_level(55.0), Pressure::Critical);
    }
}
