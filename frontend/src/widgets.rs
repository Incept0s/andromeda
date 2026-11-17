// SPDX-License-Identifier: GPL-3.0-or-later
//! Small building blocks shared by the tabs: graphs, bars, rows, polling.

use std::time::Duration;

use andromeda_core::metrics::{Unit, format};
use leptos::prelude::*;

use crate::{Data, HISTORY};

pub fn data() -> Data {
    expect_context::<Data>()
}

/// Formatted current value of a metric, or a dash.
pub fn fmt(id: &str, unit: Unit) -> String {
    data().value(id).map_or("–".into(), |v| format(v, unit))
}

/// Runs `f` now and then every `every` while the calling component is
/// mounted (i.e. while its tab is open). Used for on-demand lists.
pub fn poll(every: Duration, f: impl Fn() + 'static) {
    f();
    if let Ok(handle) = set_interval_with_handle(f, every) {
        on_cleanup(move || handle.clear());
    }
}

/// SVG sparkline of a metric's history. `max` fixes the scale (100 for
/// percentages); otherwise the graph scales to its own peak.
#[component]
pub fn Spark(
    id: String,
    #[prop(optional)] max: Option<f64>,
    #[prop(default = "accent")] color: &'static str,
) -> impl IntoView {
    let points = move || {
        data().history.with(|h| {
            let Some(q) = h.get(&id) else {
                return String::new();
            };
            let top = max.unwrap_or_else(|| q.iter().copied().fold(1e-9, f64::max));
            let offset = HISTORY.saturating_sub(q.len());
            q.iter()
                .enumerate()
                .map(|(i, v)| {
                    let x = (offset + i) as f64 * 300.0 / (HISTORY - 1) as f64;
                    let y = 30.0 - (v / top).clamp(0.0, 1.0) * 28.0 - 1.0;
                    format!("{x:.1},{y:.1}")
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
    };
    view! {
        <svg class=format!("spark {color}") viewBox="0 0 300 30" preserveAspectRatio="none">
            <polyline points=points />
        </svg>
    }
}

/// Horizontal bar for a 0..100 value given as a closure.
#[component]
pub fn Meter(
    #[prop(into)] pct: Signal<f64>,
    #[prop(default = "accent")] color: &'static str,
) -> impl IntoView {
    let width = move || format!("width:{:.1}%", pct.get().clamp(0.0, 100.0));
    view! { <div class="bar"><div class=format!("fill {color}") style=width></div></div> }
}

/// Bar for a percentage metric.
#[component]
pub fn Bar(id: String, #[prop(default = "accent")] color: &'static str) -> impl IntoView {
    view! { <Meter pct=Signal::derive(move || data().value(&id).unwrap_or(0.0)) color /> }
}

/// "name ........ value" line used inside details and lists.
#[component]
pub fn KeyValue(#[prop(into)] name: String, #[prop(into)] value: Signal<String>) -> impl IntoView {
    view! { <div class="kv"><span class="muted">{name}</span><span>{value}</span></div> }
}

/// A `KeyValue` for a metric, only if this machine has it.
pub fn kv(name: &str, id: &str, unit: Unit) -> impl IntoView + use<> {
    let id = id.to_owned();
    let has = data().info(&id).is_some();
    let name = name.to_owned();
    has.then(|| view! { <KeyValue name value=Signal::derive(move || fmt(&id, unit)) /> })
}

/// Card with a sub-heading.
#[component]
pub fn Section(#[prop(into)] title: String, children: Children) -> impl IntoView {
    view! {
        <div class="card">
            <h3 class="sub">{title}</h3>
            {children()}
        </div>
    }
}

/// Seconds since 1970 now, from the browser clock.
pub fn now_secs() -> i64 {
    (js_sys::Date::now() / 1000.0) as i64
}

/// Local "HH:MM" for a Unix time.
pub fn clock(secs: i64) -> String {
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(secs as f64 * 1000.0));
    format!("{:02}:{:02}", d.get_hours(), d.get_minutes())
}

/// Short duration for countdowns, e.g. `1 h 05 min`, `42 min`, `30 s`.
pub fn countdown(secs: i64) -> String {
    let s = secs.max(0);
    match (s / 3600, s / 60 % 60) {
        (0, 0) => format!("{s} s"),
        (0, m) => format!("{m} min"),
        (h, m) => format!("{h} h {m:02} min"),
    }
}

pub const POLL_FAST: Duration = Duration::from_secs(2);
pub const POLL_SLOW: Duration = Duration::from_secs(5);
