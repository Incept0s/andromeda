// SPDX-License-Identifier: GPL-3.0-or-later
//! Settings window: a searchable sidebar of pages. Every change is sent to
//! the daemon at once (no Save button); the daemon saves the file and
//! echoes the new settings back.

use andromeda_core::config::{self, AlertRule, Config, Direction, TOPBAR_SOFT_LIMIT, TopBarSide};
use andromeda_core::info::Check;
use andromeda_core::metrics::{Unit, format};
use leptos::prelude::*;

use crate::tauri;
use crate::widgets::data;

#[derive(Clone, Copy, PartialEq)]
enum Page {
    General,
    Features,
    TopBar,
    Alerts,
    Mixer,
    Awake,
    Check,
    About,
}

impl Page {
    const ALL: [Page; 8] = [
        Page::General,
        Page::Features,
        Page::TopBar,
        Page::Alerts,
        Page::Mixer,
        Page::Awake,
        Page::Check,
        Page::About,
    ];
    fn title(self) -> &'static str {
        match self {
            Page::General => "General",
            Page::Features => "Features",
            Page::TopBar => "Top bar",
            Page::Alerts => "Alerts",
            Page::Mixer => "Volume mixer",
            Page::Awake => "Keep awake",
            Page::Check => "System check",
            Page::About => "About",
        }
    }
    /// Extra words the search box matches.
    fn keywords(self) -> &'static str {
        match self {
            Page::General => "login startup update version",
            Page::Features => "tabs enable disable on off switch",
            Page::TopBar => "panel numbers metrics order temperature position left right vitals",
            Page::Alerts => "notifications threshold warning",
            Page::Mixer => "volume boost audio sound hidden apps",
            Page::Awake => "sleep suspend battery timer caffeine",
            Page::Check => "permissions detected missing tools install",
            Page::About => "license credits vorssaint github",
        }
    }
}

/// On/off switch bound to a config field.
#[component]
pub fn Toggle(
    #[prop(into)] label: String,
    #[prop(into, optional)] hint: String,
    get: fn(&Config) -> bool,
    set: fn(&mut Config, bool),
) -> impl IntoView {
    let d = data();
    let on = move || d.config.with(|c| c.as_ref().is_some_and(get));
    view! {
        <div class="setting">
            <div class="grow">
                <div>{label}</div>
                {(!hint.is_empty()).then(|| view! { <div class="muted small">{hint}</div> })}
            </div>
            <button class="switch" class:on=on on:click=move |_| { let v = !on(); d.edit_config(|c| set(c, v)); }><i></i></button>
        </div>
    }
}

#[derive(serde::Serialize)]
struct On {
    on: bool,
}

#[component]
fn General() -> impl IntoView {
    let autostart = RwSignal::new(None::<bool>);
    let error = RwSignal::new(String::new());
    tauri::spawn(async move { autostart.set(tauri::get("autostart").await) });
    let toggle = move |_| {
        let on = !autostart.get().unwrap_or(false);
        tauri::spawn(async move {
            match tauri::invoke::<_, ()>("set_autostart", &On { on }).await {
                Ok(()) => {
                    autostart.set(Some(on));
                    error.set(String::new());
                }
                Err(e) => error.set(e),
            }
        });
    };
    view! {
        <h2>"General"</h2>
        <div class="setting">
            <div class="grow">
                <div>"Start at login"</div>
                <div class="muted small">"The background service keeps clipboard history, alerts, keep awake and the top-bar numbers running. The panel itself only runs while you use it."</div>
                <div class="small error">{move || error.get()}</div>
            </div>
            <button class="switch" class:on=move || autostart.get() == Some(true) on:click=toggle><i></i></button>
        </div>
        <div class="setting">
            <div class="grow">
                <div>"Updates"</div>
                <div class="muted small">{format!("You have version {}. New versions are published on GitHub.", env!("CARGO_PKG_VERSION"))}</div>
            </div>
            <button class="btn small-btn" on:click=|_| tauri::call("open_releases", ())>"Open releases"</button>
        </div>
    }
}

#[component]
fn Features() -> impl IntoView {
    view! {
        <h2>"Features"</h2>
        <p class="muted">"Turning a feature off hides it and stops its background work."</p>
        <Toggle label="Keep awake" get=|c| c.features.awake set=|c, v| c.features.awake = v />
        <Toggle label="Volume mixer" hint="Takes effect after the next login." get=|c| c.features.mixer set=|c, v| c.features.mixer = v />
        <Toggle label="System" get=|c| c.features.system set=|c, v| c.features.system = v />
        <Toggle label="Network" get=|c| c.features.network set=|c, v| c.features.network = v />
        <Toggle label="Disks" get=|c| c.features.disk set=|c, v| c.features.disk = v />
        <Toggle label="Energy" get=|c| c.features.energy set=|c, v| c.features.energy = v />
        <Toggle label="Tools" get=|c| c.features.tools set=|c, v| c.features.tools = v />
        <Toggle label="Controls" get=|c| c.features.controls set=|c, v| c.features.controls = v />
        <Toggle label="Alerts" get=|c| c.features.alerts set=|c, v| c.features.alerts = v />
        <Toggle label="Numbers in the top bar" get=|c| c.features.topbar set=|c, v| c.features.topbar = v />
        <Toggle label="Clipboard history" get=|c| c.features.clipboard set=|c, v| c.features.clipboard = v />
        <Toggle label="App switcher" hint="Replaces Alt+Tab with window previews." get=|c| c.features.switcher set=|c, v| c.features.switcher = v />
    }
}

#[component]
fn TopBar() -> impl IntoView {
    let d = data();
    let chosen = move || {
        d.config.with(|c| {
            c.as_ref()
                .map(|c| config::topbar_or_default(c, &d.metrics.get()))
                .unwrap_or_default()
        })
    };
    let set = move |v: Vec<String>| d.edit_config(|c| c.topbar = Some(v));
    let name = move |id: &str| d.info(id).map_or(id.to_owned(), |m| m.name);
    let preview = move || {
        chosen()
            .iter()
            .filter_map(|id| Some(format(d.value(id)?, d.info(id)?.unit)))
            .collect::<Vec<_>>()
            .join("  ")
    };
    let dragged = RwSignal::new(None::<usize>);
    let reorder = move |from: usize, to: usize| {
        let mut v = chosen();
        if from < v.len() && to < v.len() {
            let item = v.remove(from);
            v.insert(to, item);
            set(v);
        }
    };
    let groups = move || {
        let mut g: Vec<(String, Vec<(String, String)>)> = Vec::new();
        for m in d.metrics.get() {
            // Per-core values would flood the list.
            if m.id.starts_with("cpu.core") {
                continue;
            }
            let group = format!("{:?}", m.group);
            match g.iter_mut().find(|(n, _)| *n == group) {
                Some((_, list)) => list.push((m.id, m.name)),
                None => g.push((group, vec![(m.id, m.name)])),
            }
        }
        g
    };

    let side = move || {
        d.config
            .with(|c| c.as_ref().map_or(TopBarSide::Right, |c| c.topbar_side))
    };
    let set_side = move |s: TopBarSide| d.edit_config(|c| c.topbar_side = s);

    view! {
        <h2>"Top bar"</h2>
        <p class="muted">"Plain numbers in the top bar. A click on them opens the panel. Drag to reorder."</p>
        <div class="preview">{preview}</div>
        <div class="setting">
            <div class="grow">
                <div>"Position"</div>
                <div class="muted small">"Right of the clock, before your open apps, or at the far left of the bar."</div>
            </div>
            <div class="chips">
                <button class="chip" class:sel=move || side() == TopBarSide::Left on:click=move |_| set_side(TopBarSide::Left)>"Far left"</button>
                <button class="chip" class:sel=move || side() == TopBarSide::Right on:click=move |_| set_side(TopBarSide::Right)>"Right of the clock"</button>
            </div>
        </div>
        <Show when=move || { chosen().len() > TOPBAR_SOFT_LIMIT }>
            <p class="warn-text small">{format!("More than {TOPBAR_SOFT_LIMIT} items may not fit in the top bar.")}</p>
        </Show>
        <div class="order">
            {move || chosen().into_iter().enumerate().map(|(i, id)| {
                let label = name(&id);
                let id2 = id.clone();
                view! {
                    <div class="order-item" draggable="true"
                        on:dragstart=move |_| dragged.set(Some(i))
                        on:dragover=|e| e.prevent_default()
                        on:drop=move |e| { e.prevent_default(); if let Some(from) = dragged.get() { reorder(from, i); } dragged.set(None); }>
                        <span class="grip">"⋮⋮"</span>
                        <span class="grow">{label}</span>
                        <button class="iconbtn" title="Move up" on:click=move |_| { if i > 0 { reorder(i, i - 1) } }>"↑"</button>
                        <button class="iconbtn" title="Move down" on:click=move |_| reorder(i, i + 1)>"↓"</button>
                        <button class="iconbtn" title="Remove" on:click=move |_| set(chosen().into_iter().filter(|x| *x != id2).collect())>"✕"</button>
                    </div>
                }
            }).collect_view()}
        </div>
        {move || groups().into_iter().map(|(group, list)| view! {
            <h3 class="sub">{group}</h3>
            {list.into_iter().map(|(id, label)| {
                let id2 = id.clone();
                let checked = move || chosen().contains(&id);
                view! {
                    <label class="check">
                        <input type="checkbox" prop:checked=checked on:change=move |_| {
                            let mut v = chosen();
                            if let Some(p) = v.iter().position(|x| *x == id2) { v.remove(p); } else { v.push(id2.clone()); }
                            set(v);
                        } />
                        {label}
                    </label>
                }
            }).collect_view()}
        }).collect_view()}
    }
}

/// Human description of a rule's metric pattern.
fn describe(pattern: &str) -> String {
    let d = data();
    let matches: Vec<String> = d.metrics.with(|ms| {
        ms.iter()
            .filter(|m| config::pattern_matches(pattern, &m.id))
            .map(|m| m.name.clone())
            .collect()
    });
    match matches.len() {
        0 => format!("{pattern} (not on this computer)"),
        1 => matches[0].clone(),
        n => format!("{} (+{} more)", matches[0], n - 1),
    }
}

#[component]
fn Alerts() -> impl IntoView {
    let d = data();
    let rules = move || {
        d.config.with(|c| {
            c.as_ref()
                .map(|c| c.alerts.rules.clone())
                .unwrap_or_default()
        })
    };
    let edit = move |i: usize, f: Box<dyn FnOnce(&mut AlertRule)>| {
        d.edit_config(move |c| {
            if let Some(r) = c.alerts.rules.get_mut(i) {
                f(r)
            }
        })
    };
    let new_metric = RwSignal::new(String::new());
    let unit_of = move |pattern: &str| {
        d.metrics.with(|ms| {
            ms.iter()
                .find(|m| config::pattern_matches(pattern, &m.id))
                .map(|m| m.unit)
        })
    };
    view! {
        <h2>"Alerts"</h2>
        <p class="muted">"A notification when a reading stays past its limit for 12 seconds."</p>
        {move || rules().into_iter().enumerate().map(|(i, r)| {
            let unit = unit_of(&r.metric).unwrap_or(Unit::Percent);
            view! {
                <div class="setting">
                    <button class="switch" class:on=r.enabled on:click=move |_| edit(i, Box::new(|r| r.enabled = !r.enabled))><i></i></button>
                    <span class="grow">{describe(&r.metric)}</span>
                    <select class="picker" on:change=move |e| {
                        let dir = if event_target_value(&e) == "below" { Direction::Below } else { Direction::Above };
                        edit(i, Box::new(move |r| r.direction = dir));
                    }>
                        <option value="above" selected=r.direction == Direction::Above>"above"</option>
                        <option value="below" selected=r.direction == Direction::Below>"below"</option>
                    </select>
                    <input class="num" type="number" prop:value=r.threshold
                        on:change=move |e| { if let Ok(v) = event_target_value(&e).parse::<f64>() { edit(i, Box::new(move |r| r.threshold = v)) } } />
                    <span class="muted small unit">{format(0.0, unit).trim_start_matches(|c: char| c.is_ascii_digit() || c == '.').to_owned()}</span>
                    <button class="iconbtn" title="Delete" on:click=move |_| d.edit_config(|c| { c.alerts.rules.remove(i); })>"✕"</button>
                </div>
            }
        }).collect_view()}
        <div class="setting">
            <select class="picker grow" on:change=move |e| new_metric.set(event_target_value(&e))>
                <option value="">"Add an alert for…"</option>
                {move || d.metrics.get().into_iter().filter(|m| !m.id.starts_with("cpu.core")).map(|m| view! { <option value=m.id>{m.name}</option> }).collect_view()}
            </select>
            <button class="btn small-btn" disabled=move || new_metric.get().is_empty() on:click=move |_| {
                let id = new_metric.get();
                let threshold = d.value(&id).map_or(80.0, |v| (v * 1.2).round());
                d.edit_config(|c| c.alerts.rules.push(AlertRule { metric: id, direction: Direction::Above, threshold, enabled: true }));
            }>"Add"</button>
        </div>
        <div class="setting">
            <span class="grow">"Wait between repeated alerts"</span>
            <input class="num" type="number" min="1" prop:value=move || d.config.with(|c| c.as_ref().map_or(15, |c| c.alerts.cooldown_minutes))
                on:change=move |e| { if let Ok(v) = event_target_value(&e).parse::<u32>() { d.edit_config(|c| c.alerts.cooldown_minutes = v) } } />
            <span class="muted small unit">"min"</span>
        </div>
    }
}

#[component]
fn Mixer() -> impl IntoView {
    let d = data();
    let max = move || {
        d.config
            .with(|c| c.as_ref().map_or(153, |c| c.mixer.max_volume))
    };
    let hidden = move || {
        d.config.with(|c| {
            c.as_ref()
                .map(|c| c.mixer.hidden.clone())
                .unwrap_or_default()
        })
    };
    view! {
        <h2>"Volume mixer"</h2>
        <div class="setting">
            <div class="grow">
                <div>"Maximum volume"</div>
                <div class="muted small">"Above 100 % sound is amplified in software and can distort when loud."</div>
            </div>
            <div class="chips">
                {[100u32, 153, 200].into_iter().map(|v| view! {
                    <button class="chip" class:sel=move || max() == v on:click=move |_| d.edit_config(|c| c.mixer.max_volume = v)>{format!("{v}%")}</button>
                }).collect_view()}
            </div>
        </div>
        <h3 class="sub">"Apps in the list"</h3>
        {move || {
            let hidden = hidden();
            let mut keys: Vec<(String, String)> = d.mixer.with(|m| m.apps.iter().map(|a| (a.key.clone(), a.name.clone())).collect());
            for h in &hidden {
                if !keys.iter().any(|(k, _)| k == h) { keys.push((h.clone(), h.clone())); }
            }
            if keys.is_empty() {
                return view! { <p class="muted small">"Apps that play audio appear here."</p> }.into_any();
            }
            keys.into_iter().map(|(key, name)| {
                let is_hidden = hidden.contains(&key);
                view! {
                    <div class="setting">
                        <span class="grow">{name}</span>
                        <button class="btn small-btn" on:click=move |_| {
                            let key = key.clone();
                            d.edit_config(move |c| if is_hidden { c.mixer.hidden.retain(|k| *k != key) } else { c.mixer.hidden.push(key) })
                        }>{if is_hidden { "Show" } else { "Hide" }}</button>
                    </div>
                }
            }).collect_view().into_any()
        }}
    }
}

#[component]
fn Awake() -> impl IntoView {
    let d = data();
    let get =
        move |f: fn(&Config) -> String| d.config.with(|c| c.as_ref().map(f).unwrap_or_default());
    view! {
        <h2>"Keep awake"</h2>
        <div class="setting">
            <span class="grow">"Default duration"</span>
            <select class="picker" on:change=move |e| { if let Ok(v) = event_target_value(&e).parse::<u32>() { d.edit_config(|c| c.awake.default_minutes = v) } }>
                {[(15, "15 min"), (30, "30 min"), (60, "1 hour"), (120, "2 hours"), (240, "4 hours"), (0, "Until turned off")].into_iter().map(|(m, l)| view! {
                    <option value=m selected=move || get(|c| c.awake.default_minutes.to_string()) == m.to_string()>{l}</option>
                }).collect_view()}
            </select>
        </div>
        <div class="setting">
            <div class="grow">
                <div>"Stop when the battery drops below"</div>
                <div class="muted small">"Keeps a forgotten session from draining the laptop. Only while unplugged."</div>
            </div>
            <select class="picker" on:change=move |e| {
                let v = event_target_value(&e).parse::<u32>().ok().filter(|v| *v > 0);
                d.edit_config(|c| c.awake.stop_below_battery = v);
            }>
                {[(0, "Never"), (10, "10 %"), (20, "20 %"), (30, "30 %"), (50, "50 %")].into_iter().map(|(v, l)| view! {
                    <option value=v selected=move || get(|c| c.awake.stop_below_battery.unwrap_or(0).to_string()) == v.to_string()>{l}</option>
                }).collect_view()}
            </select>
        </div>
        <Toggle label="Start keeping awake at login" hint="Uses the default duration." get=|c| c.awake.start_with_app set=|c, v| c.awake.start_with_app = v />
    }
}

#[component]
fn SystemCheck() -> impl IntoView {
    let checks = RwSignal::new(Vec::<Check>::new());
    tauri::spawn(async move {
        if let Some(v) = tauri::get("system_check").await {
            checks.set(v);
        }
    });
    view! {
        <h2>"System check"</h2>
        <p class="muted">"What Andromeda found on this computer. Missing items only hide the features that need them."</p>
        <For each=move || checks.get() key=|c| c.name.clone() let:c>
            <div class="setting">
                <span class=if c.ok { "status ok" } else { "status missing" }>{if c.ok { "✓" } else { "–" }}</span>
                <div class="grow">
                    <div>{c.name.clone()}</div>
                    <div class="muted small">{c.detail.clone()}</div>
                </div>
            </div>
        </For>
    }
}

#[component]
fn About() -> impl IntoView {
    view! {
        <h2>"Andromeda"</h2>
        <p>{format!("Version {}", env!("CARGO_PKG_VERSION"))}</p>
        <p class="muted small">"Inspired by Vorssaint Utils; not affiliated with or endorsed by Vorssaint."</p>
        <p class="muted small">"Free software under the GNU General Public License, version 3 or later."</p>
        <p class="muted small">"Interface icons: Lucide (ISC license)."</p>
        <button class="btn small-btn" on:click=|_| tauri::call("open_releases", ())>"Releases on GitHub"</button>
    }
}

#[component]
pub fn Settings() -> impl IntoView {
    let page = RwSignal::new(Page::Features);
    let query = RwSignal::new(String::new());
    let visible = move || {
        let q = query.get().to_lowercase();
        Page::ALL
            .into_iter()
            .filter(|p| {
                q.is_empty() || p.title().to_lowercase().contains(&q) || p.keywords().contains(&q)
            })
            .collect::<Vec<_>>()
    };
    view! {
        <main class="glass">
            // Our own title bar: the window has no GTK frame, so it looks like
            // the panel. Dragging it moves the window.
            <header class="titlebar" data-tauri-drag-region>
                <img src="icons/andromeda-symbolic.svg" alt="" data-tauri-drag-region />
                <span class="grow" data-tauri-drag-region>"Andromeda Settings"</span>
                <button class="iconbtn" title="Close" on:click=|_| tauri::call("close_settings", ())><crate::Icon name="x" /></button>
            </header>
        <div class="settings-shell">
            <aside class="sidebar">
                <input class="search" type="search" placeholder="Search" on:input=move |e| query.set(event_target_value(&e)) />
                <For each=visible key=|p| *p as u8 let:p>
                    <button class="nav" class:active=move || page.get() == p on:click=move |_| page.set(p)>{p.title()}</button>
                </For>
            </aside>
            <main class="page">
                <Show when=move || data().config.with(Option::is_some) fallback=|| view! { <p class="muted">"Connecting to the Andromeda service…"</p> }>
                    {move || match page.get() {
                        Page::General => view! { <General /> }.into_any(),
                        Page::Features => view! { <Features /> }.into_any(),
                        Page::TopBar => view! { <TopBar /> }.into_any(),
                        Page::Alerts => view! { <Alerts /> }.into_any(),
                        Page::Mixer => view! { <Mixer /> }.into_any(),
                        Page::Awake => view! { <Awake /> }.into_any(),
                        Page::Check => view! { <SystemCheck /> }.into_any(),
                        Page::About => view! { <About /> }.into_any(),
                    }}
                </Show>
            </main>
        </div>
        </main>
    }
}
