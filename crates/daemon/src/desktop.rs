// SPDX-License-Identifier: GPL-3.0-or-later
//! Small clients for desktop D-Bus services: notifications, GNOME's
//! session inhibitor and logind's lid lock (keep awake) and
//! power-profiles-daemon. Also the
//! microphone mute switch (PipeWire's wpctl).

use std::collections::HashMap;

use zbus::Variant;
use zbus::proxy;

#[proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
pub trait Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: HashMap<&str, &Variant<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;
}

pub async fn notify(conn: &zbus::Connection, summary: &str, body: &str) {
    let Ok(p) = NotificationsProxy::new(conn).await else {
        return;
    };
    let _ = p
        .notify(
            "Andromeda",
            0,
            "io.github.incept0s.Andromeda",
            summary,
            body,
            &[],
            HashMap::new(),
            -1,
        )
        .await;
}

/// GNOME honours this inhibitor for both suspend and screen blanking; a
/// logind "idle" inhibitor alone does not stop GNOME from blanking.
#[proxy(
    interface = "org.gnome.SessionManager",
    default_service = "org.gnome.SessionManager",
    default_path = "/org/gnome/SessionManager"
)]
pub trait SessionManager {
    fn inhibit(
        &self,
        app_id: &str,
        toplevel_xid: u32,
        reason: &str,
        flags: u32,
    ) -> zbus::Result<u32>;
    fn uninhibit(&self, cookie: u32) -> zbus::Result<()>;
}

/// logind suspends on lid close (HandleLidSwitch) and ignores ordinary
/// sleep inhibitors for it; only a "handle-lid-switch" block lock stops it.
/// gsd-power takes one itself only while an external monitor is attached.
/// Polkit allows it for the active session without a password.
#[proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
pub trait Login1 {
    /// The lock lasts until the returned file descriptor is closed.
    fn inhibit(&self, what: &str, who: &str, why: &str, mode: &str) -> zbus::Result<zbus::OwnedFd>;
}

/// Inhibit flags: 4 = suspend, 8 = idle (screen blank / idle actions).
pub const INHIBIT_SUSPEND_AND_IDLE: u32 = 4 | 8;

/// power-profiles-daemon (shipped and enabled on Fedora Workstation).
#[proxy(
    interface = "org.freedesktop.UPower.PowerProfiles",
    default_service = "org.freedesktop.UPower.PowerProfiles",
    default_path = "/org/freedesktop/UPower/PowerProfiles"
)]
pub trait PowerProfiles {
    #[zbus(property)]
    fn active_profile(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_active_profile(&self, profile: &str) -> zbus::Result<()>;
    #[zbus(property)]
    fn profiles(&self) -> zbus::Result<Vec<HashMap<String, zbus::OwnedVariant>>>;
}

/// Mutes or unmutes the default microphone; returns whether it is now muted.
/// PipeWire's own tool, run once per key press.
pub async fn toggle_mic_mute() -> Result<bool, String> {
    const SOURCE: &str = "@DEFAULT_AUDIO_SOURCE@";
    let missing = |_| "wpctl (PipeWire) is needed to mute the microphone.".to_owned();
    let set = tokio::process::Command::new("wpctl")
        .args(["set-mute", SOURCE, "toggle"])
        .status()
        .await
        .map_err(missing)?;
    if !set.success() {
        return Err("No microphone found.".into());
    }
    let out = tokio::process::Command::new("wpctl")
        .args(["get-volume", SOURCE])
        .output()
        .await
        .map_err(missing)?;
    Ok(is_muted(&String::from_utf8_lossy(&out.stdout)))
}

/// `wpctl get-volume` prints "Volume: 0.19" or "Volume: 0.19 [MUTED]".
fn is_muted(wpctl_output: &str) -> bool {
    wpctl_output.contains("[MUTED]")
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_wpctl_mute_state() {
        assert!(!super::is_muted("Volume: 0.19\n"));
        assert!(super::is_muted("Volume: 0.19 [MUTED]\n"));
        assert!(!super::is_muted(""));
    }
}
