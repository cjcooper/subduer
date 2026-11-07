//! Traces' logic: the start dialog's draft and its scope, file names, and
//! the texts of the indicator, the running panel and the saved, quit and
//! switch dialogs.

use subduer_types::{KindRef, TraceScope, TraceStatus, TraceSummary, TraceWorkload};

use crate::logic::trace_chart::local_time;
use crate::prefs::RecentTrace;

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

    /// Another level; the filter starts empty.
    pub(crate) fn set_level(&mut self, level: Level) {
        self.level = level;
        self.filter.clear();
    }

    /// Another namespace to list; the pod and filter start empty.
    pub(crate) fn set_namespace(&mut self, ns: &str) {
        self.namespace = ns.to_owned();
        self.pod.clear();
        self.filter.clear();
    }

    pub(crate) fn toggle_namespace(&mut self, ns: &str) {
        if let Some(i) = self.namespaces.iter().position(|n| n == ns) {
            self.namespaces.remove(i);
        } else {
            self.namespaces.push(ns.to_owned());
            self.namespaces.sort();
        }
    }

    pub(crate) fn toggle_workload(&mut self, w: &TraceWorkload) {
        if let Some(i) = self.workloads.iter().position(|x| x == w) {
            self.workloads.remove(i);
        } else {
            self.workloads.push(w.clone());
        }
    }

    /// The namespace whose workloads or pods must be listed, if any.
    pub(crate) fn targets_namespace(&self) -> Option<&str> {
        (matches!(self.level, Level::Workloads | Level::Pod) && !self.namespace.is_empty()).then_some(self.namespace.as_str())
    }

    /// Whether `name` passes the filter box.
    pub(crate) fn shows(&self, name: &str) -> bool {
        name.to_lowercase().contains(&self.filter.to_lowercase())
    }

    /// Below the list: how many are chosen (and, for workloads, how many of
    /// them are in other namespaces).
    pub(crate) fn chosen_text(&self) -> String {
        match self.level {
            Level::Workloads => {
                let others = self.workloads.iter().filter(|w| w.namespace != self.namespace).count();
                match (self.workloads.len(), others) {
                    (n, 0) => format!("{n} chosen"),
                    (n, o) => format!("{n} chosen ({o} in other namespaces)"),
                }
            }
            _ => format!("{} chosen", self.namespaces.len()),
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
        TraceScope::Namespaces(ns) => more(ns),
        TraceScope::Workloads(ws) => more(&ws.iter().map(|w| w.name.clone()).collect::<Vec<_>>()),
        TraceScope::Pod { name, .. } => name.clone(),
    };
    let name: String = format!("{context}-{what}-{stamp}")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+') { c } else { '-' })
        .collect();
    format!("{name}.trace")
}

/// The first name, and "+N" for the rest.
fn more(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [first, rest @ ..] => format!("{first}+{}", rest.len()),
    }
}

/// `2026-10-11-1405`: unix time `t` at `offset` seconds east of UTC.
pub(crate) fn stamp(t: f64, offset: f64) -> String {
    let d = local_time(t, offset);
    format!("{:04}-{:02}-{:02}-{:02}{:02}", d.year, d.month, d.date, d.hours, d.minutes)
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

/// The span between two times, or "no data" without both.
pub(crate) fn span_text(start: Option<f64>, end: Option<f64>) -> String {
    match (start, end) {
        (Some(a), Some(b)) => duration_text(b - a),
        _ => "no data".into(),
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

/// The indicator's tooltip.
pub(crate) fn indicator_title(status: &TraceStatus) -> String {
    format!("Tracing {} of {} into {}. Click for details.", scope_text(&status.scope), status.context, status.path)
}

/// Something about the running trace needs attention.
pub(crate) fn needs_attention(status: &TraceStatus) -> bool {
    status.last_error.is_some() || status.gaps > 0
}

/// "Trace…" on a Details header: its tooltip.
pub(crate) fn trace_this_title(kind: &str) -> String {
    format!("Record this {kind}'s CPU and memory over time")
}

/// Why the trace dialog can't open: no trace runs and there's no cluster.
pub(crate) fn open_blocked(running: bool, connected: bool) -> Option<&'static str> {
    (!running && !connected).then_some("Connect to a cluster to start a trace.")
}

/// Whether switching to `context` must stop the running trace first.
pub(crate) fn stops_on_switch(status: Option<&TraceStatus>, context: &str) -> bool {
    status.is_some_and(|t| t.context != context)
}

/// The recent-traces entry for a trace file.
pub(crate) fn recent(path: String, summary: &TraceSummary, at: f64) -> RecentTrace {
    RecentTrace { path, context: summary.context.clone(), what: scope_text(&summary.scope), at }
}

/// The running panel's line about pulls from Prometheus.
pub(crate) fn pull_text(s: &TraceStatus, now: f64) -> String {
    match (&s.last_error, s.last_pull, s.prometheus) {
        (_, _, false) => "No Prometheus found: only the app's own samples are kept, while it runs.".to_owned(),
        (Some(e), _, _) => format!("The last pull failed: {e}. The next one tries again."),
        (None, Some(t), _) => format!("Last pulled from Prometheus {}.", ago_text(now - t)),
        (None, None, _) => "Not pulled from Prometheus yet.".to_owned(),
    }
}

/// The running panel's "Pulls" fact, when it pulls.
pub(crate) fn pulls_text(s: &TraceStatus) -> Option<String> {
    (s.prometheus && s.pull_every > 0.0).then(|| format!("Every {} min", (s.pull_every / 60.0).round()))
}

/// The running panel's note about gaps, if any.
pub(crate) fn running_gaps_text(gaps: u32) -> Option<String> {
    match gaps {
        0 => None,
        1 => Some("1 gap: Prometheus no longer had some data when it was pulled.".to_owned()),
        n => Some(format!("{n} gaps: Prometheus no longer had some data when it was pulled.")),
    }
}

/// The running panel's note about pods the app samples itself, if any.
pub(crate) fn sampled_text(pods: u32) -> Option<String> {
    match pods {
        0 => None,
        1 => Some("1 running pod isn't in Prometheus; the app samples it every 30 s.".to_owned()),
        n => Some(format!("{n} running pods aren't in Prometheus; the app samples them every 30 s.")),
    }
}

/// A pull interval choice: `10 min`.
pub(crate) fn every_text(secs: i64) -> String {
    format!("{} min", secs / 60)
}

/// "Trace saved": what the file holds.
pub(crate) fn holds_text(s: &TraceSummary) -> String {
    format!("{} pods, {} containers", s.pods, s.containers)
}

/// "Trace saved": how much came from Prometheus, if anything.
pub(crate) fn pulled_text(s: &TraceSummary) -> Option<String> {
    (s.pulled_bytes > 0).then(|| format!("{} from Prometheus", crate::overview::fmt_bytes(s.pulled_bytes as f64)))
}

/// "Trace saved": its gaps, if any.
pub(crate) fn saved_gaps_text(gaps: usize) -> Option<String> {
    match gaps {
        0 => None,
        1 => Some("1 gap with no data.".to_owned()),
        n => Some(format!("{n} gaps with no data.")),
    }
}

/// The quit dialog's description of the running trace.
pub(crate) fn quit_what(s: &TraceStatus, now: f64) -> String {
    let gaps = match s.gaps {
        0 => String::new(),
        1 => ", 1 gap".into(),
        n => format!(", {n} gaps"),
    };
    format!("{} of {} ({}{gaps})", scope_text(&s.scope), s.context, duration_text(now - s.started))
}

/// The quit dialog's note and its button.
pub(crate) fn quit_texts(quit: bool) -> (&'static str, &'static str) {
    if quit {
        ("Quitting stops it. The file is saved first.", "Stop trace and quit")
    } else {
        ("Closing the window stops it. The file is saved first.", "Stop trace and close")
    }
}

#[cfg(test)]
#[path = "trace_tests.rs"]
mod tests;
