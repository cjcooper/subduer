//! User-set widths of resizable panes (the sidebar and the detail panes beside
//! the tables), per pane, and their saved form.

use std::collections::HashMap;

/// Room always left for the table beside the pane.
pub(crate) const MIN_TABLE: f64 = 240.0;
/// How far one arrow-key press moves the divider.
pub(crate) const KEY_STEP: f64 = 16.0;

/// Which side of its divider a pane is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    /// Left of the divider (the sidebar): the divider is its right edge.
    Left,
    /// Right of the divider (a detail pane): the divider is its left edge.
    Right,
}

/// A resizable pane: where it is kept, which side of its divider it's on,
/// and how narrow and wide it can be dragged.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Pane {
    pub(crate) name: &'static str,
    pub(crate) side: Side,
    pub(crate) min: f64,
    pub(crate) max: f64,
}

/// The navigation sidebar.
pub(crate) const SIDEBAR: Pane = Pane { name: "sidebar", side: Side::Left, min: 180.0, max: 480.0 };
/// The resource detail panel.
pub(crate) const DETAIL: Pane = Pane { name: "detail", side: Side::Right, min: 320.0, max: f64::INFINITY };
/// The Traces page's workload detail.
pub(crate) const REVIEW: Pane = Pane { name: "review", side: Side::Right, min: 320.0, max: f64::INFINITY };

/// `pane -> width in px`.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct PaneWidths(HashMap<String, f64>);

impl PaneWidths {
    /// Widths from their saved form: `pane\twidth` lines. Malformed lines are skipped.
    pub(crate) fn from_saved(saved: &str) -> Self {
        let map = saved
            .lines()
            .filter_map(|line| {
                let mut f = line.split('\t');
                let (pane, width) = (f.next()?, f.next()?.parse::<f64>().ok()?);
                (!pane.is_empty() && width.is_finite()).then(|| (pane.to_owned(), width))
            })
            .collect();
        Self(map)
    }

    /// The saved form read by [`Self::from_saved`], sorted so it is stable.
    pub(crate) fn saved(&self) -> String {
        let mut lines: Vec<String> = self.0.iter().map(|(p, w)| format!("{p}\t{w:.0}")).collect();
        lines.sort();
        lines.join("\n")
    }

    pub(crate) fn get(&self, pane: &str) -> Option<f64> {
        self.0.get(pane).copied()
    }

    /// Sets (`Some`) or forgets (`None`, back to the default width) a pane's width.
    pub(crate) fn put(&mut self, pane: &str, width: Option<f64>) {
        match width {
            Some(w) => self.0.insert(pane.to_owned(), w),
            None => self.0.remove(pane),
        };
    }
}

/// The width while dragging: the pane grows as the divider moves away from
/// it (left for a pane on the right, right for one on the left).
pub(crate) fn dragged_width(side: Side, start_width: f64, start_x: f64, x: f64) -> f64 {
    match side {
        Side::Left => start_width + (x - start_x),
        Side::Right => start_width - (x - start_x),
    }
}

/// A width kept within the pane's limits and leaving [`MIN_TABLE`] for the
/// table, where `room` is the pane's and table's widths together. When there
/// isn't room for both minimums, the pane keeps its minimum.
pub(crate) fn clamp_width(pane: Pane, width: f64, room: f64) -> f64 {
    width.clamp(pane.min, (room - MIN_TABLE).min(pane.max).max(pane.min))
}

/// How an arrow key changes the width: it moves the divider that way.
pub(crate) fn key_change(side: Side, key: &str) -> Option<f64> {
    let step = match key {
        "ArrowLeft" => -KEY_STEP,
        "ArrowRight" => KEY_STEP,
        _ => return None,
    };
    Some(match side {
        Side::Left => step,
        Side::Right => -step,
    })
}

/// The pane's `style`: its width once set; otherwise the stylesheet's default.
/// A detail pane gives way to the table beside it when the window gets
/// narrower; the sidebar keeps its width.
pub(crate) fn pane_style(side: Side, width: Option<f64>) -> String {
    width
        .map(|w| match side {
            Side::Left => format!("width: {w:.0}px"),
            Side::Right => format!("width: {w:.0}px; max-width: none; flex-shrink: 1"),
        })
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "pane_tests.rs"]
mod tests;
