// SPDX-License-Identifier: GPL-3.0-or-later
//! Bindings to the JS API Tauri injects into the page
//! (`withGlobalTauri: true`). This is Rust declaring existing JS
//! functions, not hand-written JS.

use std::future::Future;

use serde::Serialize;
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

/// Arguments as a plain JS object (Tauri expects camelCase keys, which the
/// `#[serde(rename_all = "camelCase")]` on argument structs provides).
fn to_js<A: Serialize>(args: &A) -> JsValue {
    args.serialize(&serde_wasm_bindgen::Serializer::json_compatible()).unwrap_or(JsValue::UNDEFINED)
}

/// Calls a command and returns its typed result or the error message.
pub async fn invoke<A: Serialize, T: DeserializeOwned>(cmd: &str, args: &A) -> Result<T, String> {
    match invoke_js(cmd, to_js(args)).await {
        Ok(v) => serde_wasm_bindgen::from_value(v).map_err(|e| e.to_string()),
        Err(e) => Err(e.as_string().unwrap_or_else(|| "failed".into())),
    }
}

/// Command without arguments; `None` on any error (the UI just waits for
/// the next attempt instead of showing a broken state).
pub async fn get<T: DeserializeOwned>(cmd: &str) -> Option<T> {
    invoke(cmd, &()).await.ok()
}

/// Fire-and-forget command.
pub fn call<A: Serialize + 'static>(cmd: &'static str, args: A) {
    spawn(async move {
        let _ = invoke::<A, ()>(cmd, &args).await;
    });
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
