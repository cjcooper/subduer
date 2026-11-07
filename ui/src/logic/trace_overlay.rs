//! The Compare overlay's logic: which traces it shows and in which colour,
//! how they line up, its frame, end labels and tooltip rows.

use subduer_types::TraceSeries;

use crate::chart::nice_ceil;
use crate::logic::trace::duration_text;
use crate::logic::trace_chart::{DAYS, H, Measure, W, local_time, paths, step_at, step_runs};

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

/// The CSS variable that gives a slot its colour.
pub(crate) fn trace_var(slot: usize) -> String {
    format!("--trace: var(--trace-{})", slot + 1)
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

/// An axis label: time into the trace, or weekday and time (`offset` is
/// the local offset at `x`, seconds east of UTC).
pub(crate) fn axis_label(align: Align, x: f64, offset: f64) -> String {
    match align {
        Align::SinceStart if x <= 0.0 => "start".into(),
        Align::SinceStart => format!("+{}", duration_text(x)),
        Align::TimeOfWeek => {
            let d = local_time(x, offset);
            format!("{} {:02}:{:02}", DAYS[d.weekday as usize], d.hours, d.minutes)
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

/// The lines with something to draw for this measure.
pub(crate) fn drawn(lines: &[Line], measure: Measure) -> Vec<Line> {
    lines.iter().filter(|l| !measure.pick(&l.series).0.is_empty()).cloned().collect()
}

/// The overlay's axes, over every drawn line on the shared axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Frame {
    pub(crate) lo: f64,
    pub(crate) hi: f64,
    pub(crate) span: f64,
    pub(crate) top: f64,
}

impl Frame {
    /// `None` without lines.
    pub(crate) fn of(lines: &[Line], measure: Measure) -> Option<Self> {
        if lines.is_empty() {
            return None;
        }
        let lo = lines.iter().map(|l| l.series.start - l.shift).fold(f64::INFINITY, f64::min);
        let hi = lines.iter().map(|l| l.series.end - l.shift).fold(f64::NEG_INFINITY, f64::max);
        let top_value = lines
            .iter()
            .flat_map(|l| {
                let (avg, _, request, _) = measure.pick(&l.series);
                avg.iter().map(|p| p.1).chain(request.iter().map(|p| p.1)).collect::<Vec<_>>()
            })
            .fold(0.0, f64::max);
        Some(Self { lo, hi, span: (hi - lo).max(1.0), top: nice_ceil(top_value * 1.05, measure.scale()) })
    }

    pub(crate) fn x(&self, v: f64) -> f64 {
        ((v - self.lo) / self.span * W).clamp(0.0, W)
    }

    pub(crate) fn y(&self, v: f64) -> f64 {
        H - (v / self.top).clamp(0.0, 1.0) * (H - HEADROOM)
    }

    /// `x` and `y` as percentages of the chart.
    pub(crate) fn xp(&self, v: f64) -> f64 {
        self.x(v) / W * 100.0
    }

    pub(crate) fn yp(&self, v: f64) -> f64 {
        self.y(v) / H * 100.0
    }

    /// The time at fraction `f` across.
    pub(crate) fn at(&self, f: f64) -> f64 {
        self.lo + f * self.span
    }

    pub(crate) fn grid(&self) -> [f64; 3] {
        [0.0, self.top / 2.0, self.top]
    }

    pub(crate) fn ticks(&self) -> [f64; 2] {
        [self.top / 2.0, self.top]
    }

    /// A line's paths: its average, and its request as steps.
    pub(crate) fn line_paths(&self, l: &Line, measure: Measure) -> (String, String) {
        let (avg, _, request, _) = measure.pick(&l.series);
        let (s, shift) = (&l.series, l.shift);
        let (line, _) = paths(avg, s.step, |t| self.x(t - shift), |v| self.y(v), H);
        let runs = step_runs(request, s.start, s.end)
            .into_iter()
            .map(|(a, b, v)| format!("M{:.1},{:.2}H{:.1}", self.x(a - shift), self.y(v), self.x(b - shift)))
            .collect();
        (line, runs)
    }

    /// Each line's number at its end, moved apart: the style for each label.
    /// Identity isn't colour alone.
    pub(crate) fn ends(&self, lines: &[Line], measure: Measure) -> Vec<(usize, String)> {
        let ends: Vec<(usize, usize, f64, f64)> = lines
            .iter()
            .filter_map(|l| {
                let &(t, v) = measure.pick(&l.series).0.last()?;
                Some((l.n, l.slot, self.xp(t + l.series.step - l.shift), self.yp(v)))
            })
            .collect();
        end_offsets(&ends.iter().map(|e| (e.2, e.3)).collect::<Vec<_>>())
            .into_iter()
            .zip(ends)
            .map(|(down, (n, slot, left, top))| (n, format!("{}; left: {left:.2}%; top: calc({top:.2}% + {down}px)", trace_var(slot))))
            .collect()
    }
}

/// The tooltip's rows at `at` on the shared axis: (slot, text, request text).
pub(crate) fn hover_rows(lines: &[Line], measure: Measure, at: f64) -> Vec<(usize, String, Option<String>)> {
    lines
        .iter()
        .filter_map(|l| {
            let (avg, _, request, _) = measure.pick(&l.series);
            let (t, v) = value_at(avg, l.series.step, l.shift, at)?;
            let r = step_at(request, t).map(|r| format!(" · request {}", measure.fmt(r)));
            Some((l.slot, format!("Trace {}: {}", l.n, measure.fmt(v)), r))
        })
        .collect()
}

/// The overlay's description for screen readers.
pub(crate) fn summary(measure: Measure, traces: usize, align: Align) -> String {
    format!(
        "{} of {traces} traces, by {}",
        measure.label(),
        if align == Align::SinceStart { "time since each started" } else { "time of week" }
    )
}

#[cfg(test)]
#[path = "trace_overlay_tests.rs"]
mod tests;
