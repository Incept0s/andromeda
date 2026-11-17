// SPDX-License-Identifier: GPL-3.0-or-later
//! Volume mixer: one row per app with a live dot, output picker, slider
//! (amber above 100 %), reset and mute; then system output and microphone.

use andromeda_core::info::{AudioDevice, MixApp};
use leptos::prelude::*;

use crate::Icon;
use crate::tauri;
use crate::widgets::data;

#[derive(serde::Serialize)]
struct Volume {
    key: String,
    percent: f64,
}
#[derive(serde::Serialize)]
struct Mute {
    key: String,
    mute: bool,
}
#[derive(serde::Serialize)]
struct Output {
    key: String,
    sink: String,
}
#[derive(serde::Serialize)]
struct Default {
    kind: &'static str,
    name: String,
}

/// <select> of devices. `value` is the device name; empty = none known.
#[component]
fn DevicePicker(
    devices: Vec<AudioDevice>,
    #[prop(into)] value: String,
    on_pick: impl Fn(String) + 'static,
    #[prop(optional)] title: &'static str,
) -> impl IntoView {
    view! {
        <select class="picker" title=title on:change=move |e| on_pick(event_target_value(&e))>
            {devices.into_iter().map(|dev| {
                let selected = dev.name == value;
                view! { <option value=dev.name selected=selected>{dev.description}</option> }
            }).collect_view()}
        </select>
    }
}

#[component]
fn AppRow(app: MixApp, outputs: Vec<AudioDevice>, max: f64) -> impl IntoView {
    let key = app.key.clone();
    // While dragging, show the local value so echoes from the server do
    // not make the thumb jump.
    let dragging = RwSignal::new(None::<f64>);
    let shown = move || dragging.get().unwrap_or(app.volume);
    let boosted = move || shown() > 100.5;
    let set = {
        let key = key.clone();
        move |v: f64| {
            tauri::call(
                "mixer_volume",
                Volume {
                    key: key.clone(),
                    percent: v,
                },
            )
        }
    };
    let set_input = set.clone();
    let set_reset = set.clone();
    let k_mute = key.clone();
    let k_out = key.clone();
    let muted = app.muted;
    let icon = app.icon.clone();
    view! {
        <div class="mix">
            <div class="mix-icon">
                {if icon.is_empty() {
                    view! { <Icon name="sliders-horizontal" /> }.into_any()
                } else {
                    view! { <img src=icon alt="" /> }.into_any()
                }}
                <span class="live" class:hidden=!app.playing title="Playing"></span>
            </div>
            <div class="mix-body">
                <div class="row">
                    <strong class="grow ellipsis">{app.name.clone()}</strong>
                    <DevicePicker devices=outputs value=app.output.clone() title="Choose output"
                        on_pick=move |sink| tauri::call("mixer_output", Output { key: k_out.clone(), sink }) />
                </div>
                <div class="row">
                    <input type="range" class="slider" class:boost=boosted min="0" max=max step="1"
                        prop:value=move || shown().round()
                        style=move || format!("--fill:{:.1}%", shown() / max * 100.0)
                        on:input=move |e| {
                            let v = event_target_value(&e).parse().unwrap_or(100.0);
                            dragging.set(Some(v));
                            set_input(v);
                        }
                        on:change=move |_| dragging.set(None) />
                    <span class="value pct" class:boost=boosted>
                        <span class:hidden=move || !boosted()><Icon name="zap" /></span>{move || format!("{:.0}%", shown())}
                    </span>
                    <button class="iconbtn" title="Reset to 100%" class:hidden=move || (shown() - 100.0).abs() < 0.5
                        on:click=move |_| set_reset(100.0)><Icon name="rotate-ccw" /></button>
                    <button class="iconbtn" class:muted-on=muted title=if muted { "Unmute" } else { "Mute" }
                        on:click=move |_| tauri::call("mixer_mute", Mute { key: k_mute.clone(), mute: !muted })>
                        {if muted { view! { <Icon name="volume-x" /> } } else { view! { <Icon name="volume-2" /> } }}
                    </button>
                </div>
            </div>
        </div>
    }
}

#[component]
pub fn MixerTab() -> impl IntoView {
    let d = data();
    let max = move || {
        d.config
            .with(|c| c.as_ref().map_or(153, |c| c.mixer.max_volume)) as f64
    };
    let hidden = move || {
        d.config.with(|c| {
            c.as_ref()
                .map(|c| c.mixer.hidden.clone())
                .unwrap_or_default()
        })
    };
    let apps = move || {
        let hidden = hidden();
        d.mixer.with(|m| {
            m.apps
                .iter()
                .filter(|a| !hidden.contains(&a.key))
                .cloned()
                .collect::<Vec<_>>()
        })
    };

    view! {
        <Show when=move || d.mixer.with(|m| m.connected)
            fallback=|| view! { <div class="card muted">"The audio service (PipeWire) is not reachable."</div> }>
            <div class="card">
                <Show when=move || !apps().is_empty()
                    fallback=|| view! { <p class="muted">"Apps that use audio show up here."</p> }>
                    {move || {
                        let outputs = d.mixer.with(|m| m.outputs.clone());
                        let max = max();
                        apps().into_iter().map(|a| view! { <AppRow app=a outputs=outputs.clone() max /> }).collect_view()
                    }}
                </Show>
            </div>
            <div class="card">
                <h3 class="sub">"Devices"</h3>
                <div class="row">
                    <span class="label">"Output"</span>
                    {move || {
                        let (list, cur) = d.mixer.with(|m| (m.outputs.clone(), m.default_output.clone()));
                        view! { <DevicePicker devices=list value=cur title="Choose system output"
                            on_pick=|name| tauri::call("mixer_default", Default { kind: "output", name }) /> }
                    }}
                </div>
                <div class="row">
                    <span class="label">"Microphone"</span>
                    {move || {
                        let (list, cur) = d.mixer.with(|m| (m.inputs.clone(), m.default_input.clone()));
                        view! { <DevicePicker devices=list value=cur title="Choose microphone"
                            on_pick=|name| tauri::call("mixer_default", Default { kind: "input", name }) /> }
                    }}
                </div>
            </div>
        </Show>
    }
}
