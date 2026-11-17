// SPDX-License-Identifier: GPL-3.0-or-later
//! The top-bar text, e.g. `CPU 79°  RAM 27%  12W`. Built by the daemon
//! (Rust); the Shell extension only displays the string. Labels are short
//! because top-bar space is scarce.

use crate::metrics::{Group, MetricInfo, Unit};

/// Compact sizes for the bar: `6.2G`, `512M`, `40K`.
fn compact_bytes(b: f64) -> String {
    const K: f64 = 1024.0;
    if b >= K * K * K {
        format!("{:.1}G", b / (K * K * K))
    } else if b >= K * K {
        format!("{:.0}M", b / (K * K))
    } else {
        format!("{:.0}K", b / K)
    }
}

/// Short prefix for a metric: "CPU", "RAM", "GPU", first word of a
/// device name, or nothing for self-explanatory values like watts.
fn prefix(m: &MetricInfo, gpu_count: usize) -> String {
    let id = m.id.as_str();
    let last = id.rsplit('.').next().unwrap_or("");
    match (m.group, last) {
        (Group::Cpu, _) => "CPU".into(),
        (Group::Memory, "used" | "used_pct") => "RAM".into(),
        (Group::Memory, "swap_used") => "SWAP".into(),
        (Group::Memory, "pressure") => "PSI".into(),
        (Group::Gpu, "vram_used") => "VRAM".into(),
        // Several GPUs: tell them apart by the first word of their name.
        (Group::Gpu, _) if gpu_count > 1 => m.name.split_whitespace().next().unwrap_or("GPU").into(),
        (Group::Gpu, _) => "GPU".into(),
        (Group::Battery, "power") => String::new(),
        (Group::Battery, _) => "BAT".into(),
        (Group::Network, "rx") => "↓".into(),
        (Group::Network, "tx") => "↑".into(),
        (Group::Fans, _) => "FAN".into(),
        (Group::Disk, _) => m.name.split_whitespace().next().unwrap_or("Disk").into(),
        _ => m.name.split_whitespace().next().unwrap_or("").into(),
    }
}

fn value(v: f64, unit: Unit) -> String {
    match unit {
        Unit::Percent => format!("{v:.0}%"),
        Unit::Celsius => format!("{v:.0}°"),
        Unit::Bytes => compact_bytes(v),
        Unit::BytesPerSecond => compact_bytes(v),
        Unit::Watts => format!("{v:.0}W"),
        Unit::Hertz => format!("{:.1}GHz", v / 1e9),
        Unit::Rpm => format!("{v:.0}"),
        Unit::Seconds => crate::metrics::format_duration(v),
        Unit::Flag => (if v != 0.0 { "on" } else { "off" }).into(),
    }
}

/// Joins the chosen metrics that currently have a value. Missing ones
/// (a sensor that vanished) are skipped rather than shown as gaps.
pub fn text(ids: &[String], metrics: &[MetricInfo], values: &std::collections::HashMap<String, f64>) -> String {
    let gpus = metrics.iter().filter(|m| m.group == Group::Gpu && m.id.ends_with(".usage")).count();
    ids.iter()
        .filter_map(|id| {
            let m = metrics.iter().find(|m| &m.id == id)?;
            let v = *values.get(id)?;
            let p = prefix(m, gpus);
            // Plugged in with no flow (full or holding a charge limit): "0W"
            // says nothing, "AC" does.
            let on_ac = values.get("power.ac").is_some_and(|a| *a > 0.0);
            let s = if m.unit == Unit::Watts && m.group == Group::Battery && v < 0.5 && on_ac {
                "AC".to_owned()
            } else {
                value(v, m.unit)
            };
            Some(if p.is_empty() { s } else if p.len() <= 3 && !p.is_ascii() { format!("{p}{s}") } else { format!("{p} {s}") })
        })
        .collect::<Vec<_>>()
        .join("  ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn default_style() {
        let m = vec![
            MetricInfo::new("cpu.temp", "CPU temperature", Unit::Celsius, Group::Cpu),
            MetricInfo::new("mem.used_pct", "Memory used", Unit::Percent, Group::Memory),
            MetricInfo::new("bat.BAT0.power", "Battery power", Unit::Watts, Group::Battery),
            MetricInfo::new("gpu.nvidia0.vram_used", "RTX 5070 Ti memory used", Unit::Bytes, Group::Gpu),
            MetricInfo::new("net.rx", "Download", Unit::BytesPerSecond, Group::Network),
        ];
        let v = HashMap::from([
            ("cpu.temp".to_string(), 42.4),
            ("mem.used_pct".to_string(), 61.0),
            ("bat.BAT0.power".to_string(), 12.3),
            ("gpu.nvidia0.vram_used".to_string(), 6.2 * 1024f64.powi(3)),
            ("net.rx".to_string(), 1.5 * 1024.0 * 1024.0),
        ]);
        let ids: Vec<String> = ["cpu.temp", "mem.used_pct", "bat.BAT0.power", "gpu.nvidia0.vram_used", "net.rx", "gone"]
            .map(String::from)
            .into();
        assert_eq!(text(&ids, &m, &v), "CPU 42°  RAM 61%  12W  VRAM 6.2G  ↓2M");
        let mut idle = v.clone();
        idle.insert("bat.BAT0.power".into(), 0.0);
        idle.insert("power.ac".into(), 1.0);
        assert_eq!(text(&ids[2..3], &m, &idle), "AC");
    }

    #[test]
    fn two_gpus_use_their_names() {
        let m = vec![
            MetricInfo::new("gpu.card1.usage", "Radeon 610M", Unit::Percent, Group::Gpu),
            MetricInfo::new("gpu.card1.temp", "Radeon 610M temperature", Unit::Celsius, Group::Gpu),
            MetricInfo::new("gpu.nvidia0.usage", "RTX 5070 Ti", Unit::Percent, Group::Gpu),
            MetricInfo::new("gpu.nvidia0.temp", "RTX 5070 Ti temperature", Unit::Celsius, Group::Gpu),
        ];
        let v = HashMap::from([("gpu.card1.temp".to_string(), 61.0), ("gpu.nvidia0.temp".to_string(), 55.0)]);
        let ids = vec!["gpu.card1.temp".to_string(), "gpu.nvidia0.temp".to_string()];
        assert_eq!(text(&ids, &m, &v), "Radeon 61°  RTX 55°");
    }
}
