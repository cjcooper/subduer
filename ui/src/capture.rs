//! Packet captures of a pod, to show which TLS versions it uses: the
//! Capture… button and dialog in the main window, and the capture window
//! (opened with `?capture=<label>`) that follows one as it runs, with the
//! TLS connections seen so far.
//!
//! The logic is in `logic::capture`.

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use subduer_types::{CapturePhase, CaptureRequest, CaptureSession, CaptureUpdate, ResourceRow};

use crate::app::Store;
use crate::logic::capture::{
    Choices, DURATIONS, HEADERS, TABLE, count_badge, file_name, flagged, image_text, label_in, minutes, phase_class, pod_ports, port_choice, progress,
    progress_attrs, rows, running, status_text, stop_button, totals_text, version_counts, way, where_text,
};
use crate::logic::rightsize::{prod_text, read_only_text};
use crate::modal::Modal;
use crate::tauri::{NoArgs, invoke, listen};

/// "Capture…" on a pod's Details header.
#[component]
pub(crate) fn CaptureButton(store: Store, row: ResourceRow) -> impl IntoView {
    move || {
        let kind = store.kind.get()?;
        let namespace = row.namespace.clone()?;
        (kind.key() == "core/Pod").then(|| {
            let (ns, pod) = (namespace.clone(), row.name.clone());
            view! {
                <button class="btn small" title="Record this pod's network traffic, to see which TLS versions it uses"
                    on:click=move |_| store.capture_prompt.set(Some((ns.clone(), pod.clone())))>"Capture…"</button>
            }
        })
    }
}

#[component]
pub(crate) fn CaptureDialog(store: Store) -> impl IntoView {
    let image = RwSignal::new(String::new());
    let secs = RwSignal::new(DURATIONS[1]);
    // Which traffic: "all", "port:<n>" or "custom"; and whether only handshakes.
    let choice = RwSignal::new("all".to_owned());
    let handshakes = RwSignal::new(true);
    let custom = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let choices = move || Choices { image: image.get(), secs: secs.get(), choice: choice.get(), custom: custom.get(), handshakes: handshakes.get() };
    // Each opening starts from the image last used on this context.
    Effect::new(move |_| {
        if store.capture_prompt.with(Option::is_some) {
            let context = store.context.get_untracked().unwrap_or_default();
            image.set(crate::prefs::load_capture_image(&context));
            // Ports differ from pod to pod.
            choice.set("all".to_owned());
            handshakes.set(true);
            error.set(None);
        }
    });
    let close = move || {
        if !busy.get_untracked() {
            store.capture_prompt.set(None);
        }
    };
    move || {
        let (namespace, pod) = store.capture_prompt.get()?;
        let ctx = store.current_context()?;
        let (locked, prod, name) = (!ctx.unlocked, ctx.prod_like, ctx.name.clone());
        let ready = move || choices().ready(busy.get(), locked);
        let start = {
            let (namespace, pod, context) = (namespace.clone(), pod.clone(), name.clone());
            move |_| {
                #[derive(Serialize)]
                struct Args {
                    request: CaptureRequest,
                }
                #[derive(Serialize)]
                struct Choose {
                    suggested: String,
                }
                let mut request = untrack(choices).request(&namespace, &pod);
                crate::prefs::save_capture_image(&context, &request.image);
                let suggested = file_name(&context, &namespace, &pod, &crate::trace::stamp_now());
                busy.set(true);
                error.set(None);
                spawn_local(async move {
                    // Where to save it first, so it can be opened in Wireshark later.
                    match invoke::<_, Option<String>>("choose_capture_file", &Choose { suggested }).await {
                        Ok(Some(path)) => request.path = path,
                        Ok(None) => {
                            busy.set(false);
                            return;
                        }
                        Err(e) => {
                            busy.set(false);
                            error.set(Some(e));
                            return;
                        }
                    }
                    match invoke::<_, String>("start_capture", &Args { request }).await {
                        Ok(_) => {
                            busy.set(false);
                            store.capture_prompt.set(None);
                        }
                        Err(e) => {
                            busy.set(false);
                            error.set(Some(e));
                        }
                    }
                });
            }
        };
        let unlock = {
            let ctx = ctx.clone();
            move |_| store.unlock_prompt.set(Some(ctx.clone()))
        };
        let option = move |value: String, label: String, hint: Option<&'static str>| {
            let picked = value.clone();
            view! {
                <label class="capture-choice">
                    <input type="radio" name="capture-traffic" prop:checked=move || choice.with(|c| *c == value) prop:disabled=move || busy.get()
                        on:change=move |_| choice.set(picked.clone()) />
                    <span><strong>{label}</strong>{hint.map(|h| view! { <span class="muted small">{h}</span> })}</span>
                </label>
            }
        };
        // The pod's own ports, from its Details.
        let ports = store.pod.with(|p| pod_ports(p.as_ref(), &namespace, &pod));
        Some(view! {
            <Modal on_cancel=move |()| close() busy=busy prod=prod wide=true class="capture-modal">
                <div class="modal-head">
                    <h2>"Capture traffic"</h2>
                    <button class="close" title="Close (Esc)" aria-label="Close" prop:disabled=move || busy.get() on:click=move |_| close()>"×"</button>
                </div>
                <p>"Records "<strong class="mono">{pod.clone()}</strong>"'s network traffic in "<strong>{namespace.clone()}</strong>
                    " on "<strong>{name.clone()}</strong>", from inside the pod, to show which TLS versions it uses."</p>
                <label class="field capture-field">
                    "Capture image"
                    <input class="mono" type="text" placeholder="e.g. 123456789012.dkr.ecr.us-east-1.amazonaws.com/netshoot:latest"
                        prop:value=move || image.get() prop:disabled=move || busy.get()
                        on:input=move |ev| image.set(event_target_value(&ev)) />
                </label>
                <p class="muted small">"Any image with tcpdump and timeout, e.g. netshoot. Remembered for this context."</p>
                <label class="field capture-field">
                    "For"
                    <select prop:disabled=move || busy.get() on:change=move |ev| if let Ok(s) = event_target_value(&ev).parse() { secs.set(s) }>
                        {DURATIONS.map(|s| view! { <option value=s.to_string() selected=move || secs.get() == s>{minutes(s)}</option> }).collect_view()}
                    </select>
                    <span class="muted small">"or until Stop"</span>
                </label>
                <div class="capture-choices" role="radiogroup" aria-label="Traffic">
                    <span class="capture-choices-title">"Traffic"</span>
                    {option("all".into(), "All traffic (TCP)".into(), Some("Every TCP connection in and out of the pod."))}
                    {ports.into_iter().map(|(port, name)| {
                        let (value, label) = port_choice(port, name.as_deref());
                        option(value, label, None)
                    }).collect_view()}
                    {option("custom".into(), "Custom filter".into(), Some("A tcpdump expression, e.g. port 443."))}
                </div>
                {move || (choice.get() == "custom").then(|| view! {
                    <input class="mono confirm-input" type="text" placeholder="tcp port 443" prop:value=move || custom.get()
                        prop:disabled=move || busy.get() on:input=move |ev| custom.set(event_target_value(&ev)) />
                })}
                <label class="capture-choice capture-handshakes">
                    <input type="checkbox" prop:checked=move || handshakes.get() prop:disabled=move || busy.get()
                        on:change=move |_| handshakes.update(|h| *h = !*h) />
                    <span><strong>"TLS handshakes only"</strong><span class="muted small">"Enough to see versions and ciphers; no application data."</span></span>
                </label>
                {move || (!handshakes.get()).then(|| view! {
                    <p class="warn-text small">"Without it, the capture holds any plain-text data the pod sends."</p>
                })}
                <p class="warn-text small">"This adds a capture container to the running pod (no restart). It stops by itself, but stays listed in the pod until the pod is replaced."</p>
                {locked.then(|| view! {
                    <div class="apply-locked">
                        <span class="warn-text">{read_only_text(&name)}</span>
                        <button class="btn small" on:click=unlock>"Allow changes…"</button>
                    </div>
                })}
                {(prod && !locked).then(|| view! {
                    <p>{prod_text(&name)}</p>
                })}
                {move || error.get().map(|e| view! { <p class="warn-text">{e}</p> })}
                <div class="modal-actions">
                    <button prop:disabled=move || busy.get() on:click=move |_| close()>"Cancel"</button>
                    <button class="primary" prop:disabled=move || !ready() on:click=start>
                        {move || if busy.get() { "Starting…" } else { "Save as and start…" }}
                    </button>
                </div>
            </Modal>
        })
    }
}

/// The window label from `?capture=<label>`, if this page is a capture window.
pub fn label_from_url() -> Option<String> {
    let search = window().location().search().ok()?;
    label_in(&search)
}

#[component]
pub fn CaptureApp(label: String) -> impl IntoView {
    let session = RwSignal::new(None::<Result<CaptureSession, String>>);
    let update = RwSignal::new(None::<CaptureUpdate>);
    // This window has no Store: its own copy of the saved sorts.
    let sorts = crate::sort::Sorts::load();
    let now = RwSignal::new(js_sys::Date::now() / 1000.0);
    set_interval(move || now.set(js_sys::Date::now() / 1000.0), std::time::Duration::from_millis(500));
    {
        let label = label.clone();
        spawn_local(async move {
            // Listen first, so nothing sent while asking is missed.
            listen::<CaptureUpdate>(&label, move |u| update.set(Some(u))).await;
            let result = invoke::<_, CaptureSession>("capture_session", &NoArgs {}).await;
            if let Ok(s) = &result
                && update.with_untracked(Option::is_none)
            {
                update.set(Some(s.update.clone()));
            }
            session.set(Some(result));
        });
    }
    let stopping = RwSignal::new(false);
    let stop = move |_| {
        stopping.set(true);
        spawn_local(async move {
            if let Err(e) = invoke::<_, ()>("stop_capture", &NoArgs {}).await {
                web_sys::console::warn_1(&e.into());
            }
            stopping.set(false);
        });
    };
    move || {
        let s = match session.get() {
            None => return view! { <p class="muted pad">"Connecting…"</p> }.into_any(),
            Some(Err(e)) => return view! { <div class="capture-app"><p class="warn-text pad">{e}</p></div> }.into_any(),
            Some(Ok(s)) => s,
        };
        let first = s.update.clone();
        let u = Signal::derive(move || update.get().unwrap_or_else(|| first.clone()));
        let status = move || {
            let u = u.get();
            view! {
                <span class=phase_class(u.phase)>{status_text(&u, now.get())}</span>
                <span class="muted">{totals_text(&u)}</span>
            }
        };
        let progress = move || {
            let done = progress(&u.get(), now.get())?;
            let (value, width) = progress_attrs(done);
            Some(view! { <div class="capture-progress" role="progressbar" aria-valuenow=value><span style=width></span></div> })
        };
        let counts = move || {
            version_counts(&u.get().scan)
                .into_iter()
                .map(|(v, n)| {
                    let (class, text) = count_badge(&v, n);
                    view! { <span class=class><span class="flag-dot" aria-hidden="true"></span>{text}</span> }
                })
                .collect_view()
        };
        let rows = move || {
            let list = rows(u.get().scan.connections, sorts.by(TABLE, &HEADERS));
            if list.is_empty() {
                return view! { <tr><td colspan="7" class="muted">"No TLS handshakes seen yet."</td></tr> }.into_any();
            }
            list.into_iter()
                .map(|c| {
                    let bad = flagged(&c);
                    view! {
                        <tr class:capture-flagged=bad>
                            <td class="mono">{crate::trace_chart::when(c.first_seen, false)}</td>
                            <td>{way(c.direction)}</td>
                            <td class="mono">{c.client.clone()}</td>
                            <td class="mono">{c.server.clone()}</td>
                            <td>{c.server_name.clone().unwrap_or_default()}</td>
                            <td>
                                {match &c.version {
                                    Some(v) if bad => view! { <span class="flag serious"><span class="flag-dot" aria-hidden="true"></span>{v.clone()}</span> }.into_any(),
                                    Some(v) => view! { <span>{v.clone()}</span> }.into_any(),
                                    None => view! { <span class="muted">"no answer"</span> }.into_any(),
                                }}
                                {c.retried.then(|| view! { <span class="muted small">" (retried)"</span> })}
                            </td>
                            <td class="mono small">{c.cipher.clone().unwrap_or_default()}</td>
                        </tr>
                    }
                })
                .collect_view()
                .into_any()
        };
        let message = move || u.get().message.map(|m| {
            let failed = u.get().phase == CapturePhase::Failed;
            view! { <pre class="capture-message" class:warn-text=failed>{m}</pre> }
        });
        let asked = s.image.clone();
        let image = move || image_text(&u.get(), &asked);
        view! {
            <div class="capture-app">
                <header class="capture-head">
                    <div>
                        <h1>{s.pod.clone()}</h1>
                        <div class="muted">{where_text(&s.namespace, &s.context)}</div>
                    </div>
                    <span class="spacer"></span>
                    {move || running(u.get().phase).then(|| view! {
                        <button class="btn small danger" prop:disabled=move || stop_button(stopping.get(), u.get().phase).0 on:click=stop>
                            {move || stop_button(stopping.get(), u.get().phase).1}
                        </button>
                    })}
                </header>
                <div class="capture-status">{status}</div>
                {progress}
                <dl class="capture-facts">
                    <dt>"Image"</dt><dd class="mono">{image}</dd>
                    <dt>"Filter"</dt><dd class="mono">{s.expression.clone()}</dd>
                    <dt>"Container"</dt><dd class="mono">{s.container.clone()}</dd>
                    <dt>"File"</dt><dd class="mono path">{s.path.clone()}</dd>
                </dl>
                {message}
                <div class="capture-counts">{counts}</div>
                <div class="capture-table-wrap">
                    <table class="resources capture-table">
                        <thead><tr>{HEADERS.map(|h| view! { <crate::sort::SortTh sorts=sorts table=TABLE header=h /> })}</tr></thead>
                        <tbody>{rows}</tbody>
                    </table>
                </div>
            </div>
        }
        .into_any()
    }
}
