//! Charts for a trace review: one measure (CPU or memory) of one workload
//! container over days, and its usage by day and hour.
//!
//! The line chart shows the average pod (2px line with a wash) and the
//! busiest pod's peak (a lighter line), the request (dashed) and limit
//! (solid) as step lines that follow changes, gaps as hatched spans, and
//! restarts and OOM kills as markers along the bottom. Hover shows the
//! values at a time; dragging across selects a span to zoom into.
//!
//! The logic is in `logic::trace_chart`.

use leptos::prelude::*;
use subduer_types::{TraceDaily, TraceSeries};
use wasm_bindgen::JsCast;

pub(crate) use crate::logic::trace_chart::Measure;
use crate::logic::trace_chart::{
    Frame, H, Names, W, capitalised, chart, count_text, day_values, dot_style, span_style, fraction, heat_level, heat_max, heat_texts, heat_title, hour_label, hover_bucket, idle_text,
    is_oom, left_style, mark_text, ref_label_texts, selection, shows_mark, step_at, summary, time_at, tip_side, top_style, value_at, zoom_span,
};

/// A time for axis labels and tips, in this browser's time zone: `14:05`,
/// or `Mon 12 14:05` over long spans.
pub(crate) fn when(t: f64, long: bool) -> String {
    crate::logic::trace_chart::when(t, long, crate::trace_overlay::local_offset(t))
}

#[component]
pub(crate) fn TraceChart(
    #[prop(into)] series: Signal<Option<TraceSeries>>,
    measure: Measure,
    on_zoom: Callback<(f64, f64)>,
    /// Every container added up rather than one row's pods: the lines are
    /// named for that, and the tooltip shows what's requested but idle.
    #[prop(optional)]
    totals: bool,
) -> impl IntoView {
    let names = Names::of(totals);
    let hover = RwSignal::new(None::<f64>);
    // Drag to zoom: where it started and where it is, as fractions of the width.
    let drag = RwSignal::new(None::<(f64, f64)>);
    let frac = |ev: &leptos::ev::PointerEvent| -> Option<f64> {
        let el = ev.current_target()?.dyn_into::<web_sys::Element>().ok()?;
        let rect = el.get_bounding_client_rect();
        fraction(ev.client_x() as f64, rect.left(), rect.width())
    };
    let at = move |f: f64| series.with_untracked(|s| s.as_ref().map(|s| time_at(s, f)));
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
            if let Some((lo, hi)) = zoom_span(a, b)
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
        let Some(c) = chart(&s, measure) else {
            return view! { <div class="tchart-wrap"><div class="tchart empty">"No data in this span"</div></div> }.into_any();
        };
        let (avg, peak, request, limit) = measure.pick(&s);
        let f = c.frame;
        let step = s.step;
        let long = f.long();
        let grid = f
            .grid()
            .into_iter()
            .map(|v| view! { <line class="lchart-grid" x1="0" x2=W y1=f.y(v) y2=f.y(v) vector-effect="non-scaling-stroke" /> })
            .collect_view();
        let gaps = s
            .gaps
            .iter()
            .map(|g| {
                let (a, b) = (f.x(g.start), f.x(g.end));
                view! { <rect class="tchart-gap" x=a y="0" width=(b - a).max(2.0) height=Frame::BASE><title>{g.reason.clone()}</title></rect> }
            })
            .collect_view();
        let ticks = f
            .ticks()
            .into_iter()
            .map(|v| view! { <span class="lchart-tick" style=top_style(f.yp(v), 15)>{measure.fmt(v)}</span> })
            .collect_view();
        // The latest request and limit in the window get labels at the right;
        // equal values share one, close ones are moved apart.
        let ref_labels = ref_label_texts(&c, names, measure)
            .into_iter()
            .map(|(text, v, nudge)| view! { <span class="lchart-ref-label" style=top_style(f.yp(v), 15 - nudge)>{text}</span> })
            .collect_view();
        let marks = s
            .events
            .iter()
            .filter(|e| shows_mark(measure, e))
            .map(|e| {
                let text = mark_text(e, &when(e.t, true));
                view! {
                    <span class="tchart-mark" class:oom=is_oom(e) style=left_style(f.xp(e.t)) title=text.clone() aria-label=text></span>
                }
            })
            .collect_view();

        let (avg2, peak2, request2, limit2, pods) = (avg.clone(), peak.clone(), request.clone(), limit.clone(), s.pods.clone());
        let hover_view = move || {
            let bucket = hover_bucket(&avg2, &peak2, hover.get()?, step)?;
            let (a, p) = (value_at(&avg2, bucket), value_at(&peak2, bucket));
            let hx = f.xp(bucket + step / 2.0);
            let line = |name: &'static str, v: Option<f64>| v.map(|v| view! { <span class="tchart-tip-row"><span class="muted">{name}</span>" "{measure.fmt(v)}</span> });
            let requested = step_at(&request2, bucket);
            let idle = totals.then(|| idle_text(measure, a, requested)).flatten().map(|text| view! {
                <span class="tchart-tip-row"><span class="muted">"idle"</span>" "{text}</span>
            });
            Some(view! {
                <span class="lchart-cross" style=left_style(hx)></span>
                {a.map(|a| view! { <span class="lchart-dot" style=dot_style(hx, f.yp(a))></span> })}
                <span class="lchart-tip tchart-tip" style=tip_side(hx)>
                    <span class="muted">{when(bucket, true)}</span>
                    {line(names.avg, a)}
                    {line(names.peak, p)}
                    {line(names.request, requested)}
                    {idle}
                    {line("limit", step_at(&limit2, bucket))}
                    {value_at(&pods, bucket).map(|n| view! { <span class="tchart-tip-row"><span class="muted">{names.count}</span>" "{count_text(n)}</span> })}
                </span>
            })
        };
        let selection = move || {
            selection(drag.get()).map(|(left, width)| view! { <span class="tchart-select" style=span_style(left, width)></span> })
        };
        let summary = summary(measure, names, &when(f.start, true), &when(f.end, true), avg, peak);
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
                        <path class="lchart-area" d=c.avg_area />
                        <path class="tchart-peak" d=c.peak_line vector-effect="non-scaling-stroke" />
                        <path class="lchart-line" d=c.avg_line vector-effect="non-scaling-stroke" />
                        <path class="tchart-request" d=c.request_path vector-effect="non-scaling-stroke" />
                        <path class="tchart-limit" d=c.limit_path vector-effect="non-scaling-stroke" />
                    </svg>
                    {ticks}
                    {ref_labels}
                    {marks}
                    {hover_view}
                    {selection}
                </div>
                <div class="lchart-x">
                    <span>{when(f.start, long)}</span>
                    <span>{when(f.start + f.span / 2.0, long)}</span>
                    <span>{when(f.end, long)}</span>
                </div>
                <div class="tchart-legend">
                    <span><span class="key key-avg"></span>{capitalised(names.avg)}</span>
                    <span><span class="key key-peak"></span>{capitalised(names.peak)}</span>
                    {(!c.request_runs.is_empty()).then(|| view! { <span><span class="key key-request"></span>{capitalised(names.request)}</span> })}
                    {(!c.limit_runs.is_empty()).then(|| view! { <span><span class="key key-limit"></span>"Limit"</span> })}
                    {(!s.gaps.is_empty()).then(|| view! { <span><span class="key key-gap"></span>"No data"</span> })}
                    {has_events.then(|| view! { <span><span class="key key-mark"></span>"Restart"</span><span><span class="key key-mark oom"></span>"OOM kill"</span> })}
                </div>
            </div>
        }
        .into_any()
    }
}

/// Usage by day (rows) and local hour (columns): p95 CPU or peak memory.
#[component]
pub(crate) fn TraceHeat(#[prop(into)] daily: Signal<Option<TraceDaily>>, measure: Measure) -> impl IntoView {
    move || {
        let d = daily.get()?;
        let max = heat_max(&d.days, measure);
        let (label, high) = heat_texts(measure, max);
        let rows = d
            .days
            .iter()
            .map(|day| {
                let cells = day_values(day, measure)
                    .into_iter()
                    .enumerate()
                    .map(|(h, v)| {
                        let title = heat_title(&day.label, h, measure, v);
                        match v {
                            Some(v) => view! { <span class=format!("heat-cell l{}", heat_level(v, max)) title=title.clone() aria-label=title></span> }.into_any(),
                            None => view! { <span class="heat-cell none" title=title></span> }.into_any(),
                        }
                    })
                    .collect_view();
                view! { <div class="heat-row"><span class="heat-day">{day.label.clone()}</span>{cells}</div> }
            })
            .collect_view();
        let hours = (0..24).map(|h| view! { <span class="heat-hour">{hour_label(h)}</span> }).collect_view();
        Some(view! {
            <div class=format!("heat {}", measure.class()) role="table" aria-label=label>
                <div class="heat-row heat-head"><span class="heat-day"></span>{hours}</div>
                {rows}
                <div class="heat-scale muted small">
                    <span>"Low"</span>
                    {(1..=5).map(|l| view! { <span class=format!("heat-cell l{l}")></span> }).collect_view()}
                    <span>{high}</span>
                </div>
            </div>
        })
    }
}
