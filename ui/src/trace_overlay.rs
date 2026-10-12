//! One review row's chart from several traces on one axis, for Compare: each
//! trace's average pod as a line in its own colour, and its request as a
//! dashed line in the same colour.
//!
//! Traces are lined up by time since each one started, or by time of week:
//! shifted by whole weeks onto the first trace's, so the same weekday and
//! hour sit above each other. Lines cross freely, so every pair of colours
//! must stay apart: three traces at most (the palette's slots that pass all
//! pairs), each labelled at its line's end.

use leptos::prelude::*;
use subduer_types::TraceSeries;
use wasm_bindgen::JsCast;

use crate::chart::nice_ceil;
use crate::trace::duration_text;
use crate::trace_chart::{Measure, paths, step_at, step_runs};

const W: f64 = 1000.0;
const H: f64 = 100.0;
const HEADROOM: f64 = 8.0;
pub(crate) const WEEK: f64 = 7.0 * 86400.0;
/// Traces one overlay shows at most.
pub(crate) const MAX_TRACES: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Align {
    SinceStart,
    TimeOfWeek,
}

/// One trace's series on the overlay.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Line {
    /// The trace's number in the comparison (1-based).
    pub(crate) n: usize,
    /// Its colour: `--trace-{slot + 1}`.
    pub(crate) slot: usize,
    pub(crate) series: TraceSeries,
    /// Subtracted from the trace's times to put it on the shared axis.
    pub(crate) shift: f64,
}

/// Which of `n` traces an overlay shows: all of up to three, else the first
/// (before) and the last two (after).
pub(crate) fn picked(n: usize) -> Vec<usize> {
    if n <= MAX_TRACES { (0..n).collect() } else { vec![0, n - 2, n - 1] }
}

/// The colour slot of trace `i` of `n` in a comparison, if overlays show it.
/// Fixed by the comparison, not the row, so a trace keeps its colour on rows
/// that other traces don't have.
pub(crate) fn trace_slot(i: usize, n: usize) -> Option<usize> {
    picked(n).iter().position(|&p| p == i)
}

/// Monday-based week number of a time with its local offset (seconds east
/// of UTC); the epoch was a Thursday.
fn week_of(t: f64, offset: f64) -> f64 {
    ((t + offset + 3.0 * 86400.0) / WEEK).floor()
}

/// What to subtract from a trace's times. Since start: its start, so the
/// axis is time into the trace. Time of week: whole weeks between its start
/// and the first trace's, so the axis is the first trace's dates.
pub(crate) fn shift(align: Align, start: f64, offset: f64, first: f64, first_offset: f64) -> f64 {
    match align {
        Align::SinceStart => start,
        Align::TimeOfWeek => (week_of(start, offset) - week_of(first, first_offset)) * WEEK,
    }
}

/// Seconds east of UTC at `t`, in this browser's time zone.
pub(crate) fn local_offset(t: f64) -> f64 {
    -js_sys::Date::new(&(t * 1000.0).into()).get_timezone_offset() * 60.0
}

/// An axis label: time into the trace, or weekday and time.
pub(crate) fn axis_label(align: Align, x: f64) -> String {
    match align {
        Align::SinceStart if x <= 0.0 => "start".into(),
        Align::SinceStart => format!("+{}", duration_text(x)),
        Align::TimeOfWeek => {
            const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
            let d = js_sys::Date::new(&(x * 1000.0).into());
            format!("{} {:02}:{:02}", DAYS[d.get_day() as usize % 7], d.get_hours(), d.get_minutes())
        }
    }
}

/// Pixels to move each end label down so none sits on another: labels are
/// (left %, top %) of the chart; ones ending close to an earlier one, in
/// both directions, go 14 px below it.
pub(crate) fn end_offsets(labels: &[(f64, f64)]) -> Vec<i32> {
    let mut placed: Vec<(f64, f64, i32)> = vec![];
    labels
        .iter()
        .map(|&(left, top)| {
            let down = placed.iter().filter(|p| (p.0 - left).abs() < 6.0 && (p.1 - top).abs() < 10.0).map(|p| p.2 + 14).max().unwrap_or(0);
            placed.push((left, top, down));
            down
        })
        .collect()
}

/// A line's value under `x` on the shared axis: the bucket holding it.
fn value_at(points: &[(f64, f64)], step: f64, shift: f64, x: f64) -> Option<(f64, f64)> {
    points.iter().copied().find(|&(t, _)| t - shift <= x && x < t - shift + step)
}

#[component]
pub(crate) fn OverlayChart(#[prop(into)] lines: Signal<Vec<Line>>, measure: Measure, #[prop(into)] align: Signal<Align>) -> impl IntoView {
    let hover = RwSignal::new(None::<f64>);
    move || {
        let lines = lines.get();
        let align = align.get();
        let drawn: Vec<&Line> = lines.iter().filter(|l| !measure.pick(&l.series).0.is_empty()).collect();
        if drawn.is_empty() {
            return view! { <div class="tchart-wrap"><div class="tchart empty">"No data"</div></div> }.into_any();
        }
        let lo = drawn.iter().map(|l| l.series.start - l.shift).fold(f64::INFINITY, f64::min);
        let hi = drawn.iter().map(|l| l.series.end - l.shift).fold(f64::NEG_INFINITY, f64::max);
        let span = (hi - lo).max(1.0);
        let top_value = drawn
            .iter()
            .flat_map(|l| {
                let (avg, _, request, _) = measure.pick(&l.series);
                avg.iter().map(|p| p.1).chain(request.iter().map(|p| p.1)).collect::<Vec<_>>()
            })
            .fold(0.0, f64::max);
        let top = nice_ceil(top_value * 1.05, measure.scale());
        let x = move |v: f64| ((v - lo) / span * W).clamp(0.0, W);
        let y = move |v: f64| H - (v / top).clamp(0.0, 1.0) * (H - HEADROOM);
        let pct = move |v: f64, of: f64| v / of * 100.0;

        let grid = [0.0, top / 2.0, top]
            .into_iter()
            .map(|v| view! { <line class="lchart-grid" x1="0" x2=W y1=y(v) y2=y(v) vector-effect="non-scaling-stroke" /> })
            .collect_view();
        let ticks = [top / 2.0, top]
            .into_iter()
            .map(|v| view! { <span class="lchart-tick" style=format!("top: calc({:.2}% - 15px)", pct(y(v), H))>{measure.fmt(v)}</span> })
            .collect_view();
        let marks = drawn
            .iter()
            .map(|l| {
                let (avg, _, request, _) = measure.pick(&l.series);
                let (s, shift) = (&l.series, l.shift);
                let (line, _) = paths(avg, s.step, move |t| x(t - shift), y, H);
                let runs = step_runs(request, s.start, s.end)
                    .into_iter()
                    .map(|(a, b, v)| format!("M{:.1},{:.2}H{:.1}", x(a - shift), y(v), x(b - shift)))
                    .collect::<String>();
                let color = format!("--trace: var(--trace-{})", l.slot + 1);
                view! {
                    <g style=color>
                        <path class="ochart-request" d=runs vector-effect="non-scaling-stroke" />
                        <path class="ochart-line" d=line vector-effect="non-scaling-stroke" />
                    </g>
                }
            })
            .collect_view();
        // The trace's number at the end of its line: identity isn't colour alone.
        let ends: Vec<(usize, usize, f64, f64)> = drawn
            .iter()
            .filter_map(|l| {
                let &(t, v) = measure.pick(&l.series).0.last()?;
                Some((l.n, l.slot, pct(x(t + l.series.step - l.shift), W), pct(y(v), H)))
            })
            .collect();
        let ends = end_offsets(&ends.iter().map(|e| (e.2, e.3)).collect::<Vec<_>>())
            .into_iter()
            .zip(ends)
            .map(|(down, (n, slot, left, top))| {
                let style = format!("--trace: var(--trace-{}); left: {left:.2}%; top: calc({top:.2}% + {down}px)", slot + 1);
                view! { <span class="ochart-end" style=style>{n.to_string()}</span> }
            })
            .collect_view();
        let hover_lines: Vec<Line> = drawn.iter().map(|l| (*l).clone()).collect();
        let hover_view = move || {
            let at = hover.get()?;
            let rows: Vec<_> = hover_lines
                .iter()
                .filter_map(|l| {
                    let (avg, _, request, _) = measure.pick(&l.series);
                    let (t, v) = value_at(avg, l.series.step, l.shift, at)?;
                    Some((l.n, l.slot, v, step_at(request, t)))
                })
                .collect();
            if rows.is_empty() {
                return None;
            }
            let hx = pct(x(at), W);
            let side = if hx > 60.0 { format!("right: {:.2}%", 100.0 - hx) } else { format!("left: {hx:.2}%") };
            let rows = rows
                .into_iter()
                .map(|(n, slot, v, r)| view! {
                    <span class="tchart-tip-row" style=format!("--trace: var(--trace-{})", slot + 1)>
                        <span class="ochart-swatch" aria-hidden="true"></span>
                        {format!("Trace {n}: {}", measure.fmt(v))}
                        {r.map(|r| view! { <span class="muted">{format!(" · request {}", measure.fmt(r))}</span> })}
                    </span>
                })
                .collect_view();
            Some(view! {
                <span class="lchart-cross" style=format!("left: {hx:.2}%")></span>
                <span class="lchart-tip tchart-tip" style=side>
                    <span class="muted">{axis_label(align, at)}</span>
                    {rows}
                </span>
            })
        };
        let on_move = move |ev: leptos::ev::PointerEvent| {
            let Some(el) = ev.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) else { return };
            let rect = el.get_bounding_client_rect();
            if rect.width() > 0.0 {
                let f = ((ev.client_x() as f64 - rect.left()) / rect.width()).clamp(0.0, 1.0);
                hover.set(Some(lo + f * span));
            }
        };
        let summary = format!(
            "{} of {} traces, by {}",
            measure.label(),
            drawn.len(),
            if align == Align::SinceStart { "time since each started" } else { "time of week" }
        );
        view! {
            <div class=format!("tchart-wrap ochart-wrap {}", measure.class())>
                <div class="tchart" role="img" aria-label=summary on:pointermove=on_move on:pointerleave=move |_| hover.set(None)>
                    <svg viewBox=format!("0 0 {W} {H}") preserveAspectRatio="none" aria-hidden="true">
                        {grid}
                        {marks}
                    </svg>
                    {ticks}
                    {ends}
                    {hover_view}
                </div>
                <div class="lchart-x">
                    <span>{axis_label(align, lo)}</span>
                    <span>{axis_label(align, lo + span / 2.0)}</span>
                    <span>{axis_label(align, hi)}</span>
                </div>
            </div>
        }
        .into_any()
    }
}

#[cfg(test)]
#[path = "trace_overlay_tests.rs"]
mod tests;
