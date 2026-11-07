//! A workload's Details tab: section summaries, info rows, condition health,
//! the Pods and Jobs tables, and template containers.

use serde::Serialize;
use subduer_types::{Condition, JobRun, KindRef, ReplicaCounts, TemplateContainer, Usage, WorkloadDetails, WorkloadPod};

use crate::app::{age_since, format_span};
use crate::sort::Key;

/// `workload_details` arguments.
#[derive(Debug, Serialize)]
pub(crate) struct DetailsArgs {
    pub(crate) kind: KindRef,
    pub(crate) namespace: String,
    pub(crate) name: String,
}

pub(crate) fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Pods still running: Jobs keep their finished pods, which use nothing.
pub(crate) fn live_pods(pods: &[WorkloadPod]) -> usize {
    pods.iter().filter(|p| !p.finished).count()
}

/// The Pods section's summary: ready ones for replicated workloads, running
/// ones for Jobs.
pub(crate) fn pods_summary(d: &WorkloadDetails) -> String {
    let ready = d.pods.iter().filter(|p| ready_all(&p.ready)).count();
    match (d.pods.len(), d.replicas.is_some()) {
        (0, _) => "none".to_owned(),
        (n, true) => format!("{n} · {ready} ready"),
        (n, false) => format!("{n} · {} running", live_pods(&d.pods)),
    }
}

/// A CronJob's Jobs section summary.
pub(crate) fn jobs_summary(jobs: &[JobRun]) -> String {
    let failed = jobs.iter().filter(|j| j.status == "Failed").count();
    match (jobs.len(), failed) {
        (0, _) => "none".to_owned(),
        (n, 0) => format!("{n}"),
        (n, f) => format!("{n} · {f} failed"),
    }
}

/// The Resources section's summary.
pub(crate) fn resources_summary(live: usize, replicated: bool) -> String {
    match (live, replicated) {
        (0, false) => "none running · request and limit per pod".to_owned(),
        (0, true) => String::new(),
        (n, _) => format!("total of {}", plural(n, "pod", "pods")),
    }
}

/// Info rows holding an RFC 3339 time, shown relative to now.
pub(crate) fn is_time(key: &str) -> bool {
    matches!(key, "Started" | "Finished" | "Last scheduled" | "Last successful" | "Next run" | "Last restart")
}

/// Info rows shown in a monospace font.
pub(crate) fn is_mono(key: &str) -> bool {
    matches!(key, "Selector" | "Revision" | "Schedule")
}

/// `5m ago` or `in 12m`, from a parsed time (ms).
pub(crate) fn relative_at(now_ms: f64, then_ms: f64) -> String {
    if then_ms > now_ms {
        format!("in {}", format_span(((then_ms - now_ms) / 1000.0) as u64))
    } else {
        format!("{} ago", age_since(now_ms, then_ms))
    }
}

/// "in 3m · 2026-10-16T12:00:00Z".
pub(crate) fn dated(text: &str, at: &str) -> String {
    format!("{text} · {at}")
}

pub(crate) fn ready_all(ready: &str) -> bool {
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

/// The Replicas row.
pub(crate) fn replicas_text(r: ReplicaCounts) -> String {
    let extra = (r.current > r.desired).then(|| format!(" · {} running during rollout", r.current));
    format!(
        "{} desired · {} ready · {} up to date · {} available{}",
        r.desired,
        r.ready,
        r.updated,
        r.available,
        extra.unwrap_or_default()
    )
}

/// "none", "3", or "3 · 1 unhealthy".
pub(crate) fn conditions_summary(conditions: &[Condition]) -> String {
    let unhealthy = conditions.iter().filter(|c| !healthy(c)).count();
    match (conditions.len(), unhealthy) {
        (0, _) => "none".to_owned(),
        (n, 0) => format!("{n}"),
        (n, u) => format!("{n} · {u} unhealthy"),
    }
}

/// Most conditions are good when true; these report a problem when true.
pub(crate) fn healthy(c: &Condition) -> bool {
    // KEDA's Active (a trigger has work) and Paused are fine either way.
    if matches!(c.type_.as_str(), "Active" | "Paused") && c.status != "Unknown" {
        return true;
    }
    // A Namespace's conditions all describe a deletion that's stuck; a Node's
    // pressure conditions are problems too, and so is KEDA's Fallback.
    let bad_when_true = matches!(
        c.type_.as_str(),
        "Failed" | "FailureTarget" | "ReplicaFailure" | "MemoryPressure" | "DiskPressure" | "PIDPressure" | "NetworkUnavailable"
            | "Fallback"
    ) || c.type_.starts_with("Namespace");
    (c.status == "True") != bad_when_true
}

pub(crate) fn condition_badge(c: &Condition) -> &'static str {
    if healthy(c) { "badge badge-ready" } else { "badge badge-warning" }
}

/// A Pods table row's tooltip.
pub(crate) fn pod_title(p: &WorkloadPod) -> String {
    match &p.node {
        Some(node) => format!("Open pod {} · on {node}", p.name),
        None => format!("Open pod {}", p.name),
    }
}

/// During a rollout, pods still on the previous template are marked "old".
pub(crate) fn is_old(rolling: bool, p: &WorkloadPod) -> bool {
    rolling && !p.updated
}

/// A usage cell, or "—" without a figure.
pub(crate) fn usage_cell(v: Option<f64>, fmt: fn(f64) -> String) -> String {
    v.map(fmt).unwrap_or_else(|| "—".into())
}

/// The class of a table heading: numeric columns are right-aligned.
pub(crate) fn heading_class(h: &str, numeric: &[&str]) -> &'static str {
    if numeric.contains(&h) { "num" } else { "" }
}

/// How a pod sorts under the Pods table's column `i`.
pub(crate) fn pod_key(p: &WorkloadPod, i: usize) -> Key {
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
pub(crate) fn job_key(j: &JobRun, i: usize, now: f64) -> Key {
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

/// A Job's duration from parsed start and end times (ms; NaN when unknown).
pub(crate) fn duration(start_ms: f64, end_ms: f64) -> Option<String> {
    (!start_ms.is_nan() && !end_ms.is_nan()).then(|| run_time(((end_ms - start_ms) / 1000.0).max(0.0) as u64))
}

/// Seconds-precise under an hour (`2m 18s`), then like ages.
pub(crate) fn run_time(secs: u64) -> String {
    match secs {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m {}s", s / 60, s % 60),
        s => format_span(s),
    }
}

/// Requests and limits are per pod in the template; the chart compares them
/// with usage summed over the pods, so they're scaled by the pod count.
pub(crate) fn scaled(u: Usage, pods: usize) -> Usage {
    let n = pods.max(1) as f64;
    Usage { requests: u.requests.map(|v| v * n), limits: u.limits.map(|v| v * n), ..u }
}

pub(crate) fn per_pod(u: Usage, fmt: fn(f64) -> String) -> String {
    let show = |v: Option<f64>| v.map_or_else(|| "not set".to_owned(), fmt);
    format!("request {} · limit {}", show(u.requests), show(u.limits))
}

/// A template container section's summary: its kind and short image name.
pub(crate) fn template_summary(c: &TemplateContainer) -> String {
    let kind = match (c.init, c.sidecar) {
        (true, true) => "Sidecar · ",
        (true, false) => "Init · ",
        _ => "",
    };
    let short_image = c.image.rsplit('/').next().unwrap_or(&c.image);
    format!("{kind}{short_image}")
}

/// Whether a template container gets usage charts: init containers that ran
/// to completion have no usage to chart.
pub(crate) fn charted(c: &TemplateContainer) -> bool {
    !c.init || c.sidecar
}

/// The note under charts that sum over several pods.
pub(crate) fn totals_note(charted: bool, pods: usize) -> Option<String> {
    (charted && pods > 1).then(|| format!("Charts show totals over {pods} pods."))
}

#[cfg(test)]
#[path = "workload_detail_tests.rs"]
mod tests;
