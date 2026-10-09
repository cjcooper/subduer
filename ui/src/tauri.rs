//! Thin bindings to the Tauri JS API (`withGlobalTauri` exposes `window.__TAURI__`).

use serde::Serialize;
use serde::de::DeserializeOwned;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke, catch)]
    async fn invoke_raw(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "event"], js_name = listen)]
    async fn listen_raw(event: &str, handler: &Closure<dyn FnMut(JsValue)>) -> JsValue;
}

/// Calls a backend command. Backend errors come back as their message.
pub async fn invoke<A: Serialize, R: DeserializeOwned>(cmd: &str, args: &A) -> Result<R, String> {
    let args = args
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(|e| e.to_string())?;
    let value = invoke_raw(cmd, args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))?;
    serde_wasm_bindgen::from_value(value).map_err(|e| format!("{cmd}: bad response: {e}"))
}

/// Subscribes to a backend event for the lifetime of the page.
pub async fn listen<T: DeserializeOwned + 'static>(event: &str, mut handler: impl FnMut(T) + 'static) {
    let name = event.to_owned();
    let closure = Closure::<dyn FnMut(JsValue)>::new(move |raw: JsValue| {
        let payload = js_sys::Reflect::get(&raw, &JsValue::from_str("payload")).unwrap_or(JsValue::NULL);
        match serde_wasm_bindgen::from_value(payload) {
            Ok(value) => handler(value),
            Err(e) => web_log(&format!("bad {name} payload: {e}")),
        }
    });
    listen_raw(event, &closure).await;
    closure.forget();
}

pub fn web_log(msg: &str) {
    web_sys_console_error(msg);
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = error)]
    fn web_sys_console_error(s: &str);
}

/// Unit argument for commands that take none.
#[derive(Serialize)]
pub struct NoArgs {}
