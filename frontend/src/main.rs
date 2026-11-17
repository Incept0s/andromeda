// SPDX-License-Identifier: GPL-3.0-or-later
//! Panel UI. Leptos uses "signals": reading a signal inside `view!` (or a
//! closure passed to it) subscribes that exact DOM node, so a new CPU value
//! updates two text nodes and one SVG attribute, nothing else.

use std::collections::VecDeque;

use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::prelude::*;

// Bindings to the JS API Tauri injects (`withGlobalTauri: true`). This is
// Rust declaring existing JS functions, not hand-written JS.
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], catch)]
    async fn invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "event"], catch)]
    async fn listen(event: &str, handler: &Closure<dyn FnMut(JsValue)>) -> Result<JsValue, JsValue>;
}

fn call(cmd: &'static str) {
    spawn_local(async move {
        let _ = invoke(cmd, JsValue::UNDEFINED).await;
    });
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
fn Icon(name: &'static str) -> impl IntoView {
    view! { <span class="icon" style=format!("--icon:url(icons/{name}.svg)")></span> }
}

const HISTORY: usize = 60;

/// SVG polyline points for a 0..100 series, newest on the right.
fn spark_points(h: &VecDeque<f64>) -> String {
    let n = HISTORY - 1;
    let offset = HISTORY - h.len();
    h.iter()
        .enumerate()
        .map(|(i, v)| {
            let x = (offset + i) as f64 * 300.0 / n as f64;
            let y = 40.0 - v.clamp(0.0, 100.0) * 0.38 - 1.0;
            format!("{x:.1},{y:.1}")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[component]
fn SystemTab(cpu: ReadSignal<Option<f64>>, history: ReadSignal<VecDeque<f64>>) -> impl IntoView {
    let pct = move || cpu.get().map_or("–".into(), |v| format!("{v:.0}%"));
    let width = move || format!("width:{:.1}%", cpu.get().unwrap_or(0.0));
    view! {
        <div class="card">
            <div class="row">
                <span class="label">"CPU"</span>
                <div class="bar"><div class="fill" style=width></div></div>
                <span class="value">{pct}</span>
            </div>
            <svg class="spark" viewBox="0 0 300 40" preserveAspectRatio="none">
                <polyline points=move || spark_points(&history.get()) />
            </svg>
        </div>
    }
}

#[component]
fn Panel() -> impl IntoView {
    let (tab, set_tab) = signal(Tab::System);
    let (cpu, set_cpu) = signal(None::<f64>);
    let (history, set_history) = signal(VecDeque::with_capacity(HISTORY));

    // Subscribe to the backend's "cpu" event. The closure must outlive this
    // function, so it is leaked on purpose: the panel lives as long as the app.
    spawn_local(async move {
        let handler = Closure::new(move |event: JsValue| {
            let payload = js_sys::Reflect::get(&event, &"payload".into()).ok();
            if let Some(v) = payload.and_then(|p| p.as_f64()) {
                set_cpu.set(Some(v));
                set_history.update(|h| {
                    if h.len() == HISTORY {
                        h.pop_front();
                    }
                    h.push_back(v);
                });
            }
        });
        let _ = listen("cpu", &handler).await;
        handler.forget();
    });

    view! {
        <main class="panel">
            <header class="logo" data-tauri-drag-region><img src="icons/andromeda-symbolic.svg" alt="" data-tauri-drag-region /></header>
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
                    Tab::System => view! { <SystemTab cpu history /> }.into_any(),
                    _ => view! { <p class="muted">"Coming in a later phase."</p> }.into_any(),
                }}
            </section>
            <footer class="footer">
                <button class="btn" on:click=|_| call("open_settings")><Icon name="settings" />"Settings"</button>
                <button class="btn" on:click=|_| call("quit")><Icon name="power" />"Quit"</button>
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
