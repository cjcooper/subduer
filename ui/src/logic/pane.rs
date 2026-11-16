//! User-set widths of the detail panes beside the tables, per pane, and their
//! saved form.

use std::collections::HashMap;

/// Narrowest a pane can be dragged.
pub(crate) const MIN_PANE: f64 = 320.0;
/// Room always left for the table beside the pane.
pub(crate) const MIN_TABLE: f64 = 240.0;
/// How far one arrow-key press moves the divider.
pub(crate) const KEY_STEP: f64 = 16.0;

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

/// The width while dragging. The divider is on the pane's left edge, so moving
/// the pointer left widens the pane.
pub(crate) fn dragged_width(start_width: f64, start_x: f64, x: f64) -> f64 {
    start_width - (x - start_x)
}

/// A width kept between [`MIN_PANE`] and what leaves [`MIN_TABLE`] for the
/// table, where `room` is the pane's and table's widths together. When there
/// isn't room for both minimums, the pane keeps its minimum.
pub(crate) fn clamp_width(width: f64, room: f64) -> f64 {
    width.clamp(MIN_PANE, (room - MIN_TABLE).max(MIN_PANE))
}

/// How an arrow key changes the width: Left widens, Right narrows.
pub(crate) fn key_change(key: &str) -> Option<f64> {
    match key {
        "ArrowLeft" => Some(KEY_STEP),
        "ArrowRight" => Some(-KEY_STEP),
        _ => None,
    }
}

/// The pane's `style`: its width once set; otherwise the stylesheet's default.
/// The table beside it still wins when the window gets narrower.
pub(crate) fn pane_style(width: Option<f64>) -> String {
    width.map(|w| format!("width: {w:.0}px; max-width: none; flex-shrink: 1")).unwrap_or_default()
}

#[cfg(test)]
#[path = "pane_tests.rs"]
mod tests;
