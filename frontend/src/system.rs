// SPDX-License-Identifier: GPL-3.0-or-later
//! System tab: temperatures, CPU/GPU/battery usage with graphs, memory,
//! all other sensors, uptime. Layout follows the reference panel.
//! Rows only render for metrics this machine actually has.

use andromeda_core::metrics::{
    Group, Pressure, Unit, format, format_bytes, format_duration, pressure_level,
};
use leptos::prelude::*;

use crate::widgets::{Bar, KeyValue, Spark, data, fmt, kv};

/// GPUs as (id prefix, display name), e.g. ("gpu.card1", "Radeon 610M").
fn gpus() -> Vec<(String, String)> {
    data().metrics.with(|ms| {
        let mut v: Vec<(String, String)> = Vec::new();
        for m in ms.iter().filter(|m| m.group == Group::Gpu) {
            let Some((prefix, kind)) = m.id.rsplit_once('.') else {
                continue;
            };
            let name = match kind {
                "usage" => m.name.clone(),
                "temp" => m.name.trim_end_matches(" temperature").to_owned(),
                _ => continue,
            };
            if !v.iter().any(|(p, _)| p == prefix) {
                v.push((prefix.to_owned(), name));
            }
        }
        v
    })
}

/// The laptop's own battery: the only kind with a health metric.
pub fn system_battery() -> Option<String> {
    data().metrics.with(|ms| {
        ms.iter()
            .find_map(|m| m.id.strip_suffix(".health").map(str::to_owned))
    })
}

/// Label / bar / value row with a sparkline below; click to expand details.
#[component]
fn UsageRow(
    label: String,
    id: String,
    #[prop(default = "accent")] color: &'static str,
    /// Shown when the row is expanded.
    children: ChildrenFn,
) -> impl IntoView {
    let open = RwSignal::new(false);
    let value_id = id.clone();
    view! {
        <div class="usage">
            <div class="row clickable" on:click=move |_| open.update(|o| *o = !*o)>
                <span class="chev" class:open=move || open.get()>"›"</span>
                <span class="label">{label}</span>
                <Bar id=id.clone() color />
                <span class="value">{move || fmt(&value_id, Unit::Percent)}</span>
            </div>
            <Spark id=id max=100.0 color />
            <Show when=move || open.get()>
                <div class="details">{children()}</div>
            </Show>
        </div>
    }
}

#[component]
fn Tile(label: String, id: String) -> impl IntoView {
    view! {
        <div class="tile">
            <span class="muted">{label}</span>
            <strong>{move || fmt(&id, Unit::Celsius)}</strong>
        </div>
    }
}

#[component]
fn CpuDetails() -> impl IntoView {
    let cores = data()
        .metrics
        .with(|ms| ms.iter().filter(|m| m.id.starts_with("cpu.core")).count());
    view! {
        <div class="cores">
            {(0..cores).map(|i| {
                let id = format!("cpu.core{i}");
                let h = move || format!("height:{:.0}%", data().value(&id).unwrap_or(0.0).clamp(2.0, 100.0));
                view! { <span class="core" title=format!("Core {i}")><span style=h></span></span> }
            }).collect_view()}
        </div>
        {kv("Clock", "cpu.freq", Unit::Hertz)}
        {kv("Temperature", "cpu.temp", Unit::Celsius)}
    }
}

#[component]
fn GpuDetails(prefix: String) -> impl IntoView {
    let p = prefix.clone();
    let vram = move || {
        let d = data();
        match (
            d.value(&format!("{p}.vram_used")),
            d.value(&format!("{p}.vram_total")),
        ) {
            (Some(u), Some(t)) => format!("{} / {}", format_bytes(u), format_bytes(t)),
            (Some(u), None) => format_bytes(u),
            _ => "–".into(),
        }
    };
    let has_vram = data().info(&format!("{prefix}.vram_used")).is_some();
    view! {
        {has_vram.then(|| view! { <KeyValue name="Memory" value=Signal::derive(vram) /> })}
        {kv("Clock", &format!("{prefix}.clock"), Unit::Hertz)}
        {kv("Power", &format!("{prefix}.power"), Unit::Watts)}
        {kv("Temperature", &format!("{prefix}.temp"), Unit::Celsius)}
    }
}

#[component]
fn BatteryDetails(prefix: String) -> impl IntoView {
    let p = prefix.clone();
    // Linux has no single "status" metric we trust across firmware, so the
    // state is derived from the AC adapter and the power flow.
    let state = move || {
        let d = data();
        let watts = d.value(&format!("{p}.power")).unwrap_or(0.0);
        match d.value("power.ac") {
            Some(ac) if ac > 0.0 && watts > 0.05 => {
                format!("Charging, {}", format(watts, Unit::Watts))
            }
            Some(ac) if ac > 0.0 => "Plugged in".into(),
            _ if watts > 0.05 => format!("On battery, {}", format(watts, Unit::Watts)),
            _ => "On battery".into(),
        }
    };
    view! {
        <KeyValue name="State" value=Signal::derive(state) />
        {kv("Health", &format!("{prefix}.health"), Unit::Percent)}
        {kv("Charge cycles", &format!("{prefix}.cycles"), Unit::Flag)}
    }
}

#[component]
fn Memory() -> impl IntoView {
    let level = move || data().value("mem.pressure").map(pressure_level);
    let badge = move || match level() {
        Some(Pressure::Normal) => ("ok", "Normal"),
        Some(Pressure::Warning) => ("warn", "Warning"),
        Some(Pressure::Critical) => ("crit", "Critical"),
        None => ("", ""),
    };
    let used = move || {
        let d = data();
        match (d.value("mem.used"), d.value("mem.total")) {
            (Some(u), Some(t)) => format!("{} / {}", format_bytes(u), format_bytes(t)),
            _ => "–".into(),
        }
    };
    let swap = move || {
        let d = data();
        match (d.value("mem.swap_used"), d.value("mem.swap_total")) {
            (Some(u), Some(t)) => format!("{} / {}", format_bytes(u), format_bytes(t)),
            _ => "–".into(),
        }
    };
    let open = RwSignal::new(false);
    view! {
        <h3 class="sub">"Memory"</h3>
        <div class="usage">
            <div class="row clickable" on:click=move |_| open.update(|o| *o = !*o)>
                <span class="chev" class:open=move || open.get()>"›"</span>
                <span class="label">"Pressure"</span>
                <span class=move || format!("badge {}", badge().0) class:hidden=move || level().is_none()>
                    <i></i>{move || badge().1}
                </span>
                <span class="grow"></span>
                <span class="value wide">{used}</span>
            </div>
            <Spark id="mem.used_pct".into() max=100.0 color="teal" />
            <Show when=move || open.get()>
                <div class="details">
                    {kv("Available", "mem.available", Unit::Bytes)}
                    {kv("Cached files", "mem.cached", Unit::Bytes)}
                    <KeyValue name="Swap" value=Signal::derive(swap) />
                    {kv("Compressed (zram)", "mem.compressed", Unit::Bytes)}
                </div>
            </Show>
        </div>
    }
}

/// Every temperature and fan not shown above, collapsed by default.
#[component]
fn Sensors() -> impl IntoView {
    let open = RwSignal::new(false);
    let list = move || {
        data().metrics.with(|ms| {
            ms.iter()
                .filter(|m| matches!(m.group, Group::Temps | Group::Fans))
                .map(|m| (m.id.clone(), m.name.clone(), m.unit))
                .collect::<Vec<_>>()
        })
    };
    view! {
        <Show when=move || !list().is_empty()>
            <div class="row clickable" on:click=move |_| open.update(|o| *o = !*o)>
                <span class="chev" class:open=move || open.get()>"›"</span>
                <span class="label wide">"All sensors"</span>
            </div>
            <Show when=move || open.get()>
                <div class="details">
                    {move || list().into_iter().map(|(id, name, unit)| view! {
                        <KeyValue name value=Signal::derive(move || fmt(&id, unit)) />
                    }).collect_view()}
                </div>
            </Show>
        </Show>
    }
}

#[component]
pub fn SystemTab() -> impl IntoView {
    // Rebuild the layout when the metric list arrives or changes; values
    // inside update on their own.
    move || {
        let d = data();
        if d.metrics.with(Vec::is_empty) {
            return view! { <p class="muted">"Waiting for sensors…"</p> }.into_any();
        }
        let gpus = gpus();
        let battery = system_battery();
        let has = |id: &str| d.info(id).is_some();

        let mut tiles: Vec<(String, String)> = Vec::new();
        if has("cpu.temp") {
            tiles.push(("CPU".into(), "cpu.temp".into()));
        }
        for (prefix, name) in &gpus {
            let id = format!("{prefix}.temp");
            if has(&id) {
                tiles.push((name.clone(), id));
            }
        }

        view! {
            <div class="card">
                {(!tiles.is_empty()).then(|| view! {
                    <h3 class="sub">"Temperatures"</h3>
                    <div class="tiles">
                        {tiles.into_iter().map(|(label, id)| view! { <Tile label id /> }).collect_view()}
                    </div>
                })}
                <h3 class="sub">"Hardware usage"</h3>
                {has("cpu.usage").then(|| view! {
                    <UsageRow label="CPU".into() id="cpu.usage".into()><CpuDetails /></UsageRow>
                })}
                {gpus.into_iter().filter(|(p, _)| has(&format!("{p}.usage"))).map(|(prefix, name)| {
                    let p = prefix.clone();
                    view! {
                        <UsageRow label=name id=format!("{prefix}.usage")><GpuDetails prefix=p.clone() /></UsageRow>
                    }
                }).collect_view()}
                {battery.map(|prefix| {
                    let p = prefix.clone();
                    view! {
                        <UsageRow label="Battery".into() id=format!("{prefix}.charge") color="green">
                            <BatteryDetails prefix=p.clone() />
                        </UsageRow>
                    }
                })}
                {has("mem.used").then(|| view! { <Memory /> })}
                <Sensors />
                {has("sys.uptime").then(|| view! {
                    <div class="uptime muted">
                        "Up for " {move || data().value("sys.uptime").map(format_duration).unwrap_or_default()}
                    </div>
                })}
            </div>
        }
        .into_any()
    }
}
