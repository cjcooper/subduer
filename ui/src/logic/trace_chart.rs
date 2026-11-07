//! The trace charts' logic: measures, step series, local times, paths, the
//! line chart's frame and texts, and the day-and-hour grid's levels.

use subduer_types::{Series, TraceDay, TraceEventMark, TraceSeries};

use crate::chart::{Scale, nice_ceil};
use crate::overview::{fmt_bytes, fmt_cpu};

pub(crate) const W: f64 = 1000.0;
pub(crate) const H: f64 = 100.0;
/// Room above the top value for the reference labels.
const HEADROOM: f64 = 12.0;
/// Room below zero for the event markers.
const FOOT: f64 = 8.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Measure {
    Cpu,
    Memory,
}

impl Measure {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::Memory => "Memory",
        }
    }

    pub(crate) fn class(self) -> &'static str {
        match self {
            Self::Cpu => "res-cpu",
            Self::Memory => "res-memory",
        }
    }

    pub(crate) fn fmt(self, v: f64) -> String {
        match self {
            Self::Cpu => fmt_cpu(v),
            Self::Memory => fmt_bytes(v),
        }
    }

    pub(crate) fn scale(self) -> Scale {
        match self {
            Self::Cpu => Scale::Decimal,
            Self::Memory => Scale::Binary,
        }
    }

    /// Average pod, busiest pod, request and limit.
    pub(crate) fn pick(self, s: &TraceSeries) -> (&Series, &Series, &Series, &Series) {
        match self {
            Self::Cpu => (&s.cpu_avg, &s.cpu_max, &s.cpu_request, &s.cpu_limit),
            Self::Memory => (&s.mem_avg, &s.mem_max, &s.mem_request, &s.mem_limit),
        }
    }

    /// An optional value, or a dash.
    pub(crate) fn opt(self, v: Option<f64>) -> String {
        v.map_or("—".into(), |v| self.fmt(v))
    }
}

/// The value of a step series (change points) at `t`, if set by then.
pub(crate) fn step_at(changes: &Series, t: f64) -> Option<f64> {
    changes.iter().take_while(|p| p.0 <= t).last().map(|p| p.1)
}

/// Horizontal runs of a step series over `[start, end]`: (from, to, value).
pub(crate) fn step_runs(changes: &Series, start: f64, end: f64) -> Vec<(f64, f64, f64)> {
    let mut runs = Vec::new();
    for (i, &(t, v)) in changes.iter().enumerate() {
        let next = changes.get(i + 1).map_or(end, |p| p.0);
        let (from, to) = (t.max(start), next.min(end));
        if to > from {
            runs.push((from, to, v));
        }
    }
    runs
}

/// A moment's local calendar date and clock time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LocalTime {
    pub(crate) year: i64,
    /// 1–12.
    pub(crate) month: u32,
    /// Day of the month, 1–31.
    pub(crate) date: u32,
    /// 0 is Sunday.
    pub(crate) weekday: u32,
    pub(crate) hours: u32,
    pub(crate) minutes: u32,
}

/// Unix time `t` (seconds) at `offset` seconds east of UTC.
pub(crate) fn local_time(t: f64, offset: f64) -> LocalTime {
    let secs = (t + offset).floor() as i64;
    let (days, of_day) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    // Days to a civil date (proleptic Gregorian), after Howard Hinnant.
    let z = days + 719_468;
    let (era, doe) = (z.div_euclid(146_097), z.rem_euclid(146_097));
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let date = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    LocalTime { year, month, date, weekday: (days + 4).rem_euclid(7) as u32, hours: (of_day / 3600) as u32, minutes: (of_day % 3600 / 60) as u32 }
}

pub(crate) const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// A time for axis labels and tips: `14:05`, or `Mon 12 14:05` over long
/// spans; `offset` is the local offset at `t`, in seconds east of UTC.
pub(crate) fn when(t: f64, long: bool, offset: f64) -> String {
    let d = local_time(t, offset);
    let clock = format!("{:02}:{:02}", d.hours, d.minutes);
    if long { format!("{} {} {clock}", DAYS[d.weekday as usize], d.date) } else { clock }
}

/// Line and area paths for a series, broken where buckets are missing.
pub(crate) fn paths(points: &Series, step: f64, x: impl Fn(f64) -> f64, y: impl Fn(f64) -> f64, base: f64) -> (String, String) {
    let mut line = String::new();
    let mut area = String::new();
    let mut seg: Vec<(f64, f64)> = vec![];
    let mut flush = |seg: &mut Vec<(f64, f64)>| {
        if seg.is_empty() {
            return;
        }
        let coords: Vec<String> = seg.iter().map(|(px, py)| format!("{px:.1},{py:.2}")).collect();
        line.push_str(&format!("M{}", coords.join("L")));
        area.push_str(&format!("M{:.1},{base}L{}L{:.1},{base}Z", seg[0].0, coords.join("L"), seg[seg.len() - 1].0));
        seg.clear();
    };
    let mut prev: Option<f64> = None;
    for &(t, v) in points {
        if prev.is_some_and(|p| t - p > step * 1.5) {
            flush(&mut seg);
        }
        // A bucket's value spans the bucket: draw it at its middle.
        seg.push((x(t + step / 2.0), y(v)));
        prev = Some(t);
    }
    flush(&mut seg);
    (line, area)
}

/// Horizontal step lines: `M x,y H x` per run.
pub(crate) fn step_path(runs: &[(f64, f64, f64)], x: impl Fn(f64) -> f64, y: impl Fn(f64) -> f64) -> String {
    runs.iter().map(|&(a, b, v)| format!("M{:.1},{:.2}H{:.1}", x(a), y(v), x(b))).collect()
}

/// What a chart's lines stand for: one row's pods, or every container added
/// up (a review's overview).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Names {
    pub(crate) avg: &'static str,
    pub(crate) peak: &'static str,
    pub(crate) request: &'static str,
    pub(crate) count: &'static str,
}

impl Names {
    pub(crate) fn of(totals: bool) -> Self {
        if totals {
            Self { avg: "used", peak: "busiest minute", request: "requested", count: "containers" }
        } else {
            Self { avg: "average pod", peak: "busiest pod", request: "request", count: "pods" }
        }
    }
}

/// Requested but not used, and its share of the request; `None` when usage
/// is at or above the request.
pub(crate) fn idle(used: f64, requested: f64) -> Option<(f64, f64)> {
    (requested > 0.0 && used < requested).then(|| (requested - used, (requested - used) / requested))
}

/// A share as a whole percentage, never rounded up to 100% while some is used.
pub(crate) fn share_text(share: f64) -> String {
    if (0.995..1.0).contains(&share) { "over 99%".into() } else { format!("{:.0}%", share * 100.0) }
}

/// The tooltip's idle line: `750m (75% of requested)`.
pub(crate) fn idle_text(measure: Measure, used: Option<f64>, requested: Option<f64>) -> Option<String> {
    let (v, share) = idle(used?, requested?)?;
    Some(format!("{} ({} of requested)", measure.fmt(v), share_text(share)))
}

/// Sentence case, for legends: "busiest pod" → "Busiest pod".
pub(crate) fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// A count with one decimal, without a trailing `.0`: `2`, `1.5`.
pub(crate) fn count_text(n: f64) -> String {
    format!("{n:.1}").trim_end_matches(".0").to_owned()
}

/// Where a pointer is across an element, as a fraction of its width.
pub(crate) fn fraction(client_x: f64, left: f64, width: f64) -> Option<f64> {
    (width > 0.0).then(|| ((client_x - left) / width).clamp(0.0, 1.0))
}

/// A drag from `a` to `b` (fractions) worth zooming into: (low, high).
pub(crate) fn zoom_span(a: f64, b: f64) -> Option<(f64, f64)> {
    let (lo, hi) = (a.min(b), a.max(b));
    (hi - lo > 0.02).then_some((lo, hi))
}

/// The selection drawn while dragging: (left %, width %).
pub(crate) fn selection(drag: Option<(f64, f64)>) -> Option<(f64, f64)> {
    drag.filter(|(a, b)| (a - b).abs() > 0.005).map(|(a, b)| {
        let (lo, hi) = (a.min(b), a.max(b));
        (lo * 100.0, (hi - lo) * 100.0)
    })
}

/// A tooltip's side: right-aligned past 60% of the width.
pub(crate) fn tip_side(hx: f64) -> String {
    if hx > 60.0 { format!("right: {:.2}%", 100.0 - hx) } else { format!("left: {hx:.2}%") }
}

/// A CSS `left` percentage.
pub(crate) fn left_style(pct: f64) -> String {
    format!("left: {pct:.2}%")
}

/// A dot at (left %, top %).
pub(crate) fn dot_style(left: f64, top: f64) -> String {
    format!("left: {left:.2}%; top: {top:.2}%")
}

/// A span from `left` % across `width` %.
pub(crate) fn span_style(left: f64, width: f64) -> String {
    format!("left: {left:.2}%; width: {width:.2}%")
}

/// A label `px` pixels above a CSS `top` percentage.
pub(crate) fn top_style(pct: f64, px: i32) -> String {
    format!("top: calc({pct:.2}% - {px}px)")
}

/// The time at fraction `f` of a series' span.
pub(crate) fn time_at(s: &TraceSeries, f: f64) -> f64 {
    s.start + f * (s.end - s.start)
}

/// The line chart's axes: times across, values up, with room for labels
/// above and markers below.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Frame {
    pub(crate) start: f64,
    pub(crate) end: f64,
    pub(crate) span: f64,
    pub(crate) top: f64,
}

impl Frame {
    pub(crate) const BASE: f64 = H - FOOT;

    pub(crate) fn x(&self, t: f64) -> f64 {
        ((t - self.start) / self.span * W).clamp(0.0, W)
    }

    pub(crate) fn y(&self, v: f64) -> f64 {
        Self::BASE - (v / self.top).clamp(0.0, 1.0) * (Self::BASE - HEADROOM)
    }

    /// `x` as a percentage of the width.
    pub(crate) fn xp(&self, t: f64) -> f64 {
        self.x(t) / W * 100.0
    }

    /// `y` as a percentage of the height.
    pub(crate) fn yp(&self, v: f64) -> f64 {
        self.y(v) / H * 100.0
    }

    /// Over 20 hours: axis labels carry the day.
    pub(crate) fn long(&self) -> bool {
        self.span > 20.0 * 3600.0
    }

    /// The grid lines' values, and the labelled ones.
    pub(crate) fn grid(&self) -> [f64; 3] {
        [0.0, self.top / 2.0, self.top]
    }

    pub(crate) fn ticks(&self) -> [f64; 2] {
        [self.top / 2.0, self.top]
    }
}

/// Everything the line chart draws from a series, before markup.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Chart {
    pub(crate) frame: Frame,
    pub(crate) request_runs: Vec<(f64, f64, f64)>,
    pub(crate) limit_runs: Vec<(f64, f64, f64)>,
    pub(crate) avg_line: String,
    pub(crate) avg_area: String,
    pub(crate) peak_line: String,
    pub(crate) request_path: String,
    pub(crate) limit_path: String,
}

/// The chart of `measure` in `s`; `None` with no usage to draw.
pub(crate) fn chart(s: &TraceSeries, measure: Measure) -> Option<Chart> {
    let (avg, peak, request, limit) = measure.pick(s);
    if avg.is_empty() && peak.is_empty() {
        return None;
    }
    let (start, end) = (s.start, s.end);
    let request_runs = step_runs(request, start, end);
    let limit_runs = step_runs(limit, start, end);
    let top_value = peak
        .iter()
        .chain(avg.iter())
        .map(|p| p.1)
        .chain(request_runs.iter().chain(&limit_runs).map(|r| r.2))
        .fold(0.0, f64::max);
    let frame = Frame { start, end, span: (end - start).max(1.0), top: nice_ceil(top_value * 1.05, measure.scale()) };
    let (x, y) = (|t| frame.x(t), |v| frame.y(v));
    let (avg_line, avg_area) = paths(avg, s.step, x, y, Frame::BASE);
    let (peak_line, _) = paths(peak, s.step, x, y, Frame::BASE);
    let (request_path, limit_path) = (step_path(&request_runs, x, y), step_path(&limit_runs, x, y));
    Some(Chart { frame, request_runs, limit_runs, avg_line, avg_area, peak_line, request_path, limit_path })
}

/// Labels for the request and limit lines: (text with `{}` for the value,
/// value, pixels to move down). Equal values share a label; values closer
/// than a tenth of the chart put the lower label below its line.
pub(crate) fn ref_labels(request: Option<f64>, limit: Option<f64>, top: f64) -> Vec<(&'static str, f64, i32)> {
    match (request, limit) {
        (Some(r), Some(l)) if r == l => vec![("request = limit {}", r, 0)],
        (Some(r), Some(l)) if (r - l).abs() < top * 0.1 => {
            let (low, high) = if r < l { (("request {}", r), ("limit {}", l)) } else { (("limit {}", l), ("request {}", r)) };
            vec![(high.0, high.1, 0), (low.0, low.1, 18)]
        }
        (r, l) => [r.map(|r| ("request {}", r, 0)), l.map(|l| ("limit {}", l, 0))].into_iter().flatten().collect(),
    }
}

/// The chart's reference labels, written out: (text, value, pixels down).
pub(crate) fn ref_label_texts(c: &Chart, names: Names, measure: Measure) -> Vec<(String, f64, i32)> {
    ref_labels(c.request_runs.last().map(|r| r.2), c.limit_runs.last().map(|r| r.2), c.frame.top)
        .into_iter()
        .map(|(text, v, nudge)| (text.replace("request", names.request).replace("{}", &measure.fmt(v)), v, nudge))
        .collect()
}

/// Whether a marker shows on this measure's chart: restarts on both, OOM
/// kills on memory only.
pub(crate) fn shows_mark(measure: Measure, e: &TraceEventMark) -> bool {
    measure == Measure::Memory || e.kind == "restart"
}

pub(crate) fn is_oom(e: &TraceEventMark) -> bool {
    e.kind == "oom" || e.reason.as_deref() == Some("OOMKilled")
}

/// A marker's tooltip; `at` is its time, written out.
pub(crate) fn mark_text(e: &TraceEventMark, at: &str) -> String {
    match (e.kind.as_str(), e.reason.as_deref()) {
        ("oom", _) => format!("OOM kill ×{} at {at}", e.count),
        (_, Some(r)) => format!("Restart ×{} ({r}) at {at}", e.count),
        _ => format!("Restart ×{} at {at}", e.count),
    }
}

/// The bucket under the pointer at `t`, if one is.
pub(crate) fn hover_bucket(avg: &Series, peak: &Series, t: f64, step: f64) -> Option<f64> {
    let bucket = avg.iter().chain(peak.iter()).map(|p| p.0).filter(|b| *b <= t).fold(f64::NEG_INFINITY, f64::max);
    (bucket.is_finite() && t - bucket <= step).then_some(bucket)
}

/// A series' value at exactly `bucket`.
pub(crate) fn value_at(s: &Series, bucket: f64) -> Option<f64> {
    s.iter().find(|p| p.0 == bucket).map(|p| p.1)
}

/// The highest value of a series, or 0.
pub(crate) fn highest(s: &Series) -> f64 {
    s.iter().map(|p| p.1).fold(0.0, f64::max)
}

/// The chart's description for screen readers; `from` and `to` are the
/// span's ends written out.
pub(crate) fn summary(measure: Measure, names: Names, from: &str, to: &str, avg: &Series, peak: &Series) -> String {
    format!(
        "{} from {from} to {to}: {} up to {}, {} up to {}",
        measure.label(),
        names.avg,
        measure.fmt(highest(avg)),
        names.peak,
        measure.fmt(highest(peak)),
    )
}

/// Shares of the row's highest value, in five steps, for the grid's cells.
pub(crate) fn heat_level(v: f64, max: f64) -> u8 {
    if max <= 0.0 {
        return 1;
    }
    ((v / max * 5.0).ceil() as u8).clamp(1, 5)
}

/// A day's 24 hours of the measure.
pub(crate) fn day_values(day: &TraceDay, measure: Measure) -> Vec<Option<f64>> {
    match measure {
        Measure::Cpu => day.cpu.clone(),
        Measure::Memory => day.mem.clone(),
    }
}

/// The highest hour of any day.
pub(crate) fn heat_max(days: &[TraceDay], measure: Measure) -> f64 {
    days.iter().flat_map(|d| day_values(d, measure)).flatten().fold(0.0, f64::max)
}

/// What the grid shows.
pub(crate) fn heat_what(measure: Measure) -> &'static str {
    match measure {
        Measure::Cpu => "p95 CPU",
        Measure::Memory => "peak memory",
    }
}

/// The grid's label, and its scale's top end.
pub(crate) fn heat_texts(measure: Measure, max: f64) -> (String, String) {
    (format!("{} by day and hour", heat_what(measure)), format!("High ({})", measure.fmt(max)))
}

/// A cell's tooltip.
pub(crate) fn heat_title(day: &str, hour: usize, measure: Measure, v: Option<f64>) -> String {
    match v {
        Some(v) => format!("{day} {hour:02}:00 · {} {}", heat_what(measure), measure.fmt(v)),
        None => format!("{day} {hour:02}:00 · no data"),
    }
}

/// The hour labels across the top: every sixth.
pub(crate) fn hour_label(h: u32) -> Option<String> {
    h.is_multiple_of(6).then(|| format!("{h:02}"))
}

#[cfg(test)]
#[path = "trace_chart_tests.rs"]
mod tests;
