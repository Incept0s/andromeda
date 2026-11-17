// SPDX-License-Identifier: GPL-3.0-or-later
//! Network details for the Network tab: interfaces with addresses, apps
//! with open connections, and the speed test.
//!
//! Per-app traffic is not available on Linux without root (it needs eBPF or
//! packet capture), so the app list counts open connections instead.

use std::collections::HashMap;
use std::ffi::CStr;
use std::fs;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::Path;

use andromeda_core::info::{Interface, LinkKind, NetApp, SpeedResult};
use andromeda_core::procs::app_id_from_cgroup;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// All interfaces except loopback, with their addresses from getifaddrs(3).
pub fn interfaces() -> Vec<Interface> {
    let mut addrs: HashMap<String, (Vec<String>, Vec<String>)> = HashMap::new();
    let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills `head` with a linked list we only read and
    // free once with freeifaddrs.
    if unsafe { libc::getifaddrs(&mut head) } == 0 {
        let mut cur = head;
        while !cur.is_null() {
            // SAFETY: `cur` is a valid node of the list until freeifaddrs.
            let ifa = unsafe { &*cur };
            cur = ifa.ifa_next;
            if ifa.ifa_addr.is_null() {
                continue;
            }
            // SAFETY: ifa_name is a NUL-terminated string owned by the list.
            let name = unsafe { CStr::from_ptr(ifa.ifa_name) }.to_string_lossy().into_owned();
            let entry = addrs.entry(name).or_default();
            // SAFETY: the family tells which sockaddr variant the pointer is.
            match i32::from(unsafe { (*ifa.ifa_addr).sa_family }) {
                libc::AF_INET => {
                    let sa = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_in) };
                    entry.0.push(Ipv4Addr::from(u32::from_be(sa.sin_addr.s_addr)).to_string());
                }
                libc::AF_INET6 => {
                    let sa = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_in6) };
                    let ip = Ipv6Addr::from(sa.sin6_addr.s6_addr);
                    // Link-local addresses are noise for users.
                    if (ip.segments()[0] & 0xffc0) != 0xfe80 {
                        entry.1.push(ip.to_string());
                    }
                }
                _ => {}
            }
        }
        // SAFETY: frees the list allocated above exactly once.
        unsafe { libc::freeifaddrs(head) };
    }

    let mut out = Vec::new();
    for dir in fs::read_dir("/sys/class/net").into_iter().flatten().flatten() {
        let name = dir.file_name().to_string_lossy().into_owned();
        if name == "lo" {
            continue;
        }
        let p = dir.path();
        let kind = if p.join("wireless").exists() {
            LinkKind::Wifi
        } else if p.join("device").exists() {
            LinkKind::Ethernet
        } else if p.join("tun_flags").exists() || name.starts_with("wg") || name.starts_with("tailscale") {
            LinkKind::Vpn
        } else {
            LinkKind::Other
        };
        let oper = fs::read_to_string(p.join("operstate")).unwrap_or_default();
        let (v4, v6) = addrs.remove(&name).unwrap_or_default();
        // VPN tunnels often report "unknown" while working; count addresses.
        let up = oper.trim() == "up" || (oper.trim() == "unknown" && !v4.is_empty());
        let speed_mbps = fs::read_to_string(p.join("speed"))
            .ok()
            .and_then(|s| s.trim().parse::<i64>().ok())
            .filter(|s| *s > 0)
            .map(|s| s as u32);
        out.push(Interface { name, kind, up, addresses: v4.into_iter().chain(v6).collect(), speed_mbps });
    }
    // Connected first, then Wi-Fi/Ethernet before VPN and virtual links.
    out.sort_by_key(|i| (!i.up, i.kind as u8, i.name.clone()));
    out
}

/// Socket inodes of established TCP connections and connected UDP sockets.
fn connected_inodes() -> HashMap<u64, ()> {
    let mut set = HashMap::new();
    for (file, tcp) in [("tcp", true), ("tcp6", true), ("udp", false), ("udp6", false)] {
        let Ok(text) = fs::read_to_string(Path::new("/proc/net").join(file)) else { continue };
        for line in text.lines().skip(1) {
            let f: Vec<&str> = line.split_whitespace().collect();
            let (Some(remote), Some(state), Some(inode)) = (f.get(2), f.get(3), f.get(9)) else { continue };
            // TCP state 01 = ESTABLISHED; UDP "connected" = non-zero remote.
            let remote_set = remote.split(':').next().is_some_and(|a| a.chars().any(|c| c != '0'));
            let ok = if tcp { *state == "01" } else { remote_set };
            if ok && let Ok(n) = inode.parse() {
                set.insert(n, ());
            }
        }
    }
    set
}

/// Apps (grouped like the Energy tab) with their open connection count.
/// Only the user's own processes are visible, which is what matters here.
pub fn apps() -> Vec<NetApp> {
    let inodes = connected_inodes();
    let mut counts: HashMap<String, u32> = HashMap::new();
    for proc_dir in fs::read_dir("/proc").into_iter().flatten().flatten() {
        if !proc_dir.file_name().to_string_lossy().chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let Ok(fds) = fs::read_dir(proc_dir.path().join("fd")) else { continue };
        let n = fds
            .flatten()
            .filter_map(|fd| fs::read_link(fd.path()).ok())
            .filter_map(|l| l.to_str()?.strip_prefix("socket:[")?.strip_suffix(']')?.parse::<u64>().ok())
            .filter(|i| inodes.contains_key(i))
            .count() as u32;
        if n == 0 {
            continue;
        }
        let dir = proc_dir.path();
        let cg = fs::read_to_string(dir.join("cgroup")).unwrap_or_default();
        let comm = fs::read_to_string(dir.join("comm")).unwrap_or_default().trim().to_owned();
        let key = app_id_from_cgroup(&cg).unwrap_or(comm);
        *counts.entry(key).or_default() += n;
    }
    let mut v: Vec<NetApp> = counts
        .into_iter()
        .map(|(key, connections)| NetApp { name: andromeda_core::procs::display_name(&key), connections })
        .collect();
    v.sort_by(|a, b| b.connections.cmp(&a.connections).then(a.name.cmp(&b.name)));
    v
}

const SPEED_HOST: &str = "https://speed.cloudflare.com";

/// Download then upload against Cloudflare's speed test, with curl. Only
/// ever runs when the user presses the button.
pub async fn speed_test() -> Result<SpeedResult, String> {
    let down = Command::new("curl")
        .args(["-s", "-o", "/dev/null", "-w", "%{speed_download}", "--max-time", "15"])
        .arg(format!("{SPEED_HOST}/__down?bytes=50000000"))
        .output()
        .await
        .map_err(|e| format!("curl not available: {e}"))?;
    let download = String::from_utf8_lossy(&down.stdout).trim().parse::<f64>().map_err(|_| "download failed")?;

    let mut up = Command::new("curl")
        .args(["-s", "-o", "/dev/null", "-w", "%{speed_upload}", "--max-time", "15", "--data-binary", "@-"])
        .arg(format!("{SPEED_HOST}/__up"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Some(mut stdin) = up.stdin.take() {
        let chunk = vec![0u8; 1 << 20];
        for _ in 0..15 {
            if stdin.write_all(&chunk).await.is_err() {
                break;
            }
        }
    }
    let out = up.wait_with_output().await.map_err(|e| e.to_string())?;
    let upload = String::from_utf8_lossy(&out.stdout).trim().parse::<f64>().unwrap_or(0.0);
    if download <= 0.0 {
        return Err("no connection to the speed test server".into());
    }
    Ok(SpeedResult { download, upload })
}
