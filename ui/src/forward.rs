//! Port-forwarding: the start dialog (from a pod container or a Service) and
//! the Port Forwards page listing running forwards.

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use subduer_types::{ForwardInfo, ForwardRequest, ForwardSource, PortOption};
use wasm_bindgen::{JsCast, JsValue};

use crate::modal::Modal;
use crate::app::Store;
use crate::tauri::invoke;

/// What the start dialog is for.
#[derive(Debug, Clone, PartialEq)]
pub struct ForwardPrompt {
    pub namespace: String,
    pub source: ForwardSource,
}

pub fn request(store: Store, namespace: String, source: ForwardSource) {
    store.forward_prompt.set(Some(ForwardPrompt { namespace, source }));
}

#[derive(Serialize)]
struct IdArgs {
    id: u64,
}

fn open_in_browser(store: Store, id: u64) {
    spawn_local(async move {
        if let Err(e) = invoke::<_, ()>("open_forward", &IdArgs { id }).await {
            store.error.set(Some(e));
        }
    });
}

fn stop(store: Store, id: u64) {
    spawn_local(async move {
        if let Err(e) = invoke::<_, ()>("stop_forward", &IdArgs { id }).await {
            store.error.set(Some(e));
        }
    });
}

pub(crate) fn copy(text: &str) {
    let navigator = js_sys::Reflect::get(&window(), &"navigator".into()).unwrap_or(JsValue::UNDEFINED);
    let clipboard = js_sys::Reflect::get(&navigator, &"clipboard".into()).unwrap_or(JsValue::UNDEFINED);
    if let Ok(write) = js_sys::Reflect::get(&clipboard, &"writeText".into())
        && let Ok(write) = write.dyn_into::<js_sys::Function>()
    {
        let _ = write.call1(&clipboard, &text.into());
    }
}

fn address(f: &ForwardInfo) -> String {
    format!("127.0.0.1:{}", f.local_port)
}

/// Fetches the current list (e.g. after a reload); changes then arrive as events.
pub fn load(store: Store) {
    spawn_local(async move {
        if let Ok(list) = invoke::<_, Vec<ForwardInfo>>("list_forwards", &crate::tauri::NoArgs {}).await {
            store.forwards.set(list);
        }
    });
}

#[component]
pub fn ForwardDialog(store: Store) -> impl IntoView {
    let options = RwSignal::new(None::<Result<Vec<PortOption>, String>>);
    let chosen = RwSignal::new(None::<u16>);
    let local = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let started = RwSignal::new(None::<ForwardInfo>);

    // Load the ports whenever the dialog opens.
    Effect::new(move |_| {
        let Some(prompt) = store.forward_prompt.get() else { return };
        options.set(None);
        chosen.set(None);
        error.set(None);
        started.set(None);
        busy.set(false);
        let asked = prompt.clone();
        spawn_local(async move {
            #[derive(Serialize)]
            struct Args {
                namespace: String,
                source: ForwardSource,
            }
            let result = invoke::<_, Vec<PortOption>>(
                "forward_ports",
                &Args { namespace: prompt.namespace, source: prompt.source },
            )
            .await;
            // Cancelled while loading, maybe reopened for something else.
            if store.forward_prompt.with_untracked(|p| p.as_ref() != Some(&asked)) {
                return;
            }
            if let Ok(first) = result.as_ref().map(|o| o.first().map(|p| p.port)) {
                chosen.set(first);
                local.set(first.map(|p| p.to_string()).unwrap_or_default());
            }
            options.set(Some(result));
        });
    });

    let close = move || store.forward_prompt.set(None);
    let local_port = move || local.with(|l| l.trim().parse::<u16>().ok());
    let start = move |_| {
        let (Some(prompt), Some(remote), Some(local_port)) = (store.forward_prompt.get_untracked(), chosen.get_untracked(), local_port())
        else {
            return;
        };
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            #[derive(Serialize)]
            struct Args {
                request: ForwardRequest,
            }
            let request = ForwardRequest { namespace: prompt.namespace, source: prompt.source, remote_port: remote, local_port };
            match invoke::<_, ForwardInfo>("start_forward", &Args { request }).await {
                Ok(info) => started.set(Some(info)),
                Err(e) => error.set(Some(e)),
            }
            busy.set(false);
        });
    };

    move || {
        store.forward_prompt.get().map(|prompt| {
            let target = format!("{}/{}", prompt.namespace, prompt.source.label());
            let body = move || match (started.get(), options.get()) {
                (Some(f), _) => {
                    let (addr, id) = (address(&f), f.id);
                    let backend = f.backend.clone().unwrap_or_default();
                    view! {
                        <p>"Forwarding "<strong class="mono">{addr.clone()}</strong>" → "<span class="mono">{backend}</span></p>
                        <p class="muted">"It keeps running until you stop it on the Port Forwards page or quit the app."</p>
                        <div class="modal-actions">
                            <button on:click=move |_| copy(&addr)>"Copy address"</button>
                            <button on:click=move |_| open_in_browser(store, id)>"Open in browser"</button>
                            <button class="primary" on:click=move |_| close()>"Done"</button>
                        </div>
                    }
                    .into_any()
                }
                (None, None) => view! {
                    <p class="muted">"Loading ports…"</p>
                    <div class="modal-actions"><button on:click=move |_| close()>"Cancel"</button></div>
                }
                .into_any(),
                (None, Some(Err(e))) => view! {
                    <p class="warn">{e}</p>
                    <div class="modal-actions"><button on:click=move |_| close()>"Close"</button></div>
                }
                .into_any(),
                (None, Some(Ok(ports))) if ports.is_empty() => view! {
                    <p class="muted">"No TCP ports are declared here. Port-forwarding a pod needs its container ports listed."</p>
                    <div class="modal-actions"><button on:click=move |_| close()>"Close"</button></div>
                }
                .into_any(),
                (None, Some(Ok(ports))) => {
                    let rows = ports
                        .into_iter()
                        .map(|p| {
                            let port = p.port;
                            let label = match (&p.name, &p.detail) {
                                (Some(n), Some(d)) => format!("{port} · {n} · {d}"),
                                (Some(n), None) => format!("{port} · {n}"),
                                (None, Some(d)) => format!("{port} · {d}"),
                                (None, None) => port.to_string(),
                            };
                            view! {
                                <label class="port-option">
                                    <input
                                        type="radio"
                                        name="remote-port"
                                        prop:checked=move || chosen.get() == Some(port)
                                        on:change=move |_| {
                                            chosen.set(Some(port));
                                            local.set(port.to_string());
                                        }
                                    />
                                    <span class="mono">{label}</span>
                                </label>
                            }
                        })
                        .collect_view();
                    view! {
                        <div class="port-options">{rows}</div>
                        <label class="local-port">
                            "Local port on 127.0.0.1"
                            <input
                                class="confirm-input"
                                inputmode="numeric"
                                prop:value=move || local.get()
                                on:input=move |ev| local.set(event_target_value(&ev))
                            />
                            <span class="muted">"0 picks a free port."</span>
                        </label>
                        {move || error.get().map(|e| view! { <p class="warn">{e}</p> })}
                        <div class="modal-actions">
                            <button prop:disabled=move || busy.get() on:click=move |_| close()>"Cancel"</button>
                            <button
                                class="primary"
                                prop:disabled=move || busy.get() || chosen.get().is_none() || local_port().is_none()
                                on:click=start
                            >{move || if busy.get() { "Starting…" } else { "Start forwarding" }}</button>
                        </div>
                    }
                    .into_any()
                }
            };
            view! {
                <Modal on_cancel=move |()| close() busy=busy>
                    <h2>"Port forward"</h2>
                    <p class="mono">{target}</p>
                    {body}
                </Modal>
            }
        })
    }
}

/// How a forward sorts under column `i`: ports by number, connections by
/// how many are open, then the total.
pub(crate) fn forward_key(f: &ForwardInfo, i: usize) -> crate::sort::Key {
    use crate::sort::Key;
    match i {
        0 => Key::text(&format!("{}/{}", f.namespace, f.source.label())),
        1 => Key::text(&f.context),
        2 => Key::num(f64::from(f.local_port)),
        3 => Key::num(f64::from(f.remote_port)),
        4 => Key::num(f.active as f64 * 1e12 + f.total as f64),
        _ => Key::text(f.last_error.as_deref().unwrap_or_default()),
    }
}

#[component]
pub fn ForwardsPage(store: Store) -> impl IntoView {
    const HEADERS: [&str; 6] = ["Target", "Context", "Local", "Remote", "Connections", "Last error"];
    let rows = move || {
        let mut forwards = store.forwards.get();
        crate::sort::sort(&mut forwards, store.sorts.by("forwards", &HEADERS), forward_key);
        forwards
            .into_iter()
            .map(|f| {
                let (id, addr) = (f.id, address(&f));
                let copy_addr = addr.clone();
                let target = format!("{}/{}", f.namespace, f.source.label());
                let remote = match &f.backend {
                    Some(b) => format!("{} → {b}", f.remote_port),
                    None => f.remote_port.to_string(),
                };
                let connections = format!("{} open · {} total", f.active, f.total);
                view! {
                    <tr>
                        <td class="mono">{target}</td>
                        <td>{f.context.clone()}</td>
                        <td class="mono">{addr}</td>
                        <td class="mono">{remote}</td>
                        <td>{connections}</td>
                        <td class="forward-error" title=f.last_error.clone().unwrap_or_default()>
                            {f.last_error.clone().map(|e| view! { <span class="warn-text">{e}</span> })}
                        </td>
                        <td>
                            <div class="forward-actions">
                                <button class="btn small" on:click=move |_| open_in_browser(store, id)>"Open"</button>
                                <button class="btn small" on:click=move |_| copy(&copy_addr)>"Copy"</button>
                                <button class="btn small danger-outline" on:click=move |_| stop(store, id)>"Stop"</button>
                            </div>
                        </td>
                    </tr>
                }
            })
            .collect_view()
    };
    view! {
        <div class="resource-header">
            <h1>"Port Forwards"</h1>
            <span class="muted">"127.0.0.1 only · they keep running across contexts until stopped"</span>
        </div>
        {move || if store.forwards.with(Vec::is_empty) {
            view! {
                <p class="muted pad">
                    "No port forwards. Start one from a pod container's Forward button or a Service's detail panel."
                </p>
            }
            .into_any()
        } else {
            view! {
                <div class="table-wrap">
                <table class="resources forwards">
                    <thead><tr>
                        {HEADERS.map(|h| view! { <crate::sort::SortTh sorts=store.sorts table="forwards" header=h /> })}
                        <th></th>
                    </tr></thead>
                    <tbody>{rows}</tbody>
                </table>
                </div>
            }
            .into_any()
        }}
    }
}

#[cfg(test)]
#[path = "forward_tests.rs"]
mod tests_unit;
