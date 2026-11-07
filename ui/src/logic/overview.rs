//! The Cluster page's logic: refresh decisions, number formatting, meters,
//! tiles, namespace bars and node rows.

use subduer_types::{ClusterOverview, NamespaceRequests, NodeSummary, PodCounts, UsageSource};

use crate::sort::Key;

/// Failed refreshes in a row before polling stops; Refresh starts it again.
pub(crate) const MAX_FAILURES: u32 = 3;
/// Bars shown per namespace chart; the rest fold into "Other".
pub(crate) const TOP_NAMESPACES: usize = 8;
/// Data younger than this is reused when switching between the Cluster and Node pages.
pub(crate) const FRESH_MS: f64 = 10_000.0;

/// Whether a refresh should fetch: not while one is in flight, and not when
/// the data (`age_ms` old) is still fresh unless `force`d.
pub(crate) fn needs_fetch(loading: bool, has_data: bool, age_ms: f64, force: bool) -> bool {
    if loading {
        return false;
    }
    let fresh = has_data && age_ms < FRESH_MS;
    force || !fresh
}

/// Whether the 30 s poll still runs after `failures` failed fetches in a row.
pub(crate) fn keep_polling(failures: u32) -> bool {
    failures < MAX_FAILURES
}

/// The error to show after a failed fetch (`failures` counts this one), and
/// whether polling gives up.
pub(crate) fn failure(failures: u32, e: String) -> (String, bool) {
    if failures >= MAX_FAILURES {
        (format!("Stopped refreshing after repeated failures: {e}"), true)
    } else {
        (e, false)
    }
}

/// "context · v1.33" under the page title.
pub(crate) fn subtitle(context: String, version: Option<String>) -> String {
    match version {
        Some(v) => format!("{context} · {v}"),
        None => context,
    }
}

/// "Updated 12s ago", once there has been a fetch.
pub(crate) fn updated(at: f64, now: f64) -> Option<String> {
    (at > 0.0).then(|| format!("Updated {} ago", age_ms(now - at)))
}

/// The Node page's rows: names containing `filter` (any case).
pub(crate) fn filter_nodes(nodes: Vec<NodeSummary>, filter: &str) -> Vec<NodeSummary> {
    let filter = filter.to_lowercase();
    nodes.into_iter().filter(|n| filter.is_empty() || n.name.to_lowercase().contains(&filter)).collect()
}

/// The four tiles at the top of the Cluster page.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Tiles {
    pub(crate) nodes: String,
    pub(crate) nodes_note: String,
    pub(crate) nodes_tone: &'static str,
    pub(crate) pods: String,
    pub(crate) pods_note: String,
    pub(crate) pods_tone: &'static str,
    pub(crate) warnings: String,
    pub(crate) warnings_note: String,
    pub(crate) warnings_tone: &'static str,
}

pub(crate) fn tiles(o: &ClusterOverview) -> Tiles {
    let ready = o.nodes.iter().filter(|n| n.ready).count();
    let node_count = o.nodes.len();
    let nodes_note = if ready == node_count { "all ready".to_owned() } else { format!("⚠ {} not ready", node_count - ready) };
    let pods = o.pods;
    let (warnings, warnings_note, warnings_tone) = match o.warnings_last_hour {
        Some(0) => ("0".to_owned(), "none".to_owned(), "good"),
        Some(n) => (n.to_string(), "⚠ see Events".to_owned(), "warning"),
        None => ("–".to_owned(), "can't read events".to_owned(), ""),
    };
    Tiles {
        nodes: node_count.to_string(),
        nodes_note,
        nodes_tone: if ready == node_count { "good" } else { "critical" },
        pods: pods.running.to_string(),
        pods_note: format!("{} pending · {} failed", pods.pending, pods.failed),
        pods_tone: if pods.failed > 0 {
            "critical"
        } else if pods.pending > 0 {
            "warning"
        } else {
            "good"
        },
        warnings,
        warnings_note,
        warnings_tone,
    }
}

/// Namespaces by CPU requested, largest first.
pub(crate) fn by_cpu(namespaces: &[NamespaceRequests]) -> Vec<NamespaceRequests> {
    let mut by_cpu = namespaces.to_vec();
    by_cpu.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
    by_cpu
}

/// A resource card's subtitle.
pub(crate) fn card_subtitle(nodes: usize, capacity: f64, fmt: fn(f64) -> String) -> String {
    format!("allocatable across {nodes} nodes · {} capacity", fmt(capacity))
}

/// "peak 3.2 GiB" over a series.
pub(crate) fn peak(points: &[(f64, f64)], fmt: fn(f64) -> String) -> String {
    format!("peak {}", fmt(points.iter().map(|p| p.1).fold(0.0, f64::max)))
}

pub(crate) fn usage_note(source: Option<UsageSource>) -> &'static str {
    match source {
        Some(UsageSource::MetricsServer) => "Usage from metrics-server.",
        Some(UsageSource::Prometheus) => {
            "Usage from Prometheus (container working set), with kubelet stats for nodes it doesn't cover."
        }
        Some(UsageSource::Kubelet) => "Usage from kubelet stats on each node. No metrics-server or Prometheus found.",
        None => "Live usage needs metrics-server, Prometheus, or access to kubelet stats (nodes/proxy).",
    }
}

/// Severity class and marker for a fraction of capacity. Colour is never the
/// only signal: warning levels also get an icon in the value text.
pub(crate) fn severity(fraction: f64) -> (&'static str, &'static str) {
    if fraction >= 0.9 {
        ("critical", "⚠ ")
    } else if fraction >= 0.75 {
        ("warning", "⚠ ")
    } else {
        ("normal", "")
    }
}

/// `value` as a share of `total`; 0 without a total.
pub(crate) fn fraction(value: f64, total: f64) -> f64 {
    if total > 0.0 { value / total } else { 0.0 }
}

/// A meter fill's width, clamped to the track.
pub(crate) fn fill_width(fraction: f64) -> String {
    format!("width: {:.1}%", (fraction * 100.0).clamp(0.0, 100.0))
}

/// What a meter shows.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MeterText {
    /// `meter-fill <severity>`.
    pub(crate) fill_class: String,
    pub(crate) text: String,
    pub(crate) width: String,
    pub(crate) title: String,
}

pub(crate) fn meter(label: &str, value: f64, total: f64, fmt: fn(f64) -> String) -> MeterText {
    let fraction = fraction(value, total);
    let (sev, icon) = severity(fraction);
    let over = if fraction > 1.0 { " · overcommitted" } else { "" };
    MeterText {
        fill_class: format!("meter-fill {sev}"),
        text: format!("{icon}{} · {}{over}", fmt(value), pct(fraction)),
        width: fill_width(fraction),
        title: format!("{label}: {} of {}", fmt(value), fmt(total)),
    }
}

/// A table cell's small meter: (fill class, width, text).
pub(crate) fn mini_meter(value: f64, total: f64) -> (String, String, String) {
    let fraction = fraction(value, total);
    let (sev, icon) = severity(fraction);
    (format!("meter-fill {sev}"), fill_width(fraction), format!("{icon}{}", pct(fraction)))
}

/// The pod status bar's segments: (label, class, count).
pub(crate) fn pod_segments(pods: PodCounts) -> Vec<(&'static str, &'static str, usize)> {
    vec![
        ("Running", "good", pods.running),
        ("Pending", "warning", pods.pending),
        ("Failed", "critical", pods.failed),
        ("Unknown", "serious", pods.unknown),
        ("Succeeded", "done", pods.succeeded),
    ]
}

/// A segment's (style, title) in a bar of `total` pods.
pub(crate) fn segment(label: &str, n: usize, total: usize) -> (String, String) {
    (format!("flex-grow: {n}"), format!("{label}: {n} ({})", pct(n as f64 / total as f64)))
}

/// Top namespaces by `value`, with the rest folded into one "Other" bar.
pub(crate) fn namespace_rows(sorted: &[NamespaceRequests], value: fn(&NamespaceRequests) -> f64) -> Vec<(String, f64, usize)> {
    let mut rows: Vec<(String, f64, usize)> = sorted
        .iter()
        .filter(|n| value(n) > 0.0)
        .take(TOP_NAMESPACES)
        .map(|n| (n.namespace.clone(), value(n), n.pods))
        .collect();
    let rest: Vec<&NamespaceRequests> = sorted.iter().filter(|n| value(n) > 0.0).skip(TOP_NAMESPACES).collect();
    if !rest.is_empty() {
        let label = format!("Other ({} namespaces)", rest.len());
        rows.push((label, rest.iter().map(|n| value(n)).sum(), rest.iter().map(|n| n.pods).sum()));
    }
    rows
}

/// The largest bar's value.
pub(crate) fn bar_max(rows: &[(String, f64, usize)]) -> f64 {
    rows.iter().map(|r| r.1).fold(0.0, f64::max)
}

/// A namespace bar's (width style, title).
pub(crate) fn bar(name: &str, value: f64, pods: usize, max: f64, fmt: fn(f64) -> String) -> (String, String) {
    let width = format!("width: {:.1}%", if max > 0.0 { value / max * 100.0 } else { 0.0 });
    (width, format!("{name}: {} across {pods} pods", fmt(value)))
}

/// The node table's headings and their classes; usage columns only with metrics.
pub(crate) fn node_headers(metrics: bool) -> Vec<(&'static str, &'static str)> {
    let mut headers: Vec<(&str, &'static str)> = vec![
        ("Name", ""),
        ("Status", ""),
        ("Type", ""),
        ("Pods", "num"),
        ("CPU requested", ""),
        ("Memory requested", ""),
    ];
    if metrics {
        headers.extend([("CPU used", ""), ("Memory used", "")]);
    }
    headers.extend([("CPU", "num"), ("Memory", "num"), ("Version", ""), ("Age", "")]);
    headers
}

/// A node's status text and badge class.
pub(crate) fn node_status(n: &NodeSummary) -> (String, &'static str) {
    let mut status = if n.ready { "Ready".to_owned() } else { "NotReady".to_owned() };
    if !n.schedulable {
        status.push_str(" · Cordoned");
    }
    let badge = match (n.ready, n.schedulable) {
        (false, _) => "badge badge-notready",
        (true, false) => "badge badge-pending",
        (true, true) => "badge badge-ready",
    };
    (status, badge)
}

/// "12/29" pods.
pub(crate) fn node_pods(n: &NodeSummary) -> String {
    format!("{}/{}", n.pods, n.pod_capacity)
}

/// How a node sorts under one heading: meters by their share, sizes by value.
pub(crate) fn node_key(n: &NodeSummary, header: &str) -> Key {
    let share = |v: f64, total: f64| if total > 0.0 { Key::num(v / total) } else { Key::Missing };
    match header {
        "Name" => Key::text(&n.name),
        // Not ready first, then cordoned.
        "Status" => Key::num(f64::from(u8::from(n.ready)) * 2.0 + f64::from(u8::from(n.schedulable))),
        "Type" => Key::text(n.instance_type.as_deref().unwrap_or_default()),
        "Pods" => Key::num(n.pods as f64),
        "CPU requested" => share(n.cpu.requests, n.cpu.allocatable),
        "Memory requested" => share(n.memory.requests, n.memory.allocatable),
        "CPU used" => n.cpu.used.map_or(Key::Missing, |u| share(u, n.cpu.allocatable)),
        "Memory used" => n.memory.used.map_or(Key::Missing, |u| share(u, n.memory.allocatable)),
        "CPU" => Key::num(n.cpu.allocatable),
        "Memory" => Key::num(n.memory.allocatable),
        "Version" => Key::text(&n.kubelet_version),
        "Age" => Key::age(n.created.as_deref().unwrap_or_default()),
        _ => Key::Missing,
    }
}

pub(crate) fn pct(fraction: f64) -> String {
    let p = fraction * 100.0;
    if p > 0.0 && p < 1.0 { "<1%".to_owned() } else { format!("{p:.0}%") }
}

pub fn fmt_bytes(b: f64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = b;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{v:.0} B") } else { format!("{v:.1} {}", UNITS[unit]) }
}

pub fn fmt_cpu(millicores: f64) -> String {
    if millicores > 0.0 && millicores < 10.0 {
        format!("{millicores:.1}m")
    } else if millicores < 1000.0 {
        format!("{millicores:.0}m")
    } else {
        let cores = millicores / 1000.0;
        match cores {
            1.0 => "1 core".to_owned(),
            c if c.fract() == 0.0 => format!("{c:.0} cores"),
            c => format!("{c:.1} cores"),
        }
    }
}

pub(crate) fn age_ms(ms: f64) -> String {
    let secs = (ms / 1000.0).max(0.0) as u64;
    if secs < 60 { format!("{secs}s") } else { format!("{}m", secs / 60) }
}

#[cfg(test)]
#[path = "overview_tests.rs"]
mod tests;
