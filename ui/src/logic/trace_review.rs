//! The Traces page's logic: flags and their texts, sorting and filtering
//! review rows, notes, the review's header facts, the selected row's
//! details, and the requests for its charts.

use std::collections::{BTreeSet, HashSet};

use serde::Serialize;
use subduer_types::{ReviewFlag, ReviewRow, TraceEnd, TraceKey, TraceReview, TraceSeries, TraceStatus, TraceSummary};

use crate::logic::rightsize::applicable;
use crate::logic::trace::{duration_text, scope_text};
use crate::logic::trace_chart::{Measure, count_text};
use crate::prefs::RecentTrace;

/// Buckets asked for per chart: about one per two pixels.
pub(crate) const CHART_POINTS: u32 = 400;

/// The file's name without its folder.
pub(crate) fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

// --- Flags ----------------------------------------------------------------------

/// Severity for a flag's badge: critical, serious, warning, or info.
pub(crate) fn flag_level(f: &ReviewFlag) -> &'static str {
    match f {
        ReviewFlag::OomKilled { .. } | ReviewFlag::OomEvents { .. } => "critical",
        ReviewFlag::Throttled { .. } | ReviewFlag::NearMemoryLimit { .. } | ReviewFlag::AboveMemoryLimit => "serious",
        ReviewFlag::Restarts { .. } | ReviewFlag::NoCpuRequest | ReviewFlag::NoMemoryRequest => "warning",
        ReviewFlag::RequestsChanged | ReviewFlag::ShortData { .. } | ReviewFlag::TooLittleData { .. } => "info",
    }
}

fn pct(v: f64) -> String {
    format!("{:.0}%", v * 100.0)
}

fn times(n: u32) -> String {
    format!("×{n}")
}

/// A short label for a badge.
pub(crate) fn flag_text(f: &ReviewFlag) -> String {
    match f {
        ReviewFlag::Throttled { worst } => format!("Throttled {}", pct(*worst)),
        ReviewFlag::NearMemoryLimit { share } => format!("Memory {} of limit", pct(*share)),
        ReviewFlag::OomKilled { count } => format!("OOMKilled {}", times(*count)),
        ReviewFlag::OomEvents { count } => format!("OOM {}", times(*count)),
        ReviewFlag::Restarts { count } => format!("Restarts {}", times(*count)),
        ReviewFlag::NoCpuRequest => "No CPU request".into(),
        ReviewFlag::NoMemoryRequest => "No memory request".into(),
        ReviewFlag::RequestsChanged => "Requests changed".into(),
        ReviewFlag::ShortData { hours } => format!("{} of data", duration_text(hours * 3600.0)),
        ReviewFlag::TooLittleData { hours } => format!("Only {} of data", duration_text(hours * 3600.0)),
        ReviewFlag::AboveMemoryLimit => "Above the memory limit".into(),
    }
}

/// What a flag means, for the detail panel.
pub(crate) fn flag_explain(f: &ReviewFlag) -> String {
    match f {
        ReviewFlag::Throttled { worst } => format!(
            "CPU was throttled by its limit: in the worst minute {} of scheduling periods were held back. Raising or removing the CPU limit helps more than the request.",
            pct(*worst)
        ),
        ReviewFlag::NearMemoryLimit { share } => format!("Peak memory reached {} of the limit. Close to an OOM kill.", pct(*share)),
        ReviewFlag::OomKilled { count } => format!(
            "The container was OOM-killed and restarted {}: it needs more memory than its limit.",
            plural(u64::from(*count), "time", "times")
        ),
        ReviewFlag::OomEvents { count } => {
            format!("The kernel killed a process for memory {} without the container restarting.", plural(u64::from(*count), "time", "times"))
        }
        ReviewFlag::Restarts { count } => format!("The container restarted {} for reasons other than memory.", plural(u64::from(*count), "time", "times")),
        ReviewFlag::NoCpuRequest => "No CPU request is set, so the scheduler assumes it needs none.".into(),
        ReviewFlag::NoMemoryRequest => "No memory request is set, so the scheduler assumes it needs none and it's evicted first under pressure.".into(),
        ReviewFlag::RequestsChanged => "Requests changed during the trace; the figures cover both settings. The current values are shown.".into(),
        ReviewFlag::ShortData { hours } => format!("Only {} of data: daily or weekly peaks may be missing.", duration_text(hours * 3600.0)),
        ReviewFlag::TooLittleData { hours } => {
            format!("Only {} of data, too little to suggest anything. Keep tracing for at least 6 hours, ideally a few days.", duration_text(hours * 3600.0))
        }
        ReviewFlag::AboveMemoryLimit => "The suggested memory request is above the memory limit; the limit must be raised with it.".into(),
    }
}

/// A badge's classes: `flag critical`.
pub(crate) fn flag_class(f: &ReviewFlag) -> String {
    format!("flag {}", flag_level(f))
}

/// Whether suggestions from this review can be applied now: only while
/// connected (`connected`, to `context`) to the cluster it was recorded on.
/// The reason when not.
pub(crate) fn apply_blocked(review: Option<&Result<TraceReview, String>>, connected: bool, context: Option<&str>) -> Option<String> {
    let trace_context = &review?.as_ref().ok()?.summary.context;
    match context {
        Some(c) if c == trace_context && connected => None,
        _ => Some(format!("Connect to {trace_context} to apply suggestions.")),
    }
}

/// Why a row's Apply… isn't offered: its kind, or `blocked`.
pub(crate) fn apply_note(kind: &str, blocked: Option<String>) -> Option<String> {
    if crate::logic::rightsize::applicable_kind(kind) { blocked } else { Some(format!("A {kind}'s requests can't be changed here.")) }
}

/// Sort key: critical first.
/// Flags weighed by severity: one critical outweighs any number of lesser ones.
pub(crate) fn flag_score(r: &ReviewRow) -> f64 {
    r.flags
        .iter()
        .map(|f| match severity(f) {
            0 => 1e6,
            1 => 1e4,
            2 => 1e2,
            _ => 1.0,
        })
        .sum()
}

pub(crate) fn severity(f: &ReviewFlag) -> u8 {
    match flag_level(f) {
        "critical" => 0,
        "serious" => 1,
        "warning" => 2,
        _ => 3,
    }
}

/// `1 pod`, `3 pods`.
pub(crate) fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Rows worth attention: anything but the data-length notes.
fn flagged(row: &ReviewRow) -> bool {
    row.flags.iter().any(|f| flag_level(f) != "info") || row.cpu_spare.is_some_and(|s| s < 0.0) || row.mem_spare.is_some_and(|s| s < 0.0)
}

/// A row's badges in the table: the two most severe non-info flags, and how
/// many more there are (the detail panel lists them all).
pub(crate) fn shown_flags(row: &ReviewRow) -> (Vec<&ReviewFlag>, usize) {
    let mut shown: Vec<&ReviewFlag> = row.flags.iter().filter(|f| flag_level(f) != "info").collect();
    shown.sort_by_key(|f| severity(f));
    let more = shown.len().saturating_sub(2);
    shown.truncate(2);
    (shown, more)
}

/// The badge for flags not shown: `+2`.
pub(crate) fn more_text(more: usize) -> Option<String> {
    (more > 0).then(|| format!("+{more}"))
}

// --- Sorting and filtering ------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SortBy {
    Workload,
    Pods,
    CpuRequest,
    CpuP95,
    CpuSpare,
    MemRequest,
    MemPeak,
    MemSpare,
    /// The suggested CPU request, then memory.
    Suggested,
    /// The most severe flags first when descending.
    Flags,
}

/// A row's number under a column; `None` when missing, and for the
/// workload, which sorts by name.
pub(crate) fn sort_value(r: &ReviewRow, by: SortBy) -> Option<f64> {
    match by {
        SortBy::Workload => None,
        SortBy::Pods => Some(r.avg_pods),
        SortBy::CpuRequest => r.cpu_request,
        SortBy::CpuP95 => Some(r.cpu_p95),
        SortBy::CpuSpare => r.cpu_spare,
        SortBy::MemRequest => r.mem_request,
        SortBy::MemPeak => Some(r.mem_max),
        SortBy::MemSpare => r.mem_spare,
        SortBy::Suggested => r.suggestion.as_ref().map(|s| s.cpu_request * 1e12 + s.mem_request / 1e6),
        SortBy::Flags => Some(flag_score(r)),
    }
}

/// Sorts rows; numbers descending by default, names ascending. Missing values go last.
pub(crate) fn sort_rows(rows: &mut [ReviewRow], by: SortBy, descending: bool) {
    let num = |r: &ReviewRow| sort_value(r, by);
    rows.sort_by(|a, b| {
        if by == SortBy::Workload {
            let ord = (&a.key.name, &a.key.container, &a.key.namespace).cmp(&(&b.key.name, &b.key.container, &b.key.namespace));
            return if descending { ord.reverse() } else { ord };
        }
        match (num(a), num(b)) {
            (Some(x), Some(y)) => {
                let ord = x.total_cmp(&y);
                if descending { ord.reverse() } else { ord }
            }
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
    });
}

/// A click on a column's heading: the same column flips; another starts
/// descending, except the workload name.
pub(crate) fn next_sort((current, descending): (SortBy, bool), by: SortBy) -> (SortBy, bool) {
    if current == by { (by, !descending) } else { (by, by != SortBy::Workload) }
}

/// The arrow after a heading.
pub(crate) fn sort_arrow((current, descending): (SortBy, bool), by: SortBy) -> &'static str {
    if current != by {
        ""
    } else if descending {
        " ↓"
    } else {
        " ↑"
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Filter {
    pub(crate) namespace: Option<String>,
    pub(crate) text: String,
    pub(crate) flagged_only: bool,
}

pub(crate) fn filter_rows(rows: &[ReviewRow], f: &Filter) -> Vec<ReviewRow> {
    let text = f.text.trim().to_lowercase();
    rows.iter()
        .filter(|r| f.namespace.as_ref().is_none_or(|ns| *ns == r.key.namespace))
        .filter(|r| {
            text.is_empty() || [&r.key.name, &r.key.container, &r.key.namespace, &r.key.kind].iter().any(|s| s.to_lowercase().contains(&text))
        })
        .filter(|r| !f.flagged_only || flagged(r))
        .cloned()
        .collect()
}

/// The namespace choice: "" is all.
pub(crate) fn namespace_choice(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

/// The table's rows: filtered, then sorted.
pub(crate) fn visible_rows(review: Option<&Result<TraceReview, String>>, filter: &Filter, (by, descending): (SortBy, bool)) -> Vec<ReviewRow> {
    match review {
        Some(Ok(r)) => {
            let mut rows = filter_rows(&r.rows, filter);
            sort_rows(&mut rows, by, descending);
            rows
        }
        _ => vec![],
    }
}

/// The namespaces of a review's rows, each once, sorted.
pub(crate) fn namespaces(review: Option<&Result<TraceReview, String>>) -> Vec<String> {
    let mut ns: Vec<String> = review.into_iter().flatten().flat_map(|r| r.rows.iter().map(|row| row.key.namespace.clone())).collect();
    ns.sort();
    ns.dedup();
    ns
}

/// `−99%`, `+12%`: the change from `from` to `to`.
pub(crate) fn change_text(from: f64, to: f64) -> String {
    if from <= 0.0 {
        return String::new();
    }
    let p = (to - from) / from * 100.0;
    if p.abs() < 0.5 { "no change".into() } else if p < 0.0 { format!("−{:.0}%", -p) } else { format!("+{p:.0}%") }
}

/// Spare capacity: `None` without a request; otherwise whether usage is
/// above it ("short", in the danger ink) and the text.
pub(crate) fn spare(v: Option<f64>, measure: Measure) -> Option<(bool, String)> {
    v.map(|v| if v < 0.0 { (true, format!("short {}", measure.fmt(-v))) } else { (false, measure.fmt(v)) })
}

/// A row's suggestion in the table: `15m · 64 MiB`.
pub(crate) fn suggestion_text(row: &ReviewRow) -> String {
    match &row.suggestion {
        Some(s) => format!("{} · {}", Measure::Cpu.fmt(s.cpu_request), Measure::Memory.fmt(s.mem_request)),
        None => "—".into(),
    }
}

/// Below a row's name: `Deployment · app · shop`.
pub(crate) fn key_subtitle(key: &TraceKey) -> String {
    format!("{} · {} · {}", key.kind, key.container, key.namespace)
}

/// The detail panel's subtitle: `Deployment · container app · shop`.
pub(crate) fn detail_subtitle(key: &TraceKey) -> String {
    format!("{} · container {} · {}", key.kind, key.container, key.namespace)
}

/// A row's checkbox tooltip.
pub(crate) fn check_title(can: bool) -> &'static str {
    if can { "Choose for Apply selected" } else { "No suggestion that can be applied" }
}

/// Adds `item` to the set, or takes it out if there.
pub(crate) fn toggle<T: std::hash::Hash + Eq + Clone>(set: &mut HashSet<T>, item: &T) {
    if !set.remove(item) {
        set.insert(item.clone());
    }
}

/// Adds `item` to the ordered set, or takes it out if there.
pub(crate) fn toggle_ordered<T: Ord + Clone>(set: &mut BTreeSet<T>, item: &T) {
    if !set.remove(item) {
        set.insert(item.clone());
    }
}

/// The keys of rows that can be applied.
pub(crate) fn applicable_keys(rows: Vec<ReviewRow>) -> Vec<TraceKey> {
    rows.into_iter().filter(applicable).map(|r| r.key).collect()
}

/// Whether every one of `shown` is checked (and there is one).
pub(crate) fn all_checked(shown: &[TraceKey], checked: &HashSet<TraceKey>) -> bool {
    !shown.is_empty() && shown.iter().all(|k| checked.contains(k))
}

/// The heading's checkbox: unchecks all shown if all are, else checks them.
pub(crate) fn toggle_all(checked: &mut HashSet<TraceKey>, shown: Vec<TraceKey>) {
    if shown.iter().all(|k| checked.contains(k)) {
        for k in &shown {
            checked.remove(k);
        }
    } else {
        checked.extend(shown);
    }
}

/// The checked rows of a review, in its order.
pub(crate) fn checked_rows(review: Option<&Result<TraceReview, String>>, checked: &HashSet<TraceKey>) -> Vec<ReviewRow> {
    match review {
        Some(Ok(r)) => r.rows.iter().filter(|row| checked.contains(&row.key)).cloned().collect(),
        _ => vec![],
    }
}

/// Apply selected (n)…: its tooltip.
pub(crate) fn apply_selected_title(blocked: Option<String>) -> String {
    blocked.unwrap_or_else(|| "Preview, then apply the suggested requests".into())
}

pub(crate) fn apply_selected_text(n: usize) -> String {
    format!("Apply selected ({n})…")
}

/// How many rows are shown, and a hint to select one.
pub(crate) fn rows_text(n: usize, none_selected: bool) -> String {
    let hint = if none_selected && n > 0 { " · select one for its charts" } else { "" };
    format!("{}{hint}", plural(n as u64, "row", "rows"))
}

/// Whether the selection survives a reload: its row is still there.
pub(crate) fn keeps_selection(review: &TraceReview, sel: &TraceKey) -> bool {
    review.rows.iter().any(|row| row.key == *sel)
}

/// The selected row.
pub(crate) fn find_row(review: Option<&Result<TraceReview, String>>, sel: &TraceKey) -> Option<ReviewRow> {
    review?.as_ref().ok()?.rows.iter().find(|row| row.key == *sel).cloned()
}

/// Compare with…: the two files, unless the same one was chosen.
pub(crate) fn compare_pair(current: String, other: String) -> Result<Vec<String>, &'static str> {
    if other != current { Ok(vec![current, other]) } else { Err("Choose a different trace to compare with.") }
}

// --- Notes ------------------------------------------------------------------------

/// Puts a saved note into the open review: on the trace, or on every row of the workload.
pub(crate) fn save_note(review: &mut TraceReview, key: Option<&TraceKey>, text: &str) {
    let note = (!text.trim().is_empty()).then(|| text.trim().to_owned());
    match key {
        None => review.note = note,
        Some(k) => {
            for row in review.rows.iter_mut().filter(|row| row.key.namespace == k.namespace && row.key.kind == k.kind && row.key.name == k.name) {
                row.note = note.clone();
            }
        }
    }
}

/// What a note's unsaved edits are called when leaving.
pub(crate) fn note_what(key: Option<&TraceKey>) -> String {
    match key {
        None => "the note on this trace".to_owned(),
        Some(k) => format!("the note on {}", k.name),
    }
}

/// The unsaved-edits name while a note's text differs from the saved one.
pub(crate) fn note_dirty(draft: &str, original: &str, what: &str) -> Option<String> {
    (draft.trim() != original.trim()).then(|| what.to_owned())
}

/// `Add a note on this trace…`.
pub(crate) fn add_note_text(label: &str) -> String {
    format!("Add a {}…", label.to_lowercase())
}

// --- The page ----------------------------------------------------------------------

/// The running trace's card on the list.
pub(crate) fn recording_text(s: &TraceStatus) -> String {
    format!("Recording {} of {}", scope_text(&s.scope), s.context)
}

/// A recent trace's card; `when` is its time, written out.
pub(crate) fn recent_text(t: &RecentTrace, when: &str) -> String {
    format!("{} · {} · {when}", t.context, t.what)
}

pub(crate) fn compare_text(n: usize) -> String {
    format!("Compare {n}…")
}

/// Whether the review's file is still being recorded.
pub(crate) fn recording(review: Option<&Result<TraceReview, String>>) -> bool {
    matches!(review, Some(Ok(r)) if r.summary.ended.is_none() && !r.summary.unfinished)
}

/// The header's State.
pub(crate) fn state_text(s: &TraceSummary) -> String {
    if s.unfinished {
        "Ended unexpectedly (data up to the last pull is intact)".to_owned()
    } else if s.ended.is_none() {
        "Still recording".to_owned()
    } else {
        match s.end_reason {
            Some(TraceEnd::Quit) => "Stopped when the app quit".into(),
            Some(TraceEnd::ContextChanged) => "Stopped on a context switch".into(),
            Some(TraceEnd::ConnectionLost) => "Stopped: connection lost".into(),
            _ => "Stopped".into(),
        }
    }
}

/// The header's "Data from".
pub(crate) fn source_text(s: &TraceSummary) -> String {
    let local = s.local_samples as f64 / (s.local_samples + s.prometheus_samples).max(1) as f64;
    if s.prometheus.is_none() {
        "Sampled by the app (no Prometheus)".to_owned()
    } else if local > 0.0 {
        format!("Prometheus, {} sampled by the app", if local < 0.01 { "<1%".into() } else { format!("{:.0}%", local * 100.0) })
    } else {
        "Prometheus".to_owned()
    }
}

/// The header's Gaps.
pub(crate) fn gaps_text(gaps: usize) -> String {
    if gaps == 0 { "None".to_owned() } else { gaps.to_string() }
}

pub(crate) fn report_error_text(e: &str) -> String {
    format!("The report wasn't saved: {e}")
}

pub(crate) fn overview_title(namespace: Option<&str>) -> String {
    match namespace {
        Some(ns) => format!("Overview of {ns}"),
        None => "Overview of the whole trace".into(),
    }
}

/// `trace_series`: one row's series over `range` (all of it when `None`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SeriesArgs {
    pub(crate) path: String,
    pub(crate) key: TraceKey,
    pub(crate) start: Option<f64>,
    pub(crate) end: Option<f64>,
    pub(crate) points: u32,
}

impl SeriesArgs {
    pub(crate) fn new(path: String, key: TraceKey, range: Option<(f64, f64)>, points: u32) -> Self {
        Self { path, key, start: range.map(|r| r.0), end: range.map(|r| r.1), points }
    }
}

/// `trace_overview`: every container added up, of the whole trace or one
/// namespace, over `range`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct OverviewArgs {
    pub(crate) path: String,
    pub(crate) namespace: Option<String>,
    pub(crate) start: Option<f64>,
    pub(crate) end: Option<f64>,
    pub(crate) points: u32,
}

impl OverviewArgs {
    pub(crate) fn new(path: String, namespace: Option<String>, range: Option<(f64, f64)>) -> Self {
        Self { path, namespace, start: range.map(|r| r.0), end: range.map(|r| r.1), points: CHART_POINTS }
    }
}

/// A quick range button ("Last 6h", or All for `None`) is on: the zoom is
/// that span up to the trace's end.
pub(crate) fn quick_active(hours: Option<f64>, zoom: Option<(f64, f64)>, series: Option<&TraceSeries>) -> bool {
    match (hours, zoom, series) {
        (None, None, _) => true,
        (Some(h), Some((a, b)), Some(s)) => (b - s.full_end).abs() < 1.0 && (b - a - h * 3600.0).abs() < 1.0,
        _ => false,
    }
}

/// What a quick range button zooms to: `Some(None)` for all, nothing until
/// the series is known.
pub(crate) fn quick_zoom(hours: Option<f64>, series: Option<&TraceSeries>) -> Option<Option<(f64, f64)>> {
    match (hours, series) {
        (None, _) => Some(None),
        (Some(h), Some(s)) => Some(Some(((s.full_end - h * 3600.0).max(s.full_start), s.full_end))),
        _ => None,
    }
}

/// One line of the suggestion card: (what, from, to, change).
pub(crate) fn suggest_line(m: Measure, now: Option<f64>, to: f64) -> (String, String, String, String) {
    let change = now.map(|n| change_text(n, to)).unwrap_or_default();
    (format!("{} request", m.label()), m.opt(now), m.fmt(to), change)
}

pub(crate) fn basis_text(basis: &str) -> String {
    format!("Based on {basis}.")
}

/// The line under the figures: pods, data, rollouts and the app's share.
pub(crate) fn facts_text(row: &ReviewRow) -> String {
    format!(
        "{} seen, {:.1} running on average · {} of data · {}{}",
        plural(u64::from(row.pods), "pod", "pods"),
        row.avg_pods,
        duration_text(row.minutes as f64 * 60.0),
        plural(u64::from(row.rollouts), "rollout", "rollouts"),
        if row.local_share > 0.0 { format!(" · {:.0}% sampled by the app", row.local_share * 100.0) } else { String::new() },
    )
}

/// Pods running on average: `2`, `1.5`.
pub(crate) fn pods_text(row: &ReviewRow) -> String {
    count_text(row.avg_pods)
}

#[cfg(test)]
#[path = "trace_review_tests.rs"]
mod tests;
