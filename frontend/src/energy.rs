// SPDX-License-Identifier: GPL-3.0-or-later
//! Energy: battery and power flow, power mode, accessory batteries and the
//! apps using the most CPU (Linux has no per-app energy figure, so CPU
//! time is the honest proxy, labelled as such).

use andromeda_core::metrics::{Group, Unit, format, format_bytes};
use andromeda_core::procs::AppUsage;
use leptos::prelude::*;

use crate::system::system_battery;
use crate::tauri;
use crate::widgets::{Bar, KeyValue, POLL_FAST, POLL_SLOW, Section, Spark, data, fmt, kv, poll};

#[derive(serde::Serialize)]
struct Limit {
    limit: u32,
}
#[derive(serde::Serialize)]
struct Profile {
    profile: String,
}

fn profile_label(p: &str) -> &str {
    match p {
        "power-saver" => "Power saver",
        "balanced" => "Balanced",
        "performance" => "Performance",
        other => other,
    }
}

/// Accessory batteries: battery metrics that have no "health" sibling.
fn accessories() -> Vec<(String, String)> {
    data().metrics.with(|ms| {
        ms.iter()
            .filter(|m| m.group == Group::Battery && m.id.ends_with(".charge"))
            .filter(|m| !ms.iter().any(|x| x.id == m.id.replace(".charge", ".health")))
            .map(|m| (m.id.clone(), m.name.clone()))
            .collect()
    })
}

#[component]
pub fn EnergyTab() -> impl IntoView {
    let apps = RwSignal::new(Vec::<AppUsage>::new());
    poll(POLL_FAST, move || {
        tauri::spawn(async move {
            if let Ok(v) = tauri::invoke::<_, Vec<AppUsage>>("top_apps", &Limit { limit: 8 }).await {
                apps.set(v);
            }
        });
    });
    let profile = RwSignal::new((String::new(), Vec::<String>::new()));
    poll(POLL_SLOW, move || {
        tauri::spawn(async move {
            if let Some(p) = tauri::get("power_profile").await {
                profile.set(p);
            }
        });
    });
    let set_profile = move |p: String| {
        profile.update(|(active, _)| *active = p.clone());
        tauri::call("set_power_profile", Profile { profile: p });
    };
    let battery = system_battery();
    // Apps under half a percent of the CPU are not worth a row.
    let busy = move || apps.get().into_iter().filter(|a| a.cpu >= 0.5).collect::<Vec<_>>();

    view! {
        {battery.map(|b| {
            let (charge, power) = (format!("{b}.charge"), format!("{b}.power"));
            let state = {
                let power = power.clone();
                move || {
                    let d = data();
                    let w = d.value(&power).unwrap_or(0.0);
                    match d.value("power.ac") {
                        Some(ac) if ac > 0.0 && w > 0.05 => "Charging",
                        Some(ac) if ac > 0.0 => "Plugged in",
                        _ => "On battery",
                    }
                }
            };
            let c2 = charge.clone();
            view! {
                <Section title="Battery">
                    <div class="row">
                        <span class="label">{state}</span>
                        <Bar id=charge color="green" />
                        <span class="value">{move || fmt(&c2, Unit::Percent)}</span>
                    </div>
                    <Spark id=format!("{b}.charge") max=100.0 color="green" />
                    {kv("Power draw", &power, Unit::Watts)}
                    <Spark id=power />
                    {kv("Health", &format!("{b}.health"), Unit::Percent)}
                    {kv("Charge cycles", &format!("{b}.cycles"), Unit::Flag)}
                </Section>
            }
        })}
        <Show when=move || !profile.with(|p| p.1.is_empty())>
            <Section title="Power mode">
                <div class="chips">
                    {move || {
                        let (active, list) = profile.get();
                        list.into_iter().map(|p| {
                            let sel = p == active;
                            let label = profile_label(&p).to_owned();
                            view! { <button class="chip" class:sel=sel on:click=move |_| set_profile(p.clone())>{label}</button> }
                        }).collect_view()
                    }}
                </div>
            </Section>
        </Show>
        <Show when=move || !accessories().is_empty()>
            <Section title="Accessories">
                {move || accessories().into_iter().map(|(id, name)| {
                    let id2 = id.clone();
                    view! { <div class="row"><span class="label">{name}</span><Bar id color="green" /><span class="value">{move || fmt(&id2, Unit::Percent)}</span></div> }
                }).collect_view()}
            </Section>
        </Show>
        <Section title="Apps using significant energy">
            <p class="muted small">"By CPU use: Linux has no per-app energy figure."</p>
            <Show when=move || !busy().is_empty() fallback=|| view! { <p class="muted small">"No significant energy use"</p> }>
                {move || busy().into_iter().map(|a| view! {
                    <KeyValue name=a.name value=Signal::stored(format!("{} · {}", format(a.cpu, Unit::Percent), format_bytes(a.memory))) />
                }).collect_view()}
            </Show>
        </Section>
    }
}
