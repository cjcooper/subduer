//! Traces: the top-bar button and indicator, the start dialog (what to
//! trace, then Save as), the running trace's panel with Stop, the "trace
//! saved" summary, and the prompts that stop a trace before switching
//! context or quitting.

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use subduer_types::{KindRef, ResourceRow, TraceEnd, TraceScope, TraceStatus, TraceSummary, TraceTargets, TraceWorkload};

use crate::app::Store;
use crate::modal::Modal;
use crate::tauri::{NoArgs, invoke, listen};

/// Backend events: the menu's Trace… item, and a trace starting or ending.
const OPEN_TRACE: &str = "open-trace";
const TRACE_CHANGED: &str = "trace-changed";
/// How often the running trace's status is re-read.
pub(crate) const STATUS_EVERY: std::time::Duration = std::time::Duration::from_secs(15);

/// What the start dialog will trace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Level {
    Cluster,
    Namespaces,
    Workloads,
    Pod,
}

/// The start dialog's choices.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Draft {
    pub(crate) level: Level,
    /// Chosen for the Namespaces level.
    pub(crate) namespaces: Vec<String>,
    /// The namespace whose workloads or pods are listed.
    pub(crate) namespace: String,
    /// Chosen for the Workloads level, from any namespace.
    pub(crate) workloads: Vec<TraceWorkload>,
    pub(crate) pod: String,
    pub(crate) filter: String,
}

impl Draft {
    /// The whole cluster, or the namespace being viewed.
    pub(crate) fn for_view(namespace: Option<String>) -> Self {
        let level = if namespace.is_some() { Level::Namespaces } else { Level::Cluster };
        Self {
            level,
            namespaces: namespace.iter().cloned().collect(),
            namespace: namespace.unwrap_or_default(),
            workloads: vec![],
            pod: String::new(),
            filter: String::new(),
        }
    }

    /// "Trace this…" from a Details tab: the object itself. `None` for kinds
    /// that can't be traced.
    pub(crate) fn for_object(kind: &KindRef, namespace: Option<String>, name: &str) -> Option<Self> {
        let mut d = Self::for_view(namespace.clone());
        match (kind.key().as_str(), namespace) {
            ("core/Namespace", _) => {
                d.level = Level::Namespaces;
                d.namespaces = vec![name.to_owned()];
                d.namespace = name.to_owned();
            }
            ("core/Pod", Some(_)) => {
                d.level = Level::Pod;
                d.pod = name.to_owned();
            }
            (_, Some(ns)) if traceable_workload(kind) => {
                d.level = Level::Workloads;
                d.workloads = vec![TraceWorkload { kind: kind.clone(), namespace: ns, name: name.to_owned() }];
            }
            _ => return None,
        }
        Some(d)
    }

    pub(crate) fn scope(&self) -> Option<TraceScope> {
        match self.level {
            Level::Cluster => Some(TraceScope::Cluster),
            Level::Namespaces => (!self.namespaces.is_empty()).then(|| TraceScope::Namespaces(self.namespaces.clone())),
            Level::Workloads => (!self.workloads.is_empty()).then(|| TraceScope::Workloads(self.workloads.clone())),
            Level::Pod => (!self.namespace.is_empty() && !self.pod.is_empty())
                .then(|| TraceScope::Pod { namespace: self.namespace.clone(), name: self.pod.clone() }),
        }
    }

    fn toggle_namespace(&mut self, ns: &str) {
        if let Some(i) = self.namespaces.iter().position(|n| n == ns) {
            self.namespaces.remove(i);
        } else {
            self.namespaces.push(ns.to_owned());
            self.namespaces.sort();
        }
    }

    fn toggle_workload(&mut self, w: &TraceWorkload) {
        if let Some(i) = self.workloads.iter().position(|x| x == w) {
            self.workloads.remove(i);
        } else {
            self.workloads.push(w.clone());
        }
    }
}

pub(crate) fn traceable_workload(kind: &KindRef) -> bool {
    matches!(kind.key().as_str(), "apps/Deployment" | "apps/StatefulSet" | "apps/DaemonSet" | "batch/Job" | "batch/CronJob")
}

/// What a scope covers, in a few words.
pub(crate) fn scope_text(scope: &TraceScope) -> String {
    match scope {
        TraceScope::Cluster => "the whole cluster".into(),
        TraceScope::Namespaces(ns) if ns.len() == 1 => format!("namespace {}", ns[0]),
        TraceScope::Namespaces(ns) => format!("{} namespaces", ns.len()),
        TraceScope::Workloads(ws) if ws.len() == 1 => format!("{} {}", ws[0].kind.kind, ws[0].name),
        TraceScope::Workloads(ws) => format!("{} workloads", ws.len()),
        TraceScope::Pod { name, .. } => format!("pod {name}"),
    }
}

/// A file name for the Save dialog: context, what's traced and when, e.g.
/// `acme-uat-shop-2026-10-11-1405.trace`.
pub(crate) fn file_name(context: &str, scope: &TraceScope, stamp: &str) -> String {
    let what = match scope {
        TraceScope::Cluster => "cluster".to_owned(),
        TraceScope::Namespaces(ns) => more(&ns[..], |n| n.clone()),
        TraceScope::Workloads(ws) => more(&ws[..], |w| w.name.clone()),
        TraceScope::Pod { name, .. } => name.clone(),
    };
    let name: String = format!("{context}-{what}-{stamp}")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+') { c } else { '-' })
        .collect();
    format!("{name}.trace")
}

/// The first item's name, and "+N" for the rest.
fn more<T>(items: &[T], name: impl Fn(&T) -> String) -> String {
    match items {
        [] => String::new(),
        [one] => name(one),
        [first, rest @ ..] => format!("{}+{}", name(first), rest.len()),
    }
}

/// `2026-10-11-1405` in local time.
fn stamp_now() -> String {
    let d = js_sys::Date::new_0();
    format!(
        "{:04}-{:02}-{:02}-{:02}{:02}",
        d.get_full_year(),
        d.get_month() + 1,
        d.get_date(),
        d.get_hours(),
        d.get_minutes()
    )
}

/// A span like `2d 4h`, `3h 12m` or `5m`.
pub(crate) fn duration_text(secs: f64) -> String {
    let mins = (secs.max(0.0) / 60.0).floor() as u64;
    let (d, h, m) = (mins / 1440, mins / 60 % 24, mins % 60);
    match (d, h) {
        (0, 0) => format!("{m}m"),
        (0, _) => format!("{h}h {m}m"),
        _ => format!("{d}d {h}h"),
    }
}

fn ago_text(secs: f64) -> String {
    if secs < 60.0 { "just now".into() } else { format!("{} ago", duration_text(secs)) }
}

/// Shown in the top bar while a trace runs.
pub(crate) fn indicator_text(status: &TraceStatus, now_secs: f64) -> String {
    let base = format!("Trace · {}", duration_text(now_secs - status.started));
    if status.stopping { format!("{base} · stopping…") } else { base }
}

/// Something about the running trace needs attention.
pub(crate) fn needs_attention(status: &TraceStatus) -> bool {
    status.last_error.is_some() || status.gaps > 0
}

/// Listens for the backend's trace events and loads the current state.
pub(crate) async fn listen_events(store: Store) {
    listen::<()>(OPEN_TRACE, move |()| open(store, None)).await;
    listen::<Option<TraceStatus>>(TRACE_CHANGED, move |status| store.trace.set(status)).await;
    refresh(store);
}

/// Re-reads the running trace's status (polled while one runs).
pub(crate) fn refresh(store: Store) {
    spawn_local(async move {
        if let Ok(status) = invoke::<_, Option<TraceStatus>>("trace_status", &NoArgs {}).await {
            store.trace.set(status);
        }
    });
}

/// Opens the trace dialog: the running trace, or the start form (with
/// `draft`, or the current view's namespace).
pub(crate) fn open(store: Store, draft: Option<Draft>) {
    if store.trace.with_untracked(Option::is_none) {
        if store.cluster.with_untracked(Option::is_none) {
            store.error.set(Some("Connect to a cluster to start a trace.".into()));
            return;
        }
        store.trace_draft.set(Some(draft.unwrap_or_else(|| Draft::for_view(store.namespace.get_untracked()))));
    }
    store.trace_open.set(true);
}

/// Runs `then` now, or, if a trace of another context is running, once the
/// user agreed to stop it (it's saved first).
pub(crate) fn before_switch(store: Store, context: String, then: impl FnOnce() + 'static) {
    let other = store.trace.with_untracked(|t| t.as_ref().is_some_and(|t| t.context != context));
    if other {
        store.trace_then.set_value(Some(Box::new(then)));
        store.trace_switch.set(Some(context));
    } else {
        then();
    }
}

/// The window's close or quit was held: ask about the running trace (and
/// unsaved edits), or just the edits.
pub(crate) fn on_close_held(store: Store, quit: bool, otherwise: impl FnOnce() + 'static) {
    if store.trace.with_untracked(Option::is_some) {
        store.trace_quit.set(Some(quit));
    } else {
        otherwise();
    }
}

async fn stop(store: Store, reason: TraceEnd) {
    #[derive(Serialize)]
    struct Args {
        reason: TraceEnd,
    }
    let result = invoke::<_, TraceSummary>("stop_trace", &Args { reason }).await;
    if let Ok(s) = &result {
        let recent = crate::prefs::RecentTrace { path: s.path.clone(), context: s.context.clone(), what: scope_text(&s.scope), at: js_sys::Date::now() };
        store.recent_traces.set(crate::prefs::remember_trace(recent));
    }
    store.trace.set(None);
    store.trace_saved.set(Some(result));
}

/// The top-bar button: Trace… when idle, the running trace's indicator otherwise.
#[component]
pub(crate) fn TraceButton(store: Store) -> impl IntoView {
    move || match store.trace.get() {
        Some(status) => {
            let text = move || store.trace.with(|t| t.as_ref().map(|t| indicator_text(t, store.now.get() / 1000.0)));
            let warn = needs_attention(&status);
            let title = format!("Tracing {} of {} into {}. Click for details.", scope_text(&status.scope), status.context, status.path);
            view! {
                <button class="trace-indicator" class:attention=warn title=title on:click=move |_| open(store, None)>
                    <span class="rec-dot" aria-hidden="true"></span>
                    {text}
                    {warn.then(|| view! {
                        <svg class="trace-warn" width="12" height="12" viewBox="0 0 12 12" role="img" aria-label="needs attention">
                            <path d="M6 1 L11.5 11 H0.5 Z" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round" />
                            <path d="M6 4.6 V7.4 M6 9.1 V9.2" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
                        </svg>
                    })}
                </button>
            }
            .into_any()
        }
        None => view! {
            <button
                class="btn trace-btn"
                prop:disabled=move || store.cluster.with(Option::is_none)
                title="Record CPU and memory beyond Prometheus' retention, for right-sizing"
                on:click=move |_| open(store, None)
            >"Trace…"</button>
        }
        .into_any(),
    }
}

/// "Trace…" in a Details panel header, for pods, namespaces and workloads.
#[component]
pub(crate) fn TraceThisButton(store: Store, row: ResourceRow) -> impl IntoView {
    move || {
        let kind = store.kind.get()?;
        let draft = Draft::for_object(&kind, row.namespace.clone(), &row.name)?;
        let title = format!("Record this {}'s CPU and memory over time", kind.kind);
        Some(view! {
            <button class="btn small" title=title on:click=move |_| open(store, Some(draft.clone()))>"Trace…"</button>
        })
    }
}

#[component]
pub(crate) fn TraceDialog(store: Store) -> impl IntoView {
    let close = move || {
        store.trace_open.set(false);
        store.trace_draft.set(None);
    };
    move || {
        if !store.trace_open.get() {
            return None;
        }
        Some(if store.trace.with(Option::is_some) {
            view! { <RunningPanel store=store close=Callback::new(move |()| close()) /> }.into_any()
        } else {
            view! { <StartForm store=store close=Callback::new(move |()| close()) /> }.into_any()
        })
    }
}

#[component]
fn RunningPanel(store: Store, close: Callback<()>) -> impl IntoView {
    let stopping = store.trace_busy;
    let do_stop = move |_| {
        stopping.set(true);
        spawn_local(async move {
            stop(store, TraceEnd::Stopped).await;
            stopping.set(false);
            close.run(());
        });
    };
    let body = move || {
        let s = store.trace.get()?;
        let now = store.now.get() / 1000.0;
        let pull = match (&s.last_error, s.last_pull, s.prometheus) {
            (_, _, false) => "No Prometheus found: only the app's own samples are kept, while it runs.".to_owned(),
            (Some(e), _, _) => format!("The last pull failed: {e}. The next one tries again."),
            (None, Some(t), _) => format!("Last pulled from Prometheus {}.", ago_text(now - t)),
            (None, None, _) => "Not pulled from Prometheus yet.".to_owned(),
        };
        let gaps = match s.gaps {
            0 => None,
            1 => Some("1 gap: Prometheus no longer had some data when it was pulled.".to_owned()),
            n => Some(format!("{n} gaps: Prometheus no longer had some data when it was pulled.")),
        };
        Some(view! {
            <dl class="trace-facts">
                <dt>"Context"</dt><dd>{s.context.clone()}</dd>
                <dt>"Tracing"</dt><dd>{scope_text(&s.scope)}</dd>
                <dt>"Running for"</dt><dd>{duration_text(now - s.started)}</dd>
                <dt>"Data covers"</dt><dd>{duration_text(s.data_end - s.data_start)}</dd>
                {(s.prometheus && s.pull_every > 0.0).then(|| view! {
                    <dt>"Pulls"</dt><dd>{format!("Every {} min", (s.pull_every / 60.0).round())}</dd>
                })}
                <dt>"File"</dt><dd class="mono path">{s.path.clone()}</dd>
            </dl>
            <p class:warn-text=s.last_error.is_some()>{pull}</p>
            {(s.sampled_pods > 0).then(|| view! {
                <p class="muted">{match s.sampled_pods {
                    1 => "1 running pod isn't in Prometheus; the app samples it every 30 s.".to_owned(),
                    n => format!("{n} running pods aren't in Prometheus; the app samples them every 30 s."),
                }}</p>
            })}
            {gaps.map(|g| view! { <p class="warn-text">{g}</p> })}
            <p class="muted small">"Closing the app stops the trace. The file is saved either way."</p>
        })
    };
    view! {
        <Modal on_cancel=close busy=stopping wide=true dismiss_outside=true class="trace-modal">
            <div class="modal-head">
                <h2><span class="rec-dot" aria-hidden="true"></span>"Trace running"</h2>
                <button class="close" title="Close (Esc)" aria-label="Close" prop:disabled=move || stopping.get() on:click=move |_| close.run(())>"×"</button>
            </div>
            {body}
            <div class="modal-actions">
                <button
                    class="danger"
                    prop:disabled=move || stopping.get() || store.trace.with(|t| t.as_ref().is_some_and(|t| t.stopping))
                    on:click=do_stop
                >{move || if stopping.get() { "Stopping…" } else { "Stop trace" }}</button>
            </div>
        </Modal>
    }
}

#[component]
fn StartForm(store: Store, close: Callback<()>) -> impl IntoView {
    let busy = store.trace_busy;
    let error = RwSignal::new(None::<String>);
    // Seconds between pulls, as last chosen.
    let every = RwSignal::new(crate::prefs::load_pull_every());
    // Workloads and pods of the listed namespace.
    let targets = RwSignal::new(None::<(String, Result<TraceTargets, String>)>);
    let draft = move || store.trace_draft.get();
    let edit = move |f: &dyn Fn(&mut Draft)| store.trace_draft.update(|d| {
        if let Some(d) = d {
            f(d);
        }
    });
    // Load the listed namespace's targets when it changes.
    Effect::new(move |_| {
        let Some(d) = store.trace_draft.get() else { return };
        let ns = d.namespace.clone();
        if !matches!(d.level, Level::Workloads | Level::Pod) || ns.is_empty() {
            return;
        }
        if targets.with_untracked(|t| t.as_ref().is_some_and(|(n, _)| *n == ns)) {
            return;
        }
        targets.set(None);
        spawn_local(async move {
            #[derive(Serialize)]
            struct Args {
                namespace: String,
            }
            let result = invoke::<_, TraceTargets>("trace_targets", &Args { namespace: ns.clone() }).await;
            if store.trace_draft.with_untracked(|d| d.as_ref().is_some_and(|d| d.namespace == ns)) {
                targets.set(Some((ns, result)));
            }
        });
    });
    let namespaces = move || store.cluster.with(|c| c.as_ref().map(|c| c.namespaces.clone()).unwrap_or_default());
    let context = move || store.context.get().unwrap_or_default();

    let start = move |_| {
        let Some(scope) = draft().and_then(|d| d.scope()) else { return };
        let context = context();
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            #[derive(Serialize)]
            struct Choose {
                suggested: String,
            }
            #[derive(Serialize)]
            struct Start {
                context: String,
                scope: TraceScope,
                path: String,
                pull_every: i64,
            }
            let pull_every = every.get_untracked();
            let suggested = file_name(&context, &scope, &stamp_now());
            let path = match invoke::<_, Option<String>>("choose_trace_file", &Choose { suggested }).await {
                Ok(Some(path)) => path,
                Ok(None) => {
                    busy.set(false);
                    return;
                }
                Err(e) => {
                    error.set(Some(e));
                    busy.set(false);
                    return;
                }
            };
            match invoke::<_, TraceStatus>("start_trace", &Start { context, scope, path, pull_every }).await {
                Ok(status) => {
                    store.trace.set(Some(status));
                    busy.set(false);
                    close.run(());
                }
                Err(e) => {
                    error.set(Some(e));
                    busy.set(false);
                }
            }
        });
    };

    let level_choice = move |level: Level, label: &'static str, hint: &'static str| {
        let on = move || draft().is_some_and(|d| d.level == level);
        view! {
            <label class="trace-level" class:on=on>
                <input
                    type="radio"
                    name="trace-level"
                    prop:checked=on
                    on:change=move |_| edit(&|d| {
                        d.level = level;
                        d.filter.clear();
                    })
                />
                <span class="trace-level-name">{label}</span>
                <span class="muted small">{hint}</span>
            </label>
        }
    };

    let namespace_picker = move || {
        let current = draft().map(|d| d.namespace).unwrap_or_default();
        view! {
            <label class="field trace-ns">
                "Namespace"
                <select on:change=move |ev| {
                    let ns = event_target_value(&ev);
                    edit(&|d| {
                        d.namespace = ns.clone();
                        d.pod.clear();
                        d.filter.clear();
                    });
                }>
                    <option value="" selected=current.is_empty() disabled=true>"Choose…"</option>
                    {namespaces().into_iter().map(|ns| {
                        let selected = ns == current;
                        view! { <option value=ns.clone() selected=selected>{ns.clone()}</option> }
                    }).collect_view()}
                </select>
            </label>
        }
    };

    let filter_box = move || {
        view! {
            <input
                class="trace-filter"
                type="search"
                placeholder="Filter…"
                prop:value=move || draft().map(|d| d.filter).unwrap_or_default()
                on:input=move |ev| {
                    let text = event_target_value(&ev);
                    edit(&|d| d.filter = text.clone());
                }
            />
        }
    };

    let targets_view = move || {
        let d = draft()?;
        match d.level {
            Level::Cluster => Some(view! {
                <p class="muted">"Every pod in "<strong>{context()}</strong>". Takes the most space: about 55 MB a day per 1,000 containers."</p>
            }.into_any()),
            Level::Namespaces => {
                let filter = d.filter.to_lowercase();
                let list = namespaces().into_iter().filter(|ns| ns.to_lowercase().contains(&filter)).map(|ns| {
                    let checked = d.namespaces.contains(&ns);
                    let name = ns.clone();
                    view! {
                        <label class="trace-option">
                            <input type="checkbox" prop:checked=checked on:change=move |_| edit(&|d| d.toggle_namespace(&name)) />
                            <span class="mono">{ns}</span>
                        </label>
                    }
                }).collect_view();
                Some(view! {
                    {filter_box()}
                    <div class="trace-options">{list}</div>
                    <p class="muted small">{format!("{} chosen", d.namespaces.len())}</p>
                }.into_any())
            }
            Level::Workloads | Level::Pod => {
                let listed = targets.get();
                let list = match listed {
                    _ if d.namespace.is_empty() => view! { <p class="muted">"Choose a namespace."</p> }.into_any(),
                    None => view! { <p class="muted">"Loading…"</p> }.into_any(),
                    Some((_, Err(e))) => view! { <p class="warn-text">{e}</p> }.into_any(),
                    Some((_, Ok(t))) => {
                        let filter = d.filter.to_lowercase();
                        if d.level == Level::Workloads {
                            let items: Vec<_> = t.workloads.into_iter().filter(|w| w.name.to_lowercase().contains(&filter)).collect();
                            if items.is_empty() {
                                view! { <p class="muted">"No workloads here."</p> }.into_any()
                            } else {
                                items.into_iter().map(|w| {
                                    let checked = d.workloads.contains(&w);
                                    let kind = w.kind.kind.clone();
                                    let name = w.name.clone();
                                    view! {
                                        <label class="trace-option">
                                            <input type="checkbox" prop:checked=checked on:change=move |_| edit(&|d| d.toggle_workload(&w)) />
                                            <span class="mono">{name}</span>
                                            <span class="muted small">{kind}</span>
                                        </label>
                                    }
                                }).collect_view().into_any()
                            }
                        } else {
                            let items: Vec<_> = t.pods.into_iter().filter(|p| p.to_lowercase().contains(&filter)).collect();
                            if items.is_empty() {
                                view! { <p class="muted">"No running pods here."</p> }.into_any()
                            } else {
                                items.into_iter().map(|p| {
                                    let checked = d.pod == p;
                                    let name = p.clone();
                                    view! {
                                        <label class="trace-option">
                                            <input type="radio" name="trace-pod" prop:checked=checked on:change=move |_| {
                                                let name = name.clone();
                                                edit(&|d| d.pod = name.clone());
                                            } />
                                            <span class="mono">{p}</span>
                                        </label>
                                    }
                                }).collect_view().into_any()
                            }
                        }
                    }
                };
                let chosen = (d.level == Level::Workloads).then(|| {
                    let others = d.workloads.iter().filter(|w| w.namespace != d.namespace).count();
                    let text = match (d.workloads.len(), others) {
                        (n, 0) => format!("{n} chosen"),
                        (n, o) => format!("{n} chosen ({o} in other namespaces)"),
                    };
                    view! { <p class="muted small">{text}</p> }
                });
                let pod_note = (d.level == Level::Pod).then(|| view! {
                    <p class="muted small">"A pod's data stops when it's replaced. To follow a workload through restarts and rollouts, trace the workload."</p>
                });
                Some(view! {
                    <div class="trace-pickers">{namespace_picker()}{filter_box()}</div>
                    <div class="trace-options">{list}</div>
                    {chosen}
                    {pod_note}
                }.into_any())
            }
        }
    };

    let ready = move || draft().and_then(|d| d.scope()).is_some();
    view! {
        <Modal on_cancel=close busy=busy wide=true dismiss_outside=true class="trace-modal">
            <div class="modal-head">
                <h2>"Start a trace"</h2>
                <button class="close" title="Close (Esc)" aria-label="Close" prop:disabled=move || busy.get() on:click=move |_| close.run(())>"×"</button>
            </div>
            <p class="muted">
                "Records CPU and memory for as long as the app stays open, beyond the hour or two Prometheus keeps, "
                "so requests can be right-sized later. It backfills what Prometheus still has."
            </p>
            <div class="trace-levels" role="radiogroup" aria-label="What to trace">
                {level_choice(Level::Cluster, "Whole cluster", "Every pod")}
                {level_choice(Level::Namespaces, "Namespaces", "One or more")}
                {level_choice(Level::Workloads, "Workloads", "Deployments, StatefulSets…")}
                {level_choice(Level::Pod, "Pod", "A single pod")}
            </div>
            <div class="trace-targets">{targets_view}</div>
            <div class="trace-every">
                <label class="field">
                    "Pull from Prometheus every"
                    <select prop:disabled=move || busy.get() on:change=move |ev| {
                        if let Ok(secs) = event_target_value(&ev).parse() {
                            every.set(secs);
                            crate::prefs::save_pull_every(secs);
                        }
                    }>
                        {subduer_types::TRACE_PULL_EVERY_CHOICES.map(|secs| view! {
                            <option value=secs.to_string() selected=move || every.get() == secs>{format!("{} min", secs / 60)}</option>
                        }).collect_view()}
                    </select>
                </label>
                <span class="muted small">"Less often asks the cluster less; stopping always pulls the rest."</span>
            </div>
            {move || error.get().map(|e| view! { <p class="warn-text">{e}</p> })}
            <div class="modal-actions">
                <button prop:disabled=move || busy.get() on:click=move |_| close.run(())>"Cancel"</button>
                <button class="primary" prop:disabled=move || busy.get() || !ready() on:click=start>
                    {move || if busy.get() { "Starting…" } else { "Save as and start…" }}
                </button>
            </div>
        </Modal>
    }
}

/// After a trace stops: where it went and what it holds.
#[component]
pub(crate) fn TraceSavedDialog(store: Store) -> impl IntoView {
    let done = move |_| store.trace_saved.set(None);
    let on_cancel = move |()| store.trace_saved.set(None);
    move || {
        let result = store.trace_saved.get()?;
        Some(match result {
            Ok(s) => {
                let span = match (s.data_start, s.data_end) {
                    (Some(a), Some(b)) => duration_text(b - a),
                    _ => "no data".into(),
                };
                let path = s.path.clone();
                let reveal = move |_| {
                    #[derive(Serialize)]
                    struct Args {
                        path: String,
                    }
                    let path = path.clone();
                    spawn_local(async move {
                        if let Err(e) = invoke::<_, ()>("reveal_trace", &Args { path }).await {
                            store.error.set(Some(e));
                        }
                    });
                };
                let gaps = s.gaps.len();
                let review_path = s.path.clone();
                let review = move |_| {
                    store.trace_saved.set(None);
                    crate::trace_review::open_file(store, review_path.clone());
                };
                view! {
                    <Modal on_cancel=on_cancel wide=true class="trace-modal">
                        <div class="modal-head">
                            <h2>"Trace saved"</h2>
                            <button class="close" title="Close (Esc)" aria-label="Close" on:click=done>"×"</button>
                        </div>
                        <dl class="trace-facts">
                            <dt>"Context"</dt><dd>{s.context.clone()}</dd>
                            <dt>"Traced"</dt><dd>{scope_text(&s.scope)}</dd>
                            <dt>"Data covers"</dt><dd>{span}</dd>
                            <dt>"Holds"</dt><dd>{format!("{} pods, {} containers", s.pods, s.containers)}</dd>
                            {(s.pulled_bytes > 0).then(|| view! {
                                <dt>"Pulled"</dt><dd>{format!("{} from Prometheus", crate::overview::fmt_bytes(s.pulled_bytes as f64))}</dd>
                            })}
                            <dt>"File"</dt><dd class="mono path">{s.path.clone()}</dd>
                        </dl>
                        {(gaps > 0).then(|| view! {
                            <p class="warn-text">{if gaps == 1 { "1 gap with no data.".to_owned() } else { format!("{gaps} gaps with no data.") }}</p>
                        })}
                        <div class="modal-actions">
                            <button on:click=reveal>{crate::prefs::reveal_label()}</button>
                            <button on:click=review>"Review"</button>
                            <button class="primary" on:click=done>"Done"</button>
                        </div>
                    </Modal>
                }
                .into_any()
            }
            Err(e) => view! {
                <Modal on_cancel=on_cancel>
                    <div class="modal-head">
                        <h2>"The trace didn't close cleanly"</h2>
                        <button class="close" title="Close (Esc)" aria-label="Close" on:click=done>"×"</button>
                    </div>
                    <p class="warn-text">{e}</p>
                    <p class="muted">"Data up to the last pull is still in the file."</p>
                    <div class="modal-actions">
                        <button class="primary" on:click=done>"OK"</button>
                    </div>
                </Modal>
            }
            .into_any(),
        })
    }
}

/// "Stop the trace and quit?", when closing or quitting while one runs.
#[component]
pub(crate) fn TraceQuitDialog(store: Store) -> impl IntoView {
    let busy = RwSignal::new(false);
    let cancel = move |_| store.trace_quit.set(None);
    let on_cancel = move |()| store.trace_quit.set(None);
    move || {
        let quit = store.trace_quit.get()?;
        let status = store.trace.get_untracked();
        let what = status.as_ref().map(|s| {
            let now = store.now.get_untracked() / 1000.0;
            let gaps = match s.gaps {
                0 => String::new(),
                1 => ", 1 gap".into(),
                n => format!(", {n} gaps"),
            };
            format!("{} of {} ({}{gaps})", scope_text(&s.scope), s.context, duration_text(now - s.started))
        });
        let dirty = store.dirty.get_untracked();
        let confirm = move |_| {
            #[derive(Serialize)]
            struct Args {
                quit: bool,
            }
            busy.set(true);
            spawn_local(async move {
                if let Err(e) = invoke::<_, ()>("close_discarding", &Args { quit }).await {
                    store.error.set(Some(e));
                }
                busy.set(false);
                store.trace_quit.set(None);
            });
        };
        let action = if quit { "Stop trace and quit" } else { "Stop trace and close" };
        Some(view! {
            <Modal on_cancel=on_cancel busy=busy>
                <h2>"A trace is running"</h2>
                <p>"Tracing "<strong>{what}</strong>"."</p>
                <p>{if quit { "Quitting stops it. The file is saved first." } else { "Closing the window stops it. The file is saved first." }}</p>
                {dirty.map(|d| view! { <p class="warn-text">"Your unsaved edits to "<strong class="mono">{d}</strong>" will be lost."</p> })}
                <div class="modal-actions">
                    <button prop:disabled=move || busy.get() on:click=cancel>"Keep tracing"</button>
                    <button class="danger" prop:disabled=move || busy.get() on:click=confirm>
                        {move || if busy.get() { "Stopping…" } else { action }}
                    </button>
                </div>
            </Modal>
        })
    }
}

/// "Switching context stops the trace", when switching away from a traced context.
#[component]
pub(crate) fn TraceSwitchDialog(store: Store) -> impl IntoView {
    let busy = RwSignal::new(false);
    let keep = move || {
        store.trace_then.set_value(None);
        store.trace_switch.set(None);
    };
    let cancel = move |_| keep();
    let confirm = move |_| {
        busy.set(true);
        spawn_local(async move {
            stop(store, TraceEnd::ContextChanged).await;
            busy.set(false);
            store.trace_switch.set(None);
            if let Some(then) = store.trace_then.try_update_value(Option::take).flatten() {
                then();
            }
        });
    };
    move || {
        let to = store.trace_switch.get()?;
        let from = store.trace.get_untracked().map(|s| s.context).unwrap_or_default();
        Some(view! {
            <Modal on_cancel=move |()| keep() busy=busy>
                <h2>"Stop the trace?"</h2>
                <p>"A trace of "<strong>{from}</strong>" is running. Switching to "<strong>{to}</strong>" stops it; the file is saved first."</p>
                <div class="modal-actions">
                    <button prop:disabled=move || busy.get() on:click=cancel>"Keep tracing"</button>
                    <button class="danger" prop:disabled=move || busy.get() on:click=confirm>
                        {move || if busy.get() { "Stopping…" } else { "Stop trace and switch" }}
                    </button>
                </div>
            </Modal>
        })
    }
}

#[cfg(test)]
#[path = "trace_tests.rs"]
mod tests;
