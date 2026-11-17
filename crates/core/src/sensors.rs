// SPDX-License-Identifier: GPL-3.0-or-later
//! The metric registry's Linux sources: `/proc` and `/sys` only.
//!
//! `Sensors::discover` looks once at what this machine has and builds a list
//! of metrics; anything missing simply never appears. `Sensors::sample`
//! reads current values. Every read is fallible and a failed read just drops
//! that value for this tick: sensors come and go (hotplug, suspended
//! devices, driver quirks), and none of that may crash the daemon.
//!
//! Both functions take the `/sys` and `/proc` roots as parameters so tests can
//! point them at a fake tree.
//!
//! NVIDIA is not here: its driver exposes nothing useful in sysfs, so the
//! daemon reads it through NVML (crates/daemon/src/nvidia.rs).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::cpu::{self, CpuTimes};
use crate::metrics::{Group, MetricInfo, Unit};

/// A metric that is one integer file times a scale (most of sysfs).
struct FileMetric {
    id: String,
    path: PathBuf,
    scale: f64,
    /// Secondary sensors (drive, Wi-Fi, RAM temperatures, fans): reading
    /// some of them makes the kernel query the device and takes up to
    /// 12 ms, so callers can skip them on most ticks.
    slow: bool,
}

struct Battery {
    id: String,
    dir: PathBuf,
}

struct Mount {
    /// Id segment: the block device name (`nvme0n1p3`), dots replaced.
    key: String,
    path: PathBuf,
}

pub struct Sensors {
    proc_: PathBuf,
    metrics: Vec<MetricInfo>,
    files: Vec<FileMetric>,
    batteries: Vec<Battery>,
    cpufreq: Vec<PathBuf>,
    zram: Vec<PathBuf>,
    has_psi: bool,
    prev_total: Option<CpuTimes>,
    prev_cores: Vec<Option<CpuTimes>>,
    /// Interfaces with a hardware device; only these count in totals, so
    /// VPN traffic (also seen on the real interface) is not counted twice.
    physical_nets: Vec<String>,
    net_prev: HashMap<String, (u64, u64)>,
    net_start: HashMap<String, (u64, u64)>,
    mounts: Vec<Mount>,
    disks: Vec<String>,
    disk_prev: HashMap<String, (u64, u64)>,
    last: Option<Instant>,
}

fn read_str(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_owned())
}

fn read_f64(path: &Path) -> Option<f64> {
    read_str(path)?.parse().ok()
}

/// Sorted directory entries; missing directories give an empty list.
fn entries(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect();
    v.sort();
    v
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Files in `dir` named `<prefix><n><suffix>`, as (n, path), sorted by n.
fn numbered(dir: &Path, prefix: &str, suffix: &str) -> Vec<(u32, PathBuf)> {
    let mut v: Vec<(u32, PathBuf)> = entries(dir)
        .into_iter()
        .filter_map(|p| {
            let name = file_name(&p);
            let n = name
                .strip_prefix(prefix)?
                .strip_suffix(suffix)?
                .parse()
                .ok()?;
            Some((n, p))
        })
        .collect();
    v.sort_by_key(|(n, _)| *n);
    v
}

/// Which hwmon temperature is "the CPU temperature", per driver.
/// shortcut: covers AMD (k10temp, zenpower), Intel (coretemp) and common ARM
/// thermal zones; other drivers just show up as generic temperatures.
fn cpu_temp_label(chip: &str, labels: &[(u32, String)]) -> Option<u32> {
    let find = |want: &str| labels.iter().find(|(_, l)| l == want).map(|(n, _)| *n);
    match chip {
        "k10temp" | "zenpower" => find("Tdie").or_else(|| find("Tctl")),
        "coretemp" => find("Package id 0"),
        "cpu_thermal" | "soc_thermal" => labels.first().map(|(n, _)| *n),
        _ => None,
    }
}

/// Readable names for common hwmon chips; others keep the driver name.
fn friendly_chip(chip: &str) -> Option<&'static str> {
    Some(match chip {
        "nvme" | "drivetemp" => "SSD",
        "spd5118" | "jc42" => "RAM",
        "acpitz" => "ACPI zone",
        "acpi_fan" => "System fan",
        "k10temp" | "zenpower" | "coretemp" => "CPU",
        c if c.starts_with("iwlwifi") || c.starts_with("mt79") || c.starts_with("ath") => "Wi-Fi",
        _ => return None,
    })
}

/// Marketing name from the PCI ID database (`hwdata` package), e.g.
/// "Raphael [Radeon 610M]" -> "Radeon 610M". Falls back to the vendor.
fn gpu_name(pci_ids: Option<&str>, vendor: &str, device: &str) -> String {
    let vendor = vendor.trim_start_matches("0x");
    let device = device.trim_start_matches("0x");
    let fallback = match vendor {
        "1002" => "AMD GPU",
        "8086" => "Intel GPU",
        "10de" => "NVIDIA GPU",
        _ => "GPU",
    };
    let Some(db) = pci_ids else {
        return fallback.into();
    };
    let mut in_vendor = false;
    for line in db.lines() {
        // Comments appear inside vendor blocks; they must not end the block.
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if !line.starts_with('\t') {
            in_vendor = line.starts_with(vendor);
        } else if in_vendor && !line.starts_with("\t\t") {
            let rest = line.trim_start();
            if let Some(name) = rest.strip_prefix(device) {
                let name = name.trim();
                // Prefer the bracketed marketing name when present.
                if let (Some(a), Some(b)) = (name.find('['), name.rfind(']')) {
                    return name[a + 1..b].to_owned();
                }
                return name.to_owned();
            }
        }
    }
    fallback.into()
}

impl Sensors {
    pub fn discover(sys: &Path, proc_: &Path) -> Self {
        let mut s = Sensors {
            proc_: proc_.to_owned(),
            metrics: Vec::new(),
            files: Vec::new(),
            batteries: Vec::new(),
            cpufreq: Vec::new(),
            zram: Vec::new(),
            has_psi: proc_.join("pressure/memory").exists(),
            prev_total: None,
            prev_cores: Vec::new(),
            physical_nets: Vec::new(),
            net_prev: HashMap::new(),
            net_start: HashMap::new(),
            mounts: Vec::new(),
            disks: Vec::new(),
            disk_prev: HashMap::new(),
            last: None,
        };
        s.discover_cpu(sys);
        s.discover_memory(sys);
        s.discover_gpus(sys);
        s.discover_hwmon(sys);
        s.discover_power(sys);
        s.discover_network(sys);
        s.discover_disks(sys);
        if proc_.join("uptime").exists() {
            s.metrics.push(MetricInfo::new(
                "sys.uptime",
                "Uptime",
                Unit::Seconds,
                Group::System,
            ));
        }
        s
    }

    pub fn metrics(&self) -> &[MetricInfo] {
        &self.metrics
    }

    fn add_file(&mut self, info: MetricInfo, path: PathBuf, scale: f64) {
        // Only offer what can be read right now.
        if read_f64(&path).is_some() {
            let slow = matches!(info.group, Group::Temps | Group::Fans);
            self.files.push(FileMetric {
                id: info.id.clone(),
                path,
                scale,
                slow,
            });
            self.metrics.push(info);
        }
    }

    fn discover_cpu(&mut self, sys: &Path) {
        let Some(stat) = read_str(&self.proc_.join("stat")) else {
            return;
        };
        if cpu::parse_proc_stat(&stat).is_none() {
            return;
        }
        self.metrics.push(MetricInfo::new(
            "cpu.usage",
            "CPU",
            Unit::Percent,
            Group::Cpu,
        ));
        for i in 0..cpu::parse_cores(&stat).len() {
            self.metrics.push(MetricInfo::new(
                format!("cpu.core{i}"),
                format!("Core {i}"),
                Unit::Percent,
                Group::Cpu,
            ));
        }
        self.cpufreq = entries(&sys.join("devices/system/cpu"))
            .into_iter()
            .map(|p| p.join("cpufreq/scaling_cur_freq"))
            .filter(|p| p.exists())
            .collect();
        if !self.cpufreq.is_empty() {
            self.metrics.push(MetricInfo::new(
                "cpu.freq",
                "CPU clock",
                Unit::Hertz,
                Group::Cpu,
            ));
        }
    }

    fn discover_memory(&mut self, sys: &Path) {
        if read_str(&self.proc_.join("meminfo")).is_none() {
            return;
        }
        for (id, name) in [
            ("mem.used", "Memory used"),
            ("mem.total", "Memory total"),
            ("mem.available", "Memory available"),
            ("mem.cached", "Cached files"),
            ("mem.swap_used", "Swap used"),
            ("mem.swap_total", "Swap total"),
        ] {
            self.metrics
                .push(MetricInfo::new(id, name, Unit::Bytes, Group::Memory));
        }
        self.metrics.push(MetricInfo::new(
            "mem.used_pct",
            "Memory used",
            Unit::Percent,
            Group::Memory,
        ));
        if self.has_psi {
            self.metrics.push(MetricInfo::new(
                "mem.pressure",
                "Memory pressure",
                Unit::Percent,
                Group::Memory,
            ));
        }
        // Fedora swaps to zram (compressed RAM) by default.
        self.zram = entries(&sys.join("block"))
            .into_iter()
            .filter(|p| file_name(p).starts_with("zram"))
            .map(|p| p.join("mm_stat"))
            .filter(|p| p.exists())
            .collect();
        if !self.zram.is_empty() {
            self.metrics.push(MetricInfo::new(
                "mem.compressed",
                "Compressed (zram)",
                Unit::Bytes,
                Group::Memory,
            ));
        }
    }

    /// AMD and Intel GPUs from DRM sysfs. NVIDIA comes from NVML instead.
    fn discover_gpus(&mut self, sys: &Path) {
        // pci.ids is not guaranteed to be valid UTF-8, so read it lossily.
        let pci_ids = fs::read("/usr/share/hwdata/pci.ids")
            .ok()
            .map(|b| String::from_utf8_lossy(&b).into_owned());
        for card in entries(&sys.join("class/drm")) {
            let cname = file_name(&card);
            // card0, card1 ... but not connectors like card1-eDP-1.
            if !cname
                .strip_prefix("card")
                .is_some_and(|n| n.parse::<u32>().is_ok())
            {
                continue;
            }
            let dev = card.join("device");
            let (Some(vendor), Some(device)) =
                (read_str(&dev.join("vendor")), read_str(&dev.join("device")))
            else {
                continue;
            };
            if vendor == "0x10de" {
                continue;
            }
            let name = gpu_name(pci_ids.as_deref(), &vendor, &device);
            let id = |m: &str| format!("gpu.{cname}.{m}");
            self.add_file(
                MetricInfo::new(id("usage"), name.clone(), Unit::Percent, Group::Gpu),
                dev.join("gpu_busy_percent"),
                1.0,
            );
            self.add_file(
                MetricInfo::new(
                    id("vram_used"),
                    format!("{name} memory used"),
                    Unit::Bytes,
                    Group::Gpu,
                ),
                dev.join("mem_info_vram_used"),
                1.0,
            );
            self.add_file(
                MetricInfo::new(
                    id("vram_total"),
                    format!("{name} memory total"),
                    Unit::Bytes,
                    Group::Gpu,
                ),
                dev.join("mem_info_vram_total"),
                1.0,
            );
            if let Some(hw) = entries(&dev.join("hwmon")).into_iter().next() {
                self.add_file(
                    MetricInfo::new(
                        id("temp"),
                        format!("{name} temperature"),
                        Unit::Celsius,
                        Group::Gpu,
                    ),
                    hw.join("temp1_input"),
                    1e-3,
                );
                self.add_file(
                    MetricInfo::new(
                        id("clock"),
                        format!("{name} clock"),
                        Unit::Hertz,
                        Group::Gpu,
                    ),
                    hw.join("freq1_input"),
                    1.0,
                );
                let power = ["power1_average", "power1_input"]
                    .map(|f| hw.join(f))
                    .into_iter()
                    .find(|p| p.exists());
                if let Some(p) = power {
                    self.add_file(
                        MetricInfo::new(
                            id("power"),
                            format!("{name} power"),
                            Unit::Watts,
                            Group::Gpu,
                        ),
                        p,
                        1e-6,
                    );
                }
            }
        }
    }

    /// Temperatures and fans from every hwmon chip except GPUs (handled
    /// above) and power supplies (handled below).
    fn discover_hwmon(&mut self, sys: &Path) {
        // Several chips of one kind (two SSDs, two RAM sticks) get numbered.
        let mut seen: std::collections::HashMap<String, u32> = Default::default();
        for chip in entries(&sys.join("class/hwmon")) {
            let Some(name) = read_str(&chip.join("name")) else {
                continue;
            };
            if name == "amdgpu" || name == "i915" || name == "xe" || name == "nouveau" {
                continue;
            }
            // Stable key: chip name plus the device it belongs to. hwmonN
            // numbers change between boots, so they never appear in ids.
            let dev = fs::read_link(chip.join("device"))
                .ok()
                .map(|p| file_name(&p));
            let key = match &dev {
                Some(d) => format!("{name}@{d}"),
                None => name.clone(),
            };
            let label = |kind: &str, n: u32| read_str(&chip.join(format!("{kind}{n}_label")));
            let base = friendly_chip(&name).unwrap_or(&name).to_owned();
            let count = seen.entry(base.clone()).or_default();
            *count += 1;
            let base = if *count > 1 {
                format!("{base} {count}")
            } else {
                base
            };

            let temps: Vec<(u32, String)> = numbered(&chip, "temp", "_input")
                .into_iter()
                .map(|(n, _)| (n, label("temp", n).unwrap_or_default()))
                .collect();
            let cpu_n = cpu_temp_label(&name, &temps);
            for (n, l) in &temps {
                let path = chip.join(format!("temp{n}_input"));
                let shown = if l.is_empty() || temps.len() == 1 {
                    base.clone()
                } else {
                    format!("{base} {l}")
                };
                let info = if Some(*n) == cpu_n && !self.metrics.iter().any(|m| m.id == "cpu.temp")
                {
                    MetricInfo::new("cpu.temp", "CPU temperature", Unit::Celsius, Group::Cpu)
                } else {
                    MetricInfo::new(
                        format!("temp.{key}.{n}"),
                        shown,
                        Unit::Celsius,
                        Group::Temps,
                    )
                };
                self.add_file(info, path, 1e-3);
            }
            for (n, path) in numbered(&chip, "fan", "_input") {
                let shown = match label("fan", n) {
                    Some(l) => l.replace('_', " "),
                    None => base.clone(),
                };
                self.add_file(
                    MetricInfo::new(format!("fan.{key}.{n}"), shown, Unit::Rpm, Group::Fans),
                    path,
                    1.0,
                );
            }
        }
    }

    fn discover_power(&mut self, sys: &Path) {
        for ps in entries(&sys.join("class/power_supply")) {
            let name = file_name(&ps);
            match read_str(&ps.join("type")).as_deref() {
                Some("Mains") => {
                    self.add_file(
                        MetricInfo::new("power.ac", "AC adapter", Unit::Flag, Group::Battery),
                        ps.join("online"),
                        1.0,
                    );
                }
                // Peripherals (mouse, headset) report scope=Device.
                Some("Battery") if read_str(&ps.join("scope")).as_deref() == Some("Device") => {
                    let model = read_str(&ps.join("model_name")).unwrap_or(name.clone());
                    self.add_file(
                        MetricInfo::new(
                            format!("bat.{name}.charge"),
                            model,
                            Unit::Percent,
                            Group::Battery,
                        ),
                        ps.join("capacity"),
                        1.0,
                    );
                }
                Some("Battery") => {
                    let id = |m: &str| format!("bat.{name}.{m}");
                    if read_f64(&ps.join("capacity")).is_none() {
                        continue;
                    }
                    self.metrics.push(MetricInfo::new(
                        id("charge"),
                        "Battery",
                        Unit::Percent,
                        Group::Battery,
                    ));
                    self.metrics.push(MetricInfo::new(
                        id("health"),
                        "Battery health",
                        Unit::Percent,
                        Group::Battery,
                    ));
                    self.metrics.push(MetricInfo::new(
                        id("power"),
                        "Battery power",
                        Unit::Watts,
                        Group::Battery,
                    ));
                    self.add_file(
                        MetricInfo::new(id("cycles"), "Charge cycles", Unit::Flag, Group::Battery),
                        ps.join("cycle_count"),
                        1.0,
                    );
                    // A cycle count of 0 means "not reported" (e.g. ASUS), not "new".
                    if read_f64(&ps.join("cycle_count")) == Some(0.0) {
                        self.files.retain(|f| f.id != id("cycles"));
                        self.metrics.retain(|m| m.id != id("cycles"));
                    }
                    self.batteries.push(Battery {
                        id: format!("bat.{name}"),
                        dir: ps,
                    });
                }
                _ => {}
            }
        }
    }

    fn discover_network(&mut self, sys: &Path) {
        let Some(text) = read_str(&self.proc_.join("net/dev")) else {
            return;
        };
        let ifaces = parse_net_dev(&text);
        if ifaces.is_empty() {
            return;
        }
        for (name, _) in &ifaces {
            if name == "lo" {
                continue;
            }
            let key = name.replace('.', "_");
            self.metrics.push(MetricInfo::new(
                format!("net.{key}.rx"),
                format!("{name} download"),
                Unit::BytesPerSecond,
                Group::Network,
            ));
            self.metrics.push(MetricInfo::new(
                format!("net.{key}.tx"),
                format!("{name} upload"),
                Unit::BytesPerSecond,
                Group::Network,
            ));
            if sys.join("class/net").join(name).join("device").exists() {
                self.physical_nets.push(name.clone());
            }
        }
        for (id, name, unit) in [
            ("net.rx", "Download", Unit::BytesPerSecond),
            ("net.tx", "Upload", Unit::BytesPerSecond),
            ("net.rx_session", "Downloaded this session", Unit::Bytes),
            ("net.tx_session", "Uploaded this session", Unit::Bytes),
        ] {
            self.metrics
                .push(MetricInfo::new(id, name, unit, Group::Network));
        }
        self.net_start = ifaces.into_iter().collect();
    }

    fn discover_disks(&mut self, sys: &Path) {
        if let Some(text) = read_str(&self.proc_.join("self/mountinfo")) {
            for m in parse_mountinfo(&text) {
                // statvfs must work now, or the mount is not offered.
                if statvfs(&m.1).is_none() {
                    continue;
                }
                let key = m.0.replace('.', "_");
                let name = mount_name(&m.1);
                self.metrics.push(MetricInfo::new(
                    format!("disk.{key}.used"),
                    format!("{name} used"),
                    Unit::Bytes,
                    Group::Disk,
                ));
                self.metrics.push(MetricInfo::new(
                    format!("disk.{key}.total"),
                    format!("{name} size"),
                    Unit::Bytes,
                    Group::Disk,
                ));
                self.metrics.push(MetricInfo::new(
                    format!("disk.{key}.free_pct"),
                    format!("{name} free"),
                    Unit::Percent,
                    Group::Disk,
                ));
                self.mounts.push(Mount { key, path: m.1 });
            }
        }
        // Whole disks for I/O speed: /sys/block minus virtual devices.
        for d in entries(&sys.join("block")) {
            let name = file_name(&d);
            if ["loop", "ram", "zram", "dm-", "sr"]
                .iter()
                .any(|p| name.starts_with(p))
            {
                continue;
            }
            self.metrics.push(MetricInfo::new(
                format!("disk.{name}.read"),
                format!("{name} read"),
                Unit::BytesPerSecond,
                Group::Disk,
            ));
            self.metrics.push(MetricInfo::new(
                format!("disk.{name}.write"),
                format!("{name} write"),
                Unit::BytesPerSecond,
                Group::Disk,
            ));
            self.disks.push(name);
        }
    }

    /// Ids of the slow secondary sensors (see `FileMetric::slow`).
    pub fn slow_ids(&self) -> Vec<String> {
        self.files
            .iter()
            .filter(|f| f.slow)
            .map(|f| f.id.clone())
            .collect()
    }

    /// Reads every discovered metric once, or everything except the slow
    /// sensors when `slow` is false. CPU usage and all rates need two
    /// readings, so they are absent from the very first sample.
    pub fn sample(&mut self) -> Vec<(String, f64)> {
        self.sample_with(true)
    }

    pub fn sample_with(&mut self, slow: bool) -> Vec<(String, f64)> {
        let mut out = Vec::with_capacity(self.metrics.len());
        let now = Instant::now();
        let elapsed = self
            .last
            .map(|t| now.duration_since(t).as_secs_f64())
            .filter(|e| *e > 0.0);
        self.last = Some(now);
        let rate = |cur: u64, prev: u64| elapsed.map(|e| cur.saturating_sub(prev) as f64 / e);

        if let Some(stat) = read_str(&self.proc_.join("stat")) {
            let total = cpu::parse_proc_stat(&stat);
            if let (Some(p), Some(c)) = (self.prev_total, total)
                && let Some(u) = cpu::usage_percent(p, c)
            {
                out.push(("cpu.usage".into(), u));
            }
            self.prev_total = total;
            let cores = cpu::parse_cores(&stat);
            for (i, c) in cores.iter().enumerate() {
                if let (Some(Some(p)), Some(c)) = (self.prev_cores.get(i), c)
                    && let Some(u) = cpu::usage_percent(*p, *c)
                {
                    out.push((format!("cpu.core{i}"), u));
                }
            }
            self.prev_cores = cores;
        }

        let freqs: Vec<f64> = self.cpufreq.iter().filter_map(|p| read_f64(p)).collect();
        if !freqs.is_empty() {
            // scaling_cur_freq is in kHz.
            out.push((
                "cpu.freq".into(),
                freqs.iter().sum::<f64>() / freqs.len() as f64 * 1e3,
            ));
        }

        if let Some(text) = read_str(&self.proc_.join("meminfo")) {
            out.extend(memory_values(&text));
        }
        if self.has_psi
            && let Some(v) =
                read_str(&self.proc_.join("pressure/memory")).and_then(|t| parse_psi_some_avg10(&t))
        {
            out.push(("mem.pressure".into(), v));
        }
        // An empty iterator would sum to Some(0), so check for devices first.
        let zram: Option<f64> = self
            .zram
            .iter()
            .map(|p| read_str(p).and_then(|t| parse_zram_mem_used(&t)))
            .sum();
        if !self.zram.is_empty()
            && let Some(z) = zram
        {
            out.push(("mem.compressed".into(), z));
        }

        for f in self.files.iter().filter(|f| slow || !f.slow) {
            if let Some(v) = read_f64(&f.path) {
                out.push((f.id.clone(), v * f.scale));
            }
        }

        for b in &self.batteries {
            let get = |f: &str| read_f64(&b.dir.join(f));
            if let Some(c) = get("capacity") {
                out.push((format!("{}.charge", b.id), c));
            }
            let health = match (get("energy_full"), get("energy_full_design")) {
                (Some(f), Some(d)) if d > 0.0 => Some(f / d * 100.0),
                _ => match (get("charge_full"), get("charge_full_design")) {
                    (Some(f), Some(d)) if d > 0.0 => Some(f / d * 100.0),
                    _ => None,
                },
            };
            if let Some(h) = health {
                out.push((format!("{}.health", b.id), h));
            }
            // µW, or µA × µV. Some firmware reports discharge as negative.
            let power =
                get("power_now").or_else(|| Some(get("current_now")? * get("voltage_now")? / 1e6));
            if let Some(p) = power {
                out.push((format!("{}.power", b.id), p.abs() / 1e6));
            }
        }

        if let Some(text) = read_str(&self.proc_.join("net/dev")) {
            let (mut rx, mut tx, mut rx_s, mut tx_s) = (0.0, 0.0, 0.0, 0.0);
            let mut have_rate = false;
            for (name, (r, t)) in parse_net_dev(&text) {
                if name == "lo" {
                    continue;
                }
                let key = name.replace('.', "_");
                let prev = self.net_prev.insert(name.clone(), (r, t));
                let physical = self.physical_nets.contains(&name);
                if let Some((pr, pt)) = prev
                    && let (Some(rr), Some(tr)) = (rate(r, pr), rate(t, pt))
                {
                    out.push((format!("net.{key}.rx"), rr));
                    out.push((format!("net.{key}.tx"), tr));
                    if physical {
                        rx += rr;
                        tx += tr;
                        have_rate = true;
                    }
                }
                if physical {
                    let (sr, st) = self.net_start.get(&name).copied().unwrap_or((r, t));
                    rx_s += r.saturating_sub(sr) as f64;
                    tx_s += t.saturating_sub(st) as f64;
                }
            }
            if have_rate {
                out.push(("net.rx".into(), rx));
                out.push(("net.tx".into(), tx));
            }
            if !self.physical_nets.is_empty() {
                out.push(("net.rx_session".into(), rx_s));
                out.push(("net.tx_session".into(), tx_s));
            }
        }

        for m in &self.mounts {
            if let Some((total, used, avail)) = statvfs(&m.path) {
                out.push((format!("disk.{}.used", m.key), used));
                out.push((format!("disk.{}.total", m.key), total));
                if total > 0.0 {
                    out.push((format!("disk.{}.free_pct", m.key), avail / total * 100.0));
                }
            }
        }
        if let Some(text) = read_str(&self.proc_.join("diskstats")) {
            for (name, (r, w)) in parse_diskstats(&text) {
                if !self.disks.contains(&name) {
                    continue;
                }
                if let Some((pr, pw)) = self.disk_prev.insert(name.clone(), (r, w))
                    && let (Some(rr), Some(wr)) = (rate(r, pr), rate(w, pw))
                {
                    out.push((format!("disk.{name}.read"), rr));
                    out.push((format!("disk.{name}.write"), wr));
                }
            }
        }

        if let Some(up) = read_str(&self.proc_.join("uptime"))
            .and_then(|t| t.split_whitespace().next()?.parse().ok())
        {
            out.push(("sys.uptime".into(), up));
        }
        out
    }
}

/// (interface, (rx bytes, tx bytes)) from `/proc/net/dev`.
pub fn parse_net_dev(text: &str) -> Vec<(String, (u64, u64))> {
    text.lines()
        .skip(2)
        .filter_map(|l| {
            let (name, rest) = l.split_once(':')?;
            let f: Vec<u64> = rest
                .split_whitespace()
                .filter_map(|x| x.parse().ok())
                .collect();
            Some((name.trim().to_owned(), (*f.first()?, *f.get(8)?)))
        })
        .collect()
}

/// (device name, mount point) for real block-device mounts, one per device
/// (btrfs `/` and `/home` are the same device). Boot partitions and system
/// internals are left out: they are not what people mean by "my disks".
pub fn parse_mountinfo(text: &str) -> Vec<(String, PathBuf)> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        let Some(sep) = f.iter().position(|x| *x == "-") else {
            continue;
        };
        let (Some(devno), Some(mnt), Some(source)) = (f.get(2), f.get(4), f.get(sep + 2)) else {
            continue;
        };
        // Mount points escape spaces as \040.
        let mnt = mnt.replace("\\040", " ");
        let skip = !source.starts_with("/dev/")
            || mnt.starts_with("/boot")
            || mnt.starts_with("/var/lib/")
            || mnt.starts_with("/snap")
            || mnt.starts_with("/sysroot");
        if skip || !seen.insert(*devno) {
            continue;
        }
        let dev = source.rsplit('/').next().unwrap_or(source).to_owned();
        out.push((dev, PathBuf::from(mnt)));
    }
    out
}

/// (whole disk, (bytes read, bytes written)) from `/proc/diskstats`.
/// Sector counts there are always in 512-byte units.
pub fn parse_diskstats(text: &str) -> Vec<(String, (u64, u64))> {
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            let r: u64 = f.get(5)?.parse().ok()?;
            let w: u64 = f.get(9)?.parse().ok()?;
            Some((f.get(2)?.to_string(), (r * 512, w * 512)))
        })
        .collect()
}

/// "System" for `/`, the volume label for removable media, else the path.
fn mount_name(p: &Path) -> String {
    let s = p.to_string_lossy();
    if s == "/" {
        return "System".into();
    }
    if s.starts_with("/run/media/") || s.starts_with("/media/") {
        return file_name(p);
    }
    s.into_owned()
}

/// (total, used, available to the user) in bytes.
fn statvfs(path: &Path) -> Option<(f64, f64, f64)> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid NUL-terminated path and `st` a valid out-pointer.
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let frsize = st.f_frsize as f64;
    let total = st.f_blocks as f64 * frsize;
    let used = total - st.f_bfree as f64 * frsize;
    Some((total, used, st.f_bavail as f64 * frsize))
}

/// Memory figures matching GNOME System Monitor: used = total - available.
pub fn memory_values(meminfo: &str) -> Vec<(String, f64)> {
    let kb = |key: &str| -> Option<f64> {
        meminfo
            .lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))?
            .split_whitespace()
            .next()?
            .parse::<f64>()
            .ok()
            .map(|v| v * 1024.0)
    };
    let mut out = Vec::new();
    let (Some(total), Some(avail)) = (kb("MemTotal"), kb("MemAvailable")) else {
        return out;
    };
    let used = (total - avail).max(0.0);
    out.push(("mem.total".into(), total));
    out.push(("mem.available".into(), avail));
    out.push(("mem.used".into(), used));
    if total > 0.0 {
        out.push(("mem.used_pct".into(), used / total * 100.0));
    }
    if let (Some(c), Some(b)) = (kb("Cached"), kb("Buffers")) {
        out.push(("mem.cached".into(), c + b - kb("Shmem").unwrap_or(0.0)));
    }
    if let (Some(t), Some(f)) = (kb("SwapTotal"), kb("SwapFree")) {
        out.push(("mem.swap_total".into(), t));
        out.push(("mem.swap_used".into(), (t - f).max(0.0)));
    }
    out
}

/// `some avg10=1.23 ...` from `/proc/pressure/memory`.
pub fn parse_psi_some_avg10(text: &str) -> Option<f64> {
    text.lines()
        .find(|l| l.starts_with("some "))?
        .split_whitespace()
        .find_map(|f| f.strip_prefix("avg10="))?
        .parse()
        .ok()
}

/// Third field of zram `mm_stat`: RAM actually used by the compressed store.
pub fn parse_zram_mem_used(text: &str) -> Option<f64> {
    text.split_whitespace().nth(2)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    /// A throwaway fake `/sys` + `/proc` tree, removed on drop.
    struct Tree(PathBuf);
    impl Tree {
        fn new(name: &str) -> Self {
            // Tests run in parallel threads of one process: make each tree unique.
            static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let p = std::env::temp_dir()
                .join(format!("andromeda-test-{name}-{}-{n}", std::process::id()));
            let _ = fs::remove_dir_all(&p);
            Tree(p)
        }
        fn put(&self, rel: &str, content: &str) {
            let p = self.0.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, content).unwrap();
        }
        fn sys(&self) -> PathBuf {
            self.0.join("sys")
        }
        fn proc_(&self) -> PathBuf {
            self.0.join("proc")
        }
    }
    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const MEMINFO: &str = "MemTotal:       16000000 kB\nMemFree:  1000 kB\nMemAvailable:    6000000 kB\nBuffers:  100 kB\nCached:  2000 kB\nShmem: 100 kB\nSwapTotal: 8000 kB\nSwapFree: 6000 kB\n";

    fn laptop() -> Tree {
        let t = Tree::new("laptop");
        t.put("proc/stat", "cpu  100 0 100 800 0 0 0 0 0 0\ncpu0 50 0 50 400 0 0 0 0 0 0\ncpu1 50 0 50 400 0 0 0 0 0 0\n");
        t.put("proc/meminfo", MEMINFO);
        t.put("proc/pressure/memory", "some avg10=1.50 avg60=0.00 avg300=0.00 total=8\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=8\n");
        t.put("proc/uptime", "4260.50 1000.00\n");
        t.put(
            "sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq",
            "3000000\n",
        );
        t.put(
            "sys/devices/system/cpu/cpu1/cpufreq/scaling_cur_freq",
            "1000000\n",
        );
        t.put("sys/block/zram0/mm_stat", "4096 80 12288 0 12288 0 0 0 0\n");
        // k10temp with Tctl and a chiplet sensor.
        t.put("sys/class/hwmon/hwmon9/name", "k10temp\n");
        t.put("sys/class/hwmon/hwmon9/temp1_input", "72125\n");
        t.put("sys/class/hwmon/hwmon9/temp1_label", "Tctl\n");
        t.put("sys/class/hwmon/hwmon9/temp3_input", "69750\n");
        t.put("sys/class/hwmon/hwmon9/temp3_label", "Tccd1\n");
        // Two identical NVMe chips, told apart by their device.
        for (h, dev) in [("hwmon4", "nvme0"), ("hwmon5", "nvme1")] {
            t.put(&format!("sys/class/hwmon/{h}/name"), "nvme\n");
            t.put(&format!("sys/class/hwmon/{h}/temp1_input"), "44850\n");
            t.put(&format!("sys/class/hwmon/{h}/temp1_label"), "Composite\n");
            t.put(&format!("sys/devices/{dev}/.keep"), "");
            symlink(
                t.0.join(format!("sys/devices/{dev}")),
                t.0.join(format!("sys/class/hwmon/{h}/device")),
            )
            .unwrap();
        }
        t.put("sys/class/hwmon/hwmon10/name", "asus\n");
        t.put("sys/class/hwmon/hwmon10/fan1_input", "4200\n");
        t.put("sys/class/hwmon/hwmon10/fan1_label", "cpu_fan\n");
        // AMD iGPU.
        t.put("sys/class/drm/card1/device/vendor", "0x1002\n");
        t.put("sys/class/drm/card1/device/device", "0x164e\n");
        t.put("sys/class/drm/card1/device/gpu_busy_percent", "34\n");
        t.put(
            "sys/class/drm/card1/device/mem_info_vram_used",
            "486584320\n",
        );
        t.put(
            "sys/class/drm/card1/device/hwmon/hwmon8/temp1_input",
            "61000\n",
        );
        t.put(
            "sys/class/drm/card1/device/hwmon/hwmon8/power1_input",
            "12290000\n",
        );
        t.put("sys/class/drm/card1-eDP-1/status", "connected\n");
        // NVIDIA card: must be skipped here.
        t.put("sys/class/drm/card0/device/vendor", "0x10de\n");
        t.put("sys/class/drm/card0/device/device", "0x2f58\n");
        // Laptop battery (ASUS: cycle_count 0), AC adapter, Logitech headset.
        t.put("sys/class/power_supply/BAT0/type", "Battery\n");
        t.put("sys/class/power_supply/BAT0/capacity", "80\n");
        t.put("sys/class/power_supply/BAT0/energy_full", "77867000\n");
        t.put(
            "sys/class/power_supply/BAT0/energy_full_design",
            "90001000\n",
        );
        t.put("sys/class/power_supply/BAT0/power_now", "-12500000\n");
        t.put("sys/class/power_supply/BAT0/cycle_count", "0\n");
        t.put("sys/class/power_supply/ADP0/type", "Mains\n");
        t.put("sys/class/power_supply/ADP0/online", "1\n");
        t.put("sys/class/power_supply/hidpp_battery_0/type", "Battery\n");
        t.put("sys/class/power_supply/hidpp_battery_0/scope", "Device\n");
        t.put("sys/class/power_supply/hidpp_battery_0/capacity", "15\n");
        t.put(
            "sys/class/power_supply/hidpp_battery_0/model_name",
            "PRO X 2\n",
        );
        t
    }

    fn value(v: &[(String, f64)], id: &str) -> Option<f64> {
        v.iter().find(|(k, _)| k == id).map(|(_, x)| *x)
    }

    #[test]
    fn discovers_laptop() {
        let t = laptop();
        let s = Sensors::discover(&t.sys(), &t.proc_());
        let ids: Vec<&str> = s.metrics().iter().map(|m| m.id.as_str()).collect();
        for want in [
            "cpu.usage",
            "cpu.core0",
            "cpu.core1",
            "cpu.freq",
            "cpu.temp",
            "temp.k10temp.3",
            "temp.nvme@nvme0.1",
            "temp.nvme@nvme1.1",
            "fan.asus.1",
            "mem.used",
            "mem.pressure",
            "mem.compressed",
            "gpu.card1.usage",
            "gpu.card1.vram_used",
            "gpu.card1.temp",
            "gpu.card1.power",
            "bat.BAT0.charge",
            "bat.BAT0.health",
            "bat.BAT0.power",
            "power.ac",
            "bat.hidpp_battery_0.charge",
            "sys.uptime",
        ] {
            assert!(ids.contains(&want), "missing {want} in {ids:?}");
        }
        // Absent or unreadable things never appear.
        for absent in [
            "gpu.card1.vram_total",
            "gpu.card1.clock",
            "bat.BAT0.cycles",
            "gpu.card0.usage",
        ] {
            assert!(!ids.contains(&absent), "unexpected {absent}");
        }
        assert!(
            !ids.iter().any(|i| i.contains("hwmon")),
            "hwmon index leaked into an id"
        );
        let fan = s.metrics().iter().find(|m| m.id == "fan.asus.1").unwrap();
        assert_eq!(fan.name, "cpu fan");
        let name = |id: &str| {
            s.metrics()
                .iter()
                .find(|m| m.id == id)
                .unwrap()
                .name
                .clone()
        };
        assert_eq!(name("temp.nvme@nvme0.1"), "SSD");
        assert_eq!(name("temp.nvme@nvme1.1"), "SSD 2");
        assert_eq!(name("temp.k10temp.3"), "CPU Tccd1");
    }

    #[test]
    fn samples_laptop() {
        let t = laptop();
        let mut s = Sensors::discover(&t.sys(), &t.proc_());
        let first = s.sample();
        assert_eq!(value(&first, "cpu.usage"), None, "usage needs two readings");
        t.put("proc/stat", "cpu  200 0 200 900 0 0 0 0 0 0\ncpu0 100 0 100 400 0 0 0 0 0 0\ncpu1 50 0 50 500 0 0 0 0 0 0\n");
        let v = s.sample();
        assert!((value(&v, "cpu.usage").unwrap() - 200.0 / 3.0).abs() < 1e-9);
        assert_eq!(value(&v, "cpu.core0"), Some(100.0));
        assert_eq!(value(&v, "cpu.core1"), Some(0.0));
        assert_eq!(value(&v, "cpu.freq"), Some(2e9));
        assert_eq!(value(&v, "cpu.temp"), Some(72.125));
        assert_eq!(value(&v, "mem.pressure"), Some(1.5));
        assert_eq!(value(&v, "mem.compressed"), Some(12288.0));
        assert_eq!(value(&v, "gpu.card1.usage"), Some(34.0));
        assert!((value(&v, "gpu.card1.power").unwrap() - 12.29).abs() < 1e-9);
        assert!((value(&v, "bat.BAT0.health").unwrap() - 86.52).abs() < 0.01);
        assert_eq!(value(&v, "bat.BAT0.power"), Some(12.5));
        assert_eq!(value(&v, "power.ac"), Some(1.0));
        assert_eq!(value(&v, "sys.uptime"), Some(4260.5));
    }

    #[test]
    fn fast_sample_skips_secondary_sensors() {
        let t = laptop();
        let mut s = Sensors::discover(&t.sys(), &t.proc_());
        let fast = s.sample_with(false);
        assert!(
            value(&fast, "cpu.temp").is_some(),
            "CPU temperature stays fast"
        );
        assert!(value(&fast, "temp.nvme@nvme0.1").is_none());
        assert!(value(&fast, "fan.asus.1").is_none());
        assert!(s.slow_ids().contains(&"fan.asus.1".to_string()));
    }

    #[test]
    fn empty_machine_has_no_metrics_and_no_panic() {
        let t = Tree::new("empty");
        fs::create_dir_all(&t.0).unwrap();
        let mut s = Sensors::discover(&t.sys(), &t.proc_());
        assert!(s.metrics().is_empty());
        assert!(s.sample().is_empty());
    }

    #[test]
    fn sensor_vanishing_drops_only_that_value() {
        let t = laptop();
        let mut s = Sensors::discover(&t.sys(), &t.proc_());
        fs::remove_file(t.0.join("sys/class/drm/card1/device/gpu_busy_percent")).unwrap();
        let v = s.sample();
        assert_eq!(value(&v, "gpu.card1.usage"), None);
        assert!(value(&v, "mem.used").is_some());
    }

    #[test]
    fn memory_matches_gnome() {
        let v = memory_values(MEMINFO);
        assert_eq!(
            value(&v, "mem.used"),
            Some((16000000.0 - 6000000.0) * 1024.0)
        );
        assert_eq!(value(&v, "mem.cached"), Some(2000.0 * 1024.0));
        assert_eq!(value(&v, "mem.swap_used"), Some(2000.0 * 1024.0));
        assert!(memory_values("garbage").is_empty());
    }

    #[test]
    fn net_dev() {
        let t = "Inter-|   Receive\n face |bytes\n    lo: 3104447 50711 0 0 0 0 0 0 3104447 50711 0 0 0 0 0 0\n  wlp4s0: 1000 5 0 0 0 0 0 0 2000 7 0 0 0 0 0 0\n";
        assert_eq!(
            parse_net_dev(t),
            vec![
                ("lo".into(), (3104447, 3104447)),
                ("wlp4s0".into(), (1000, 2000))
            ]
        );
    }

    #[test]
    fn mountinfo_dedupes_btrfs_and_skips_boot() {
        let t = "\
29 1 0:38 /root / rw,relatime shared:1 - btrfs /dev/nvme0n1p3 rw
30 29 0:38 /home /home rw,relatime shared:2 - btrfs /dev/nvme0n1p3 rw
31 29 259:2 / /boot rw - ext4 /dev/nvme0n1p2 rw
40 29 0:22 / /tmp rw - tmpfs tmpfs rw
50 29 8:17 / /run/media/me/My\\040Stick rw - exfat /dev/sdb1 rw
";
        let m = parse_mountinfo(t);
        assert_eq!(
            m,
            vec![
                ("nvme0n1p3".into(), PathBuf::from("/")),
                ("sdb1".into(), PathBuf::from("/run/media/me/My Stick")),
            ]
        );
        assert_eq!(mount_name(&m[1].1), "My Stick");
    }

    #[test]
    fn diskstats() {
        let t = " 259       0 nvme0n1 1 2 100 4 5 6 200 8 9 10 11\n";
        assert_eq!(
            parse_diskstats(t),
            vec![("nvme0n1".into(), (100 * 512, 200 * 512))]
        );
    }

    #[test]
    fn gpu_names() {
        let db = "1002  Advanced Micro Devices, Inc. [AMD/ATI]\n# Used in the Steam Deck\n\n\t164e  Raphael\n\t15bf  Phoenix1 [Radeon 780M]\n\t\t1002 1234  Sub\n10de  NVIDIA\n";
        assert_eq!(gpu_name(Some(db), "0x1002", "0x15bf"), "Radeon 780M");
        assert_eq!(gpu_name(Some(db), "0x1002", "0x164e"), "Raphael");
        assert_eq!(gpu_name(Some(db), "0x1002", "0xffff"), "AMD GPU");
        assert_eq!(gpu_name(None, "0x8086", "0x1"), "Intel GPU");
    }
}
