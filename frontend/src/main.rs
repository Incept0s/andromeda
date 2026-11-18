// SPDX-License-Identifier: GPL-3.0-or-later
//! Panel UI. Leptos uses "signals": reading a signal inside `view!` (or a
//! closure passed to it) subscribes that exact DOM node, so a new value
//! updates only the text and bars that show it.
//!
//! The frontend never touches the system. It asks the Tauri backend for
//! data and receives events (`values` every second while visible, plus
//! `mixer`, `config` and `awake` on change).

mod awake;
mod controls;
mod disk;
mod energy;
mod mixer;
mod network;
mod settings;
mod system;
mod tauri;
mod tools;
mod widgets;

use std::collections::{HashMap, VecDeque};

use andromeda_core::config::Config;
use andromeda_core::info::MixerState;
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
    pub config: RwSignal<Option<Config>>,
    /// -1 off, 0 until turned off, else end time (Unix seconds).
    pub awake: RwSignal<i64>,
    pub mixer: RwSignal<MixerState>,
}

impl Data {
    pub fn value(&self, id: &str) -> Option<f64> {
        self.values.with(|m| m.get(id).copied())
    }
    pub fn info(&self, id: &str) -> Option<MetricInfo> {
        self.metrics
            .with(|m| m.iter().find(|x| x.id == id).cloned())
    }
    /// Applies `f` to a copy of the settings and sends it to the daemon,
    /// which saves it and echoes it back as a `config` event.
    pub fn edit_config(&self, f: impl FnOnce(&mut Config)) {
        let Some(mut c) = self.config.get_untracked() else {
            return;
        };
        f(&mut c);
        self.config.set(Some(c.clone()));
        tauri::call("set_config", ConfigArg { config: c });
    }
}

#[derive(serde::Serialize)]
struct Sticky {
    sticky: bool,
}

#[derive(serde::Serialize)]
struct ConfigArg {
    config: Config,
}

/// Loads everything once, then follows events. The daemon may still be
/// starting, so the metric list is fetched again until it arrives.
fn start_data() -> Data {
    let data = Data {
        metrics: RwSignal::new(Vec::new()),
        values: RwSignal::new(HashMap::new()),
        history: RwSignal::new(HashMap::new()),
        config: RwSignal::new(None),
        awake: RwSignal::new(-1),
        mixer: RwSignal::new(MixerState::default()),
    };
    let load = move || {
        tauri::spawn(async move {
            if let Some(list) = tauri::get::<Vec<MetricInfo>>("metrics").await {
                data.metrics.set(list);
            }
            if let Some(h) = tauri::get::<HashMap<String, Vec<f64>>>("history").await {
                data.history
                    .set(h.into_iter().map(|(k, v)| (k, v.into())).collect());
            }
            if let Some(c) = tauri::get::<Config>("config").await {
                data.config.set(Some(c));
            }
            if let Some(a) = tauri::get::<i64>("awake_until").await {
                data.awake.set(a);
            }
            if let Some(m) = tauri::get::<MixerState>("mixer").await {
                data.mixer.set(m);
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
    tauri::listen::<Config>("config", move |c| data.config.set(Some(c)));
    tauri::listen::<i64>("awake", move |a| data.awake.set(a));
    tauri::listen::<MixerState>("mixer", move |m| data.mixer.set(m));
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
        Tab::System,
        Tab::Mixer,
        Tab::Network,
        Tab::Disk,
        Tab::Energy,
        Tab::Awake,
        Tab::Tools,
        Tab::Controls,
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
    /// A tab is shown unless its feature is switched off in Settings.
    fn enabled(self, c: &Config) -> bool {
        let f = &c.features;
        match self {
            Tab::Awake => f.awake,
            Tab::Mixer => f.mixer,
            Tab::System => f.system,
            Tab::Network => f.network,
            Tab::Disk => f.disk,
            Tab::Energy => f.energy,
            Tab::Tools => f.tools,
            Tab::Controls => f.controls,
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
    let data = start_data();
    provide_context(data);
    let (tab, set_tab) = signal(Tab::System);
    let tabs = move || {
        data.config.with(|c| match c {
            Some(c) => Tab::ALL
                .into_iter()
                .filter(|t| t.enabled(c))
                .collect::<Vec<_>>(),
            None => Tab::ALL.to_vec(),
        })
    };
    // If the open tab gets switched off, fall back to the first one left.
    Effect::new(move |_| {
        let list = tabs();
        if !list.contains(&tab.get_untracked())
            && let Some(first) = list.first()
        {
            set_tab.set(*first);
        }
    });

    // While Tools is open the panel stays when it loses focus, so files can
    // be dragged in from another window. Escape always closes the panel.
    Effect::new(move |_| {
        tauri::call(
            "set_sticky",
            Sticky {
                sticky: tab.get() == Tab::Tools,
            },
        )
    });
    let _ = window_event_listener(leptos::ev::keydown, |e| {
        if e.key() == "Escape" {
            tauri::call("hide", ());
        }
    });

    view! {
        <main class="glass panel">
            <header class="logo"><img src="icons/andromeda-symbolic.svg" alt="" /></header>
            <nav class="tabs" style=move || format!("grid-template-columns:repeat({},1fr)", tabs().len().max(1))>
                <For each=tabs key=|t| *t as u8 let:t>
                    <button
                        class="tab"
                        class:active=move || tab.get() == t
                        title=t.title()
                        on:click=move |_| set_tab.set(t)
                    >
                        <Icon name=t.icon() />
                    </button>
                </For>
            </nav>
            <h2 class="section">{move || tab.get().title()}</h2>
            <section class="content">
                {move || match tab.get() {
                    Tab::Awake => view! { <awake::AwakeTab /> }.into_any(),
                    Tab::Mixer => view! { <mixer::MixerTab /> }.into_any(),
                    Tab::System => view! { <system::SystemTab /> }.into_any(),
                    Tab::Network => view! { <network::NetworkTab /> }.into_any(),
                    Tab::Disk => view! { <disk::DiskTab /> }.into_any(),
                    Tab::Energy => view! { <energy::EnergyTab /> }.into_any(),
                    Tab::Tools => view! { <tools::ToolsTab /> }.into_any(),
                    Tab::Controls => view! { <controls::ControlsTab /> }.into_any(),
                }}
            </section>
            <footer class="footer">
                <button class="btn" on:click=|_| tauri::call("open_settings", ())><Icon name="settings" />"Settings"</button>
                <button class="btn" on:click=|_| tauri::call("quit", ())><Icon name="power" />"Quit"</button>
            </footer>
        </main>
    }
}

fn main() {
    let is_settings = window().location().hash().is_ok_and(|h| h == "#settings");
    if is_settings {
        mount_to_body(|| {
            provide_context(start_data());
            view! { <settings::Settings /> }
        });
    } else {
        mount_to_body(Panel);
    }
}
