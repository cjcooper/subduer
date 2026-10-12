//! A single-series line chart over a time window, with labelled reference
//! lines (request, limit) and a crosshair tooltip. The SVG stretches to the
//! container; dots and text are HTML overlays so they don't distort.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

/// Plot height in viewBox units; the top `HEADROOM` stays clear for labels.
const H: f64 = 100.0;
const HEADROOM: f64 = 18.0;
const W: f64 = 1000.0;

#[derive(Debug, Clone, PartialEq)]
pub struct RefLine {
    pub name: &'static str,
    pub value: f64,
    /// Dashed lines are softer context (request); solid is a hard cap (limit).
    pub dashed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChartData {
    /// Window bounds, unix seconds.
    pub start: f64,
    pub end: f64,
    /// Gaps longer than ~2.5 steps break the line.
    pub step: f64,
    pub points: Vec<(f64, f64)>,
    pub refs: Vec<RefLine>,
}

/// How tick values are rounded: decimal for CPU, binary units for bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scale {
    Decimal,
    Binary,
}

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
        if rect.width() <= 0.0 {
            return;
        }
        let frac = ((ev.client_x() as f64 - rect.left()) / rect.width()).clamp(0.0, 1.0);
        data.with_untracked(|d| {
            let nearest = d.as_ref().and_then(|d| {
                let t = d.start + frac * (d.end - d.start);
                d.points.iter().copied().min_by(|a, b| (a.0 - t).abs().total_cmp(&(b.0 - t).abs()))
            });
            hover.set(nearest);
        });
    };

    move || {
        let Some(d) = data.get() else {
            return view! { <div class="lchart-wrap"><div class="lchart empty">"Loading…"</div></div> }.into_any();
        };
        if d.points.is_empty() {
            return view! { <div class="lchart-wrap"><div class="lchart empty">"No usage data yet"</div></div> }
                .into_any();
        }
        let span = (d.end - d.start).max(1.0);
        let data_max = d.points.iter().map(|p| p.1).fold(0.0, f64::max);
        let ref_max = d.refs.iter().map(|r| r.value).fold(0.0, f64::max);
        let top = nice_ceil(data_max.max(ref_max) * 1.05, scale);
        let x = move |t: f64| ((t - d.start) / span * W).clamp(0.0, W);
        let y = move |v: f64| H - (v / top).clamp(0.0, 1.0) * (H - HEADROOM);
        // Percentages for HTML overlays.
        let (xp, yp) = (move |t: f64| x(t) / W * 100.0, move |v: f64| y(v) / H * 100.0);

        let (line, area) = paths(&d.points, d.step, x, y);
        let grid = [0.0, top / 2.0, top]
            .into_iter()
            .map(|v| {
                let gy = y(v);
                view! {
                    <line class="lchart-grid" x1="0" x2=W y1=gy y2=gy vector-effect="non-scaling-stroke" />
                }
            })
            .collect_view();
        let ticks = [top / 2.0, top]
            .into_iter()
            .map(|v| view! { <span class="lchart-tick" style=format!("top: calc({:.2}% - 15px)", yp(v))>{tidy(fmt(v))}</span> })
            .collect_view();
        let refs = d
            .refs
            .iter()
            .map(|r| {
                let ry = y(r.value);
                view! {
                    <line
                        class="lchart-ref"
                        class:dashed=r.dashed
                        x1="0" x2=W y1=ry y2=ry
                        vector-effect="non-scaling-stroke"
                    />
                }
            })
            .collect_view();
        let ref_values: Vec<f64> = d.refs.iter().map(|r| r.value).collect();
        let ref_labels = d
            .refs
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let style = if label_below(&ref_values, i, top) {
                    format!("top: calc({:.2}% + 1px)", yp(r.value))
                } else {
                    format!("top: calc({:.2}% - 15px)", yp(r.value))
                };
                view! {
                    <span class="lchart-ref-label" style=style>
                        {format!("{} {}", r.name, tidy(fmt(r.value)))}
                    </span>
                }
            })
            .collect_view();
        let last = *d.points.last().unwrap();
        let summary = {
            let min = d.points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
            let avg = d.points.iter().map(|p| p.1).sum::<f64>() / d.points.len() as f64;
            format!("{label} over the last hour: now {}, min {}, average {}, max {}", fmt(last.1), fmt(min), fmt(avg), fmt(data_max))
        };
        let hover_view = move || {
            hover.get().map(|(t, v)| {
                let (hx, hy) = (xp(t), yp(v));
                let side = if hx > 65.0 { "right" } else { "left" };
                let pos = if side == "right" { format!("right: {:.2}%", 100.0 - hx) } else { format!("left: {hx:.2}%") };
                view! {
                    <span class="lchart-cross" style=format!("left: {hx:.2}%")></span>
                    <span class="lchart-dot" style=format!("left: {hx:.2}%; top: {hy:.2}%")></span>
                    <span class=format!("lchart-tip {side}") style=pos>
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
                    aria-label=summary
                    on:pointermove=on_move
                    on:pointerleave=move |_| hover.set(None)
                >
                    <svg viewBox=format!("0 0 {W} {H}") preserveAspectRatio="none" aria-hidden="true">
                        {grid}
                        {refs}
                        <path class="lchart-area" d=area />
                        <path class="lchart-line" d=line vector-effect="non-scaling-stroke" />
                    </svg>
                    {ticks}
                    {ref_labels}
                    <span class="lchart-dot end" style=format!("left: {:.2}%; top: {:.2}%", xp(last.0), yp(last.1))></span>
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

/// Line and area paths, broken wherever samples are missing.
fn paths(points: &[(f64, f64)], step: f64, x: impl Fn(f64) -> f64, y: impl Fn(f64) -> f64) -> (String, String) {
    let mut segments: Vec<Vec<(f64, f64)>> = vec![];
    let mut prev: Option<f64> = None;
    for &(t, v) in points {
        if prev.is_none_or(|p| t - p > step * 2.5) {
            segments.push(vec![]);
        }
        segments.last_mut().unwrap().push((x(t), y(v)));
        prev = Some(t);
    }
    let mut line = String::new();
    let mut area = String::new();
    for seg in &segments {
        let coords: Vec<String> = seg.iter().map(|(px, py)| format!("{px:.1},{py:.2}")).collect();
        line.push_str(&format!("M{}", coords.join("L")));
        let (first, last) = (seg[0].0, seg[seg.len() - 1].0);
        area.push_str(&format!("M{first:.1},{H}L{}L{last:.1},{H}Z", coords.join("L")));
    }
    (line, area)
}

/// Smallest "round" number ≥ `v` (1, 1.2, 1.5, 2, 2.5, 3, 4, 5, 6 or 8 × a power of ten), in
/// binary units (KiB, MiB, …) for bytes.
pub fn nice_ceil(v: f64, scale: Scale) -> f64 {
    if v <= 0.0 || !v.is_finite() {
        return 1.0;
    }
    let unit = match scale {
        Scale::Binary if v >= 1024.0 => 1024f64.powf((v.log2() / 10.0).floor()),
        _ => 1.0,
    };
    let scaled = v / unit;
    let magnitude = 10f64.powf(scaled.log10().floor());
    let m = [1.0, 1.2, 1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0].into_iter().find(|m| m * magnitude >= scaled * 0.9999).unwrap_or(10.0);
    m * magnitude * unit
}

/// `500.0 MiB` → `500 MiB` for axis labels.
fn tidy(s: String) -> String {
    s.replace(".0 ", " ").replace(".0m", "m")
}

/// Local `HH:MM` for a unix time in seconds.
pub fn clock(t: f64) -> String {
    let d = js_sys::Date::new(&(t * 1000.0).into());
    format!("{:02}:{:02}", d.get_hours(), d.get_minutes())
}

/// Whether reference line `i`'s label goes under its line rather than over
/// it: when a higher line (or an equal one listed first) is so close that
/// both labels above would overlap. Labels are about a sixth of the chart's height.
pub(crate) fn label_below(values: &[f64], i: usize, top: f64) -> bool {
    let close = |a: f64, b: f64| top > 0.0 && (a - b).abs() / top * (H - HEADROOM) < 17.0;
    values.iter().enumerate().any(|(j, &v)| {
        j != i && (v > values[i] || (v == values[i] && j < i)) && close(v, values[i])
    })
}

#[cfg(test)]
#[path = "chart_tests.rs"]
mod tests_unit;
