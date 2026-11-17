// SPDX-License-Identifier: GPL-3.0-or-later
//! Drives, SMART data and eject through UDisks2 (system bus). Reading SMART
//! works without a password for NVMe and ATA drives; ejecting removable
//! media in the active session does too.

use std::collections::HashMap;

use andromeda_core::info::Drive;
use zbus::fdo::ObjectManagerProxy;
use zbus::{OwnedObjectPath, OwnedVariant as OwnedValue};

const UD: &str = "org.freedesktop.UDisks2";
type Props = HashMap<String, OwnedValue>;
type Objects = HashMap<OwnedObjectPath, HashMap<String, Props>>;

async fn objects(sys: &zbus::Connection) -> zbus::Result<Objects> {
    let om = ObjectManagerProxy::builder(sys).destination(UD).path("/org/freedesktop/UDisks2").build().await?;
    Ok(om
        .get_managed_objects()
        .await?
        .into_iter()
        .map(|(p, ifaces)| (p, ifaces.into_iter().map(|(k, v)| (k.to_string(), v)).collect()))
        .collect())
}

fn get<T>(p: &Props, key: &str) -> Option<T>
where
    T: TryFrom<OwnedValue>,
{
    p.get(key)?.try_clone().ok()?.try_into().ok()
}

/// Mount points are arrays of NUL-terminated byte strings.
fn mount_points(fs: &Props) -> Vec<String> {
    let raw: Vec<Vec<u8>> = get(fs, "MountPoints").unwrap_or_default();
    raw.into_iter()
        .map(|b| String::from_utf8_lossy(b.strip_suffix(&[0]).unwrap_or(&b)).into_owned())
        .collect()
}

pub async fn list(sys: &zbus::Connection) -> zbus::Result<Vec<Drive>> {
    let objs = objects(sys).await?;
    let mut drives = Vec::new();
    for (path, ifaces) in &objs {
        let Some(d) = ifaces.get(&format!("{UD}.Drive")) else { continue };
        let mut drive = Drive {
            path: path.to_string(),
            model: get::<String>(d, "Model").filter(|m| !m.is_empty()).unwrap_or_else(|| "Drive".into()),
            size: get::<u64>(d, "Size").unwrap_or(0) as f64,
            removable: get(d, "Removable").unwrap_or(false),
            can_eject: get::<bool>(d, "Ejectable").unwrap_or(false) || get::<bool>(d, "CanPowerOff").unwrap_or(false),
            bus: get(d, "ConnectionBus").unwrap_or_default(),
            ..Default::default()
        };
        // Mounts of this drive's filesystems.
        for b in objs.values() {
            if let (Some(block), Some(fs)) = (b.get(&format!("{UD}.Block")), b.get(&format!("{UD}.Filesystem")))
                && get::<OwnedObjectPath>(block, "Drive").is_some_and(|p| p == *path)
            {
                drive.mounts.extend(mount_points(fs));
            }
        }
        if let Some(n) = ifaces.get(&format!("{UD}.NVMe.Controller")) {
            // Kelvin; 0 means unknown.
            drive.temp_c = get::<u16>(n, "SmartTemperature").filter(|t| *t > 0).map(|t| f64::from(t) - 273.15);
            drive.power_on_hours = get::<u64>(n, "SmartPowerOnHours").map(|h| h as f64);
            drive.failing = get::<Vec<String>>(n, "SmartCriticalWarning").is_some_and(|w| !w.is_empty());
            if let Ok(attrs) = nvme_attributes(sys, path).await {
                let a = |k: &str| attrs.get(k).and_then(|v| v.try_clone().ok()).and_then(as_f64);
                drive.health = a("percent_used").map(|u| (100.0 - u).max(0.0));
                drive.power_cycles = a("power_cycles");
                drive.unsafe_shutdowns = a("unsafe_shutdowns");
                drive.media_errors = a("media_errors");
                // NVMe counts data in units of 512 000 bytes, but UDisks
                // already converts to bytes.
                drive.read_bytes = a("total_data_read");
                drive.written_bytes = a("total_data_written");
            }
        } else if let Some(ata) = ifaces.get(&format!("{UD}.Drive.Ata")) {
            if get::<bool>(ata, "SmartEnabled").unwrap_or(false) {
                drive.temp_c = get::<f64>(ata, "SmartTemperature").filter(|t| *t > 0.0).map(|t| t - 273.15);
                drive.power_on_hours = get::<u64>(ata, "SmartPowerOnSeconds").map(|s| s as f64 / 3600.0);
                drive.failing = get(ata, "SmartFailing").unwrap_or(false);
            }
        }
        drives.push(drive);
    }
    // Internal drives first, then removable ones.
    drives.sort_by(|a, b| a.removable.cmp(&b.removable).then(a.model.cmp(&b.model)));
    Ok(drives)
}

fn as_f64(v: OwnedValue) -> Option<f64> {
    if let Ok(x) = u64::try_from(v.try_clone().ok()?) {
        return Some(x as f64);
    }
    if let Ok(x) = u8::try_from(v.try_clone().ok()?) {
        return Some(f64::from(x));
    }
    u32::try_from(v).ok().map(f64::from)
}

async fn nvme_attributes(sys: &zbus::Connection, drive: &OwnedObjectPath) -> zbus::Result<HashMap<String, OwnedValue>> {
    let reply = sys
        .call_method(
            Some(UD),
            drive.as_str(),
            Some(format!("{UD}.NVMe.Controller").as_str()),
            "SmartGetAttributes",
            &(HashMap::<String, OwnedValue>::new(),),
        )
        .await?;
    reply.body().deserialize()
}

/// Unmounts every filesystem on the drive, then powers it off (or ejects
/// the medium) so it is safe to unplug.
pub async fn eject(sys: &zbus::Connection, drive: &str) -> Result<(), String> {
    let objs = objects(sys).await.map_err(|e| e.to_string())?;
    let opts = (HashMap::<String, OwnedValue>::new(),);
    for (path, ifaces) in &objs {
        if let (Some(block), Some(fs)) = (ifaces.get(&format!("{UD}.Block")), ifaces.get(&format!("{UD}.Filesystem")))
            && get::<OwnedObjectPath>(block, "Drive").is_some_and(|p| p.as_str() == drive)
            && !mount_points(fs).is_empty()
        {
            sys.call_method(Some(UD), path.as_str(), Some(format!("{UD}.Filesystem").as_str()), "Unmount", &opts)
                .await
                .map_err(|e| format!("Could not unmount: {e}"))?;
        }
    }
    let iface = format!("{UD}.Drive");
    let power_off = sys.call_method(Some(UD), drive, Some(iface.as_str()), "PowerOff", &opts).await;
    if power_off.is_err() {
        sys.call_method(Some(UD), drive, Some(iface.as_str()), "Eject", &opts)
            .await
            .map_err(|e| format!("Could not eject: {e}"))?;
    }
    Ok(())
}
