//! A pod's Details tab: container groups, section toggles, chart headers and
//! data, and the texts of the info, container, environment and event rows.
//! Shared by the workload and object Details tabs.

use std::collections::HashSet;

use serde::Serialize;
use subduer_types::containers::State;
use subduer_types::{
    ContainerDetails, EnvValue, EnvVar, EventSummary, HistorySource, KindRef, LogSource, LogTarget, PodDetails, PodHistory,
    ResourceTotals, Usage, UsageHistory,
};

use crate::chart::{ChartData, RefLine, Scale};
use crate::logic::overview::{fmt_bytes, fmt_cpu};

/// `pod_details` arguments.
#[derive(Debug, Serialize)]
pub(crate) struct PodArgs {
    pub(crate) namespace: String,
    pub(crate) name: String,
}

/// Arguments for a `*_history` command.
#[derive(Debug, Serialize)]
pub(crate) struct HistoryArgs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) kind: Option<KindRef>,
    // Not for cluster-scoped objects (nodes).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) namespace: Option<String>,
    pub(crate) name: String,
}

/// `secret_value` arguments.
#[derive(Debug, Serialize)]
pub(crate) struct SecretArgs {
    pub(crate) namespace: String,
    pub(crate) secret: String,
    pub(crate) key: String,
}

/// Whether a fetched Details result replaces what's shown: a failed
/// background (`quiet`) refresh keeps the last good render.
pub(crate) fn replaces(quiet: bool, failed: bool, showing_good: bool) -> bool {
    !(quiet && failed && showing_good)
}

/// Whether a fetched history replaces what's shown: a failed refresh keeps
/// the last good chart.
pub(crate) fn history_replaces(ok: bool, showing_good: bool) -> bool {
    ok || !showing_good
}

/// A group of a pod's containers in the Details tab.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Group {
    pub(crate) id: &'static str,
    pub(crate) title: &'static str,
    /// Starts closed.
    pub(crate) closed: bool,
    pub(crate) containers: Vec<ContainerDetails>,
}

/// The pod's containers by group, top to bottom: running (app containers,
/// and sidecars, which run alongside them), init, ephemeral. Only running
/// starts open; empty groups are left out.
pub(crate) fn container_groups(all: &[ContainerDetails]) -> Vec<Group> {
    let pick = |f: fn(&ContainerDetails) -> bool| all.iter().filter(|c| f(c)).cloned().collect::<Vec<_>>();
    [
        Group { id: "containers:running", title: "Running containers", closed: false, containers: pick(|c| !c.ephemeral && (!c.init || c.sidecar)) },
        Group { id: "containers:init", title: "Init containers", closed: true, containers: pick(|c| c.init && !c.sidecar) },
        Group { id: "containers:ephemeral", title: "Ephemeral containers", closed: true, containers: pick(|c| c.ephemeral) },
    ]
    .into_iter()
    .filter(|g| !g.containers.is_empty())
    .collect()
}

/// "3", or "3 · 1 failed".
pub(crate) fn group_summary(containers: &[ContainerDetails]) -> String {
    match containers.iter().filter(|c| c.state == State::Failed).count() {
        0 => containers.len().to_string(),
        failed => format!("{} · {failed} failed", containers.len()),
    }
}

/// "none", "4", or "4 · ⚠ 1 warning".
pub(crate) fn events_summary(events: &[EventSummary]) -> String {
    let warnings = events.iter().filter(|e| e.type_ == "Warning").count();
    match (events.len(), warnings) {
        (0, _) => "none".to_owned(),
        (n, 0) => format!("{n}"),
        (n, w) => format!("{n} · ⚠ {w} warning{}", if w == 1 { "" } else { "s" }),
    }
}

/// Whether a section is open, given the remembered toggles: one that starts
/// closed is open once toggled, the others until toggled.
pub(crate) fn section_open(toggled: &HashSet<String>, id: &str, closed: bool) -> bool {
    toggled.contains(&toggle_key(id, closed)) == closed
}

/// The remembered key: sections that start closed remember being opened.
pub(crate) fn toggle_key(id: &str, closed: bool) -> String {
    if closed { format!("+{id}") } else { id.to_owned() }
}

/// Flips a section's remembered toggle.
pub(crate) fn toggle(toggled: &mut HashSet<String>, key: &str) {
    if !toggled.remove(key) {
        toggled.insert(key.to_owned());
    }
}

/// Whether a closed section's body is rendered (hidden) so an open find can search it.
pub(crate) fn finding(find_open: bool, query: &str) -> bool {
    find_open && !query.trim().is_empty()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Resource {
    Cpu,
    Memory,
}

impl Resource {
    /// The class that colours its charts: CPU and memory each have a colour.
    pub(crate) fn css(self) -> &'static str {
        match self {
            Self::Cpu => "res-cpu",
            Self::Memory => "res-memory",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::Memory => "Memory",
        }
    }

    pub(crate) fn fmt(self) -> fn(f64) -> String {
        match self {
            Self::Cpu => fmt_cpu,
            Self::Memory => fmt_bytes,
        }
    }

    pub(crate) fn scale(self) -> Scale {
        match self {
            Self::Cpu => Scale::Decimal,
            Self::Memory => Scale::Binary,
        }
    }

    pub(crate) fn series(self, h: &UsageHistory) -> &[(f64, f64)] {
        match self {
            Self::Cpu => &h.cpu,
            Self::Memory => &h.memory,
        }
    }
}

/// The chart's header: usage now (`used`), its share of the limit (or the
/// node without one), the request and the limit.
pub(crate) fn usage_header(u: Usage, fmt: fn(f64) -> String) -> String {
    let Usage { requests, limits, used: now, node } = u;
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
}

/// A node's chart header: usage now and its share of allocatable, what its
/// pods request (and that share), their limits, and allocatable.
pub(crate) fn node_header(now: Option<f64>, t: ResourceTotals, fmt: fn(f64) -> String) -> String {
    let share = |v: f64| if t.allocatable > 0.0 { Some(v / t.allocatable) } else { None };
    let pct = |s: Option<f64>| match s {
        Some(s) if s > 0.0 && s < 0.01 => " (<1%)".to_owned(),
        Some(s) => format!(" ({:.0}%)", s * 100.0),
        None => String::new(),
    };
    let (used_share, req_share) = (now.and_then(share), share(t.requests));
    let warn = used_share.is_some_and(|s| s >= 0.75) || req_share.is_some_and(|s| s >= 0.9);
    format!(
        "{}now {}{} · req {}{} · limits {} · allocatable {}",
        if warn { "⚠ " } else { "" },
        now.map_or_else(|| "—".to_owned(), fmt),
        pct(used_share),
        fmt(t.requests),
        pct(req_share),
        if t.limits > 0.0 { fmt(t.limits) } else { "none".to_owned() },
        fmt(t.allocatable),
    )
}

/// The selected object's last-hour series for `resource` (the whole object
/// with `container: None`): none while loading, empty after an error.
pub(crate) fn series_points(
    history: Option<&Result<PodHistory, String>>,
    resource: Resource,
    container: Option<&str>,
) -> Option<Vec<(f64, f64)>> {
    match history {
        None => None,
        Some(Err(_)) => Some(vec![]),
        Some(Ok(h)) => Some(match container {
            None => resource.series(&h.pod).to_vec(),
            Some(name) => {
                h.containers.iter().find(|c| c.name == name).map(|c| resource.series(&c.usage).to_vec()).unwrap_or_default()
            }
        }),
    }
}

/// The newest point's value, or `fallback` without one.
pub(crate) fn latest(points: Option<&[(f64, f64)]>, fallback: Option<f64>) -> Option<f64> {
    points.and_then(|p| p.last()).map(|p| p.1).or(fallback)
}

/// Chart data for `points`, with `refs` that have a value as reference lines.
pub(crate) fn chart_data(
    points: Vec<(f64, f64)>,
    history: Option<&Result<PodHistory, String>>,
    refs: [(&'static str, Option<f64>, bool); 2],
) -> ChartData {
    let (start, end, step) = match history {
        Some(Ok(h)) => (h.start, h.end, h.step),
        _ => (0.0, 1.0, 1.0),
    };
    let refs = refs.into_iter().filter_map(|(name, value, dashed)| Some(RefLine { name, value: value?, dashed })).collect();
    ChartData { start, end, step, points, refs }
}

/// Where the charts' data comes from, and how far back it goes; `clock`
/// shows a unix time as local `HH:MM`.
pub(crate) fn history_note(h: &PodHistory, subject: &str, clock: impl Fn(f64) -> String) -> String {
    match h.source {
        HistorySource::Prometheus => "Last hour from Prometheus · updates every 15 s".to_owned(),
        HistorySource::Local => match h.pod.memory.first().or(h.pod.cpu.first()) {
            Some(&(t, _)) => format!(
                "No Prometheus data for {subject}, so the app records usage itself: since {} · updates every 15 s",
                clock(t)
            ),
            None => "No usage source: needs Prometheus, metrics-server or kubelet stats.".to_owned(),
        },
    }
}

pub(crate) fn history_error(e: &str) -> String {
    format!("Can't load usage history: {e}")
}

/// "5m ago · 2026-10-16T12:00:00Z".
pub(crate) fn ago_at(age: &str, at: &str) -> String {
    format!("{age} ago · {at}")
}

/// A label as a chip: `app=web`.
pub(crate) fn label_chip(k: &str, v: &str) -> String {
    format!("{k}={v}")
}

/// The Pod section's rows after Status, without the empty ones.
pub(crate) fn pod_rows(d: &PodDetails) -> Vec<(&'static str, String)> {
    [
        ("Namespace", Some(d.namespace.clone())),
        ("Node", d.node.clone()),
        ("Pod IP", d.pod_ip.clone()),
        ("Host IP", d.host_ip.clone()),
        ("QoS class", d.qos_class.clone()),
        ("Service account", d.service_account.clone()),
        ("Controlled by", d.owner.clone()),
        ("Restart policy", d.restart_policy.clone()),
        ("Priority class", d.priority_class.clone()),
    ]
    .into_iter()
    .filter_map(|(k, v)| v.filter(|v| !v.is_empty()).map(|v| (k, v)))
    .collect()
}

/// A container section's summary: sidecar marker, state, restarts.
pub(crate) fn container_summary(c: &ContainerDetails) -> String {
    // The group says init or ephemeral; sidecars sit with the running ones.
    let kind = if c.sidecar { "Sidecar · " } else { "" };
    match c.restarts {
        0 => format!("{kind}{}", c.detail),
        1 => format!("{kind}{} · 1 restart", c.detail),
        n => format!("{kind}{} · {n} restarts", c.detail),
    }
}

/// " (not ready)" after a running app container's state when it isn't ready.
pub(crate) fn not_ready(c: &ContainerDetails) -> Option<&'static str> {
    (!c.init && !c.ready && c.state == State::Running).then_some(" (not ready)")
}

/// Which buttons a container section shows, and their tooltips.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ContainerActions {
    /// Shell, Upload and Download need a running container.
    pub(crate) running: bool,
    /// Forward also needs declared ports.
    pub(crate) forward: bool,
    pub(crate) logs: String,
    pub(crate) shell: String,
    pub(crate) upload: String,
    pub(crate) download: String,
}

pub(crate) fn container_actions(c: &ContainerDetails) -> ContainerActions {
    let running = c.state == State::Running;
    ContainerActions {
        running,
        forward: running && !c.ports.is_empty(),
        logs: format!("Open {} logs in a new window", c.name),
        shell: format!("Open a shell in {} in a new window (needs changes allowed)", c.name),
        upload: format!("Upload a file into {} (needs changes allowed)", c.name),
        download: format!("Download a file from {}", c.name),
    }
}

/// What the Logs button opens for one container.
pub(crate) fn logs_target(namespace: &str, pod: &str, container: &str) -> LogTarget {
    LogTarget {
        namespace: namespace.to_owned(),
        source: LogSource::Container { pod: pod.to_owned(), container: container.to_owned() },
        filter: None,
    }
}

/// The Environment section's summary: named variables, or "none".
pub(crate) fn env_summary(env: &[EnvVar]) -> String {
    match env.iter().filter(|e| !e.name.is_empty()).count() {
        0 => "none".to_owned(),
        n => n.to_string(),
    }
}

/// How an environment variable's value is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EnvShown {
    Empty,
    Literal(String),
    /// A ConfigMap key: its value (`None` when missing) and where it's from.
    ConfigMap { value: Option<String>, source: String },
    /// Masked until revealed.
    Secret { name: String, key: String },
    /// A field or resource reference.
    Reference(String),
    Warning(String),
}

pub(crate) fn env_shown(value: &EnvValue) -> EnvShown {
    match value.clone() {
        EnvValue::Literal(v) if v.is_empty() => EnvShown::Empty,
        EnvValue::Literal(v) => EnvShown::Literal(v),
        EnvValue::ConfigMap { name, key, value } => EnvShown::ConfigMap { value, source: format!("ConfigMap {name} · {key}") },
        EnvValue::Secret { name, key } => EnvShown::Secret { name, key },
        EnvValue::Field(path) => EnvShown::Reference(format!("field {path}")),
        EnvValue::Resource(r) => EnvShown::Reference(format!("resource {r}")),
        EnvValue::Unreadable { source, error } => EnvShown::Warning(format!("Can't read {source}: {error}")),
    }
}

/// "Secret db · password" under a revealable value.
pub(crate) fn secret_source(secret: &str, key: &str) -> String {
    format!("Secret {secret} · {key}")
}

/// An event's badge class.
pub(crate) fn event_badge(type_: &str) -> &'static str {
    if type_ == "Warning" { "badge badge-warning" } else { "badge" }
}

/// "×3 · last 5m ago", or "5m ago" for a one-off.
pub(crate) fn event_when(count: i64, age: &str) -> String {
    if count > 1 { format!("×{count} · last {age} ago") } else { format!("{age} ago") }
}

#[cfg(test)]
#[path = "pod_detail_tests.rs"]
mod tests;
