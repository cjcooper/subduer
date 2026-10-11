//! Charts for a trace review: one measure (CPU or memory) of one workload
//! container over days, and its usage by day and hour.
//!
//! The line chart shows the average pod (2px line with a wash) and the
//! busiest pod's peak (a lighter line), the request (dashed) and limit
//! (solid) as step lines that follow changes, gaps as hatched spans, and
//! restarts and OOM kills as markers along the bottom. Hover shows the
//! values at a time; dragging across selects a span to zoom into.

use leptos::prelude::*;
use subduer_types::{Series, TraceDaily, TraceSeries};
use wasm_bindgen::JsCast;

use crate::chart::{Scale, nice_ceil};
use crate::overview::{fmt_bytes, fmt_cpu};

const W: f64 = 1000.0;
const H: f64 = 100.0;
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

    fn scale(self) -> Scale {
        match self {
            Self::Cpu => Scale::Decimal,
            Self::Memory => Scale::Binary,
        }
    }

    /// Average pod, busiest pod, request and limit.
    fn pick(self, s: &TraceSeries) -> (&Series, &Series, &Series, &Series) {
        match self {
            Self::Cpu => (&s.cpu_avg, &s.cpu_max, &s.cpu_request, &s.cpu_limit),
            Self::Memory => (&s.mem_avg, &s.mem_max, &s.mem_request, &s.mem_limit),
        }
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

/// A time for axis labels and tips: `14:05`, or `Mon 12 14:05` over long spans.
pub(crate) fn when(t: f64, long: bool) -> String {
    let d = js_sys::Date::new(&(t * 1000.0).into());
    let clock = format!("{:02}:{:02}", d.get_hours(), d.get_minutes());
    if long {
        const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
        format!("{} {} {clock}", DAYS[d.get_day() as usize % 7], d.get_date())
    } else {
        clock
    }
}

/// Line and area paths for a series, broken where buckets are missing.
fn paths(points: &Series, step: f64, x: impl Fn(f64) -> f64, y: impl Fn(f64) -> f64, base: f64) -> (String, String) {
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

#[component]
pub(crate) fn TraceChart(
    #[prop(into)] series: Signal<Option<TraceSeries>>,
    measure: Measure,
    on_zoom: Callback<(f64, f64)>,
) -> impl IntoView {
    let hover = RwSignal::new(None::<f64>);
    // Drag to zoom: where it started and where it is, as fractions of the width.
    let drag = RwSignal::new(None::<(f64, f64)>);
    let frac = |ev: &leptos::ev::PointerEvent| -> Option<f64> {
        let el = ev.current_target()?.dyn_into::<web_sys::Element>().ok()?;
        let rect = el.get_bounding_client_rect();
        (rect.width() > 0.0).then(|| ((ev.client_x() as f64 - rect.left()) / rect.width()).clamp(0.0, 1.0))
    };
    let at = move |f: f64| series.with_untracked(|s| s.as_ref().map(|s| s.start + f * (s.end - s.start)));
    let on_down = move |ev: leptos::ev::PointerEvent| {
        if let Some(f) = frac(&ev) {
            drag.set(Some((f, f)));
            if let Some(el) = ev.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
                let _ = el.set_pointer_capture(ev.pointer_id());
            }
        }
    };
    let on_move = move |ev: leptos::ev::PointerEvent| {
        let Some(f) = frac(&ev) else { return };
        if let Some((a, _)) = drag.get_untracked() {
            drag.set(Some((a, f)));
        }
        hover.set(at(f));
    };
    let on_up = move |_| {
        if let Some((a, b)) = drag.get_untracked() {
            drag.set(None);
            let (lo, hi) = (a.min(b), a.max(b));
            if hi - lo > 0.02
                && let (Some(t0), Some(t1)) = (at(lo), at(hi))
            {
                hover.set(None);
                on_zoom.run((t0, t1));
            }
        }
    };

    move || {
        let Some(s) = series.get() else {
            return view! { <div class="tchart-wrap"><div class="tchart empty">"Loading…"</div></div> }.into_any();
        };
        let (avg, peak, request, limit) = measure.pick(&s);
        if avg.is_empty() && peak.is_empty() {
            return view! { <div class="tchart-wrap"><div class="tchart empty">"No data in this span"</div></div> }.into_any();
        }
        let (start, end, step) = (s.start, s.end, s.step);
        let span = (end - start).max(1.0);
        let long = span > 20.0 * 3600.0;
        let request_runs = step_runs(request, start, end);
        let limit_runs = step_runs(limit, start, end);
        let top_value = peak
            .iter()
            .chain(avg.iter())
            .map(|p| p.1)
            .chain(request_runs.iter().chain(&limit_runs).map(|r| r.2))
            .fold(0.0, f64::max);
        let top = nice_ceil(top_value * 1.05, measure.scale());
        let base = H - FOOT;
        let x = move |t: f64| ((t - start) / span * W).clamp(0.0, W);
        let y = move |v: f64| base - (v / top).clamp(0.0, 1.0) * (base - HEADROOM);
        let (xp, yp) = (move |t: f64| x(t) / W * 100.0, move |v: f64| y(v) / H * 100.0);

        let (avg_line, avg_area) = paths(avg, step, x, y, base);
        let (peak_line, _) = paths(peak, step, x, y, base);
        let steps = |runs: &[(f64, f64, f64)]| {
            runs.iter()
                .map(|&(a, b, v)| format!("M{:.1},{:.2}H{:.1}", x(a), y(v), x(b)))
                .collect::<String>()
        };
        let (request_path, limit_path) = (steps(&request_runs), steps(&limit_runs));
        let grid = [0.0, top / 2.0, top]
            .into_iter()
            .map(|v| view! { <line class="lchart-grid" x1="0" x2=W y1=y(v) y2=y(v) vector-effect="non-scaling-stroke" /> })
            .collect_view();
        let gaps = s
            .gaps
            .iter()
            .map(|g| {
                let (a, b) = (x(g.start), x(g.end));
                view! { <rect class="tchart-gap" x=a y="0" width=(b - a).max(2.0) height=base><title>{g.reason.clone()}</title></rect> }
            })
            .collect_view();
        let ticks = [top / 2.0, top]
            .into_iter()
            .map(|v| view! { <span class="lchart-tick" style=format!("top: calc({:.2}% - 15px)", yp(v))>{measure.fmt(v)}</span> })
            .collect_view();
        // The latest request and limit in the window get labels at the right;
        // equal values share one, close ones are moved apart.
        let ref_labels = ref_labels(request_runs.last().map(|r| r.2), limit_runs.last().map(|r| r.2), top)
            .into_iter()
            .map(|(text, v, nudge)| {
                let text = text.replace("{}", &measure.fmt(v));
                view! { <span class="lchart-ref-label" style=format!("top: calc({:.2}% - {}px)", yp(v), 15 - nudge)>{text}</span> }
            })
            .collect_view();
        let marks = s
            .events
            .iter()
            .filter(|e| measure == Measure::Memory || e.kind == "restart")
            .map(|e| {
                let oom = e.kind == "oom" || e.reason.as_deref() == Some("OOMKilled");
                let text = match (e.kind.as_str(), e.reason.as_deref()) {
                    ("oom", _) => format!("OOM kill ×{} at {}", e.count, when(e.t, true)),
                    (_, Some(r)) => format!("Restart ×{} ({r}) at {}", e.count, when(e.t, true)),
                    _ => format!("Restart ×{} at {}", e.count, when(e.t, true)),
                };
                view! {
                    <span class="tchart-mark" class:oom=oom style=format!("left: {:.2}%", xp(e.t)) title=text.clone() aria-label=text></span>
                }
            })
            .collect_view();

        let (avg2, peak2, request2, limit2, pods) = (avg.clone(), peak.clone(), request.clone(), limit.clone(), s.pods.clone());
        let hover_view = move || {
            let t = hover.get()?;
            // The bucket under the pointer.
            let bucket = avg2.iter().chain(peak2.iter()).map(|p| p.0).filter(|b| *b <= t).fold(f64::NEG_INFINITY, f64::max);
            if !bucket.is_finite() || t - bucket > step {
                return None;
            }
            let value = |s: &Series| s.iter().find(|p| p.0 == bucket).map(|p| p.1);
            let (a, p) = (value(&avg2), value(&peak2));
            let hx = xp(bucket + step / 2.0);
            let side = if hx > 60.0 { format!("right: {:.2}%", 100.0 - hx) } else { format!("left: {hx:.2}%") };
            let line = |name: &'static str, v: Option<f64>| v.map(|v| view! { <span class="tchart-tip-row"><span class="muted">{name}</span>" "{measure.fmt(v)}</span> });
            Some(view! {
                <span class="lchart-cross" style=format!("left: {hx:.2}%")></span>
                {a.map(|a| view! { <span class="lchart-dot" style=format!("left: {hx:.2}%; top: {:.2}%", yp(a))></span> })}
                <span class="lchart-tip tchart-tip" style=side>
                    <span class="muted">{when(bucket, true)}</span>
                    {line("average pod", a)}
                    {line("busiest pod", p)}
                    {line("request", step_at(&request2, bucket))}
                    {line("limit", step_at(&limit2, bucket))}
                    {value(&pods).map(|n| view! { <span class="tchart-tip-row"><span class="muted">"pods"</span>" "{format!("{n:.1}").trim_end_matches(".0").to_owned()}</span> })}
                </span>
            })
        };
        let selection = move || {
            drag.get().filter(|(a, b)| (a - b).abs() > 0.005).map(|(a, b)| {
                let (lo, hi) = (a.min(b), a.max(b));
                view! { <span class="tchart-select" style=format!("left: {:.2}%; width: {:.2}%", lo * 100.0, (hi - lo) * 100.0)></span> }
            })
        };
        let summary = format!(
            "{} from {} to {}: average pod up to {}, busiest pod up to {}",
            measure.label(),
            when(start, true),
            when(end, true),
            measure.fmt(avg.iter().map(|p| p.1).fold(0.0, f64::max)),
            measure.fmt(peak.iter().map(|p| p.1).fold(0.0, f64::max)),
        );
        let has_events = !s.events.is_empty();
        view! {
            <div class=format!("tchart-wrap {}", measure.class())>
                <div
                    class="tchart"
                    role="img"
                    aria-label=summary
                    on:pointerdown=on_down
                    on:pointermove=on_move
                    on:pointerup=on_up
                    on:pointercancel=move |_| drag.set(None)
                    on:pointerleave=move |_| if drag.get_untracked().is_none() { hover.set(None) }
                >
                    <svg viewBox=format!("0 0 {W} {H}") preserveAspectRatio="none" aria-hidden="true">
                        <defs>
                            <pattern id="tchart-hatch" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
                                <line x1="0" y1="0" x2="0" y2="6" class="tchart-hatch-line" />
                            </pattern>
                        </defs>
                        {gaps}
                        {grid}
                        <path class="lchart-area" d=avg_area />
                        <path class="tchart-peak" d=peak_line vector-effect="non-scaling-stroke" />
                        <path class="lchart-line" d=avg_line vector-effect="non-scaling-stroke" />
                        <path class="tchart-request" d=request_path vector-effect="non-scaling-stroke" />
                        <path class="tchart-limit" d=limit_path vector-effect="non-scaling-stroke" />
                    </svg>
                    {ticks}
                    {ref_labels}
                    {marks}
                    {hover_view}
                    {selection}
                </div>
                <div class="lchart-x">
                    <span>{when(start, long)}</span>
                    <span>{when(start + span / 2.0, long)}</span>
                    <span>{when(end, long)}</span>
                </div>
                <div class="tchart-legend">
                    <span><span class="key key-avg"></span>"Average pod"</span>
                    <span><span class="key key-peak"></span>"Busiest pod"</span>
                    {(!request_runs.is_empty()).then(|| view! { <span><span class="key key-request"></span>"Request"</span> })}
                    {(!limit_runs.is_empty()).then(|| view! { <span><span class="key key-limit"></span>"Limit"</span> })}
                    {(!s.gaps.is_empty()).then(|| view! { <span><span class="key key-gap"></span>"No data"</span> })}
                    {has_events.then(|| view! { <span><span class="key key-mark"></span>"Restart"</span><span><span class="key key-mark oom"></span>"OOM kill"</span> })}
                </div>
            </div>
        }
        .into_any()
    }
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

/// Shares of the row's highest value, in five steps, for the grid's cells.
pub(crate) fn heat_level(v: f64, max: f64) -> u8 {
    if max <= 0.0 {
        return 1;
    }
    ((v / max * 5.0).ceil() as u8).clamp(1, 5)
}

/// Usage by day (rows) and local hour (columns): p95 CPU or peak memory.
#[component]
pub(crate) fn TraceHeat(#[prop(into)] daily: Signal<Option<TraceDaily>>, measure: Measure) -> impl IntoView {
    move || {
        let d = daily.get()?;
        let pick = |day: &subduer_types::TraceDay| -> Vec<Option<f64>> {
            match measure {
                Measure::Cpu => day.cpu.clone(),
                Measure::Memory => day.mem.clone(),
            }
        };
        let max = d.days.iter().flat_map(&pick).flatten().fold(0.0, f64::max);
        let what = match measure {
            Measure::Cpu => "p95 CPU",
            Measure::Memory => "peak memory",
        };
        let rows = d
            .days
            .iter()
            .map(|day| {
                let cells = pick(day)
                    .into_iter()
                    .enumerate()
                    .map(|(h, v)| match v {
                        Some(v) => {
                            let title = format!("{} {h:02}:00 · {what} {}", day.label, measure.fmt(v));
                            view! { <span class=format!("heat-cell l{}", heat_level(v, max)) title=title.clone() aria-label=title></span> }.into_any()
                        }
                        None => view! { <span class="heat-cell none" title=format!("{} {h:02}:00 · no data", day.label)></span> }.into_any(),
                    })
                    .collect_view();
                view! { <div class="heat-row"><span class="heat-day">{day.label.clone()}</span>{cells}</div> }
            })
            .collect_view();
        let hours = (0..24).map(|h| view! { <span class="heat-hour">{(h % 6 == 0).then(|| format!("{h:02}"))}</span> }).collect_view();
        Some(view! {
            <div class=format!("heat {}", measure.class()) role="table" aria-label=format!("{what} by day and hour")>
                <div class="heat-row heat-head"><span class="heat-day"></span>{hours}</div>
                {rows}
                <div class="heat-scale muted small">
                    <span>"Low"</span>
                    {(1..=5).map(|l| view! { <span class=format!("heat-cell l{l}")></span> }).collect_view()}
                    <span>{format!("High ({})", measure.fmt(max))}</span>
                </div>
            </div>
        })
    }
}

#[cfg(test)]
#[path = "trace_chart_tests.rs"]
mod tests;
