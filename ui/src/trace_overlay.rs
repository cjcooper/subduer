//! One review row's chart from several traces on one axis, for Compare: each
//! trace's average pod as a line in its own colour, and its request as a
//! dashed line in the same colour.
//!
//! Traces are lined up by time since each one started, or by time of week:
//! shifted by whole weeks onto the first trace's, so the same weekday and
//! hour sit above each other. Lines cross freely, so every pair of colours
//! must stay apart: three traces at most (the palette's slots that pass all
//! pairs), each labelled at its line's end.
//!
//! The logic is in `logic::trace_overlay`.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::logic::trace_chart::{H, Measure, W, fraction, left_style, tip_side, top_style};
use crate::logic::trace_overlay::{Align, Frame, Line, drawn, hover_rows, summary, trace_var};

/// Seconds east of UTC at `t`, in this browser's time zone.
pub(crate) fn local_offset(t: f64) -> f64 {
    -js_sys::Date::new(&(t * 1000.0).into()).get_timezone_offset() * 60.0
}

/// An axis label in this browser's time zone.
fn axis_label(align: Align, x: f64) -> String {
    crate::logic::trace_overlay::axis_label(align, x, local_offset(x))
}

#[component]
pub(crate) fn OverlayChart(#[prop(into)] lines: Signal<Vec<Line>>, measure: Measure, #[prop(into)] align: Signal<Align>) -> impl IntoView {
    let hover = RwSignal::new(None::<f64>);
    move || {
        let drawn = drawn(&lines.get(), measure);
        let align = align.get();
        let Some(f) = Frame::of(&drawn, measure) else {
            return view! { <div class="tchart-wrap"><div class="tchart empty">"No data"</div></div> }.into_any();
        };
        let grid = f
            .grid()
            .into_iter()
            .map(|v| view! { <line class="lchart-grid" x1="0" x2=W y1=f.y(v) y2=f.y(v) vector-effect="non-scaling-stroke" /> })
            .collect_view();
        let ticks = f
            .ticks()
            .into_iter()
            .map(|v| view! { <span class="lchart-tick" style=top_style(f.yp(v), 15)>{measure.fmt(v)}</span> })
            .collect_view();
        let marks = drawn
            .iter()
            .map(|l| {
                let (line, runs) = f.line_paths(l, measure);
                view! {
                    <g style=trace_var(l.slot)>
                        <path class="ochart-request" d=runs vector-effect="non-scaling-stroke" />
                        <path class="ochart-line" d=line vector-effect="non-scaling-stroke" />
                    </g>
                }
            })
            .collect_view();
        let ends = f
            .ends(&drawn, measure)
            .into_iter()
            .map(|(n, style)| view! { <span class="ochart-end" style=style>{n.to_string()}</span> })
            .collect_view();
        let summary = summary(measure, drawn.len(), align);
        let hover_lines = drawn;
        let hover_view = move || {
            let at = hover.get()?;
            let rows = hover_rows(&hover_lines, measure, at);
            if rows.is_empty() {
                return None;
            }
            let hx = f.xp(at);
            let rows = rows
                .into_iter()
                .map(|(slot, text, request)| view! {
                    <span class="tchart-tip-row" style=trace_var(slot)>
                        <span class="ochart-swatch" aria-hidden="true"></span>
                        {text}
                        {request.map(|r| view! { <span class="muted">{r}</span> })}
                    </span>
                })
                .collect_view();
            Some(view! {
                <span class="lchart-cross" style=left_style(hx)></span>
                <span class="lchart-tip tchart-tip" style=tip_side(hx)>
                    <span class="muted">{axis_label(align, at)}</span>
                    {rows}
                </span>
            })
        };
        let on_move = move |ev: leptos::ev::PointerEvent| {
            let Some(el) = ev.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) else { return };
            let rect = el.get_bounding_client_rect();
            if let Some(fr) = fraction(ev.client_x() as f64, rect.left(), rect.width()) {
                hover.set(Some(f.at(fr)));
            }
        };
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
                    <span>{axis_label(align, f.lo)}</span>
                    <span>{axis_label(align, f.at(0.5))}</span>
                    <span>{axis_label(align, f.hi)}</span>
                </div>
            </div>
        }
        .into_any()
    }
}
