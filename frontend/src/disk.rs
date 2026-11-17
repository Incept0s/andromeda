// SPDX-License-Identifier: GPL-3.0-or-later
//! Disks: space per volume, read/write speed, drive health (SMART) and
//! eject for removable drives.

use andromeda_core::info::Drive;
use andromeda_core::metrics::{Group, Unit, format, format_bytes};
use leptos::prelude::*;

use crate::tauri;
use crate::widgets::{KeyValue, Meter, POLL_SLOW, Section, Spark, data, fmt, poll};

#[derive(serde::Serialize)]
struct Path {
    path: String,
}
#[derive(serde::Serialize)]
struct Eject {
    drive: String,
}

/// Volumes from the registry: (id key, display name).
fn volumes() -> Vec<(String, String)> {
    data().metrics.with(|ms| {
        ms.iter()
            .filter(|m| m.group == Group::Disk)
            .filter_map(|m| {
                let key = m.id.strip_prefix("disk.")?.strip_suffix(".used")?;
                Some((key.to_owned(), m.name.trim_end_matches(" used").to_owned()))
            })
            .collect()
    })
}

/// Whole disks with I/O metrics.
fn io_disks() -> Vec<String> {
    data().metrics.with(|ms| {
        ms.iter()
            .filter_map(|m| m.id.strip_prefix("disk.")?.strip_suffix(".read").map(str::to_owned))
            .collect()
    })
}

fn opt(v: Option<f64>, f: impl Fn(f64) -> String) -> String {
    v.map_or("–".into(), f)
}

#[component]
fn DriveCard(drive: Drive, error: RwSignal<Option<String>>) -> impl IntoView {
    let open = RwSignal::new(false);
    let health_class = match drive.health {
        _ if drive.failing => "crit",
        Some(h) if h < 50.0 => "warn",
        Some(_) => "ok",
        None => "",
    };
    let path = drive.path.clone();
    let ejecting = RwSignal::new(false);
    let eject = move |_| {
        ejecting.set(true);
        let drive = path.clone();
        tauri::spawn(async move {
            if let Err(e) = tauri::invoke::<_, ()>("eject", &Eject { drive }).await {
                error.set(Some(e));
            }
            ejecting.set(false);
        });
    };
    let d = drive.clone();
    view! {
        <div class="usage">
            <div class="row clickable" on:click=move |_| open.update(|o| *o = !*o)>
                <span class="chev" class:open=move || open.get()>"›"</span>
                <span class="grow ellipsis">{drive.model.clone()}</span>
                {drive.health.map(|h| view! { <span class=format!("badge {health_class}")><i></i>{format!("{h:.0}%")}</span> })}
                {drive.failing.then(|| view! { <span class="badge crit"><i></i>"Failing"</span> })}
            </div>
            <Show when=move || open.get()>
                <div class="details">
                    {(d.size > 0.0).then(|| view! { <KeyValue name="Size" value=Signal::stored(format_bytes(d.size)) /> })}
                    <KeyValue name="Temperature" value=Signal::stored(opt(d.temp_c, |t| format(t, Unit::Celsius))) />
                    <KeyValue name="Power-on hours" value=Signal::stored(opt(d.power_on_hours, |h| format!("{h:.0} h"))) />
                    <KeyValue name="Power cycles" value=Signal::stored(opt(d.power_cycles, |c| format!("{c:.0}"))) />
                    <KeyValue name="Unsafe shutdowns" value=Signal::stored(opt(d.unsafe_shutdowns, |c| format!("{c:.0}"))) />
                    <KeyValue name="Media errors" value=Signal::stored(opt(d.media_errors, |c| format!("{c:.0}"))) />
                    <KeyValue name="Total read" value=Signal::stored(opt(d.read_bytes, format_bytes)) />
                    <KeyValue name="Total written" value=Signal::stored(opt(d.written_bytes, format_bytes)) />
                    {d.mounts.iter().filter(|m| !m.starts_with("/boot")).map(|m| {
                        let p = m.clone();
                        view! {
                            <div class="kv"><span class="muted">"Mounted at"</span>
                                <span>{m.clone()}" "<button class="link" on:click=move |_| tauri::call("open_path", Path { path: p.clone() })>"Open"</button></span>
                            </div>
                        }
                    }).collect_view()}
                </div>
            </Show>
            {drive.can_eject.then(|| view! {
                <button class="btn small-btn" disabled=move || ejecting.get() on:click=eject>
                    {move || if ejecting.get() { "Ejecting…" } else { "Eject" }}
                </button>
            })}
        </div>
    }
}

#[component]
pub fn DiskTab() -> impl IntoView {
    let drives = RwSignal::new(Vec::<Drive>::new());
    let error = RwSignal::new(None::<String>);
    poll(POLL_SLOW, move || {
        tauri::spawn(async move {
            if let Some(v) = tauri::get("drives").await {
                drives.set(v);
            }
        });
    });

    view! {
        <Section title="Volumes">
            {move || volumes().into_iter().map(|(key, name)| {
                let (u, t, f) = (format!("disk.{key}.used"), format!("disk.{key}.total"), format!("disk.{key}.free_pct"));
                let f2 = f.clone();
                let pct = Signal::derive(move || 100.0 - data().value(&f).unwrap_or(100.0));
                let line = move || {
                    let d = data();
                    match (d.value(&u), d.value(&t), d.value(&f2)) {
                        (Some(used), Some(total), Some(free)) => format!("{} of {} used · {:.0}% free", format_bytes(used), format_bytes(total), free),
                        _ => "–".into(),
                    }
                };
                view! {
                    <div class="usage">
                        <div class="row"><strong class="grow">{name}</strong></div>
                        <Meter pct />
                        <div class="muted small">{line}</div>
                    </div>
                }
            }).collect_view()}
        </Section>
        <Section title="Activity">
            {move || io_disks().into_iter().map(|dev| {
                let (r, w) = (format!("disk.{dev}.read"), format!("disk.{dev}.write"));
                let (r2, w2) = (r.clone(), w.clone());
                view! {
                    <div class="usage">
                        <div class="row">
                            <span class="grow">{dev}</span>
                            <span class="small">"R "{move || fmt(&r2, Unit::BytesPerSecond)}" · W "{move || fmt(&w2, Unit::BytesPerSecond)}</span>
                        </div>
                        <Spark id=r />
                        <Spark id=w color="teal" />
                    </div>
                }
            }).collect_view()}
        </Section>
        <Section title="Drives">
            {move || error.get().map(|e| view! { <p class="small error">{e}</p> })}
            <Show when=move || !drives.with(Vec::is_empty) fallback=|| view! { <p class="muted small">"Reading drives…"</p> }>
                <For each=move || drives.get() key=|d| d.path.clone() let:drive>
                    <DriveCard drive error />
                </For>
            </Show>
        </Section>
    }
}
