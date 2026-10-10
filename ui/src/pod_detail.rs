//! The Details tab for a selected pod: resources, pod info, one section per
//! container (resources, state, environment) and events. Every section
//! collapses; the collapsed set is remembered across selections.

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use subduer_types::{
    ContainerDetails, EnvValue, EnvVar, EventSummary, ExecTarget, HistorySource, LogSource, LogTarget, PodDetails, PodHistory, Usage,
    UsageHistory,
};

use crate::app::{ContainerIcon, Store, format_age};
use crate::chart::{ChartData, LineChart, RefLine, Scale, clock};
use crate::overview::{fmt_bytes, fmt_cpu};
use crate::tauri::invoke;

/// Re-fetches the selected pod's details (metrics, state, events).
pub fn load(store: Store, quiet: bool) {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        name: String,
    }
    load_history(store, quiet);
    let Some(row) = store.selected.get_untracked() else { return };
    let Some(namespace) = row.namespace.clone() else { return };
    if store.pod_loading.get_untracked() {
        return;
    }
    if !quiet {
        store.pod.set(None);
    }
    store.pod_loading.set(true);
    spawn_local(async move {
        let result = invoke::<_, PodDetails>("pod_details", &Args { namespace, name: row.name.clone() }).await;
        if store.selected.with_untracked(|s| s.as_ref().is_some_and(|s| s.id == row.id)) {
            // A failed background refresh keeps the last good render.
            if !(quiet && result.is_err() && store.pod.with_untracked(|p| matches!(p, Some(Ok(_))))) {
                store.pod.set(Some(result));
            }
        }
        store.pod_loading.set(false);
    });
}

/// Re-fetches the selected pod's last-hour usage. Background refreshes skip
/// while one is in flight, so the cadence stays one request per tick.
fn load_history(store: Store, quiet: bool) {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        name: String,
    }
    let Some(row) = store.selected.get_untracked() else { return };
    let Some(namespace) = row.namespace.clone() else { return };
    if quiet && store.history_loading.get_untracked() {
        return;
    }
    store.history_loading.set(true);
    spawn_local(async move {
        let result = invoke::<_, PodHistory>("pod_history", &Args { namespace, name: row.name.clone() }).await;
        if store.selected.with_untracked(|s| s.as_ref().is_some_and(|s| s.id == row.id)) {
            // A failed refresh keeps the last good chart.
            if result.is_ok() || !store.pod_history.with_untracked(|h| matches!(h, Some(Ok(_)))) {
                store.pod_history.set(Some(result));
            }
        }
        store.history_loading.set(false);
    });
}

#[component]
pub fn PodDetailView(store: Store) -> impl IntoView {
    view! {
        <div class="fade" class:stale=move || store.pod_loading.get() && store.pod.with(Option::is_some)>
            {move || match store.pod.get() {
                None => view! { <p class="muted pad">"Loading…"</p> }.into_any(),
                Some(Err(e)) => view! { <p class="warn">{e}</p> }.into_any(),
                Some(Ok(d)) => view! { <PodSections store=store d=d /> }.into_any(),
            }}
        </div>
    }
}

#[component]
fn PodSections(store: Store, d: PodDetails) -> impl IntoView {
    let namespace = d.namespace.clone();
    let containers = d
        .containers
        .clone()
        .into_iter()
        .map(|c| {
            let (ns, pod) = (namespace.clone(), d.name.clone());
            view! { <ContainerSection store=store c=c pod=pod namespace=ns /> }
        })
        .collect_view();
    let events = d.events.clone();
    let event_count = events.len();
    let warnings = events.iter().filter(|e| e.type_ == "Warning").count();
    let events_summary = match (event_count, warnings) {
        (0, _) => "none".to_owned(),
        (n, 0) => format!("{n}"),
        (n, w) => format!("{n} · ⚠ {w} warning{}", if w == 1 { "" } else { "s" }),
    };
    let cpu = d.cpu;
    let memory = d.memory;
    let info = d.clone();

    view! {
        <Section store=store id="resources" title="Resources".to_owned() summary=String::new()>
            <UsageChart store=store resource=Resource::Cpu usage=cpu container=None />
            <UsageChart store=store resource=Resource::Memory usage=memory container=None />
            <HistoryNote store=store />
        </Section>
        <Section store=store id="pod" title="Pod".to_owned() summary=String::new()>
            <PodInfo store=store d=info.clone() />
        </Section>
        {containers}
        <Section store=store id="events" title="Events".to_owned() summary=events_summary>
            <EventList store=store events=events.clone() />
        </Section>
    }
}

/// A collapsible section. `id` is remembered so the same section stays
/// collapsed when you select another pod.
#[component]
fn Section(
    store: Store,
    #[prop(into)] id: String,
    title: String,
    summary: String,
    #[prop(optional)] icon: Option<AnyView>,
    #[prop(optional)] nested: bool,
    /// Buttons beside the title (outside it, so they don't toggle the section).
    #[prop(optional)]
    actions: Option<AnyView>,
    children: ChildrenFn,
) -> impl IntoView {
    let open = {
        let id = id.clone();
        move || !store.detail_collapsed.with(|c| c.contains(&id))
    };
    let toggle = move |_| {
        store.detail_collapsed.update(|c| {
            if !c.remove(&id) {
                c.insert(id.clone());
            }
        })
    };
    let expanded = {
        let open = open.clone();
        move || open().to_string()
    };
    let chevron_open = Signal::derive(open.clone());
    view! {
        <section class="dsection" class:nested=nested>
            <div class="dsection-head">
            <button class="dsection-title" on:click=toggle aria-expanded=expanded>
                <crate::icons::Chevron open=chevron_open />
                {icon}
                <span class="dsection-name">{title}</span>
                <span class="dsection-summary">{summary}</span>
            </button>
                {actions}
            </div>
            <Show when=open>
                <div class="dsection-body">{children()}</div>
            </Show>
        </section>
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Resource {
    Cpu,
    Memory,
}

impl Resource {
    fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::Memory => "Memory",
        }
    }

    fn fmt(self) -> fn(f64) -> String {
        match self {
            Self::Cpu => fmt_cpu,
            Self::Memory => fmt_bytes,
        }
    }

    fn series(self, h: &UsageHistory) -> &[(f64, f64)] {
        match self {
            Self::Cpu => &h.cpu,
            Self::Memory => &h.memory,
        }
    }
}

/// Last-hour line chart for the pod (`container: None`) or one container,
/// with request and limit lines. Without a limit, the node is the ceiling.
#[component]
fn UsageChart(store: Store, resource: Resource, usage: Usage, container: Option<String>) -> impl IntoView {
    let Usage { requests, limits, used, node } = usage;
    let fmt = resource.fmt();
    let points = Memo::new(move |_| {
        store.pod_history.with(|h| match h {
            None => None,
            Some(Err(_)) => Some(vec![]),
            Some(Ok(h)) => Some(match &container {
                None => resource.series(&h.pod).to_vec(),
                Some(name) => h
                    .containers
                    .iter()
                    .find(|c| &c.name == name)
                    .map(|c| resource.series(&c.usage).to_vec())
                    .unwrap_or_default(),
            }),
        })
    });
    let data = Signal::derive(move || {
        let points = points.get()?;
        let (start, end, step) = store.pod_history.with(|h| match h {
            Some(Ok(h)) => (h.start, h.end, h.step),
            _ => (0.0, 1.0, 1.0),
        });
        let refs = [("request", requests, true), ("limit", limits, false)]
            .into_iter()
            .filter_map(|(name, value, dashed)| Some(RefLine { name, value: value?, dashed }))
            .collect();
        Some(ChartData { start, end, step, points, refs })
    });
    let header = move || {
        let now = points.with(|p| p.as_ref().and_then(|p| p.last()).map(|p| p.1)).or(used);
        let ceiling = limits.or(node);
        let share = now.zip(ceiling).filter(|(_, c)| *c > 0.0).map(|(u, c)| u / c);
        let icon = if share.is_some_and(|s| s >= 0.75) { "⚠ " } else { "" };
        let of = if limits.is_some() { "limit" } else { "node" };
        let share_text = match share {
            Some(s) if s > 0.0 && s < 0.01 => format!(" · <1% of {of}"),
            Some(s) => format!(" · {:.0}% of {of}", s * 100.0),
            None => String::new(),
        };
        let limit_text = match (limits, node) {
            (Some(l), _) => fmt(l),
            (None, Some(n)) => format!("unlimited (node {})", fmt(n)),
            (None, None) => "unlimited".to_owned(),
        };
        format!(
            "{icon}now {}{share_text} · req {} · limit {limit_text}",
            now.map_or_else(|| "—".to_owned(), fmt),
            requests.map_or_else(|| "not set".to_owned(), fmt),
        )
    };
    let scale = match resource {
        Resource::Cpu => Scale::Decimal,
        Resource::Memory => Scale::Binary,
    };
    view! {
        <div class="usage-chart">
            <div class="meter-row">
                <span class="meter-label">{resource.label()}</span>
                <span class="meter-value">{header}</span>
            </div>
            <LineChart label=resource.label() data=data fmt=fmt scale=scale />
        </div>
    }
}

/// Where the charts' data comes from, and how far back it goes.
#[component]
fn HistoryNote(store: Store) -> impl IntoView {
    move || {
        store.pod_history.with(|h| match h {
            None => view! { <p class="legend small"></p> }.into_any(),
            Some(Err(e)) => view! { <p class="legend small warn-text">{format!("Can't load usage history: {e}")}</p> }.into_any(),
            Some(Ok(h)) => {
                let text = match h.source {
                    HistorySource::Prometheus => "Last hour from Prometheus · updates every 15 s".to_owned(),
                    HistorySource::Local => match h.pod.memory.first().or(h.pod.cpu.first()) {
                        Some(&(t, _)) => format!(
                            "No Prometheus data for this pod, so the app records usage itself: since {} · updates every 15 s",
                            clock(t)
                        ),
                        None => "No usage source: needs Prometheus, metrics-server or kubelet stats.".to_owned(),
                    },
                };
                view! { <p class="legend small">{text}</p> }.into_any()
            }
        })
    }
}

#[component]
fn PodInfo(store: Store, d: PodDetails) -> impl IntoView {
    let created = d.created.clone().unwrap_or_default();
    let created_text = {
        let created = created.clone();
        move || format!("{} ago · {}", format_age(store.now.get(), &created), created)
    };
    let row = |k: &'static str, v: Option<String>| {
        v.filter(|v| !v.is_empty()).map(|v| view! { <dt>{k}</dt><dd>{v}</dd> })
    };
    let badge_slug = d
        .status
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let labels = d
        .labels
        .iter()
        .map(|(k, v)| view! { <span class="chip">{format!("{k}={v}")}</span> })
        .collect_view();
    view! {
        <dl class="kv">
            <dt>"Status"</dt>
            <dd><span class=format!("badge badge-{badge_slug}")>{d.status.clone()}</span></dd>
            {row("Namespace", Some(d.namespace.clone()))}
            {row("Node", d.node.clone())}
            {row("Pod IP", d.pod_ip.clone())}
            {row("Host IP", d.host_ip.clone())}
            {row("QoS class", d.qos_class.clone())}
            {row("Service account", d.service_account.clone())}
            {row("Controlled by", d.owner.clone())}
            {row("Restart policy", d.restart_policy.clone())}
            {row("Priority class", d.priority_class.clone())}
            {(!created.is_empty()).then(|| view! { <dt>"Created"</dt><dd>{created_text}</dd> })}
            {(!d.labels.is_empty()).then(|| view! { <dt>"Labels"</dt><dd class="chips">{labels}</dd> })}
        </dl>
    }
}

#[component]
fn ContainerSection(store: Store, c: ContainerDetails, pod: String, namespace: String) -> impl IntoView {
    let kind = match (c.init, c.sidecar) {
        (true, true) => "Sidecar · ",
        (true, false) => "Init · ",
        _ => "",
    };
    let summary = match c.restarts {
        0 => format!("{kind}{}", c.detail),
        1 => format!("{kind}{} · 1 restart", c.detail),
        n => format!("{kind}{} · {n} restarts", c.detail),
    };
    let id = format!("container:{}", c.name);
    let env_id = format!("env:{}", c.name);
    let env_count = c.env.iter().filter(|e| !e.name.is_empty()).count();
    let icon = view! { <ContainerIcon c=subduer_types::containers::Container {
        name: c.name.clone(), state: c.state, detail: c.detail.clone(), init: c.init,
    } /> }
    .into_any();
    let started = c.started.clone();
    let info = c.clone();
    let logs_target = LogTarget {
        namespace: namespace.clone(),
        source: LogSource::Container { pod: pod.clone(), container: c.name.clone() },
    };
    // A shell needs a running container.
    let running = c.state == subduer_types::containers::State::Running;
    let exec_target = ExecTarget { namespace: namespace.clone(), pod: pod.clone(), container: c.name.clone() };
    let has_ports = !c.ports.is_empty();
    let (upload_ns, upload_pod, upload_container) = (namespace.clone(), pod.clone(), c.name.clone());
    let (download_ns, download_pod, download_container) = (namespace.clone(), pod.clone(), c.name.clone());
    let forward_ns = namespace.clone();
    let forward_source = subduer_types::ForwardSource::Pod { pod: pod.clone() };
    let actions = view! {
        <div class="dsection-actions">
            {(running && has_ports).then(|| view! {
                <button
                    class="btn small"
                    title="Forward a local port to this pod"
                    on:click=move |_| crate::forward::request(store, forward_ns.clone(), forward_source.clone())
                >"Forward"</button>
            })}
            {running.then(|| view! {
                <button
                    class="btn small"
                    title=format!("Upload a file into {} (needs changes allowed)", c.name)
                    on:click=move |_| crate::upload::request(
                        store, upload_ns.clone(), upload_pod.clone(), upload_container.clone(),
                    )
                >"Upload"</button>
            })}
            {running.then(|| view! {
                <button
                    class="btn small"
                    title=format!("Download a file from {}", c.name)
                    on:click=move |_| crate::download::request(
                        store, download_ns.clone(), download_pod.clone(), download_container.clone(),
                    )
                >"Download"</button>
            })}
            {running.then(|| view! {
                <button
                    class="btn small"
                    title=format!("Open a shell in {} in a new window (needs changes allowed)", c.name)
                    on:click=move |_| crate::exec::request(store, exec_target.clone())
                >"Shell"</button>
            })}
            <button
                class="btn small"
                title=format!("Open {} logs in a new window", c.name)
                on:click=move |_| crate::logs::open(logs_target.clone(), move |e| store.error.set(Some(e)))
            >"Logs"</button>
        </div>
    }
    .into_any();
    view! {
        <Section store=store id=id title=c.name.clone() summary=summary icon=icon actions=actions>
            <UsageChart store=store resource=Resource::Cpu usage=info.cpu container=Some(info.name.clone()) />
            <UsageChart store=store resource=Resource::Memory usage=info.memory container=Some(info.name.clone()) />
            <dl class="kv">
                <dt>"Image"</dt><dd class="mono">{info.image.clone()}</dd>
                <dt>"State"</dt><dd>{info.detail.clone()}{(!info.init && !info.ready && info.state == subduer_types::containers::State::Running).then_some(" (not ready)")}</dd>
                {started.clone().map(|s| {
                    let at = s.clone();
                    view! { <dt>"Started"</dt><dd>{move || format!("{} ago · {}", format_age(store.now.get(), &at), s)}</dd> }
                })}
                {info.last_termination.clone().map(|t| view! { <dt>"Last termination"</dt><dd>{t}</dd> })}
                {(!info.ports.is_empty()).then(|| view! { <dt>"Ports"</dt><dd>{info.ports.join(", ")}</dd> })}
            </dl>
            {
                let (env, ns) = (info.env.clone(), namespace.clone());
                view! {
                    <Section
                        store=store
                        id=env_id.clone()
                        title="Environment".to_owned()
                        summary=if env_count == 0 { "none".to_owned() } else { env_count.to_string() }
                        nested=true
                    >
                        <EnvTable store=store vars=env.clone() namespace=ns.clone() />
                    </Section>
                }
            }
        </Section>
    }
}

#[component]
fn EnvTable(store: Store, vars: Vec<EnvVar>, namespace: String) -> impl IntoView {
    if vars.is_empty() {
        return view! { <p class="muted">"No environment variables."</p> }.into_any();
    }
    let rows = vars
        .into_iter()
        .map(|v| {
            let from = v.from.clone().map(|f| view! { <span class="env-from">{f}</span> });
            let value = env_value(store, &v.value, &namespace);
            view! {
                <tr>
                    <td class="env-name">{v.name.clone()}{from}</td>
                    <td class="env-value">{value}</td>
                </tr>
            }
        })
        .collect_view();
    view! { <table class="env"><tbody>{rows}</tbody></table> }.into_any()
}

fn env_value(store: Store, value: &EnvValue, namespace: &str) -> AnyView {
    match value.clone() {
        EnvValue::Literal(v) if v.is_empty() => view! { <span class="muted">"(empty)"</span> }.into_any(),
        EnvValue::Literal(v) => view! { <span class="mono">{v}</span> }.into_any(),
        EnvValue::ConfigMap { name, key, value } => {
            let source = format!("ConfigMap {name} · {key}");
            match value {
                Some(v) => view! { <span class="mono">{v}</span><span class="env-ref">{source}</span> }.into_any(),
                None => view! { <span class="warn-text">"not found"</span><span class="env-ref">{source}</span> }.into_any(),
            }
        }
        EnvValue::Secret { name, key } => view! {
            <SecretValue store=store namespace=namespace.to_owned() secret=name key=key />
        }
        .into_any(),
        EnvValue::Field(path) => view! { <span class="env-ref">{format!("field {path}")}</span> }.into_any(),
        EnvValue::Resource(r) => view! { <span class="env-ref">{format!("resource {r}")}</span> }.into_any(),
        EnvValue::Unreadable { source, error } => {
            view! { <span class="warn-text">{format!("Can't read {source}: {error}")}</span> }.into_any()
        }
    }
}

/// Masked until "Show" is clicked; then fetched on demand and shown until "Hide".
#[component]
fn SecretValue(store: Store, namespace: String, secret: String, key: String) -> impl IntoView {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        secret: String,
        key: String,
    }
    let cache_key = (secret.clone(), key.clone());
    let source = format!("Secret {secret} · {key}");
    let revealed = {
        let k = cache_key.clone();
        move || store.revealed.with(|r| r.get(&k).cloned())
    };
    let loading = RwSignal::new(false);
    let show = {
        let k = cache_key.clone();
        move |_| {
            let args = Args {
                namespace: namespace.clone(),
                secret: secret.clone(),
                key: key.clone(),
            };
            let k = k.clone();
            loading.set(true);
            spawn_local(async move {
                let result = invoke::<_, String>("secret_value", &args).await;
                store.revealed.update(|r| {
                    r.insert(k, result);
                });
                loading.set(false);
            });
        }
    };
    let hide = {
        let k = cache_key.clone();
        move |_| {
            store.revealed.update(|r| {
                r.remove(&k);
            });
        }
    };
    view! {
        {move || match revealed() {
            None => view! {
                <span class="secret-mask" aria-label="hidden secret value">"••••••••"</span>
                <button class="btn small" prop:disabled=move || loading.get() on:click=show.clone()>
                    {move || if loading.get() { "…" } else { "Show" }}
                </button>
            }
            .into_any(),
            Some(Ok(v)) => view! {
                <span class="mono secret-value">{v}</span>
                <button class="btn small" on:click=hide.clone()>"Hide"</button>
            }
            .into_any(),
            Some(Err(e)) => view! {
                <span class="warn-text">{e}</span>
                <button class="btn small" on:click=hide.clone()>"Dismiss"</button>
            }
            .into_any(),
        }}
        <span class="env-ref">{source}</span>
    }
}

#[component]
fn EventList(store: Store, events: Vec<EventSummary>) -> impl IntoView {
    if events.is_empty() {
        return view! { <p class="muted">"No events for this pod. Events expire after about an hour."</p> }.into_any();
    }
    events
        .into_iter()
        .map(|e| {
            let badge = if e.type_ == "Warning" { "badge badge-warning" } else { "badge" };
            let last = e.last_seen.clone();
            let when = move || {
                let age = format_age(store.now.get(), &last);
                if e.count > 1 { format!("×{} · last {age} ago", e.count) } else { format!("{age} ago") }
            };
            view! {
                <div class="event">
                    <div class="event-head">
                        <span class=badge>{e.type_.clone()}</span>
                        <strong>{e.reason.clone()}</strong>
                        <span class="muted">{e.source.clone()}</span>
                        <span class="event-when muted">{when}</span>
                    </div>
                    <div class="event-message">{e.message.clone()}</div>
                </div>
            }
        })
        .collect_view()
        .into_any()
}
