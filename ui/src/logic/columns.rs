//! User-set column widths, per table and header, and their saved form.

use std::collections::HashMap;

pub(crate) const MIN_WIDTH: f64 = 40.0;
/// Width for a column that appears after the table's widths were set.
const FALLBACK_WIDTH: f64 = 120.0;

/// `(table, header) -> width in px`.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct WidthState(HashMap<(String, String), f64>);

impl WidthState {
    /// Widths from their saved form: `table\theader\twidth` lines. Malformed
    /// lines are skipped.
    pub(crate) fn from_saved(saved: &str) -> Self {
        let map = saved
            .lines()
            .filter_map(|line| {
                let mut f = line.split('\t');
                let (table, header, width) = (f.next()?, f.next()?, f.next()?.parse().ok()?);
                Some(((table.to_owned(), header.to_owned()), width))
            })
            .collect();
        Self(map)
    }

    /// The saved form read by [`Self::from_saved`].
    pub(crate) fn saved(&self) -> String {
        self.0.iter().map(|((t, h), w)| format!("{t}\t{h}\t{w:.0}")).collect::<Vec<_>>().join("\n")
    }

    pub(crate) fn get(&self, table: &str, header: &str) -> Option<f64> {
        self.0.get(&(table.to_owned(), header.to_owned())).copied()
    }

    pub(crate) fn has_table(&self, table: &str) -> bool {
        self.0.keys().any(|(t, _)| t == table)
    }

    /// Sets a width, no narrower than [`MIN_WIDTH`].
    pub(crate) fn set(&mut self, table: &str, header: &str, width: f64) {
        self.0.insert((table.to_owned(), header.to_owned()), width.max(MIN_WIDTH));
    }

    /// Back to automatic widths for `table`.
    pub(crate) fn forget(&mut self, table: &str) {
        self.0.retain(|(t, _), _| t != table);
    }

    /// Fixed layout at the summed width once the table has custom widths;
    /// otherwise the browser's automatic layout.
    pub(crate) fn table_style(&self, table: &str, headers: &[String]) -> String {
        if !self.has_table(table) {
            return String::new();
        }
        let total: f64 = headers.iter().map(|h| self.get(table, h).unwrap_or(FALLBACK_WIDTH)).sum();
        format!("table-layout: fixed; width: max(100%, {total:.0}px)")
    }

    /// A header cell's `style`: its width, once set.
    pub(crate) fn cell_style(&self, table: &str, header: &str) -> String {
        self.get(table, header).map(|w| format!("width: {w:.0}px")).unwrap_or_default()
    }
}

/// The width while dragging: the width at the start plus how far the pointer moved.
pub(crate) fn dragged_width(start_width: f64, start_x: f64, x: f64) -> f64 {
    start_width + x - start_x
}

/// A heading's `aria-sort`: only for a sortable table, from its sorted column.
pub(crate) fn column_aria(sortable: bool, sorted: Option<(String, bool)>, header: &str) -> Option<&'static str> {
    match sorted {
        Some((s, d)) if s == header => Some(if d { "descending" } else { "ascending" }),
        _ if sortable => Some("none"),
        _ => None,
    }
}

#[cfg(test)]
#[path = "columns_tests.rs"]
mod tests;
