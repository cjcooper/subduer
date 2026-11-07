//! The Details tab for a selected Deployment, StatefulSet, Job or CronJob,
//! laid out like a pod's: resources summed over its pods, the workload itself
//! (status, replicas or completions, strategy or schedule, conditions), a
//! CronJob's Jobs, its pods, the pod template's containers, and events.
//! Sections collapse like the pod ones.

use leptos::prelude::*;
use leptos::task::spawn_local;
use subduer_types::{Condition, JobRun, TemplateContainer, WorkloadDetails, WorkloadPod};

use crate::app::{ContainerIcon, Store, format_age};
use crate::logic::pod_detail as pod_logic;
use crate::logic::workload_detail::{self as logic, DetailsArgs};
pub(crate) use crate::logic::workload_detail::{badge_class, healthy};
use crate::overview::{fmt_bytes, fmt_cpu};
use crate::pod_detail::{EnvTable, EventList, HistoryNote, Resource, Section, SpecRows, UsageChart};
use crate::tauri::invoke;

/// Re-fetches the selected workload's details and usage history.
pub fn load(store: Store, quiet: bool) {
    let Some(kind) = store.kind.get_untracked() else { return };
    crate::pod_detail::load_history(store, quiet, "workload_history", Some(kind.clone()));
    let Some(row) = store.selected.get_untracked() else { return };
    let Some(namespace) = row.namespace.clone() else { return };
    if store.pod_loading.get_untracked() {
        return;
    }
    if !quiet {
        store.workload.set(None);
    }
    store.pod_loading.set(true);
    spawn_local(async move {
        let args = DetailsArgs { kind, namespace, name: row.name.clone() };
        let result = invoke::<_, WorkloadDetails>("workload_details", &args).await;
        if store.selected.with_untracked(|s| s.as_ref().is_some_and(|s| s.id == row.id)) {
            // A failed background refresh keeps the last good render.
            if pod_logic::replaces(quiet, result.is_err(), store.workload.with_untracked(|w| matches!(w, Some(Ok(_))))) {
                store.workload.set(Some(result));
            }
        }
        store.pod_loading.set(false);
    });
}

#[component]
pub fn WorkloadDetailView(store: Store) -> impl IntoView {
    view! {
        <div class="fade" class:stale=move || store.pod_loading.get() && store.workload.with(Option::is_some)>
            {move || match store.workload.get() {
                None => view! { <p class="muted pad">"Loading…"</p> }.into_any(),
                Some(Err(e)) => view! { <p class="warn">{e}</p> }.into_any(),
                Some(Ok(d)) => view! { <WorkloadSections store=store d=d /> }.into_any(),
            }}
        </div>
    }
}

#[component]
fn WorkloadSections(store: Store, d: WorkloadDetails) -> impl IntoView {
    // Jobs keep their finished pods; only running ones use resources.
    let live = logic::live_pods(&d.pods);
    let pods_summary = logic::pods_summary(&d);
    let rolling = d.status == "Rolling out";
    let containers = d
        .containers
        .clone()
        .into_iter()
        .map(|c| view! { <TemplateSection store=store c=c pods=live namespace=d.namespace.clone() /> })
        .collect_view();
    let jobs = (d.kind == "CronJob").then(|| {
        let (jobs, ns) = (d.jobs.clone(), d.namespace.clone());
        let summary = logic::jobs_summary(&jobs);
        view! {
            <Section store=store id="jobs" title="Jobs".to_owned() summary=summary>
                <JobTable store=store jobs=jobs.clone() namespace=ns.clone() />
            </Section>
        }
    });
    let events = d.events.clone();
    let events_summary = pod_logic::events_summary(&events);
    let resources_summary = logic::resources_summary(live, d.replicas.is_some());
    let kind_id = d.kind.to_lowercase();
    let (cpu, memory) = (d.cpu, d.memory);
    let (pods, namespace) = (d.pods.clone(), d.namespace.clone());
    let (title, status) = (d.kind.clone(), d.status.clone());
    view! {
        <Section store=store id="resources" title="Resources".to_owned() summary=resources_summary>
            <UsageChart store=store resource=Resource::Cpu usage=cpu container=None />
            <UsageChart store=store resource=Resource::Memory usage=memory container=None />
            <HistoryNote store=store subject="its pods" />
        </Section>
        <Section store=store id=kind_id title=title summary=status>
            <WorkloadInfo store=store d=d.clone() />
        </Section>
        {jobs}
        <Section store=store id="pods" title="Pods".to_owned() summary=pods_summary>
            <PodTable store=store pods=pods.clone() namespace=namespace.clone() rolling=rolling />
        </Section>
        {containers}
        <Section store=store id="events" title="Events".to_owned() summary=events_summary>
            <EventList store=store events=events.clone() empty="No events for this workload or its pods." />
        </Section>
    }
}

/// `5m ago` or `in 12m`.
pub(crate) fn relative(now_ms: f64, time: &str) -> String {
    logic::relative_at(now_ms, js_sys::Date::parse(time))
}

#[component]
fn WorkloadInfo(store: Store, d: WorkloadDetails) -> impl IntoView {
    let replicas = d.replicas.map(logic::replicas_text);
    let created = d.created.clone().unwrap_or_default();
    let created_text = {
        let created = created.clone();
        move || pod_logic::ago_at(&format_age(store.now.get(), &created), &created)
    };
    let info = d
        .info
        .iter()
        .map(|(k, v)| {
            if logic::is_time(k) && !js_sys::Date::parse(v).is_nan() {
                let v = v.clone();
                let text = {
                    let v = v.clone();
                    move || logic::dated(&relative(store.now.get(), &v), &v)
                };
                return view! { <dt>{k.clone()}</dt><dd>{text}</dd> }.into_any();
            }
            let mono = logic::is_mono(k);
            view! { <dt>{k.clone()}</dt><dd class:mono=mono>{v.clone()}</dd> }.into_any()
        })
        .collect_view();
    let labels = d
        .labels
        .iter()
        .map(|(k, v)| view! { <span class="chip">{pod_logic::label_chip(k, v)}</span> })
        .collect_view();
    let conditions = d.conditions.clone();
    let conditions_summary = logic::conditions_summary(&conditions);
    view! {
        <dl class="kv">
            <dt>"Status"</dt>
            <dd><span class=badge_class(&d.status)>{d.status.clone()}</span></dd>
            {replicas.map(|r| view! { <dt>"Replicas"</dt><dd>{r}</dd> })}
            {info}
            {(!created.is_empty()).then(|| view! { <dt>"Created"</dt><dd>{created_text}</dd> })}
            {(!d.labels.is_empty()).then(|| view! { <dt>"Labels"</dt><dd class="chips">{labels}</dd> })}
        </dl>
        {(!conditions.is_empty()).then(|| view! {
            <Section store=store id="conditions" title="Conditions".to_owned() summary=conditions_summary nested=true>
                <ConditionList store=store conditions=conditions.clone() />
            </Section>
        })}
    }
}

#[component]
pub(crate) fn ConditionList(store: Store, conditions: Vec<Condition>) -> impl IntoView {
    conditions
        .into_iter()
        .map(|c| {
            let badge = logic::condition_badge(&c);
            let when = c.last_transition.clone().map(|t| {
                move || format!("{} ago", format_age(store.now.get(), &t))
            });
            view! {
                <div class="event">
                    <div class="event-head">
                        <strong>{c.type_.clone()}</strong>
                        <span class=badge>{c.status.clone()}</span>
                        <span class="muted">{c.reason.clone()}</span>
                        <span class="event-when muted">{when}</span>
                    </div>
                    {(!c.message.is_empty()).then(|| view! { <div class="event-message">{c.message.clone()}</div> })}
                </div>
            }
        })
        .collect_view()
}

#[component]
fn PodTable(store: Store, pods: Vec<WorkloadPod>, namespace: String, rolling: bool) -> impl IntoView {
    if pods.is_empty() {
        return view! { <p class="muted">"No pods."</p> }.into_any();
    }
    const HEADERS: [&str; 6] = ["Name", "Status", "Restarts", "CPU", "Memory", "Age"];
    let rows = move || {
        let mut pods = pods.clone();
        crate::sort::sort(&mut pods, store.sorts.by("pods", &HEADERS), logic::pod_key);
        pods
        .into_iter()
        .map(|p| {
            let icons = p.containers.iter().cloned().map(|c| view! { <ContainerIcon c=c /> }).collect_view();
            let title = logic::pod_title(&p);
            let created = p.created.clone().unwrap_or_default();
            let age = move || format_age(store.now.get(), &created);
            let (ns, name) = (namespace.clone(), p.name.clone());
            let old = logic::is_old(rolling, &p);
            view! {
                <tr title=title on:click=move |_| crate::app::open_pod(store, ns.clone(), name.clone())>
                    <td class="name">
                        <span class="ctr-icons">{icons}</span>
                        " "{p.name.clone()}
                    </td>
                    <td>
                        <span class=badge_class(&p.status)>{p.status.clone()}</span>
                        {old.then(|| view! { <span class="chip old-rev" title="Still runs the previous pod template">"old"</span> })}
                    </td>
                    <td class="num">{p.restarts}</td>
                    <td class="num">{logic::usage_cell(p.cpu, fmt_cpu)}</td>
                    <td class="num">{logic::usage_cell(p.memory, fmt_bytes)}</td>
                    <td class="age">{age}</td>
                </tr>
            }
        })
        .collect_view()
    };
    view! {
        <table class="resources wpods">
            <thead>
                <tr>
                    {HEADERS.map(|h| {
                        let class = logic::heading_class(h, &["Restarts", "CPU", "Memory"]);
                        view! { <crate::sort::SortTh sorts=store.sorts table="pods" header=h class=class /> }
                    })}
                </tr>
            </thead>
            <tbody>{rows}</tbody>
        </table>
    }
    .into_any()
}

#[component]
fn JobTable(store: Store, jobs: Vec<JobRun>, namespace: String) -> impl IntoView {
    if jobs.is_empty() {
        return view! { <p class="muted">"No Jobs kept."</p> }.into_any();
    }
    const HEADERS: [&str; 5] = ["Name", "Status", "Done", "Duration", "Age"];
    let rows = move || {
        let mut jobs = jobs.clone();
        let now = store.now.get_untracked();
        crate::sort::sort(&mut jobs, store.sorts.by("jobs", &HEADERS), |j, i| logic::job_key(j, i, now));
        jobs
        .into_iter()
        .map(|j| {
            let created = j.created.clone().unwrap_or_default();
            let age = move || format_age(store.now.get(), &created);
            let (started, finished) = (j.started.clone(), j.finished.clone());
            // Ticks while it runs.
            let duration = move || {
                let start = js_sys::Date::parse(started.as_deref()?);
                let end = finished.as_deref().map_or_else(|| store.now.get(), js_sys::Date::parse);
                logic::duration(start, end)
            };
            let (ns, name) = (namespace.clone(), j.name.clone());
            view! {
                <tr title=format!("Open Job {}", j.name) on:click=move |_| crate::app::open_object(store, "batch/Job", Some(ns.clone()), name.clone())>
                    <td class="name">
                        {j.name.clone()}
                        {j.manual.then(|| view! { <span class="chip" title="Started by hand, not on schedule">"manual"</span> })}
                    </td>
                    <td><span class=badge_class(&j.status)>{j.status.clone()}</span></td>
                    <td class="num">{j.completions.clone()}</td>
                    <td class="num">{move || duration().unwrap_or_else(|| "—".into())}</td>
                    <td class="age">{age}</td>
                </tr>
            }
        })
        .collect_view()
    };
    view! {
        <table class="resources wpods">
            <thead>
                <tr>
                    {HEADERS.map(|h| {
                        let class = logic::heading_class(h, &["Done", "Duration"]);
                        view! { <crate::sort::SortTh sorts=store.sorts table="jobs" header=h class=class /> }
                    })}
                </tr>
            </thead>
            <tbody>{rows}</tbody>
        </table>
    }
    .into_any()
}

#[component]
fn TemplateSection(store: Store, c: TemplateContainer, pods: usize, namespace: String) -> impl IntoView {
    let summary = logic::template_summary(&c);
    let id = format!("wcontainer:{}", c.name);
    let env_id = format!("wenv:{}", c.name);
    let env_summary = pod_logic::env_summary(&c.env);
    // Init containers that ran to completion have no usage to chart.
    let charted = logic::charted(&c);
    let totals = logic::totals_note(charted, pods);
    let (cpu, memory, name) = (logic::scaled(c.cpu, pods), logic::scaled(c.memory, pods), c.name.clone());
    view! {
        <Section store=store id=id title=c.name.clone() summary=summary>
            {charted.then(|| view! {
                <UsageChart store=store resource=Resource::Cpu usage=cpu container=Some(name.clone()) />
                <UsageChart store=store resource=Resource::Memory usage=memory container=Some(name.clone()) />
            })}
            {totals.clone().map(|t| view! { <p class="legend small">{t}</p> })}
            <dl class="kv">
                <dt>"Image"</dt><dd class="mono">{c.image.clone()}</dd>
                <dt>"CPU per pod"</dt><dd>{logic::per_pod(c.cpu, fmt_cpu)}</dd>
                <dt>"Memory per pod"</dt><dd>{logic::per_pod(c.memory, fmt_bytes)}</dd>
                {(!c.ports.is_empty()).then(|| view! { <dt>"Ports"</dt><dd>{c.ports.join(", ")}</dd> })}
                <SpecRows spec=c.spec.clone() />
            </dl>
            {
                let (env, ns) = (c.env.clone(), namespace.clone());
                view! {
                    <Section
                        store=store
                        id=env_id.clone()
                        title="Environment".to_owned()
                        summary=env_summary.clone()
                        nested=true
                    >
                        <EnvTable store=store vars=env.clone() namespace=ns.clone() />
                    </Section>
                }
            }
        </Section>
    }
}
