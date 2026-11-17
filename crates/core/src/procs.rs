// SPDX-License-Identifier: GPL-3.0-or-later
//! Per-app CPU and memory, for "Apps using significant energy".
//!
//! Linux has no per-process energy figure, so CPU time is the proxy. To show
//! one Firefox row instead of twelve helpers, processes are grouped by the
//! systemd scope GNOME starts every app in (`app-gnome-firefox-1234.scope`,
//! `app-flatpak-org.mozilla.firefox-5678.scope`, D-Bus activated
//! `dbus-:1.1-org.gnome.Loupe@0.service`). Processes outside an app scope
//! (services, shells) are grouped by their process name.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::cpu;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AppUsage {
    /// App id (`firefox`, `org.gnome.Loupe`) or process name.
    pub key: String,
    /// Display name from the app's .desktop file when there is one.
    pub name: String,
    /// Share of all CPU time since the previous sample, 0..=100.
    pub cpu: f64,
    /// Resident memory of all its processes, in bytes.
    pub memory: f64,
    pub processes: u32,
}

/// App id from the last component of `/proc/<pid>/cgroup`, if the process
/// runs in an app scope.
pub fn app_id_from_cgroup(cgroup: &str) -> Option<String> {
    let unit = cgroup.lines().last()?.rsplit('/').next()?;
    // systemd escapes '-' inside ids as \x2d.
    let unescape = |s: &str| s.replace("\\x2d", "-");
    if let Some(rest) = unit.strip_prefix("dbus-:") {
        // dbus-:1.1-org.gnome.Loupe@0.service
        let after = rest.split_once('-')?.1;
        return Some(unescape(after.split('@').next()?));
    }
    let rest = unit.strip_prefix("app-")?;
    let base = rest
        .strip_suffix(".scope")
        .or_else(|| rest.strip_suffix(".service"))?;
    let base = base.split('@').next()?;
    // Drop the launcher (gnome, flatpak, ...) and the trailing -<number>.
    let (_, id) = base.split_once('-')?;
    let id = match id.rsplit_once('-') {
        Some((head, tail)) if tail.chars().all(|c| c.is_ascii_digit()) => head,
        _ => id,
    };
    // `app-gnome-23707.scope` has no id, only a number.
    (!id.is_empty() && !id.chars().all(|c| c.is_ascii_digit())).then(|| unescape(id))
}

/// `utime + stime` (fields 14 and 15) from `/proc/<pid>/stat`. The process
/// name in field 2 may contain spaces and parentheses, so parse after the
/// last ')'.
pub fn parse_stat_ticks(stat: &str) -> Option<(String, u64)> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let comm = stat.get(open + 1..close)?.to_owned();
    let mut f = stat.get(close + 2..)?.split_whitespace();
    // After ')': state is field 3, so utime (14) is the 12th item here.
    let utime: u64 = f.nth(11)?.parse().ok()?;
    let stime: u64 = f.next()?.parse().ok()?;
    Some((comm, utime + stime))
}

pub struct Procs {
    proc_: PathBuf,
    prev: HashMap<u32, u64>,
    prev_total: Option<cpu::CpuTimes>,
    names: HashMap<String, String>,
    page_size: f64,
}

impl Procs {
    pub fn new(proc_: &Path) -> Self {
        Self {
            proc_: proc_.to_owned(),
            prev: HashMap::new(),
            prev_total: None,
            names: HashMap::new(),
            // shortcut: 4 KiB pages (x86_64, most aarch64); 16/64 KiB kernels
            // would under-report memory until this reads sysconf.
            page_size: 4096.0,
        }
    }

    /// Apps sorted by CPU, busiest first. The first call has no previous
    /// reading, so it reports 0 % CPU for everything.
    pub fn sample(&mut self) -> Vec<AppUsage> {
        let total = fs::read_to_string(self.proc_.join("stat"))
            .ok()
            .and_then(|t| cpu::parse_proc_stat(&t));
        let elapsed = match (self.prev_total, total) {
            (Some(p), Some(c)) => c.total.saturating_sub(p.total) as f64,
            _ => 0.0,
        };
        let mut apps: HashMap<String, AppUsage> = HashMap::new();
        let mut now = HashMap::new();
        for entry in fs::read_dir(&self.proc_).into_iter().flatten().flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|s| s.parse::<u32>().ok())
            else {
                continue;
            };
            let dir = entry.path();
            let Some((comm, ticks)) = fs::read_to_string(dir.join("stat"))
                .ok()
                .and_then(|s| parse_stat_ticks(&s))
            else {
                continue;
            };
            // Kernel threads have no memory map; skip them.
            let rss_pages: f64 = fs::read_to_string(dir.join("statm"))
                .ok()
                .and_then(|s| s.split_whitespace().nth(1)?.parse().ok())
                .unwrap_or(0.0);
            if rss_pages == 0.0 {
                continue;
            }
            now.insert(pid, ticks);
            let delta = self.prev.get(&pid).map_or(0, |p| ticks.saturating_sub(*p));
            let cg = fs::read_to_string(dir.join("cgroup")).unwrap_or_default();
            let key = app_id_from_cgroup(&cg).unwrap_or(comm);
            let name = self.display_name(&key);
            let app = apps.entry(key.clone()).or_insert(AppUsage {
                key,
                name,
                cpu: 0.0,
                memory: 0.0,
                processes: 0,
            });
            if elapsed > 0.0 {
                // Process ticks and /proc/stat jiffies use the same clock
                // (USER_HZ), so the ratio is the share of all CPU time.
                app.cpu += delta as f64 * 100.0 / elapsed;
            }
            app.memory += rss_pages * self.page_size;
            app.processes += 1;
        }
        self.prev = now;
        self.prev_total = total;
        let mut v: Vec<AppUsage> = apps.into_values().collect();
        v.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then(b.memory.total_cmp(&a.memory)));
        v
    }

    /// `Name=` from the app's .desktop file, cached; else the key itself.
    fn display_name(&mut self, key: &str) -> String {
        if let Some(n) = self.names.get(key) {
            return n.clone();
        }
        let n = display_name(key);
        self.names.insert(key.to_owned(), n.clone());
        n
    }
}

/// `Name=` from the app's .desktop file, else the key itself.
pub fn display_name(key: &str) -> String {
    desktop_entry(key)
        .map(|(n, _)| n)
        .unwrap_or_else(|| key.to_owned())
}

/// (`Name=`, `Icon=`) of `<id>.desktop` (also tried lowercase) from the
/// user, Flatpak and system application folders.
pub fn desktop_entry(id: &str) -> Option<(String, Option<String>)> {
    let home = std::env::var("HOME").unwrap_or_default();
    let dirs = [
        format!("{home}/.local/share/applications"),
        format!("{home}/.local/share/flatpak/exports/share/applications"),
        "/var/lib/flatpak/exports/share/applications".into(),
        "/usr/share/applications".into(),
    ];
    let ids = [id.to_owned(), id.to_lowercase()];
    let text = dirs
        .iter()
        .flat_map(|d| ids.iter().map(move |i| format!("{d}/{i}.desktop")))
        .find_map(|p| fs::read_to_string(p).ok())?;
    // Only the main [Desktop Entry] group, not actions.
    let main = text.split("\n[").next().unwrap_or(&text);
    let name = main
        .lines()
        .find_map(|l| l.strip_prefix("Name="))?
        .to_owned();
    let icon = main
        .lines()
        .find_map(|l| l.strip_prefix("Icon="))
        .map(str::to_owned);
    Some((name, icon))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_ids() {
        let cg = |unit: &str| {
            format!("0::/user.slice/user-1000.slice/user@1000.service/app.slice/{unit}\n")
        };
        assert_eq!(
            app_id_from_cgroup(&cg("app-gnome-firefox-12345.scope")),
            Some("firefox".into())
        );
        assert_eq!(
            app_id_from_cgroup(&cg("app-flatpak-com.discordapp.Discord-3687345403.scope")),
            Some("com.discordapp.Discord".into())
        );
        assert_eq!(
            app_id_from_cgroup(&cg("app-gnome-google\\x2dchrome-77.scope")),
            Some("google-chrome".into())
        );
        assert_eq!(
            app_id_from_cgroup(&cg("dbus-:1.1-org.gnome.Loupe@0.service")),
            Some("org.gnome.Loupe".into())
        );
        assert_eq!(
            app_id_from_cgroup(&cg("app-gnome-dev.warp.Warp-98922.scope")),
            Some("dev.warp.Warp".into())
        );
        assert_eq!(
            app_id_from_cgroup(&cg("org.gnome.Shell@user.service")),
            None
        );
        assert_eq!(app_id_from_cgroup(&cg("app-gnome-23707.scope")), None);
        assert_eq!(app_id_from_cgroup(""), None);
    }

    #[test]
    fn stat_ticks_with_odd_names() {
        let s = "1234 (Web Content (x)) S 1 2 3 4 5 6 7 8 9 10 150 50 0 0 20 0 1 0 100 0 0";
        assert_eq!(parse_stat_ticks(s), Some(("Web Content (x)".into(), 200)));
        assert_eq!(parse_stat_ticks("garbage"), None);
    }
}
