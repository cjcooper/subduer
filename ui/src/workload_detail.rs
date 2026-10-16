//! The Details tab for a selected Deployment, StatefulSet, Job or CronJob,
//! laid out like a pod's: resources summed over its pods, the workload itself
//! (status, replicas or completions, strategy or schedule, conditions), a
//! CronJob's Jobs, its pods, the pod template's containers, and events.
//! Sections collapse like the pod ones.

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use subduer_types::{Condition, JobRun, KindRef, TemplateContainer, Usage, WorkloadDetails, WorkloadPod};

use crate::app::{ContainerIcon, Store, age_since, format_age, format_span};
use crate::overview::{fmt_bytes, fmt_cpu};
use crate::pod_detail::{EnvTable, EventList, HistoryNote, Resource, Section, UsageChart};
use crate::tauri::invoke;

/// Re-fetches the selected workload's details and usage history.
pub fn load(store: Store, quiet: bool) {
    #[derive(Serialize)]
    struct Args {
        kind: KindRef,
        namespace: String,
        name: String,
    }
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
        let args = Args { kind, namespace, name: row.name.clone() };
        let result = invoke::<_, WorkloadDetails>("workload_details", &args).await;
        if store.selected.with_untracked(|s| s.as_ref().is_some_and(|s| s.id == row.id)) {
            // A failed background refresh keeps the last good render.
            if !(quiet && result.is_err() && store.workload.with_untracked(|w| matches!(w, Some(Ok(_))))) {
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

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

#[component]
fn WorkloadSections(store: Store, d: WorkloadDetails) -> impl IntoView {
    let pod_count = d.pods.len();
    // Jobs keep their finished pods; only running ones use resources.
    let live = d.pods.iter().filter(|p| !p.finished).count();
    let ready = d.pods.iter().filter(|p| ready_all(&p.ready)).count();
    let pods_summary = match (pod_count, d.replicas.is_some()) {
        (0, _) => "none".to_owned(),
        (n, true) => format!("{n} · {ready} ready"),
        (n, false) => format!("{n} · {live} running"),
    };
    let rolling = d.status == "Rolling out";
    let containers = d
        .containers
        .clone()
        .into_iter()
        .map(|c| view! { <TemplateSection store=store c=c pods=live namespace=d.namespace.clone() /> })
        .collect_view();
    let jobs = (d.kind == "CronJob").then(|| {
        let (jobs, ns) = (d.jobs.clone(), d.namespace.clone());
        let failed = jobs.iter().filter(|j| j.status == "Failed").count();
        let summary = match (jobs.len(), failed) {
            (0, _) => "none".to_owned(),
            (n, 0) => format!("{n}"),
            (n, f) => format!("{n} · {f} failed"),
        };
        view! {
            <Section store=store id="jobs" title="Jobs".to_owned() summary=summary>
                <JobTable store=store jobs=jobs.clone() namespace=ns.clone() />
            </Section>
        }
    });
    let events = d.events.clone();
    let warnings = events.iter().filter(|e| e.type_ == "Warning").count();
    let events_summary = match (events.len(), warnings) {
        (0, _) => "none".to_owned(),
        (n, 0) => format!("{n}"),
        (n, w) => format!("{n} · ⚠ {}", plural(w, "warning", "warnings")),
    };
    let resources_summary = match (live, d.replicas.is_some()) {
        (0, false) => "none running · request and limit per pod".to_owned(),
        (0, true) => String::new(),
        (n, _) => format!("total of {}", plural(n, "pod", "pods")),
    };
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

/// Info rows holding an RFC 3339 time, shown relative to now.
fn is_time(key: &str) -> bool {
    matches!(key, "Started" | "Finished" | "Last scheduled" | "Last successful" | "Next run" | "Last restart")
}

/// `5m ago` or `in 12m`.
pub(crate) fn relative(now_ms: f64, time: &str) -> String {
    relative_at(now_ms, js_sys::Date::parse(time))
}

/// [`relative`] from a parsed time (ms).
fn relative_at(now_ms: f64, then_ms: f64) -> String {
    if then_ms > now_ms {
        format!("in {}", format_span(((then_ms - now_ms) / 1000.0) as u64))
    } else {
        format!("{} ago", age_since(now_ms, then_ms))
    }
}

fn ready_all(ready: &str) -> bool {
    ready.split_once('/').is_some_and(|(r, t)| r == t && t != "0")
}

pub(crate) fn badge_class(status: &str) -> String {
    let slug = status
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    format!("badge badge-{slug}")
}

#[component]
fn WorkloadInfo(store: Store, d: WorkloadDetails) -> impl IntoView {
    let replicas = d.replicas.map(|r| {
        let extra = (r.current > r.desired).then(|| format!(" · {} running during rollout", r.current));
        format!(
            "{} desired · {} ready · {} up to date · {} available{}",
            r.desired,
            r.ready,
            r.updated,
            r.available,
            extra.unwrap_or_default()
        )
    });
    let created = d.created.clone().unwrap_or_default();
    let created_text = {
        let created = created.clone();
        move || format!("{} ago · {}", format_age(store.now.get(), &created), created)
    };
    let info = d
        .info
        .iter()
        .map(|(k, v)| {
            if is_time(k) && !js_sys::Date::parse(v).is_nan() {
                let v = v.clone();
                let text = {
                    let v = v.clone();
                    move || format!("{} · {v}", relative(store.now.get(), &v))
                };
                return view! { <dt>{k.clone()}</dt><dd>{text}</dd> }.into_any();
            }
            let mono = matches!(k.as_str(), "Selector" | "Revision" | "Schedule");
            view! { <dt>{k.clone()}</dt><dd class:mono=mono>{v.clone()}</dd> }.into_any()
        })
        .collect_view();
    let labels = d
        .labels
        .iter()
        .map(|(k, v)| view! { <span class="chip">{format!("{k}={v}")}</span> })
        .collect_view();
    let conditions = d.conditions.clone();
    let unhealthy = conditions.iter().filter(|c| !healthy(c)).count();
    let conditions_summary = match (conditions.len(), unhealthy) {
        (0, _) => "none".to_owned(),
        (n, 0) => format!("{n}"),
        (n, u) => format!("{n} · {u} unhealthy"),
    };
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

/// Most conditions are good when true; these report a problem when true.
pub(crate) fn healthy(c: &Condition) -> bool {
    // A Namespace's conditions all describe a deletion that's stuck; a Node's
    // pressure conditions are problems too.
    let bad_when_true = matches!(
        c.type_.as_str(),
        "Failed" | "FailureTarget" | "ReplicaFailure" | "MemoryPressure" | "DiskPressure" | "PIDPressure" | "NetworkUnavailable"
    ) || c.type_.starts_with("Namespace");
    (c.status == "True") != bad_when_true
}

#[component]
pub(crate) fn ConditionList(store: Store, conditions: Vec<Condition>) -> impl IntoView {
    conditions
        .into_iter()
        .map(|c| {
            let badge = if healthy(&c) { "badge badge-ready" } else { "badge badge-warning" };
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
        crate::sort::sort(&mut pods, store.sorts.by("pods", &HEADERS), pod_key);
        pods
        .into_iter()
        .map(|p| {
            let icons = p.containers.iter().cloned().map(|c| view! { <ContainerIcon c=c /> }).collect_view();
            let title = match &p.node {
                Some(node) => format!("Open pod {} · on {node}", p.name),
                None => format!("Open pod {}", p.name),
            };
            let created = p.created.clone().unwrap_or_default();
            let age = move || format_age(store.now.get(), &created);
            let (ns, name) = (namespace.clone(), p.name.clone());
            let old = rolling && !p.updated;
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
                    <td class="num">{p.cpu.map(fmt_cpu).unwrap_or_else(|| "—".into())}</td>
                    <td class="num">{p.memory.map(fmt_bytes).unwrap_or_else(|| "—".into())}</td>
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
                        let class = if matches!(h, "Restarts" | "CPU" | "Memory") { "num" } else { "" };
                        view! { <crate::sort::SortTh sorts=store.sorts table="pods" header=h class=class /> }
                    })}
                </tr>
            </thead>
            <tbody>{rows}</tbody>
        </table>
    }
    .into_any()
}

/// How a pod sorts under the Pods table's column `i`.
pub(crate) fn pod_key(p: &WorkloadPod, i: usize) -> crate::sort::Key {
    use crate::sort::Key;
    match i {
        0 => Key::text(&p.name),
        1 => Key::text(&p.status),
        2 => Key::num(p.restarts as f64),
        3 => Key::opt(p.cpu),
        4 => Key::opt(p.memory),
        _ => Key::age(p.created.as_deref().unwrap_or_default()),
    }
}

/// How a Job sorts under the Jobs table's column `i`; a running Job's
/// duration counts up to `now` (ms).
pub(crate) fn job_key(j: &JobRun, i: usize, now: f64) -> crate::sort::Key {
    use crate::sort::Key;
    match i {
        0 => Key::text(&j.name),
        1 => Key::text(&j.status),
        2 => Key::text(&j.completions),
        3 => {
            let (Key::Num(start), end) = (Key::time(j.started.as_deref().unwrap_or_default()), j.finished.as_deref()) else {
                return Key::Missing;
            };
            let end = match end.map(Key::time) {
                Some(Key::Num(e)) => e,
                Some(_) => return Key::Missing,
                None => now,
            };
            Key::num(end - start)
        }
        _ => Key::age(j.created.as_deref().unwrap_or_default()),
    }
}

/// Seconds-precise under an hour (`2m 18s`), then like ages.
fn run_time(secs: u64) -> String {
    match secs {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m {}s", s / 60, s % 60),
        s => format_span(s),
    }
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
        crate::sort::sort(&mut jobs, store.sorts.by("jobs", &HEADERS), |j, i| job_key(j, i, now));
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
                (!start.is_nan() && !end.is_nan()).then(|| run_time(((end - start) / 1000.0).max(0.0) as u64))
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
                        let class = if matches!(h, "Done" | "Duration") { "num" } else { "" };
                        view! { <crate::sort::SortTh sorts=store.sorts table="jobs" header=h class=class /> }
                    })}
                </tr>
            </thead>
            <tbody>{rows}</tbody>
        </table>
    }
    .into_any()
}

/// Requests and limits are per pod in the template; the chart compares them
/// with usage summed over the pods, so they're scaled by the pod count.
fn scaled(u: Usage, pods: usize) -> Usage {
    let n = pods.max(1) as f64;
    Usage { requests: u.requests.map(|v| v * n), limits: u.limits.map(|v| v * n), ..u }
}

fn per_pod(u: Usage, fmt: fn(f64) -> String) -> String {
    let show = |v: Option<f64>| v.map_or_else(|| "not set".to_owned(), fmt);
    format!("request {} · limit {}", show(u.requests), show(u.limits))
}

#[component]
fn TemplateSection(store: Store, c: TemplateContainer, pods: usize, namespace: String) -> impl IntoView {
    let kind = match (c.init, c.sidecar) {
        (true, true) => "Sidecar · ",
        (true, false) => "Init · ",
        _ => "",
    };
    let short_image = c.image.rsplit('/').next().unwrap_or(&c.image).to_owned();
    let summary = format!("{kind}{short_image}");
    let id = format!("wcontainer:{}", c.name);
    let env_id = format!("wenv:{}", c.name);
    let env_count = c.env.iter().filter(|e| !e.name.is_empty()).count();
    // Init containers that ran to completion have no usage to chart.
    let charted = !c.init || c.sidecar;
    let totals = (charted && pods > 1).then(|| format!("Charts show totals over {pods} pods."));
    let (cpu, memory, name) = (scaled(c.cpu, pods), scaled(c.memory, pods), c.name.clone());
    view! {
        <Section store=store id=id title=c.name.clone() summary=summary>
            {charted.then(|| view! {
                <UsageChart store=store resource=Resource::Cpu usage=cpu container=Some(name.clone()) />
                <UsageChart store=store resource=Resource::Memory usage=memory container=Some(name.clone()) />
            })}
            {totals.clone().map(|t| view! { <p class="legend small">{t}</p> })}
            <dl class="kv">
                <dt>"Image"</dt><dd class="mono">{c.image.clone()}</dd>
                <dt>"CPU per pod"</dt><dd>{per_pod(c.cpu, fmt_cpu)}</dd>
                <dt>"Memory per pod"</dt><dd>{per_pod(c.memory, fmt_bytes)}</dd>
                {(!c.ports.is_empty()).then(|| view! { <dt>"Ports"</dt><dd>{c.ports.join(", ")}</dd> })}
            </dl>
            {
                let (env, ns) = (c.env.clone(), namespace.clone());
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

#[cfg(test)]
#[path = "workload_detail_tests.rs"]
mod tests_unit;
