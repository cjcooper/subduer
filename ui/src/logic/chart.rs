//! The line chart's geometry: scales, paths, labels and the hover tooltip's
//! placement. Plot coordinates are viewBox units; overlays are percentages.

/// Plot height in viewBox units; the top `HEADROOM` stays clear for labels.
pub(crate) const H: f64 = 100.0;
const HEADROOM: f64 = 18.0;
pub(crate) const W: f64 = 1000.0;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RefLine {
    pub(crate) name: &'static str,
    pub(crate) value: f64,
    /// Dashed lines are softer context (request); solid is a hard cap (limit).
    pub(crate) dashed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ChartData {
    /// Window bounds, unix seconds.
    pub(crate) start: f64,
    pub(crate) end: f64,
    /// Gaps longer than ~2.5 steps break the line.
    pub(crate) step: f64,
    pub(crate) points: Vec<(f64, f64)>,
    pub(crate) refs: Vec<RefLine>,
}

/// How tick values are rounded: decimal for CPU, binary units for bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scale {
    Decimal,
    Binary,
}

/// What to show instead of a chart: still loading, or nothing to draw.
pub(crate) fn empty_text(data: Option<&ChartData>) -> Option<&'static str> {
    match data {
        None => Some("Loading…"),
        Some(d) if d.points.is_empty() => Some("No usage data yet"),
        Some(_) => None,
    }
}

/// The SVG's `viewBox`.
pub(crate) fn view_box() -> String {
    format!("0 0 {W} {H}")
}

/// Maps times and values onto the plot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Frame {
    start: f64,
    span: f64,
    /// The value at the top of the plot.
    pub(crate) top: f64,
}

impl Frame {
    /// The frame for `d`: its time window, and room for its highest point or
    /// reference line, rounded up.
    pub(crate) fn new(d: &ChartData, scale: Scale) -> Self {
        let data_max = d.points.iter().map(|p| p.1).fold(0.0, f64::max);
        let ref_max = d.refs.iter().map(|r| r.value).fold(0.0, f64::max);
        Self { start: d.start, span: (d.end - d.start).max(1.0), top: nice_ceil(data_max.max(ref_max) * 1.05, scale) }
    }

    pub(crate) fn x(self, t: f64) -> f64 {
        ((t - self.start) / self.span * W).clamp(0.0, W)
    }

    pub(crate) fn y(self, v: f64) -> f64 {
        H - (v / self.top).clamp(0.0, 1.0) * (H - HEADROOM)
    }

    /// [`Self::x`] as a percentage, for HTML overlays.
    pub(crate) fn xp(self, t: f64) -> f64 {
        self.x(t) / W * 100.0
    }

    /// [`Self::y`] as a percentage, for HTML overlays.
    pub(crate) fn yp(self, v: f64) -> f64 {
        self.y(v) / H * 100.0
    }
}

/// Everything drawn for one chart, worked out from its data.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Layout {
    pub(crate) frame: Frame,
    pub(crate) line: String,
    pub(crate) area: String,
    /// The grid lines' y.
    pub(crate) grid: Vec<f64>,
    /// Axis labels: (style, text).
    pub(crate) ticks: Vec<(String, String)>,
    /// Reference lines: (y, dashed).
    pub(crate) refs: Vec<(f64, bool)>,
    /// Reference line labels: (style, text).
    pub(crate) ref_labels: Vec<(String, String)>,
    /// The latest point's dot.
    pub(crate) end_dot: String,
    /// What the chart shows, for screen readers.
    pub(crate) summary: String,
}

/// Lays out a chart of `d` (which has points). `fmt` formats values.
pub(crate) fn layout(d: &ChartData, label: &str, fmt: fn(f64) -> String, scale: Scale) -> Layout {
    let frame = Frame::new(d, scale);
    let top = frame.top;
    let (line, area) = paths(&d.points, d.step, |t| frame.x(t), |v| frame.y(v));
    let grid = [0.0, top / 2.0, top].into_iter().map(|v| frame.y(v)).collect();
    let ticks = [top / 2.0, top]
        .into_iter()
        .map(|v| (format!("top: calc({:.2}% - 15px)", frame.yp(v)), tidy(fmt(v))))
        .collect();
    let refs = d.refs.iter().map(|r| (frame.y(r.value), r.dashed)).collect();
    let ref_values: Vec<f64> = d.refs.iter().map(|r| r.value).collect();
    let ref_labels = d
        .refs
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let style = if label_below(&ref_values, i, top) {
                format!("top: calc({:.2}% + 1px)", frame.yp(r.value))
            } else {
                format!("top: calc({:.2}% - 15px)", frame.yp(r.value))
            };
            (style, format!("{} {}", r.name, tidy(fmt(r.value))))
        })
        .collect();
    let last = d.points.last().copied().unwrap_or_default();
    let end_dot = format!("left: {:.2}%; top: {:.2}%", frame.xp(last.0), frame.yp(last.1));
    let values = || d.points.iter().map(|p| p.1);
    let max = values().fold(0.0, f64::max);
    let min = values().fold(f64::INFINITY, f64::min);
    let avg = values().sum::<f64>() / d.points.len() as f64;
    let summary =
        format!("{label} over the last hour: now {}, min {}, average {}, max {}", fmt(last.1), fmt(min), fmt(avg), fmt(max));
    Layout { frame, line, area, grid, ticks, refs, ref_labels, end_dot, summary }
}

/// Where the pointer is across the plot (0 to 1), or `None` if the plot has no width.
pub(crate) fn pointer_frac(client_x: f64, left: f64, width: f64) -> Option<f64> {
    (width > 0.0).then(|| ((client_x - left) / width).clamp(0.0, 1.0))
}

/// The point nearest in time to `frac` of the way across the window.
pub(crate) fn nearest(d: &ChartData, frac: f64) -> Option<(f64, f64)> {
    let t = d.start + frac * (d.end - d.start);
    d.points.iter().copied().min_by(|a, b| (a.0 - t).abs().total_cmp(&(b.0 - t).abs()))
}

/// The hover crosshair, dot and tooltip for a point.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Hover {
    pub(crate) cross: String,
    pub(crate) dot: String,
    pub(crate) tip_class: String,
    pub(crate) tip_style: String,
}

/// Places the hover marks for point `(t, v)`; past two thirds of the way
/// across, the tooltip opens to the left.
pub(crate) fn hover(frame: Frame, t: f64, v: f64) -> Hover {
    let (hx, hy) = (frame.xp(t), frame.yp(v));
    let right = hx > 65.0;
    Hover {
        cross: format!("left: {hx:.2}%"),
        dot: format!("left: {hx:.2}%; top: {hy:.2}%"),
        tip_class: format!("lchart-tip {}", if right { "right" } else { "left" }),
        tip_style: if right { format!("right: {:.2}%", 100.0 - hx) } else { format!("left: {hx:.2}%") },
    }
}

/// Line and area paths, broken wherever samples are missing.
pub(crate) fn paths(points: &[(f64, f64)], step: f64, x: impl Fn(f64) -> f64, y: impl Fn(f64) -> f64) -> (String, String) {
    let mut segments: Vec<Vec<(f64, f64)>> = vec![];
    let mut prev: Option<f64> = None;
    for &(t, v) in points {
        if prev.is_none_or(|p| t - p > step * 2.5) {
            segments.push(vec![]);
        }
        if let Some(segment) = segments.last_mut() {
            segment.push((x(t), y(v)));
        }
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
pub(crate) fn nice_ceil(v: f64, scale: Scale) -> f64 {
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
pub(crate) fn tidy(s: String) -> String {
    s.replace(".0 ", " ").replace(".0m", "m")
}

/// `HH:MM` for an hour and minute.
pub(crate) fn hh_mm(hours: u32, minutes: u32) -> String {
    format!("{hours:02}:{minutes:02}")
}

/// Whether reference line `i`'s label goes under its line rather than over
/// it: when a higher line (or an equal one listed first) is so close that
/// both labels above would overlap. Labels are about a sixth of the chart's height.
pub(crate) fn label_below(values: &[f64], i: usize, top: f64) -> bool {
    let close = |a: f64, b: f64| top > 0.0 && (a - b).abs() / top * (H - HEADROOM) < 17.0;
    values.iter().enumerate().any(|(j, &v)| j != i && (v > values[i] || (v == values[i] && j < i)) && close(v, values[i]))
}

#[cfg(test)]
#[path = "chart_tests.rs"]
mod tests;
