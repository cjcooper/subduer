//! Traces: the top-bar button and indicator, the start dialog (what to
//! trace, then Save as), the running trace's panel with Stop, the "trace
//! saved" summary, and the prompts that stop a trace before switching
//! context or quitting.
//!
//! The logic is in `logic::trace`.

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use subduer_types::{ResourceRow, TraceEnd, TraceScope, TraceStatus, TraceSummary, TraceTargets};

use crate::app::Store;
pub(crate) use crate::logic::trace::Draft;
use crate::logic::trace::{
    Level, duration_text, every_text, file_name, holds_text, indicator_text, indicator_title, needs_attention, open_blocked, pull_text, pulled_text, pulls_text,
    quit_texts, quit_what, recent, running_gaps_text, sampled_text, saved_gaps_text, scope_text, span_text, stops_on_switch, trace_this_title,
};
use crate::modal::Modal;
use crate::tauri::{NoArgs, invoke, listen};

/// Backend events: the menu's Trace… item, and a trace starting or ending.
const OPEN_TRACE: &str = "open-trace";
const TRACE_CHANGED: &str = "trace-changed";
/// How often the running trace's status is re-read.
pub(crate) const STATUS_EVERY: std::time::Duration = std::time::Duration::from_secs(15);

/// `2026-10-11-1405` in local time.
pub(crate) fn stamp_now() -> String {
    let now = js_sys::Date::now() / 1000.0;
    crate::logic::trace::stamp(now, crate::trace_overlay::local_offset(now))
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
    if let Some(e) = open_blocked(store.trace.with_untracked(Option::is_some), store.cluster.with_untracked(Option::is_some)) {
        store.error.set(Some(e.into()));
        return;
    }
    if store.trace.with_untracked(Option::is_none) {
        store.trace_draft.set(Some(draft.unwrap_or_else(|| Draft::for_view(store.namespace.get_untracked()))));
    }
    store.trace_open.set(true);
}

/// Runs `then` now, or, if a trace of another context is running, once the
/// user agreed to stop it (it's saved first).
pub(crate) fn before_switch(store: Store, context: String, then: impl FnOnce() + 'static) {
    let other = store.trace.with_untracked(|t| stops_on_switch(t.as_ref(), &context));
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
        store.recent_traces.set(crate::prefs::remember_trace(recent(s.path.clone(), s, js_sys::Date::now())));
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
            let title = indicator_title(&status);
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
        let title = trace_this_title(&kind.kind);
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
        let pull = pull_text(&s, now);
        let gaps = running_gaps_text(s.gaps);
        Some(view! {
            <dl class="trace-facts">
                <dt>"Context"</dt><dd>{s.context.clone()}</dd>
                <dt>"Tracing"</dt><dd>{scope_text(&s.scope)}</dd>
                <dt>"Running for"</dt><dd>{duration_text(now - s.started)}</dd>
                <dt>"Data covers"</dt><dd>{duration_text(s.data_end - s.data_start)}</dd>
                {pulls_text(&s).map(|every| view! {
                    <dt>"Pulls"</dt><dd>{every}</dd>
                })}
                <dt>"File"</dt><dd class="mono path">{s.path.clone()}</dd>
            </dl>
            <p class:warn-text=s.last_error.is_some()>{pull}</p>
            {sampled_text(s.sampled_pods).map(|text| view! { <p class="muted">{text}</p> })}
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
        let Some(ns) = d.targets_namespace().map(str::to_owned) else { return };
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
                    on:change=move |_| edit(&|d| d.set_level(level))
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
                    edit(&|d| d.set_namespace(&ns));
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
                let list = namespaces().into_iter().filter(|ns| d.shows(ns)).map(|ns| {
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
                    <p class="muted small">{d.chosen_text()}</p>
                }.into_any())
            }
            Level::Workloads | Level::Pod => {
                let listed = targets.get();
                let list = match listed {
                    _ if d.namespace.is_empty() => view! { <p class="muted">"Choose a namespace."</p> }.into_any(),
                    None => view! { <p class="muted">"Loading…"</p> }.into_any(),
                    Some((_, Err(e))) => view! { <p class="warn-text">{e}</p> }.into_any(),
                    Some((_, Ok(t))) => {
                        if d.level == Level::Workloads {
                            let items: Vec<_> = t.workloads.into_iter().filter(|w| d.shows(&w.name)).collect();
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
                            let items: Vec<_> = t.pods.into_iter().filter(|p| d.shows(p)).collect();
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
                let chosen = (d.level == Level::Workloads).then(|| view! { <p class="muted small">{d.chosen_text()}</p> });
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
                            <option value=secs.to_string() selected=move || every.get() == secs>{every_text(secs)}</option>
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
                let span = span_text(s.data_start, s.data_end);
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
                let gaps = saved_gaps_text(s.gaps.len());
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
                            <dt>"Holds"</dt><dd>{holds_text(&s)}</dd>
                            {pulled_text(&s).map(|pulled| view! {
                                <dt>"Pulled"</dt><dd>{pulled}</dd>
                            })}
                            <dt>"File"</dt><dd class="mono path">{s.path.clone()}</dd>
                        </dl>
                        {gaps.map(|g| view! { <p class="warn-text">{g}</p> })}
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
        let what = status.as_ref().map(|s| quit_what(s, store.now.get_untracked() / 1000.0));
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
        let (note, action) = quit_texts(quit);
        Some(view! {
            <Modal on_cancel=on_cancel busy=busy>
                <h2>"A trace is running"</h2>
                <p>"Tracing "<strong>{what}</strong>"."</p>
                <p>{note}</p>
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
