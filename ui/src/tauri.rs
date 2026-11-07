//! Thin bindings to the Tauri JS API (`withGlobalTauri` exposes `window.__TAURI__`).

use serde::Serialize;
use serde::de::DeserializeOwned;
use wasm_bindgen::prelude::*;

use crate::logic::tauri as logic;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke, catch)]
    async fn invoke_raw(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke, catch)]
    async fn invoke_with_options(cmd: &str, args: JsValue, options: JsValue) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "event"], js_name = listen)]
    async fn listen_raw(event: &str, handler: &Closure<dyn FnMut(JsValue)>, options: JsValue) -> JsValue;
}

/// Calls a backend command. Backend errors come back as their message.
pub async fn invoke<A: Serialize, R: DeserializeOwned>(cmd: &str, args: &A) -> Result<R, String> {
    let args = args
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(|e| e.to_string())?;
    let value = invoke_raw(cmd, args)
        .await
        .map_err(|e| logic::call_error(e.as_string(), &|| format!("{e:?}")))?;
    serde_wasm_bindgen::from_value(value).map_err(|e| logic::bad_response(cmd, e))
}

/// Calls a backend command with a binary body (sent as is, not as JSON) and
/// request headers for the metadata.
pub async fn invoke_bytes<R: DeserializeOwned>(
    cmd: &str,
    body: &js_sys::Uint8Array,
    headers: &[(&str, &str)],
) -> Result<R, String> {
    let header_obj = js_sys::Object::new();
    for (k, v) in headers {
        let _ = js_sys::Reflect::set(&header_obj, &(*k).into(), &(*v).into());
    }
    let options = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&options, &"headers".into(), &header_obj);
    let value = invoke_with_options(cmd, body.into(), options.into())
        .await
        .map_err(|e| logic::call_error(e.as_string(), &|| format!("{e:?}")))?;
    serde_wasm_bindgen::from_value(value).map_err(|e| logic::bad_response(cmd, e))
}

/// This window's label (`main`, `main-2`, `logs-3`, …), if Tauri says.
fn window_label() -> Option<String> {
    let get = |obj: &JsValue, key: &str| js_sys::Reflect::get(obj, &JsValue::from_str(key)).ok().filter(|v| !v.is_undefined());
    let tauri = get(&leptos::prelude::window().into(), "__TAURI__")?;
    let module = get(&tauri, "webviewWindow")?;
    let current = get(&module, "getCurrentWebviewWindow")?.dyn_into::<js_sys::Function>().ok()?.call0(&module).ok()?;
    get(&current, "label")?.as_string()
}

/// Subscribes to a backend event sent to this window, for the lifetime of the
/// page. Only this window's: without a target, Tauri hands a listener the
/// events sent to every window, and each main window has its own cluster.
pub async fn listen<T: DeserializeOwned + 'static>(event: &str, mut handler: impl FnMut(T) + 'static) {
    let name = event.to_owned();
    let closure = Closure::<dyn FnMut(JsValue)>::new(move |raw: JsValue| {
        let payload = js_sys::Reflect::get(&raw, &JsValue::from_str("payload")).unwrap_or(JsValue::NULL);
        match serde_wasm_bindgen::from_value(payload) {
            Ok(value) => handler(value),
            Err(e) => web_log(&logic::bad_payload(&name, e)),
        }
    });
    let options = js_sys::Object::new();
    if let Some(label) = window_label() {
        let _ = js_sys::Reflect::set(&options, &"target".into(), &label.into());
    }
    listen_raw(event, &closure, options.into()).await;
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
