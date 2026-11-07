//! Comparing traces' logic: matching rows' ends, sorting and filtering
//! them, their cells' texts, and the overlay's lines.

use subduer_types::{CompareCell, CompareRow, TraceSeries, TraceSummary};

use crate::logic::trace::duration_text;
use crate::logic::trace_chart::{Measure, count_text};
use crate::logic::trace_overlay::{Align, Line, MAX_TRACES, picked, shift, trace_slot};
use crate::logic::trace_review::{change_text, file_name};

/// Buckets per trace on an overlay.
pub(crate) const OVERLAY_POINTS: u32 = 400;

/// The first and last traces that have the row.
pub(crate) fn ends(row: &CompareRow) -> Option<(&CompareCell, &CompareCell)> {
    let mut present = row.cells.iter().flatten();
    let first = present.next()?;
    Some((first, present.last().unwrap_or(first)))
}

/// The sortable headings: `Workload`, `Trace 1` … `Trace n`, `Change`.
pub(crate) fn headers(n: usize) -> Vec<String> {
    std::iter::once("Workload".to_owned()).chain((0..n).map(trace_header)).chain(["Change".to_owned()]).collect()
}

/// The heading of trace `i` (0-based): `Trace 1`.
pub(crate) fn trace_header(i: usize) -> String {
    format!("Trace {}", i + 1)
}

/// How a row sorts under column `i` of `n` traces: the workload by name, a
/// trace by its CPU request, Change by how much the request changed.
pub(crate) fn row_key(row: &CompareRow, i: usize, n: usize) -> crate::sort::Key {
    use crate::sort::Key;
    match i {
        0 => Key::text(&format!("{} {} {}", row.key.name, row.key.container, row.key.namespace)),
        i if i <= n => row.cells.get(i - 1).and_then(Option::as_ref).map_or(Key::Missing, |c| Key::opt(c.cpu_request.or(Some(c.cpu_p95)))),
        _ => {
            let Some((a, b)) = ends(row) else { return Key::Missing };
            let ratio = |x: Option<f64>, y: Option<f64>| match (x, y) {
                (Some(x), Some(y)) if x > 0.0 => Some((y - x) / x),
                _ => None,
            };
            Key::opt(ratio(a.cpu_request, b.cpu_request).or_else(|| ratio(a.mem_request, b.mem_request)))
        }
    }
}

/// Sorts rows by the chosen column (index and descending) of `n` traces.
pub(crate) fn sorted(mut rows: Vec<CompareRow>, by: Option<(usize, bool)>, n: usize) -> Vec<CompareRow> {
    crate::sort::sort(&mut rows, by, |r, i| row_key(r, i, n));
    rows
}

/// A request or limit differs between the traces that have the row.
pub(crate) fn requests_changed(row: &CompareRow) -> bool {
    let mut cells = row.cells.iter().flatten();
    let Some(first) = cells.next() else { return false };
    cells.any(|c| c.cpu_request != first.cpu_request || c.mem_request != first.mem_request)
}

/// Rows matching the text, and only changed ones if asked.
pub(crate) fn filter(rows: &[CompareRow], text: &str, changed_only: bool) -> Vec<CompareRow> {
    let text = text.trim().to_lowercase();
    rows.iter()
        .filter(|r| text.is_empty() || [&r.key.name, &r.key.container, &r.key.namespace].iter().any(|s| s.to_lowercase().contains(&text)))
        .filter(|r| !changed_only || requests_changed(r))
        .cloned()
        .collect()
}

/// `1.5 cores → 15m (−99%)`, or the value alone when it didn't change.
pub(crate) fn from_to(a: Option<f64>, b: Option<f64>, m: Measure) -> String {
    match (a, b) {
        (Some(a), Some(b)) if a == b => m.fmt(a),
        (Some(a), Some(b)) => format!("{} → {} ({})", m.fmt(a), m.fmt(b), change_text(a, b)),
        (None, Some(b)) => format!("not set → {}", m.fmt(b)),
        (Some(a), None) => format!("{} → not set", m.fmt(a)),
        (None, None) => "not set".into(),
    }
}

/// One trace's cell: its CPU line, memory line and pods line.
pub(crate) fn cell_lines(c: &CompareCell) -> (String, String, String) {
    (
        format!("{} · p95 {}", Measure::Cpu.opt(c.cpu_request), Measure::Cpu.fmt(c.cpu_p95)),
        format!("{} · peak {}", Measure::Memory.opt(c.mem_request), Measure::Memory.fmt(c.mem_max)),
        format!("{} pods · {}", count_text(c.avg_pods), duration_text(c.hours * 3600.0)),
    )
}

/// A cell's badges: OOM kills and the worst throttling, if any.
pub(crate) fn cell_flags(c: &CompareCell) -> (Option<String>, Option<String>) {
    ((c.ooms > 0).then(|| format!("OOM ×{}", c.ooms)), c.throttled.map(|t| format!("Throttled {:.0}%", t * 100.0)))
}

/// "Only in trace 1, 3" when a row isn't in every one of the `n` traces.
pub(crate) fn only_text(row: &CompareRow, n: usize) -> Option<String> {
    let present: Vec<String> = row.cells.iter().enumerate().filter(|(_, c)| c.is_some()).map(|(i, _)| (i + 1).to_string()).collect();
    (present.len() < n).then(|| format!("Only in trace {}", present.join(", ")))
}

/// The Change column: (CPU request, p95, memory request, peak).
pub(crate) fn change_lines(a: &CompareCell, b: &CompareCell) -> [String; 4] {
    [
        from_to(a.cpu_request, b.cpu_request, Measure::Cpu),
        from_to(Some(a.cpu_p95), Some(b.cpu_p95), Measure::Cpu),
        from_to(a.mem_request, b.mem_request, Measure::Memory),
        from_to(Some(a.mem_max), Some(b.mem_max), Measure::Memory),
    ]
}

/// A trace's heading: `12/10/2026 · 2d 4h · acme-uat`; `started` is its date, written out.
pub(crate) fn head_text(t: &TraceSummary, started: &str) -> String {
    format!("{started} · {} · {}", crate::logic::trace::span_text(t.data_start, t.data_end), t.context)
}

/// Above the table: how many rows show.
pub(crate) fn count_text_of(shown: usize, of: usize) -> String {
    format!("{shown} of {of} rows · select one to overlay its charts")
}

pub(crate) fn title(paths: Option<&Vec<String>>) -> String {
    format!("Comparing {} traces", paths.map_or(0, Vec::len))
}

pub(crate) fn change_heading(n: usize) -> String {
    format!("Change, 1 → {n}")
}

/// The overlaid row: another click on it closes the overlay.
pub(crate) fn pick<T: PartialEq>(current: &mut Option<T>, key: T) {
    *current = if current.as_ref() == Some(&key) { None } else { Some(key) };
}

/// When a trace's data starts.
pub(crate) fn data_start(t: &TraceSummary) -> f64 {
    t.data_start.unwrap_or(t.started)
}

/// "Showing traces 1, 4 and 5 of 5", when the overlay can't hold them all.
pub(crate) fn shown_text(shown: &[usize], of: usize) -> Option<String> {
    (shown.len() < of).then(|| {
        let mut n: Vec<String> = shown.iter().map(|i| (i + 1).to_string()).collect();
        let last = n.pop().unwrap_or_default();
        format!("Showing traces {} and {last} of {of}: an overlay holds {MAX_TRACES} at most.", n.join(", "))
    })
}

/// The comparison's overlaid traces that have the row (`present` per trace).
pub(crate) fn overlaid(n: usize, present: &[bool]) -> Vec<usize> {
    picked(n).into_iter().filter(|&i| present.get(i) == Some(&true)).collect()
}

/// The overlay's lines from the series loaded so far (by trace index):
/// each trace's start and its local offset in `starts`, lined up on the
/// first shown trace's (`first`, also with its offset).
pub(crate) fn lines(loaded: &[(usize, Result<TraceSeries, String>)], starts: &[(f64, f64)], first: (f64, f64), align: Align) -> Vec<Line> {
    let n = starts.len();
    let mut lines: Vec<Line> = loaded
        .iter()
        .filter_map(|(i, r)| {
            let series = r.as_ref().ok()?.clone();
            let (start, offset) = starts[*i];
            Some(Line { n: i + 1, slot: trace_slot(*i, n)?, series, shift: shift(align, start, offset, first.0, first.1) })
        })
        .collect();
    lines.sort_by_key(|l| l.n);
    lines
}

/// The series that failed to load: `Trace 2: …`.
pub(crate) fn errors(loaded: &[(usize, Result<TraceSeries, String>)]) -> String {
    loaded.iter().filter_map(|(i, r)| r.as_ref().err().map(|e| format!("Trace {}: {e}", i + 1))).collect::<Vec<_>>().join(" ")
}

/// A legend entry; `started` is its date, written out.
pub(crate) fn legend_text(i: usize, t: &TraceSummary, started: &str) -> String {
    format!("Trace {} · {} · {started}", i + 1, file_name(&t.path))
}

#[cfg(test)]
#[path = "compare_tests.rs"]
mod tests;
