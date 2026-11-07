//! A single-series line chart over a time window, with labelled reference
//! lines (request, limit) and a crosshair tooltip. The SVG stretches to the
//! container; dots and text are HTML overlays so they don't distort. The
//! geometry is in [`crate::logic::chart`].

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::logic::chart::{self as logic, W};
pub(crate) use crate::logic::chart::{ChartData, RefLine, Scale, nice_ceil};

#[component]
pub fn LineChart(
    label: &'static str,
    #[prop(into)] data: Signal<Option<ChartData>>,
    fmt: fn(f64) -> String,
    scale: Scale,
) -> impl IntoView {
    let hover = RwSignal::new(None::<(f64, f64)>);
    let on_move = move |ev: leptos::ev::PointerEvent| {
        let Some(el) = ev.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) else { return };
        let rect = el.get_bounding_client_rect();
        let Some(frac) = logic::pointer_frac(ev.client_x() as f64, rect.left(), rect.width()) else { return };
        data.with_untracked(|d| hover.set(d.as_ref().and_then(|d| logic::nearest(d, frac))));
    };

    move || {
        let d = data.get();
        if let Some(text) = logic::empty_text(d.as_ref()) {
            return view! { <div class="lchart-wrap"><div class="lchart empty">{text}</div></div> }.into_any();
        }
        let Some(d) = d else { return ().into_any() };
        let layout = logic::layout(&d, label, fmt, scale);
        let frame = layout.frame;
        let grid = layout
            .grid
            .into_iter()
            .map(|gy| view! { <line class="lchart-grid" x1="0" x2=W y1=gy y2=gy vector-effect="non-scaling-stroke" /> })
            .collect_view();
        let ticks = layout
            .ticks
            .into_iter()
            .map(|(style, text)| view! { <span class="lchart-tick" style=style>{text}</span> })
            .collect_view();
        let refs = layout
            .refs
            .into_iter()
            .map(|(ry, dashed)| {
                view! {
                    <line
                        class="lchart-ref"
                        class:dashed=dashed
                        x1="0" x2=W y1=ry y2=ry
                        vector-effect="non-scaling-stroke"
                    />
                }
            })
            .collect_view();
        let ref_labels = layout
            .ref_labels
            .into_iter()
            .map(|(style, text)| view! { <span class="lchart-ref-label" style=style>{text}</span> })
            .collect_view();
        let hover_view = move || {
            hover.get().map(|(t, v)| {
                let marks = logic::hover(frame, t, v);
                view! {
                    <span class="lchart-cross" style=marks.cross></span>
                    <span class="lchart-dot" style=marks.dot></span>
                    <span class=marks.tip_class style=marks.tip_style>
                        <span class="muted">{clock(t)}</span>" "{fmt(v)}
                    </span>
                }
            })
        };

        view! {
            <div class="lchart-wrap">
                <div
                    class="lchart"
                    role="img"
                    aria-label=layout.summary
                    on:pointermove=on_move
                    on:pointerleave=move |_| hover.set(None)
                >
                    <svg viewBox=logic::view_box() preserveAspectRatio="none" aria-hidden="true">
                        {grid}
                        {refs}
                        <path class="lchart-area" d=layout.area />
                        <path class="lchart-line" d=layout.line vector-effect="non-scaling-stroke" />
                    </svg>
                    {ticks}
                    {ref_labels}
                    <span class="lchart-dot end" style=layout.end_dot></span>
                    {hover_view}
                </div>
                <div class="lchart-x">
                    <span>"60 min ago"</span>
                    <span>"30 min ago"</span>
                    <span>"now"</span>
                </div>
            </div>
        }
        .into_any()
    }
}

/// Local `HH:MM` for a unix time in seconds.
pub fn clock(t: f64) -> String {
    let d = js_sys::Date::new(&(t * 1000.0).into());
    logic::hh_mm(d.get_hours(), d.get_minutes())
}
