//! Packet captures of a pod, to show which TLS versions it uses: the
//! Capture… button and dialog in the main window, and the capture window
//! (opened with `?capture=<label>`) that follows one as it runs, with the
//! TLS connections seen so far.

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use subduer_types::{
    CaptureFilter, CapturePhase, CaptureRequest, CaptureSession, CaptureTraffic, CaptureUpdate, PodDetails, ResourceRow, TlsConnection, TlsDirection,
    TlsScan,
};

use crate::app::Store;
use crate::modal::Modal;
use crate::tauri::{NoArgs, invoke, listen};

/// The dialog's choices of how long to capture, in seconds.
pub(crate) const DURATIONS: [u32; 4] = [60, 5 * 60, 10 * 60, 30 * 60];
/// The version auditors expect; anything else is flagged.
pub(crate) const GOOD: &str = "TLS 1.3";

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

/// The filter from the dialog: its traffic choice ("all", "port:<n>" or
/// "custom"), the custom text, and the handshakes box.
pub(crate) fn filter_from(choice: &str, custom: &str, handshakes_only: bool) -> CaptureFilter {
    let traffic = match choice.strip_prefix("port:").and_then(|p| p.parse().ok()) {
        Some(port) => CaptureTraffic::Inbound(port),
        None if choice == "custom" => CaptureTraffic::Custom(custom.trim().to_owned()),
        None => CaptureTraffic::All,
    };
    CaptureFilter { traffic, handshakes_only }
}

/// The TCP ports the pod's containers declare, each once, in order, with a
/// name if one container gives it one: "http 8080/TCP" → (8080, "http").
/// Ephemeral containers declare none.
pub(crate) fn inbound_ports(d: &PodDetails) -> Vec<(u16, Option<String>)> {
    let mut ports: Vec<(u16, Option<String>)> = vec![];
    for spec in d.containers.iter().filter(|c| !c.ephemeral).flat_map(|c| &c.ports) {
        let (name, port) = match spec.rsplit_once(' ') {
            Some((name, port)) => (Some(name.to_owned()), port),
            None => (None, spec.as_str()),
        };
        let Some((number, proto)) = port.split_once('/') else { continue };
        let Ok(number) = number.parse::<u16>() else { continue };
        if proto != "TCP" {
            continue;
        }
        match ports.iter_mut().find(|p| p.0 == number) {
            Some(known) if known.1.is_none() => known.1 = name,
            Some(_) => {}
            None => ports.push((number, name)),
        }
    }
    ports.sort_by_key(|p| p.0);
    ports
}

/// The Save dialog's suggestion: `acme-uat-shop-web-1-2026-10-12-1430.pcap`.
pub(crate) fn file_name(context: &str, namespace: &str, pod: &str, stamp: &str) -> String {
    let safe = |s: &str| s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '-' }).collect::<String>();
    format!("{}-{}-{}-{stamp}.pcap", safe(context), safe(namespace), safe(pod))
}

/// "1 min", "30 min".
pub(crate) fn minutes(secs: u32) -> String {
    format!("{} min", secs / 60)
}

#[component]
pub(crate) fn CaptureDialog(store: Store) -> impl IntoView {
    let image = RwSignal::new(String::new());
    let secs = RwSignal::new(DURATIONS[1]);
    // Which traffic: "all", "port:<n>" or "custom"; and whether only handshakes.
    let choice = RwSignal::new("all".to_owned());
    let handshakes = RwSignal::new(true);
    let custom = RwSignal::new(String::new());
    let typed = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    // Each opening starts from the image last used on this context.
    Effect::new(move |_| {
        if store.capture_prompt.with(Option::is_some) {
            let context = store.context.get_untracked().unwrap_or_default();
            image.set(crate::prefs::load_capture_image(&context));
            // Ports differ from pod to pod.
            choice.set("all".to_owned());
            handshakes.set(true);
            typed.set(String::new());
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
        let ready = {
            let name = name.clone();
            move || {
                !busy.get()
                    && !locked
                    && !image.with(|i| i.trim().is_empty())
                    && !(choice.get() == "custom" && custom.with(|c| c.trim().is_empty()))
                    && (!prod || typed.with(|t| t.trim() == name))
            }
        };
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
                let mut request = CaptureRequest {
                    namespace: namespace.clone(),
                    pod: pod.clone(),
                    image: image.get_untracked().trim().to_owned(),
                    filter: filter_from(&choice.get_untracked(), &custom.get_untracked(), handshakes.get_untracked()),
                    secs: secs.get_untracked(),
                    path: String::new(),
                };
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
        let ports = store.pod.with(|p| match p {
            Some(Ok(d)) if d.name == pod && d.namespace == namespace => inbound_ports(d),
            _ => vec![],
        });
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
                        let label = match name {
                            Some(n) => format!("Inbound to port {port} ({n})"),
                            None => format!("Inbound to port {port}"),
                        };
                        option(format!("port:{port}"), label, None)
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
                        <span class="warn-text">{format!("{name} is read-only.")}</span>
                        <button class="btn small" on:click=unlock>"Allow changes…"</button>
                    </div>
                })}
                {(prod && !locked).then(|| view! {
                    <p>{format!("{name} looks like production. Type its name to capture:")}</p>
                    <input class="confirm-input" prop:value=move || typed.get() on:input=move |ev| typed.set(event_target_value(&ev)) />
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

/// Only labels the backend gives capture windows (`capture-<n>`).
fn label_in(search: &str) -> Option<String> {
    search
        .trim_start_matches('?')
        .split('&')
        .find_map(|kv| kv.strip_prefix("capture="))
        .filter(|l| l.starts_with("capture-"))
        .map(str::to_owned)
}

/// Connections by negotiated version, TLS 1.3 first, then the rest by
/// count; "no answer" for connections whose server answer wasn't captured.
pub(crate) fn version_counts(scan: &TlsScan) -> Vec<(String, usize)> {
    let mut counts: Vec<(String, usize)> = vec![];
    for c in &scan.connections {
        let v = c.version.clone().unwrap_or_else(|| "no answer".into());
        match counts.iter_mut().find(|(k, _)| *k == v) {
            Some((_, n)) => *n += 1,
            None => counts.push((v, 1)),
        }
    }
    counts.sort_by(|a, b| (b.0 == GOOD).cmp(&(a.0 == GOOD)).then(b.1.cmp(&a.1)).then(a.0.cmp(&b.0)));
    counts
}

/// Not the expected version: an older one was chosen.
pub(crate) fn flagged(c: &TlsConnection) -> bool {
    c.version.as_deref().is_some_and(|v| v != GOOD)
}

/// "12.3 KiB".
pub(crate) fn size_text(bytes: u64) -> String {
    crate::overview::fmt_bytes(bytes as f64)
}

/// "4:05" left.
pub(crate) fn clock_text(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn phase_text(p: CapturePhase) -> &'static str {
    match p {
        CapturePhase::Starting => "Starting the capture container…",
        CapturePhase::Capturing => "Capturing",
        CapturePhase::Stopping => "Stopping…",
        CapturePhase::Done => "Finished",
        CapturePhase::Failed => "Failed",
    }
}

/// The capture window.
#[component]
pub fn CaptureApp(label: String) -> impl IntoView {
    let session = RwSignal::new(None::<Result<CaptureSession, String>>);
    let update = RwSignal::new(None::<CaptureUpdate>);
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
            let left = match (u.phase, u.ends) {
                (CapturePhase::Capturing, Some(end)) => format!(" · {} left", clock_text(end - now.get())),
                _ => String::new(),
            };
            let class = match u.phase {
                CapturePhase::Failed => "capture-phase failed",
                CapturePhase::Done => "capture-phase done",
                _ => "capture-phase live",
            };
            view! {
                <span class=class>{phase_text(u.phase)}{left}</span>
                <span class="muted">{format!("{} packets · {} · {} TLS connections", u.scan.packets, size_text(u.bytes), u.scan.connections.len())}</span>
            }
        };
        let progress = move || {
            let u = u.get();
            let (Some(a), Some(b)) = (u.started, u.ends) else { return None };
            let done = if u.phase == CapturePhase::Capturing { ((now.get() - a) / (b - a)).clamp(0.0, 1.0) } else { 1.0 };
            Some(view! { <div class="capture-progress" role="progressbar" aria-valuenow=(done * 100.0).round().to_string()><span style=format!("width: {:.1}%", done * 100.0)></span></div> })
        };
        let counts = move || {
            version_counts(&u.get().scan)
                .into_iter()
                .map(|(v, n)| {
                    let class = if v == GOOD { "flag good" } else if v == "no answer" { "flag info" } else { "flag serious" };
                    view! { <span class=class><span class="flag-dot" aria-hidden="true"></span>{format!("{v} · {n}")}</span> }
                })
                .collect_view()
        };
        let rows = move || {
            let mut list = u.get().scan.connections;
            // Newest first: what just happened is on top.
            list.reverse();
            if list.is_empty() {
                return view! { <tr><td colspan="7" class="muted">"No TLS handshakes seen yet."</td></tr> }.into_any();
            }
            list.into_iter()
                .map(|c| {
                    let bad = flagged(&c);
                    let way = match c.direction {
                        TlsDirection::Inbound => "in",
                        TlsDirection::Outbound => "out",
                        TlsDirection::Other => "",
                    };
                    view! {
                        <tr class:capture-flagged=bad>
                            <td class="mono">{crate::trace_chart::when(c.first_seen, false)}</td>
                            <td>{way}</td>
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
        let running = move || matches!(u.get().phase, CapturePhase::Starting | CapturePhase::Capturing);
        let message = move || u.get().message.map(|m| {
            let failed = u.get().phase == CapturePhase::Failed;
            view! { <pre class="capture-message" class:warn-text=failed>{m}</pre> }
        });
        let asked = s.image.clone();
        let image = move || u.get().image_id.unwrap_or_else(|| asked.clone());
        view! {
            <div class="capture-app">
                <header class="capture-head">
                    <div>
                        <h1>{s.pod.clone()}</h1>
                        <div class="muted">{format!("{} · {}", s.namespace, s.context)}</div>
                    </div>
                    <span class="spacer"></span>
                    {move || running().then(|| view! {
                        <button class="btn small danger" prop:disabled=move || stopping.get() || u.get().phase != CapturePhase::Capturing on:click=stop>
                            {move || if stopping.get() || u.get().phase == CapturePhase::Stopping { "Stopping…" } else { "Stop" }}
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
                        <thead><tr><th>"Time"</th><th>"Way"</th><th>"Client"</th><th>"Server"</th><th>"Server name"</th><th>"Version"</th><th>"Cipher"</th></tr></thead>
                        <tbody>{rows}</tbody>
                    </table>
                </div>
            </div>
        }
        .into_any()
    }
}

#[cfg(test)]
#[path = "capture_tests.rs"]
mod tests;
