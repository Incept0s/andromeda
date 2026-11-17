// SPDX-License-Identifier: GPL-3.0-or-later
//! Total CPU usage from `/proc/stat`.
//!
//! The first line of `/proc/stat` holds cumulative jiffies since boot:
//! `cpu  user nice system idle iowait irq softirq steal guest guest_nice`.
//! Usage is the share of non-idle time between two readings.

/// Cumulative jiffies from the aggregate `cpu` line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuTimes {
    /// idle + iowait: time the CPU had nothing to run.
    pub idle: u64,
    /// Every field except guest/guest_nice, which the kernel already counts
    /// inside user/nice (adding them would count guest time twice).
    pub total: u64,
}

/// Parses the aggregate `cpu ` line. Returns `None` on unexpected input
/// instead of panicking: kernels and containers vary.
pub fn parse_proc_stat(text: &str) -> Option<CpuTimes> {
    parse_line(text.lines().find(|l| l.starts_with("cpu "))?)
}

/// Per-core lines (`cpu0`, `cpu1`, ...) in order. Offline cores have no
/// line, so the count can be lower than the number of installed cores.
pub fn parse_cores(text: &str) -> Vec<Option<CpuTimes>> {
    text.lines()
        .filter(|l| l.starts_with("cpu") && l.as_bytes().get(3).is_some_and(u8::is_ascii_digit))
        .map(parse_line)
        .collect()
}

fn parse_line(line: &str) -> Option<CpuTimes> {
    let fields: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .map(|f| f.parse().ok())
        .collect::<Option<_>>()?;
    // Old kernels have only 4 fields (user nice system idle); iowait onwards
    // may be missing, so treat absent fields as 0.
    let get = |i: usize| fields.get(i).copied().unwrap_or(0);
    if fields.len() < 4 {
        return None;
    }
    let idle = get(3) + get(4);
    let total = (0..8).map(get).sum();
    Some(CpuTimes { idle, total })
}

/// Usage in percent (0..=100) between two readings, or `None` if no time
/// passed or the counters went backwards (e.g. after a CPU was hot-unplugged).
pub fn usage_percent(prev: CpuTimes, cur: CpuTimes) -> Option<f64> {
    let total = cur.total.checked_sub(prev.total)?;
    let idle = cur.idle.checked_sub(prev.idle)?;
    if total == 0 {
        return None;
    }
    let busy = total.saturating_sub(idle);
    Some(busy as f64 * 100.0 / total as f64)
}

/// Reads `/proc/stat` now.
pub fn read() -> Option<CpuTimes> {
    parse_proc_stat(&std::fs::read_to_string("/proc/stat").ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "cpu  264069 2003 81630 16578297 6999 24760 11461 0 0 0\n\
                          cpu0 27951 234 8291 492351 499 1035 2033 0 0 0\n";

    #[test]
    fn parses_aggregate_line() {
        let t = parse_proc_stat(SAMPLE).unwrap();
        assert_eq!(t.idle, 16578297 + 6999);
        assert_eq!(t.total, 264069 + 2003 + 81630 + 16578297 + 6999 + 24760 + 11461);
    }

    #[test]
    fn parses_cores() {
        let cores = parse_cores(SAMPLE);
        assert_eq!(cores.len(), 1);
        assert_eq!(cores[0].unwrap().idle, 492351 + 499);
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(parse_proc_stat(""), None);
        assert_eq!(parse_proc_stat("cpu  1 2 x 4\n"), None);
        assert_eq!(parse_proc_stat("cpu  1 2\n"), None);
    }

    #[test]
    fn computes_usage() {
        let a = CpuTimes { idle: 100, total: 200 };
        let b = CpuTimes { idle: 150, total: 300 };
        assert_eq!(usage_percent(a, b), Some(50.0));
        assert_eq!(usage_percent(a, a), None);
        assert_eq!(usage_percent(b, a), None);
    }
}
