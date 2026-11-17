// SPDX-License-Identifier: GPL-3.0-or-later
//! NVIDIA GPUs through NVML (the library behind `nvidia-smi`).
//!
//! The proprietary driver exposes almost nothing in sysfs, so this is the
//! only way to read usage, VRAM, clocks and temperature. NVML is loaded at
//! runtime: on machines without the NVIDIA driver `init()` returns `None` and
//! no NVIDIA metrics exist. Reading may wake a sleeping dGPU; the user chose
//! to accept that (docs/SPEC.md, section 0).

use std::ffi::OsStr;

use andromeda_core::metrics::{Group, MetricInfo, Unit};
use nvml_wrapper::Nvml;
use nvml_wrapper::enum_wrappers::device::{Clock, TemperatureSensor};

pub struct Nvidia {
    nvml: Nvml,
    /// (NVML index, short name)
    gpus: Vec<(u32, String)>,
}

/// "NVIDIA GeForce RTX 5070 Ti Laptop GPU" -> "RTX 5070 Ti".
fn short_name(full: &str) -> String {
    full.replace("NVIDIA ", "")
        .replace("GeForce ", "")
        .replace(" Laptop GPU", "")
        .trim()
        .to_owned()
}

impl Nvidia {
    pub fn init() -> Option<Self> {
        // The versioned name exists wherever the driver is installed; the
        // bare `libnvidia-ml.so` only comes with development packages.
        let nvml = Nvml::builder()
            .lib_path(OsStr::new("libnvidia-ml.so.1"))
            .init()
            .ok()?;
        let count = nvml.device_count().ok()?;
        let gpus = (0..count)
            .filter_map(|i| Some((i, short_name(&nvml.device_by_index(i).ok()?.name().ok()?))))
            .collect::<Vec<_>>();
        (!gpus.is_empty()).then_some(Self { nvml, gpus })
    }

    pub fn metrics(&self) -> Vec<MetricInfo> {
        let mut v = Vec::new();
        for (i, name) in &self.gpus {
            let id = |m: &str| format!("gpu.nvidia{i}.{m}");
            v.push(MetricInfo::new(
                id("usage"),
                name.clone(),
                Unit::Percent,
                Group::Gpu,
            ));
            v.push(MetricInfo::new(
                id("vram_used"),
                format!("{name} memory used"),
                Unit::Bytes,
                Group::Gpu,
            ));
            v.push(MetricInfo::new(
                id("vram_total"),
                format!("{name} memory total"),
                Unit::Bytes,
                Group::Gpu,
            ));
            v.push(MetricInfo::new(
                id("temp"),
                format!("{name} temperature"),
                Unit::Celsius,
                Group::Gpu,
            ));
            v.push(MetricInfo::new(
                id("clock"),
                format!("{name} clock"),
                Unit::Hertz,
                Group::Gpu,
            ));
            v.push(MetricInfo::new(
                id("power"),
                format!("{name} power"),
                Unit::Watts,
                Group::Gpu,
            ));
        }
        v
    }

    /// Each query can fail on its own (e.g. power on some laptop GPUs);
    /// failures just leave that value out.
    pub fn sample(&self) -> Vec<(String, f64)> {
        let mut out = Vec::new();
        for (i, _) in &self.gpus {
            let Ok(d) = self.nvml.device_by_index(*i) else {
                continue;
            };
            let id = |m: &str| format!("gpu.nvidia{i}.{m}");
            if let Ok(u) = d.utilization_rates() {
                out.push((id("usage"), u.gpu as f64));
            }
            if let Ok(m) = d.memory_info() {
                out.push((id("vram_used"), m.used as f64));
                out.push((id("vram_total"), m.total as f64));
            }
            if let Ok(t) = d.temperature(TemperatureSensor::Gpu) {
                out.push((id("temp"), t as f64));
            }
            if let Ok(c) = d.clock_info(Clock::Graphics) {
                out.push((id("clock"), c as f64 * 1e6));
            }
            if let Ok(p) = d.power_usage() {
                out.push((id("power"), p as f64 / 1e3));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn short_names() {
        assert_eq!(
            super::short_name("NVIDIA GeForce RTX 5070 Ti Laptop GPU"),
            "RTX 5070 Ti"
        );
        assert_eq!(super::short_name("NVIDIA RTX A2000"), "RTX A2000");
    }
}
