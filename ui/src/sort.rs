//! Sortable tables. Clicking a column heading sorts by it ascending, again
//! descending, and a third time goes back to the table's own order. The
//! choice is kept per table (a kind key, `nodes`, a Details section, …) and
//! saved in localStorage. How rows sort is in [`crate::logic::sort`].

use leptos::prelude::*;

use crate::logic::sort::SortState;
pub(crate) use crate::logic::sort::{Key, sort};
use crate::prefs::storage;

const KEY: &str = "table.sort";

impl Key {
    /// An RFC 3339 time shown as an age: the youngest sorts first.
    pub fn age(time: &str) -> Self {
        Key::age_ms(js_date(time))
    }

    /// An RFC 3339 time shown as itself: the earliest sorts first.
    pub fn time(time: &str) -> Self {
        Key::time_ms(js_date(time))
    }
}

/// The browser's date parser (ms; NaN if it doesn't parse).
#[cfg(target_arch = "wasm32")]
pub(crate) fn js_date(s: &str) -> f64 {
    if s.trim().is_empty() { f64::NAN } else { js_sys::Date::parse(s) }
}

/// Host tests: whole-second UTC times (`2026-10-16T12:00:00Z`) only.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn js_date(s: &str) -> f64 {
    crate::logic::sort::parse_utc_ms(s)
}

/// `table -> (header, descending)`.
#[derive(Clone, Copy)]
pub struct Sorts(RwSignal<SortState>);

impl Sorts {
    pub fn load() -> Self {
        let saved = storage().and_then(|s| s.get_item(KEY).ok().flatten()).unwrap_or_default();
        Self::from_saved(&saved)
    }

    /// From `table\theader\tasc|desc` lines; malformed lines are skipped.
    pub(crate) fn from_saved(saved: &str) -> Self {
        Self(RwSignal::new(SortState::from_saved(saved)))
    }

    fn save(&self) {
        // Host tests have no window to save into.
        if !cfg!(target_arch = "wasm32") {
            return;
        }
        let text = self.0.with_untracked(SortState::saved);
        if let Some(s) = storage() {
            let _ = s.set_item(KEY, &text);
        }
    }

    /// The sorted header and whether it's descending.
    pub fn get(&self, table: &str) -> Option<(String, bool)> {
        self.0.with(|m| m.get(table))
    }

    /// The sorted column's index among `headers`, if it's one of them.
    pub fn by(&self, table: &str, headers: &[&str]) -> Option<(usize, bool)> {
        self.0.with(|m| m.by(table, headers))
    }

    /// Ascending, then descending, then the table's own order.
    pub fn click(&self, table: &str, header: &str) {
        self.0.update(|m| m.click(table, header));
        self.save();
    }

    /// `↑`, `↓` or nothing for one heading (spaced by CSS).
    pub fn arrow(&self, table: &str, header: &str) -> &'static str {
        self.0.with(|m| m.arrow(table, header))
    }

    /// Whether `header` is the column `table` is sorted by.
    pub fn is_sorted_by(&self, table: &str, header: &str) -> bool {
        self.0.with(|m| m.is_sorted_by(table, header))
    }

    /// The heading's `aria-sort`.
    pub fn aria(&self, table: &str, header: &str) -> &'static str {
        self.0.with(|m| m.aria(table, header))
    }
}

/// The clickable label inside a heading: the header text and its arrow.
#[component]
pub fn SortButton(sorts: Sorts, table: String, header: String) -> impl IntoView {
    let (t, h) = (table.clone(), header.clone());
    let on = move || sorts.is_sorted_by(&t, &h);
    let (t, h) = (table.clone(), header.clone());
    let arrow = move || sorts.arrow(&t, &h);
    let title = crate::logic::sort::sort_title(&header);
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
        move || sorts.aria(&t, &h)
    };
    view! {
        <th class=class aria-sort=aria><SortButton sorts=sorts table=table header=header /></th>
    }
}
