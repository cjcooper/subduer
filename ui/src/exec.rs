//! The shell window (opened with `?exec=<label>`): an xterm.js terminal wired
//! to a `kubectl exec -it` session in the backend. Also the main window's
//! helper for opening one, which asks to unlock the context first.

use js_sys::{Array, Function, Object, Reflect, Uint8Array};
use leptos::html::Div;
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use subduer_types::{ExecOutput, ExecSession, ExecTarget};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use crate::app::Store;
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
    #[derive(Serialize)]
    struct Args {
        target: ExecTarget,
    }
    spawn_local(async move {
        if let Err(e) = invoke::<_, ()>("open_exec", &Args { target }).await {
            store.error.set(Some(e));
        }
    });
}

/// The window label from `?exec=<label>`, if this page is a shell window.
pub fn label_from_url() -> Option<String> {
    let search = window().location().search().ok()?;
    label_in(&search)
}

/// The `exec` parameter of a URL's query string (`?a=b&exec=x`).
fn label_in(search: &str) -> Option<String> {
    search
        .trim_start_matches('?')
        .split('&')
        .find_map(|kv| kv.strip_prefix("exec="))
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
}

/// The xterm.js `Terminal` and its fit add-on, as loaded by `index.html`.
#[derive(Clone)]
struct Term {
    term: JsValue,
    fit: JsValue,
}

fn get(obj: &JsValue, key: &str) -> JsValue {
    Reflect::get(obj, &key.into()).unwrap_or(JsValue::UNDEFINED)
}

fn call(obj: &JsValue, method: &str, args: &[JsValue]) -> JsValue {
    match get(obj, method).dyn_into::<Function>() {
        Ok(f) => f.apply(obj, &args.iter().collect::<Array>()).unwrap_or(JsValue::UNDEFINED),
        Err(_) => JsValue::UNDEFINED,
    }
}

fn construct(ctor: &JsValue, args: &[JsValue]) -> Option<JsValue> {
    let ctor = ctor.clone().dyn_into::<Function>().ok()?;
    Reflect::construct(&ctor, &args.iter().collect::<Array>()).ok()
}

fn object(pairs: &[(&str, JsValue)]) -> JsValue {
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
        let dark = crate::prefs::is_dark();
        // Matches the app's --panel / --text / --accent tokens.
        let (bg, fg, cursor, selection) =
            if dark { ("#1d2128", "#e3e7ed", "#5b9bff", "#2f4a7a") } else { ("#ffffff", "#1c2430", "#2f6fe4", "#cfe0fb") };
        let mut theme = vec![
            ("background", bg.into()),
            ("foreground", fg.into()),
            ("cursor", cursor.into()),
            ("selectionBackground", selection.into()),
        ];
        // xterm's default ANSI colours are tuned for dark backgrounds; on white
        // the bright green/yellow/cyan prompts are unreadable, so darken them.
        if !dark {
            theme.extend([
                ("black", "#1c2430".into()),
                ("red", "#c6362c".into()),
                ("green", "#1d7a3f".into()),
                ("yellow", "#8a5300".into()),
                ("blue", "#2457c5".into()),
                ("magenta", "#9b3fb5".into()),
                ("cyan", "#0f7285".into()),
                ("white", "#6b7686".into()),
                ("brightBlack", "#4b5563".into()),
                ("brightRed", "#a8261d".into()),
                ("brightGreen", "#166534".into()),
                ("brightYellow", "#734500".into()),
                ("brightBlue", "#1d4ab0".into()),
                ("brightMagenta", "#7e2f96".into()),
                ("brightCyan", "#0b5d6c".into()),
                ("brightWhite", "#1c2430".into()),
            ]);
        }
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
        let n = |key| get(&self.term, key).as_f64().unwrap_or(80.0) as u16;
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
            let n = |key| get(&e, key).as_f64().unwrap_or(0.0) as u16;
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
            #[derive(Serialize)]
            struct Args {
                cols: u16,
                rows: u16,
            }
            let result = invoke::<_, ExecSession>("exec_start", &Args { cols, rows }).await;
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
            #[derive(Serialize)]
            struct Args {
                data: String,
            }
            spawn_local(async move {
                let _ = invoke::<_, ()>("exec_input", &Args { data }).await;
            });
        });
        t.on_resize(|cols, rows| {
            #[derive(Serialize)]
            struct Args {
                cols: u16,
                rows: u16,
            }
            spawn_local(async move {
                let _ = invoke::<_, ()>("exec_resize", &Args { cols, rows }).await;
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
                            t.write_text(&format!("\r\n\x1b[2m[{message}]\x1b[0m\r\n"));
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

    let heading = move || match session.get() {
        None => "Connecting…".to_owned(),
        Some(Ok(s)) => s.title,
        Some(Err(_)) => "Shell".to_owned(),
    };

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
