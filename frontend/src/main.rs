// SPDX-License-Identifier: GPL-3.0-or-later
//! Panel UI. Leptos uses "signals": reading a signal inside `view!` (or a
//! closure passed to it) subscribes that exact DOM node, so a new value
//! updates only the text and bars that show it.
//!
//! The frontend never touches the system. It asks the Tauri backend for the
//! metric list and history once, then receives a `values` event per second
//! while the panel is visible.

mod system;
mod tauri;

use std::collections::{HashMap, VecDeque};

use andromeda_core::metrics::MetricInfo;
use leptos::prelude::*;

/// Graph length; matches the daemon's history (one minute at 1 s).
pub const HISTORY: usize = 60;

/// Everything the tabs read. Shared through Leptos context, so any
/// component can call `use_context::<Data>()`.
#[derive(Clone, Copy)]
pub struct Data {
    pub metrics: RwSignal<Vec<MetricInfo>>,
    pub values: RwSignal<HashMap<String, f64>>,
    pub history: RwSignal<HashMap<String, VecDeque<f64>>>,
}

impl Data {
    pub fn value(&self, id: &str) -> Option<f64> {
        self.values.with(|m| m.get(id).copied())
    }
    pub fn info(&self, id: &str) -> Option<MetricInfo> {
        self.metrics.with(|m| m.iter().find(|x| x.id == id).cloned())
    }
}

/// Loads the metric list and history, then follows live values. The
/// daemon may still be starting, so the list is fetched again until it
/// arrives.
fn start_data() -> Data {
    let data = Data {
        metrics: RwSignal::new(Vec::new()),
        values: RwSignal::new(HashMap::new()),
        history: RwSignal::new(HashMap::new()),
    };
    let load = move || {
        tauri::spawn(async move {
            if let Some(list) = tauri::get::<Vec<MetricInfo>>("metrics").await {
                data.metrics.set(list);
            }
            if let Some(h) = tauri::get::<HashMap<String, Vec<f64>>>("history").await {
                data.history.set(h.into_iter().map(|(k, v)| (k, v.into())).collect());
            }
        });
    };
    load();
    tauri::listen::<HashMap<String, f64>>("values", move |values| {
        if data.metrics.with(Vec::is_empty) {
            load();
        }
        data.history.update(|h| {
            for (id, v) in &values {
                let q = h.entry(id.clone()).or_default();
                if q.len() >= HISTORY {
                    q.pop_front();
                }
                q.push_back(*v);
            }
        });
        data.values.set(values);
    });
    data
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Awake,
    Mixer,
    System,
    Network,
    Disk,
    Energy,
    Tools,
    Controls,
}

impl Tab {
    const ALL: [Tab; 8] = [
        Tab::Awake, Tab::Mixer, Tab::System, Tab::Network,
        Tab::Disk, Tab::Energy, Tab::Tools, Tab::Controls,
    ];
    fn title(self) -> &'static str {
        match self {
            Tab::Awake => "Keep awake",
            Tab::Mixer => "Volume mixer",
            Tab::System => "System",
            Tab::Network => "Network",
            Tab::Disk => "Disks",
            Tab::Energy => "Energy",
            Tab::Tools => "Tools",
            Tab::Controls => "Controls",
        }
    }
    fn icon(self) -> &'static str {
        match self {
            Tab::Awake => "moon",
            Tab::Mixer => "sliders-horizontal",
            Tab::System => "cpu",
            Tab::Network => "globe",
            Tab::Disk => "hard-drive",
            Tab::Energy => "zap",
            Tab::Tools => "wrench",
            Tab::Controls => "toggle-right",
        }
    }
}

/// A monochrome icon: the SVG is used as a CSS mask over `currentColor`,
/// so it follows the text color (active tab, hover, ...).
#[component]
pub fn Icon(name: &'static str) -> impl IntoView {
    view! { <span class="icon" style=format!("--icon:url(icons/{name}.svg)")></span> }
}

#[component]
fn Panel() -> impl IntoView {
    provide_context(start_data());
    let (tab, set_tab) = signal(Tab::System);

    view! {
        <main class="panel">
            <header class="logo"><img src="icons/andromeda-symbolic.svg" alt="" /></header>
            <nav class="tabs">
                {Tab::ALL.into_iter().map(|t| view! {
                    <button
                        class="tab"
                        class:active=move || tab.get() == t
                        title=t.title()
                        on:click=move |_| set_tab.set(t)
                    >
                        <Icon name=t.icon() />
                    </button>
                }).collect_view()}
            </nav>
            <h2 class="section">{move || tab.get().title()}</h2>
            <section class="content">
                {move || match tab.get() {
                    Tab::System => view! { <system::SystemTab /> }.into_any(),
                    _ => view! { <p class="muted">"Coming in a later phase."</p> }.into_any(),
                }}
            </section>
            <footer class="footer">
                <button class="btn" on:click=|_| tauri::call("open_settings")><Icon name="settings" />"Settings"</button>
                <button class="btn" on:click=|_| tauri::call("quit")><Icon name="power" />"Quit"</button>
            </footer>
        </main>
    }
}

#[component]
fn Settings() -> impl IntoView {
    view! {
        <main class="settings">
            <h1>"Settings"</h1>
            <p class="muted">"Placeholder. The real Settings window arrives in Phase 3."</p>
        </main>
    }
}

fn main() {
    let is_settings = window().location().hash().is_ok_and(|h| h == "#settings");
    if is_settings {
        let _ = document().body().map(|b| b.set_class_name("opaque"));
        mount_to_body(Settings);
    } else {
        mount_to_body(Panel);
    }
}
