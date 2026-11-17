// SPDX-License-Identifier: GPL-3.0-or-later
//! Packages: RPMs through PackageKit (D-Bus, the same path GNOME Software
//! uses; polkit asks for the password when needed) and Flatpak apps through
//! the `flatpak` command with stable `--columns` output.

use futures_util::StreamExt;
use zbus::message::Type;
use zbus::{MatchRule, MessageStream, OwnedObjectPath};

use andromeda_core::tools::{Package, PkgSource};
use tokio::process::Command;

const PK: &str = "org.freedesktop.PackageKit";
const TX: &str = "org.freedesktop.PackageKit.Transaction";

/// PackageKit filter bits (1 << PkFilterEnum).
const FILTER_NEWEST: u64 = 1 << 16;
const FILTER_ARCH: u64 = 1 << 18;
const FILTER_NOT_SOURCE: u64 = 1 << 21;
/// PkTransactionFlagEnum ONLY_TRUSTED: refuse unsigned packages.
const ONLY_TRUSTED: u64 = 1 << 1;
/// PkInfoEnum INSTALLED.
const INFO_INSTALLED: u32 = 1;

/// Runs one PackageKit transaction: creates it, subscribes to its
/// signals, calls `method(args)`, collects `Package` signals until
/// `Finished`. Errors carry PackageKit's own message.
async fn transaction<B>(sys: &zbus::Connection, method: &str, args: &B) -> Result<Vec<(u32, String, String)>, String>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    let reply = sys
        .call_method(Some(PK), "/org/freedesktop/PackageKit", Some(PK), "CreateTransaction", &())
        .await
        .map_err(|e| format!("PackageKit is not available: {e}"))?;
    let path: OwnedObjectPath = reply.body().deserialize().map_err(|e| e.to_string())?;
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender(PK)
        .path(path.as_str())
        .build()
        .map_err(|e| e.to_string())?;
    let mut stream = MessageStream::for_match_rule(rule, sys, std::num::NonZeroUsize::new(256)).await.map_err(|e| e.to_string())?;
    sys.call_method(Some(PK), path.as_str(), Some(TX), method, args).await.map_err(|e| e.to_string())?;

    let mut packages = Vec::new();
    let mut error = None;
    while let Some(Ok(msg)) = stream.next().await {
        let h = msg.header();
        match h.member().map(|m| m.as_str()) {
            Some("Package") => {
                if let Ok(p) = msg.body().deserialize::<(u32, String, String)>() {
                    packages.push(p);
                }
            }
            Some("ErrorCode") => {
                if let Ok((_, details)) = msg.body().deserialize::<(u32, String)>() {
                    error = Some(details);
                }
            }
            Some("Finished") => break,
            _ => {}
        }
    }
    match error {
        Some(e) => Err(e),
        None => Ok(packages),
    }
}

/// "name;version;arch;repo" -> Package.
fn rpm(info: u32, id: String, summary: String) -> Package {
    let mut f = id.split(';');
    let name = f.next().unwrap_or_default().to_owned();
    let version = f.next().unwrap_or_default().to_owned();
    let repo = id.rsplit(';').next().unwrap_or_default();
    Package {
        name,
        version,
        summary,
        installed: info == INFO_INSTALLED || repo.starts_with("installed") || repo.starts_with('@'),
        id,
        source: PkgSource::Rpm,
    }
}

fn dedupe(mut v: Vec<Package>) -> Vec<Package> {
    // Installed and available copies of the same name: keep the installed.
    v.sort_by(|a, b| a.name.cmp(&b.name).then(b.installed.cmp(&a.installed)));
    v.dedup_by(|a, b| a.name == b.name);
    v
}

pub async fn search_rpm(sys: &zbus::Connection, query: &str) -> Result<Vec<Package>, String> {
    let filter = FILTER_NEWEST | FILTER_ARCH | FILTER_NOT_SOURCE;
    let words: Vec<&str> = query.split_whitespace().collect();
    let found = transaction(sys, "SearchNames", &(filter, words)).await?;
    let mut v = dedupe(found.into_iter().map(|(i, id, s)| rpm(i, id, s)).collect());
    // Exact and prefix matches first.
    let q = query.to_lowercase();
    v.sort_by_key(|p| (p.name != q, !p.name.starts_with(&q), p.name.len()));
    v.truncate(50);
    Ok(v)
}

pub async fn install_rpm(sys: &zbus::Connection, id: &str) -> Result<(), String> {
    transaction(sys, "InstallPackages", &(ONLY_TRUSTED, vec![id])).await.map(|_| ())
}

pub async fn remove_rpm(sys: &zbus::Connection, id: &str) -> Result<(), String> {
    // allow_deps = true (remove what depends on it), autoremove = true.
    transaction(sys, "RemovePackages", &(ONLY_TRUSTED, vec![id], true, true)).await.map(|_| ())
}

pub async fn rpm_updates(sys: &zbus::Connection) -> Result<Vec<Package>, String> {
    let found = transaction(sys, "GetUpdates", &(0u64,)).await?;
    Ok(found.into_iter().map(|(_, id, s)| Package { installed: true, ..rpm(0, id, s) }).collect())
}

pub async fn update_rpms(sys: &zbus::Connection, ids: &[String]) -> Result<(), String> {
    transaction(sys, "UpdatePackages", &(ONLY_TRUSTED, ids)).await.map(|_| ())
}

/// Runs flatpak with tab-separated columns and splits the rows.
async fn flatpak(args: &[&str]) -> Result<Vec<Vec<String>>, String> {
    let out = Command::new("flatpak").args(args).output().await.map_err(|_| "Flatpak is not installed.".to_owned())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("flatpak failed").to_owned());
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.split('\t').map(str::to_owned).collect())
        .collect())
}

pub async fn flatpak_installed() -> Vec<String> {
    flatpak(&["list", "--app", "--columns=application"]).await.unwrap_or_default().into_iter().filter_map(|r| r.into_iter().next()).collect()
}

pub async fn search_flatpak(query: &str) -> Result<Vec<Package>, String> {
    let installed = flatpak_installed().await;
    let rows = flatpak(&["search", "--columns=application,name,description,version", query]).await?;
    Ok(rows
        .into_iter()
        .filter(|r| r.len() >= 2 && r[0].contains('.'))
        .take(50)
        .map(|r| Package {
            installed: installed.contains(&r[0]),
            name: r[1].clone(),
            summary: r.get(2).cloned().unwrap_or_default(),
            version: r.get(3).cloned().unwrap_or_default(),
            id: r[0].clone(),
            source: PkgSource::Flatpak,
        })
        .collect())
}

pub async fn flatpak_updates() -> Result<Vec<Package>, String> {
    let rows = flatpak(&["remote-ls", "--updates", "--app", "--columns=application,name,version"]).await?;
    Ok(rows
        .into_iter()
        .filter(|r| !r.is_empty())
        .map(|r| Package {
            name: r.get(1).cloned().unwrap_or_else(|| r[0].clone()),
            version: r.get(2).cloned().unwrap_or_default(),
            summary: String::new(),
            installed: true,
            id: r[0].clone(),
            source: PkgSource::Flatpak,
        })
        .collect())
}

/// Install / uninstall / update. Flatpak asks polkit itself for system
/// installs. Ids come from flatpak's own output; a leading '-' is refused
/// so an id can never be read as an option.
pub async fn flatpak_action(action: &str, id: &str) -> Result<(), String> {
    if id.starts_with('-') {
        return Err("invalid id".into());
    }
    let args: Vec<&str> = match action {
        "install" => vec!["install", "-y", "--noninteractive", "flathub", id],
        "remove" => vec!["uninstall", "-y", "--noninteractive", id],
        "update" => vec!["update", "-y", "--noninteractive"],
        _ => return Err("unknown action".into()),
    };
    flatpak(&args).await.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_package_ids() {
        let p = rpm(2, "htop;3.3.0-1.fc44;x86_64;fedora".into(), "Viewer".into());
        assert_eq!((p.name.as_str(), p.version.as_str(), p.installed), ("htop", "3.3.0-1.fc44", false));
        assert!(rpm(1, "bash;5;x86_64;installed".into(), String::new()).installed);
        let v = dedupe(vec![
            rpm(2, "a;2;x86_64;fedora".into(), String::new()),
            rpm(1, "a;1;x86_64;installed".into(), String::new()),
        ]);
        assert_eq!(v.len(), 1);
        assert!(v[0].installed);
    }
}
