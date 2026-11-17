// SPDX-License-Identifier: GPL-3.0-or-later
//! Bindings to the JS API Tauri injects into the page
//! (`withGlobalTauri: true`). This is Rust declaring existing JS
//! functions, not hand-written JS.

use std::future::Future;

use serde::de::DeserializeOwned;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke, catch)]
    async fn invoke_js(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "event"], js_name = listen, catch)]
    async fn listen_js(event: &str, handler: &Closure<dyn FnMut(JsValue)>) -> Result<JsValue, JsValue>;
}

pub fn spawn(f: impl Future<Output = ()> + 'static) {
    leptos::task::spawn_local(f);
}

/// Fire-and-forget command without arguments.
pub fn call(cmd: &'static str) {
    spawn(async move {
        let _ = invoke_js(cmd, JsValue::UNDEFINED).await;
    });
}

/// Command with a typed result; `None` on any error (the UI just waits for
/// the next attempt instead of showing a broken state).
pub async fn get<T: DeserializeOwned>(cmd: &str) -> Option<T> {
    let v = invoke_js(cmd, JsValue::UNDEFINED).await.ok()?;
    serde_wasm_bindgen::from_value(v).ok()
}

/// Subscribes to a backend event with a typed payload. The closure must
/// outlive this call, so it is leaked on purpose: listeners live as long as
/// the page.
pub fn listen<T: DeserializeOwned + 'static>(event: &'static str, mut f: impl FnMut(T) + 'static) {
    spawn(async move {
        let handler = Closure::new(move |e: JsValue| {
            let payload = js_sys::Reflect::get(&e, &"payload".into()).ok();
            if let Some(v) = payload.and_then(|p| serde_wasm_bindgen::from_value(p).ok()) {
                f(v);
            }
        });
        let _ = listen_js(event, &handler).await;
        handler.forget();
    });
}
