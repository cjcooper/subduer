//! Sortable tables. Clicking a column heading sorts by it ascending, again
//! descending, and a third time goes back to the table's own order. The
//! choice is kept per table (a kind key, `nodes`, a Details section, …) and
//! saved in localStorage.
//!
//! Rows sort by what the column shows: numbers by value (with units, so
//! `512Mi` < `2Gi`), times shown as ages youngest first, text in natural
//! order (`pod-2` < `pod-10`). Empty cells go last either way.

use std::cmp::Ordering;
use std::collections::HashMap;

use leptos::prelude::*;

use crate::prefs::storage;

const KEY: &str = "table.sort";

/// One cell's sort key.
#[derive(Debug, Clone, PartialEq)]
pub enum Key {
    Num(f64),
    Text(String),
    /// Empty or unknown: last, ascending or descending.
    Missing,
}

impl Key {
    pub fn text(s: &str) -> Self {
        let s = s.trim();
        if s.is_empty() || s == "—" { Key::Missing } else { Key::Text(s.to_lowercase()) }
    }

    pub fn num(n: f64) -> Self {
        if n.is_nan() { Key::Missing } else { Key::Num(n) }
    }

    pub fn opt(n: Option<f64>) -> Self {
        n.map_or(Key::Missing, Key::num)
    }

    /// An RFC 3339 time shown as an age: the youngest sorts first.
    pub fn age(time: &str) -> Self {
        let ms = js_date(time);
        if ms.is_nan() { Key::Missing } else { Key::Num(-ms) }
    }

    /// An RFC 3339 time shown as itself: the earliest sorts first.
    pub fn time(time: &str) -> Self {
        let ms = js_date(time);
        if ms.is_nan() { Key::Missing } else { Key::Num(ms) }
    }

    /// A value that starts with a number, maybe with a unit (`20Gi`,
    /// `16.9 GiB`, `250m`, `45%`, `3 (5m ago)`); text otherwise.
    pub fn quantity(s: &str) -> Self {
        quantity(s).map_or_else(|| Key::text(s), Key::Num)
    }
}

#[cfg(target_arch = "wasm32")]
fn js_date(s: &str) -> f64 {
    if s.trim().is_empty() { f64::NAN } else { js_sys::Date::parse(s) }
}

/// Host tests: whole-second UTC times (`2026-10-16T12:00:00Z`) only.
#[cfg(not(target_arch = "wasm32"))]
fn js_date(s: &str) -> f64 {
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
pub fn quantity(s: &str) -> Option<f64> {
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
pub fn natural(a: &str, b: &str) -> Ordering {
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
pub fn sort<T>(items: &mut [T], by: Option<(usize, bool)>, key: impl Fn(&T, usize) -> Key) {
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

/// `table -> (header, descending)`.
#[derive(Clone, Copy)]
pub struct Sorts(RwSignal<HashMap<String, (String, bool)>>);

impl Sorts {
    pub fn load() -> Self {
        let saved = storage().and_then(|s| s.get_item(KEY).ok().flatten()).unwrap_or_default();
        Self::from_saved(&saved)
    }

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
        Self(RwSignal::new(map))
    }

    pub(crate) fn saved(&self) -> String {
        self.0.with_untracked(|m| {
            let mut lines: Vec<String> =
                m.iter().map(|(t, (h, d))| format!("{t}\t{h}\t{}", if *d { "desc" } else { "asc" })).collect();
            lines.sort();
            lines.join("\n")
        })
    }

    fn save(&self) {
        // Host tests have no window to save into.
        if !cfg!(target_arch = "wasm32") {
            return;
        }
        let text = self.saved();
        if let Some(s) = storage() {
            let _ = s.set_item(KEY, &text);
        }
    }

    /// The sorted header and whether it's descending.
    pub fn get(&self, table: &str) -> Option<(String, bool)> {
        self.0.with(|m| m.get(table).cloned())
    }

    /// The sorted column's index among `headers`, if it's one of them.
    pub fn by(&self, table: &str, headers: &[&str]) -> Option<(usize, bool)> {
        let (header, descending) = self.get(table)?;
        headers.iter().position(|h| *h == header).map(|i| (i, descending))
    }

    /// Ascending, then descending, then the table's own order.
    pub fn click(&self, table: &str, header: &str) {
        self.0.update(|m| match m.get(table) {
            Some((h, false)) if h == header => {
                m.insert(table.to_owned(), (header.to_owned(), true));
            }
            Some((h, true)) if h == header => {
                m.remove(table);
            }
            _ => {
                m.insert(table.to_owned(), (header.to_owned(), false));
            }
        });
        self.save();
    }

    /// `↑`, `↓` or nothing for one heading (spaced by CSS).
    pub fn arrow(&self, table: &str, header: &str) -> &'static str {
        match self.get(table) {
            Some((h, d)) if h == header => if d { "↓" } else { "↑" },
            _ => "",
        }
    }
}

/// The clickable label inside a heading: the header text and its arrow.
#[component]
pub fn SortButton(sorts: Sorts, table: String, header: String) -> impl IntoView {
    let (t, h) = (table.clone(), header.clone());
    let on = move || sorts.get(&t).is_some_and(|(s, _)| s == h);
    let (t, h) = (table.clone(), header.clone());
    let arrow = move || sorts.arrow(&t, &h);
    let title = format!("Sort by {header}");
    view! {
        <button class="th-sort" class:on=on title=title on:click=move |ev| {
            ev.stop_propagation();
            sorts.click(&table, &header);
        }>{header.clone()}<span class="sort-arrow">{arrow}</span></button>
    }
}

/// A plain sortable heading.
#[component]
pub fn SortTh(
    sorts: Sorts,
    #[prop(into)] table: String,
    #[prop(into)] header: String,
    #[prop(optional)] class: &'static str,
) -> impl IntoView {
    let aria = {
        let (t, h) = (table.clone(), header.clone());
        move || match sorts.get(&t) {
            Some((s, d)) if s == h => if d { "descending" } else { "ascending" },
            _ => "none",
        }
    };
    view! {
        <th class=class aria-sort=aria><SortButton sorts=sorts table=table header=header /></th>
    }
}

#[cfg(test)]
#[path = "sort_tests.rs"]
mod tests_unit;
