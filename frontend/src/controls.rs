// SPDX-License-Identifier: GPL-3.0-or-later
//! Controls tab: quick switches for the features that work in the
//! background, like the original's Controls tab (most of its mouse and
//! keyboard items have no place on Linux and were dropped).

use leptos::prelude::*;

use crate::settings::Toggle;

#[component]
pub fn ControlsTab() -> impl IntoView {
    view! {
        <div class="card">
            <h3 class="sub">"Windows"</h3>
            <Toggle label="App switcher" hint="Alt+Tab shows a large preview of every window, minimized ones too, grouped by app."
                get=|c| c.features.switcher set=|c, v| c.features.switcher = v />
        </div>
        <div class="card">
            <h3 class="sub">"In the background"</h3>
            <Toggle label="Clipboard history" hint="Keeps text you copy, on this computer only."
                get=|c| c.features.clipboard set=|c, v| c.features.clipboard = v />
            <Toggle label="Numbers in the top bar" hint="Choose which in Settings → Top bar."
                get=|c| c.features.topbar set=|c, v| c.features.topbar = v />
            <Toggle label="Alerts" hint="Notifications when a reading stays past its limit."
                get=|c| c.features.alerts set=|c, v| c.features.alerts = v />
        </div>
    }
}
