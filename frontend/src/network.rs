// SPDX-License-Identifier: GPL-3.0-or-later
//! Network: live speed with graphs, session totals, interfaces with
//! addresses, apps with open connections, and an explicit speed test.

use andromeda_core::info::{Interface, LinkKind, NetApp, SpeedResult};
use andromeda_core::metrics::{Unit, format};
use leptos::prelude::*;

use crate::tauri;
use crate::widgets::{KeyValue, POLL_SLOW, Section, Spark, data, fmt, poll};

#[derive(Clone, PartialEq)]
enum Test {
    Idle,
    Running,
    Done(SpeedResult),
    Failed(String),
}

fn kind(k: LinkKind) -> &'static str {
    match k {
        LinkKind::Wifi => "Wi-Fi",
        LinkKind::Ethernet => "Ethernet",
        LinkKind::Vpn => "VPN",
        LinkKind::Other => "Virtual",
    }
}

#[component]
pub fn NetworkTab() -> impl IntoView {
    let ifaces = RwSignal::new(Vec::<Interface>::new());
    let apps = RwSignal::new(Vec::<NetApp>::new());
    poll(POLL_SLOW, move || {
        tauri::spawn(async move {
            if let Some(v) = tauri::get("interfaces").await {
                ifaces.set(v);
            }
            if let Some(v) = tauri::get("network_apps").await {
                apps.set(v);
            }
        });
    });
    let test = RwSignal::new(Test::Idle);
    let run_test = move |_| {
        test.set(Test::Running);
        tauri::spawn(async move {
            test.set(match tauri::invoke::<(), SpeedResult>("speed_test", &()).await {
                Ok(r) => Test::Done(r),
                Err(e) => Test::Failed(e),
            });
        });
    };
    let mbit = |bps: f64| format!("{:.0} Mbit/s", bps * 8.0 / 1e6);
    let has = data().info("net.rx").is_some();

    view! {
        {has.then(|| view! {
            <div class="card">
                <div class="tiles">
                    <div class="tile"><span class="muted">"Download"</span><strong>{move || fmt("net.rx", Unit::BytesPerSecond)}</strong></div>
                    <div class="tile"><span class="muted">"Upload"</span><strong>{move || fmt("net.tx", Unit::BytesPerSecond)}</strong></div>
                </div>
                <Spark id="net.rx".into() />
                <Spark id="net.tx".into() color="teal" />
                <h3 class="sub">"This session"</h3>
                <KeyValue name="Downloaded" value=Signal::derive(|| fmt("net.rx_session", Unit::Bytes)) />
                <KeyValue name="Uploaded" value=Signal::derive(|| fmt("net.tx_session", Unit::Bytes)) />
            </div>
        })}
        <Section title="Interfaces">
            <For each=move || ifaces.get() key=|i| i.name.clone() let:i>
                <div class="iface">
                    <div class="row">
                        <span class="dot" class:ok=i.up></span>
                        <strong class="grow">{i.name.clone()}</strong>
                        <span class="muted small">{kind(i.kind)}{i.speed_mbps.map(|s| format!(" · {s} Mb/s"))}</span>
                    </div>
                    {i.addresses.iter().map(|a| view! { <div class="addr small">{a.clone()}</div> }).collect_view()}
                    {(i.up && i.addresses.is_empty()).then(|| view! { <div class="muted small">"No address"</div> })}
                </div>
            </For>
        </Section>
        <Section title="Apps using network">
            <Show when=move || !apps.with(Vec::is_empty) fallback=|| view! { <p class="muted small">"No apps using network now"</p> }>
                {move || apps.get().into_iter().take(8).map(|a| view! {
                    <KeyValue name=a.name value=Signal::stored(format!("{} conn.", a.connections)) />
                }).collect_view()}
            </Show>
        </Section>
        <Section title="Speed test">
            <p class="muted small">"Measures against Cloudflare. Runs only when you press the button."</p>
            {move || match test.get() {
                Test::Idle => ().into_any(),
                Test::Running => view! { <p class="small">"Measuring…"</p> }.into_any(),
                Test::Done(r) => view! {
                    <KeyValue name="Download" value=Signal::stored(format!("{} ({})", mbit(r.download), format(r.download, Unit::BytesPerSecond))) />
                    <KeyValue name="Upload" value=Signal::stored(format!("{} ({})", mbit(r.upload), format(r.upload, Unit::BytesPerSecond))) />
                }.into_any(),
                Test::Failed(e) => view! { <p class="small error">{e}</p> }.into_any(),
            }}
            <button class="btn small-btn" disabled=move || test.get() == Test::Running on:click=run_test>"Run speed test"</button>
        </Section>
    }
}
