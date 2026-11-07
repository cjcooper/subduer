//! Sorting table rows, and each table's sorted column.
//!
//! Rows sort by what the column shows: numbers by value (with units, so
//! `512Mi` < `2Gi`), times shown as ages youngest first, text in natural
//! order (`pod-2` < `pod-10`). Empty cells go last either way.

use std::cmp::Ordering;
use std::collections::HashMap;

/// One cell's sort key.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Key {
    Num(f64),
    Text(String),
    /// Empty or unknown: last, ascending or descending.
    Missing,
}

impl Key {
    pub(crate) fn text(s: &str) -> Self {
        let s = s.trim();
        if s.is_empty() || s == "—" { Key::Missing } else { Key::Text(s.to_lowercase()) }
    }

    pub(crate) fn num(n: f64) -> Self {
        if n.is_nan() { Key::Missing } else { Key::Num(n) }
    }

    pub(crate) fn opt(n: Option<f64>) -> Self {
        n.map_or(Key::Missing, Key::num)
    }

    /// A time (ms, NaN if it didn't parse) shown as an age: the youngest sorts first.
    pub(crate) fn age_ms(ms: f64) -> Self {
        Key::num(-ms)
    }

    /// A time (ms, NaN if it didn't parse) shown as itself: the earliest sorts first.
    pub(crate) fn time_ms(ms: f64) -> Self {
        Key::num(ms)
    }

    /// A value that starts with a number, maybe with a unit (`20Gi`,
    /// `16.9 GiB`, `250m`, `45%`, `3 (5m ago)`); text otherwise.
    pub(crate) fn quantity(s: &str) -> Self {
        quantity(s).map_or_else(|| Key::text(s), Key::Num)
    }
}

/// Milliseconds since the epoch for a whole-second UTC time
/// (`2026-10-16T12:00:00Z`); NaN for anything else. Stands in for the
/// browser's date parser on the host.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn parse_utc_ms(s: &str) -> f64 {
    let s = s.trim();
    let digits = |r: std::ops::Range<usize>| s.get(r).and_then(|d| d.parse::<i64>().ok());
    let (Some(y), Some(mo), Some(d), Some(h), Some(mi), Some(se)) =
        (digits(0..4), digits(5..7), digits(8..10), digits(11..13), digits(14..16), digits(17..19))
    else {
        return f64::NAN;
    };
    // Days since 1970-01-01 (civil-from-days, inverted).
    let (y, m) = if mo <= 2 { (y - 1, mo + 9) } else { (y, mo - 3) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    ((days * 86_400 + h * 3600 + mi * 60 + se) * 1000) as f64
}

/// The leading number of `s` scaled by its unit, if it starts with one.
pub(crate) fn quantity(s: &str) -> Option<f64> {
    let s = s.trim().trim_start_matches(['<', '~', '≈', '+']);
    let end = s
        .char_indices()
        .find(|&(i, c)| !(c.is_ascii_digit() || c == '.' || c == ',' || (c == '-' && i == 0)))
        .map_or(s.len(), |(i, _)| i);
    let number: f64 = s[..end].replace(',', "").parse().ok()?;
    let unit = s[end..].trim_start();
    let unit = unit.split(|c: char| c.is_whitespace() || c == '(' || c == '/' || c == '·').next().unwrap_or_default();
    let scale = match unit {
        "m" => 1e-3,
        "k" | "K" | "kB" | "KB" => 1e3,
        "M" | "MB" => 1e6,
        "G" | "GB" => 1e9,
        "T" | "TB" => 1e12,
        "Ki" | "KiB" => 1024.0,
        "Mi" | "MiB" => 1024f64.powi(2),
        "Gi" | "GiB" => 1024f64.powi(3),
        "Ti" | "TiB" => 1024f64.powi(4),
        "core" | "cores" => 1.0,
        _ => 1.0,
    };
    Some(number * scale)
}

/// Text in natural order: runs of digits compare as numbers.
pub(crate) fn natural(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut n = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        n.push(c);
                        it.next();
                    }
                    n
                };
                let (n, m) = (take(&mut a), take(&mut b));
                let (n, m) = (n.trim_start_matches('0'), m.trim_start_matches('0'));
                let ord = n.len().cmp(&m.len()).then_with(|| n.cmp(m));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                a.next();
                b.next();
            }
        }
    }
}

/// Ascending order of two keys; numbers before text. Missing isn't compared
/// here: [`sort`] keeps it last.
fn compare(a: &Key, b: &Key) -> Ordering {
    match (a, b) {
        (Key::Num(x), Key::Num(y)) => x.total_cmp(y),
        (Key::Num(_), Key::Text(_)) => Ordering::Less,
        (Key::Text(_), Key::Num(_)) => Ordering::Greater,
        (Key::Text(x), Key::Text(y)) => natural(x, y),
        _ => Ordering::Equal,
    }
}

/// Sorts `items` by column `by` (`(index, descending)`), keeping the order of
/// equal rows; `None` leaves them as they are. Missing keys go last.
pub(crate) fn sort<T>(items: &mut [T], by: Option<(usize, bool)>, key: impl Fn(&T, usize) -> Key) {
    let Some((column, descending)) = by else { return };
    let mut keyed: Vec<(Key, usize)> = items.iter().enumerate().map(|(i, t)| (key(t, column), i)).collect();
    keyed.sort_by(|(a, _), (b, _)| match (a, b) {
        (Key::Missing, Key::Missing) => Ordering::Equal,
        (Key::Missing, _) => Ordering::Greater,
        (_, Key::Missing) => Ordering::Less,
        _ => {
            let ord = compare(a, b);
            if descending { ord.reverse() } else { ord }
        }
    });
    let order: Vec<usize> = keyed.into_iter().map(|(_, i)| i).collect();
    apply_order(items, &order);
}

/// Rearranges `items` so position `n` holds what was at `order[n]`.
fn apply_order<T>(items: &mut [T], order: &[usize]) {
    let mut done = vec![false; items.len()];
    for start in 0..items.len() {
        if done[start] {
            continue;
        }
        // Follow the cycle starting here.
        let mut at = start;
        loop {
            done[at] = true;
            let from = order[at];
            if done[from] {
                break;
            }
            items.swap(at, from);
            at = from;
        }
    }
}

/// Each table's sorted column: `table -> (header, descending)`.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct SortState(HashMap<String, (String, bool)>);

impl SortState {
    /// From `table\theader\tasc|desc` lines; malformed lines are skipped.
    pub(crate) fn from_saved(saved: &str) -> Self {
        let map = saved
            .lines()
            .filter_map(|line| {
                let mut f = line.split('\t');
                let (table, header, dir) = (f.next()?, f.next()?, f.next()?);
                let descending = match dir {
                    "asc" => false,
                    "desc" => true,
                    _ => return None,
                };
                Some((table.to_owned(), (header.to_owned(), descending)))
            })
            .collect();
        Self(map)
    }

    /// The saved form read by [`Self::from_saved`].
    pub(crate) fn saved(&self) -> String {
        let mut lines: Vec<String> =
            self.0.iter().map(|(t, (h, d))| format!("{t}\t{h}\t{}", if *d { "desc" } else { "asc" })).collect();
        lines.sort();
        lines.join("\n")
    }

    /// The sorted header and whether it's descending.
    pub(crate) fn get(&self, table: &str) -> Option<(String, bool)> {
        self.0.get(table).cloned()
    }

    /// The sorted column's index among `headers`, if it's one of them.
    pub(crate) fn by(&self, table: &str, headers: &[&str]) -> Option<(usize, bool)> {
        let (header, descending) = self.get(table)?;
        headers.iter().position(|h| *h == header).map(|i| (i, descending))
    }

    /// Ascending, then descending, then the table's own order.
    pub(crate) fn click(&mut self, table: &str, header: &str) {
        match self.0.get(table) {
            Some((h, false)) if h == header => {
                self.0.insert(table.to_owned(), (header.to_owned(), true));
            }
            Some((h, true)) if h == header => {
                self.0.remove(table);
            }
            _ => {
                self.0.insert(table.to_owned(), (header.to_owned(), false));
            }
        }
    }

    /// Whether `header` is the one `table` is sorted by.
    pub(crate) fn is_sorted_by(&self, table: &str, header: &str) -> bool {
        self.0.get(table).is_some_and(|(h, _)| h == header)
    }

    /// `↑`, `↓` or nothing for one heading (spaced by CSS).
    pub(crate) fn arrow(&self, table: &str, header: &str) -> &'static str {
        match self.0.get(table) {
            Some((h, d)) if h == header => if *d { "↓" } else { "↑" },
            _ => "",
        }
    }

    /// The heading's `aria-sort`.
    pub(crate) fn aria(&self, table: &str, header: &str) -> &'static str {
        match self.0.get(table) {
            Some((h, d)) if h == header => if *d { "descending" } else { "ascending" },
            _ => "none",
        }
    }
}

/// A sort button's tooltip.
pub(crate) fn sort_title(header: &str) -> String {
    format!("Sort by {header}")
}

#[cfg(test)]
#[path = "sort_tests.rs"]
mod tests;
