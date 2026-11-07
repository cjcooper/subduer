//! The shell window (opened with `?exec=<label>`): an xterm.js terminal wired
//! to a `kubectl exec -it` session in the backend. Also the main window's
//! helper for opening one, which asks to unlock the context first.

use js_sys::{Array, Function, Object, Reflect, Uint8Array};
use leptos::html::Div;
use leptos::prelude::*;
use leptos::task::spawn_local;
use subduer_types::{ExecOutput, ExecSession, ExecTarget};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use crate::app::Store;
use crate::logic::exec::{self as logic, InputArgs, OpenArgs, SizeArgs, label_in};
use crate::tauri::{invoke, listen};

/// Opens a shell window, or the unlock prompt first if the context is
/// read-only; the shell then opens once it's unlocked.
pub fn request(store: Store, target: ExecTarget) {
    let Some(context) = store.current_context_untracked() else { return };
    if context.unlocked {
        open(store, target);
    } else {
        store.pending_exec.set(Some((context.name.clone(), target)));
        store.unlock_prompt.set(Some(context));
    }
}

pub fn open(store: Store, target: ExecTarget) {
    spawn_local(async move {
        if let Err(e) = invoke::<_, ()>("open_exec", &OpenArgs { target }).await {
            store.error.set(Some(e));
        }
    });
}

/// The window label from `?exec=<label>`, if this page is a shell window.
pub fn label_from_url() -> Option<String> {
    let search = window().location().search().ok()?;
    label_in(&search)
}

/// The xterm.js `Terminal` and its fit add-on, as loaded by `index.html`.
#[derive(Clone)]
struct Term {
    term: JsValue,
    fit: JsValue,
}

pub(crate) fn get(obj: &JsValue, key: &str) -> JsValue {
    Reflect::get(obj, &key.into()).unwrap_or(JsValue::UNDEFINED)
}

pub(crate) fn call(obj: &JsValue, method: &str, args: &[JsValue]) -> JsValue {
    match get(obj, method).dyn_into::<Function>() {
        Ok(f) => f.apply(obj, &args.iter().collect::<Array>()).unwrap_or(JsValue::UNDEFINED),
        Err(_) => JsValue::UNDEFINED,
    }
}

fn construct(ctor: &JsValue, args: &[JsValue]) -> Option<JsValue> {
    let ctor = ctor.clone().dyn_into::<Function>().ok()?;
    Reflect::construct(&ctor, &args.iter().collect::<Array>()).ok()
}

pub(crate) fn object(pairs: &[(&str, JsValue)]) -> JsValue {
    let obj = Object::new();
    for (k, v) in pairs {
        let _ = Reflect::set(&obj, &(*k).into(), v);
    }
    obj.into()
}

impl Term {
    fn new(host: &web_sys::HtmlElement) -> Option<Self> {
        let w: JsValue = window().into();
        // The Appearance setting, or the system's when it's Auto.
        let theme: Vec<(&str, JsValue)> =
            logic::theme(crate::prefs::is_dark()).into_iter().map(|(k, v)| (k, JsValue::from_str(v))).collect();
        let theme = object(&theme);
        let options = object(&[
            ("cursorBlink", true.into()),
            ("fontFamily", "\"SF Mono\", Menlo, monospace".into()),
            ("fontSize", 13.into()),
            ("scrollback", 5000.into()),
            ("macOptionIsMeta", true.into()),
            ("theme", theme),
        ]);
        let term = construct(&get(&w, "Terminal"), &[options])?;
        let fit = construct(&get(&get(&w, "FitAddon"), "FitAddon"), &[])?;
        call(&term, "loadAddon", std::slice::from_ref(&fit));
        call(&term, "open", &[host.clone().into()]);
        let t = Self { term, fit };
        t.fit();
        call(&t.term, "focus", &[]);
        Some(t)
    }

    fn fit(&self) {
        call(&self.fit, "fit", &[]);
    }

    fn size(&self) -> (u16, u16) {
        let n = |key| logic::dimension(get(&self.term, key).as_f64(), 80.0);
        (n("cols"), n("rows"))
    }

    fn write_bytes(&self, bytes: &[u8]) {
        call(&self.term, "write", &[Uint8Array::from(bytes).into()]);
    }

    fn write_text(&self, text: &str) {
        call(&self.term, "write", &[text.into()]);
    }

    fn reset(&self) {
        call(&self.term, "reset", &[]);
    }

    /// Calls `f` with each chunk of typed or pasted input.
    fn on_data(&self, f: impl FnMut(String) + 'static) {
        let cb = Closure::<dyn FnMut(String)>::new(f);
        call(&self.term, "onData", &[cb.as_ref().clone()]);
        cb.forget();
    }

    /// Calls `f` with the new (cols, rows) whenever the terminal is resized.
    fn on_resize(&self, mut f: impl FnMut(u16, u16) + 'static) {
        let cb = Closure::<dyn FnMut(JsValue)>::new(move |e: JsValue| {
            let n = |key| logic::dimension(get(&e, key).as_f64(), 0.0);
            f(n("cols"), n("rows"));
        });
        call(&self.term, "onResize", &[cb.as_ref().clone()]);
        cb.forget();
    }
}

fn decode_base64(b64: &str) -> Vec<u8> {
    window().atob(b64).map(|bin| bin.chars().map(|c| c as u8).collect()).unwrap_or_default()
}

#[component]
pub fn ExecApp(label: String) -> impl IntoView {
    let host = NodeRef::<Div>::new();
    let session = RwSignal::new(None::<Result<ExecSession, String>>);
    let ended = RwSignal::new(None::<String>);
    let term = StoredValue::new_local(None::<Term>);

    let connect = move || {
        ended.set(None);
        let Some((cols, rows)) = term.with_value(|t| t.as_ref().map(Term::size)) else { return };
        spawn_local(async move {
            let result = invoke::<_, ExecSession>("exec_start", &SizeArgs { cols, rows }).await;
            if let Err(e) = &result {
                ended.set(Some(e.clone()));
            }
            session.set(Some(result));
        });
    };

    // Build the terminal once the host element exists, then listen and connect.
    Effect::new(move |done: Option<bool>| {
        if done == Some(true) {
            return true;
        }
        let Some(el) = host.get() else { return false };
        let Some(t) = Term::new(&el) else {
            ended.set(Some("The terminal (xterm.js) failed to load.".into()));
            return true;
        };
        t.on_data(|data| {
            spawn_local(async move {
                let _ = invoke::<_, ()>("exec_input", &InputArgs { data }).await;
            });
        });
        t.on_resize(|cols, rows| {
            spawn_local(async move {
                let _ = invoke::<_, ()>("exec_resize", &SizeArgs { cols, rows }).await;
            });
        });
        term.set_value(Some(t));
        let label = label.clone();
        spawn_local(async move {
            listen::<ExecOutput>(&label, move |out| {
                term.with_value(|t| {
                    let Some(t) = t else { return };
                    match out {
                        ExecOutput::Data(b64) => t.write_bytes(&decode_base64(&b64)),
                        ExecOutput::Exited(message) => {
                            t.write_text(&logic::exited_line(&message));
                            ended.set(Some(message));
                        }
                    }
                });
            })
            .await;
            connect();
        });
        true
    });

    let _resize = window_event_listener(leptos::ev::resize, move |_| {
        term.with_value(|t| {
            if let Some(t) = t {
                t.fit();
            }
        });
    });
    let reconnect = move |_| {
        term.with_value(|t| {
            if let Some(t) = t {
                t.reset();
                call(&t.term, "focus", &[]);
            }
        });
        connect();
    };

    let heading = move || logic::heading(session.get());

    view! {
        <div class="exec-app">
            <header class="logs-header">
                <div class="logs-title">
                    <span class="logs-heading" title=heading>{heading}</span>
                </div>
                {move || ended.get().map(|message| view! {
                    <div class="logs-tools">
                        <span class="exec-ended">{message}</span>
                        <button class="btn small" on:click=reconnect>"Reconnect"</button>
                    </div>
                })}
            </header>
            <div class="exec-term" node_ref=host></div>
        </div>
    }
}

#[cfg(test)]
#[path = "exec_tests.rs"]
mod tests_unit;
