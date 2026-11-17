// SPDX-License-Identifier: GPL-3.0-or-later
//! Small clients for desktop D-Bus services: notifications, GNOME's
//! session inhibitor (keep awake) and power-profiles-daemon.

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
