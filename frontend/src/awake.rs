// SPDX-License-Identifier: GPL-3.0-or-later
//! Keep awake: stops suspend and screen blanking for a while, via GNOME's
//! session inhibitor in the daemon.

use std::time::Duration;

use leptos::prelude::*;

use crate::tauri;
use crate::widgets::{clock, countdown, data, now_secs};

#[derive(serde::Serialize)]
struct Minutes {
    minutes: u32,
}

const CHOICES: [(u32, &str); 6] = [(15, "15 min"), (30, "30 min"), (60, "1 h"), (120, "2 h"), (240, "4 h"), (0, "Until off")];

#[component]
pub fn AwakeTab() -> impl IntoView {
    let d = data();
    // Re-render the countdown every second.
    let now = RwSignal::new(now_secs());
    if let Ok(h) = set_interval_with_handle(move || now.set(now_secs()), Duration::from_secs(1)) {
        on_cleanup(move || h.clear());
    }
    let default = move || d.config.with(|c| c.as_ref().map_or(60, |c| c.awake.default_minutes));
    let chosen = RwSignal::new(None::<u32>);
    let minutes = move || chosen.get().unwrap_or_else(default);
    let active = move || d.awake.get() >= 0;

    let status = move || match d.awake.get() {
        -1 => "The computer follows its normal energy rules".to_owned(),
        0 => "Active until you turn it off".to_owned(),
        end => format!("Active until {}", clock(end)),
    };
    let remaining = move || {
        let end = d.awake.get();
        (end > 0).then(|| format!("Ends in {}", countdown(end - now.get())))
    };
    let toggle = move |_| {
        if active() {
            tauri::call("awake_stop", ());
        } else {
            tauri::call("awake_start", Minutes { minutes: minutes() });
        }
    };
    let battery_note = move || {
        d.config.with(|c| c.as_ref().and_then(|c| c.awake.stop_below_battery))
            .map(|b| format!("Stops by itself below {b} % battery when unplugged."))
    };

    view! {
        <div class="card awake" class:on=active>
            <div class="row">
                <div class="grow">
                    <strong>{move || if active() { "Keeping awake" } else { "Off" }}</strong>
                    <div class="muted small">{status}</div>
                    <div class="small accent-text">{remaining}</div>
                </div>
                <button class="switch" class:on=active on:click=toggle aria-label="Keep awake"><i></i></button>
            </div>
            <div class="chips">
                {CHOICES.into_iter().map(|(m, label)| view! {
                    <button class="chip" class:sel=move || minutes() == m
                        on:click=move |_| {
                            chosen.set(Some(m));
                            // Changing the duration of a running session restarts its timer.
                            if active() {
                                tauri::call("awake_start", Minutes { minutes: m });
                            }
                        }>{label}</button>
                }).collect_view()}
            </div>
            <p class="muted small">{battery_note}</p>
        </div>
    }
}
