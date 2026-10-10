//! User-resizable table columns. Widths are kept per table (a kind key, or
//! `nodes`) and per header, and saved in localStorage.

use std::collections::HashMap;

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::prefs::storage;

const KEY: &str = "table.columnWidths";
const MIN_WIDTH: f64 = 40.0;
/// Width for a column that appears after the table's widths were set.
const FALLBACK_WIDTH: f64 = 120.0;

/// `(table, header) -> width in px`.
#[derive(Clone, Copy)]
pub struct ColumnWidths(RwSignal<HashMap<(String, String), f64>>);

impl ColumnWidths {
    pub fn load() -> Self {
        let saved = storage().and_then(|s| s.get_item(KEY).ok().flatten()).unwrap_or_default();
        let map = saved
            .lines()
            .filter_map(|line| {
                let mut f = line.split('\t');
                let (table, header, width) = (f.next()?, f.next()?, f.next()?.parse().ok()?);
                Some(((table.to_owned(), header.to_owned()), width))
            })
            .collect();
        Self(RwSignal::new(map))
    }

    fn save(&self) {
        let text = self.0.with_untracked(|m| {
            m.iter()
                .map(|((t, h), w)| format!("{t}\t{h}\t{w:.0}"))
                .collect::<Vec<_>>()
                .join("\n")
        });
        if let Some(s) = storage() {
            let _ = s.set_item(KEY, &text);
        }
    }

    pub fn get(&self, table: &str, header: &str) -> Option<f64> {
        self.0.with(|m| m.get(&(table.to_owned(), header.to_owned())).copied())
    }

    pub fn has_table(&self, table: &str) -> bool {
        self.0.with(|m| m.keys().any(|(t, _)| t == table))
    }

    fn set(&self, table: &str, header: &str, width: f64) {
        self.0.update(|m| {
            m.insert((table.to_owned(), header.to_owned()), width.max(MIN_WIDTH));
        });
    }

    fn reset(&self, table: &str) {
        self.0.update(|m| m.retain(|(t, _), _| t != table));
        self.save();
    }

    /// Fixed layout at the summed width once the table has custom widths;
    /// otherwise the browser's automatic layout.
    pub fn table_style(&self, table: &str, headers: &[String]) -> String {
        if !self.has_table(table) {
            return String::new();
        }
        let total: f64 = headers
            .iter()
            .map(|h| self.get(table, h).unwrap_or(FALLBACK_WIDTH))
            .sum();
        format!("table-layout: fixed; width: {total:.0}px")
    }
}

/// A header cell with a drag handle on its right edge. Double-click the handle
/// to go back to automatic widths for the whole table.
#[component]
pub fn ResizableTh(
    widths: ColumnWidths,
    table: String,
    header: String,
    #[prop(optional)] class: &'static str,
    #[prop(optional, into)] title: String,
) -> impl IntoView {
    // (pointer x, column width) when the drag started.
    let drag = StoredValue::new(None::<(f64, f64)>);
    let style = {
        let (table, header) = (table.clone(), header.clone());
        move || widths.get(&table, &header).map(|w| format!("width: {w:.0}px")).unwrap_or_default()
    };
    let down = {
        let table = table.clone();
        move |ev: leptos::ev::PointerEvent| {
            ev.prevent_default();
            ev.stop_propagation();
            let Some(handle) = ev.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) else { return };
            let Some(th) = handle.parent_element() else { return };
            // First resize of this table: pin every column at its current width
            // so switching to fixed layout doesn't make the others jump.
            if !widths.has_table(&table)
                && let Some(row) = th.parent_element()
            {
                let cells = row.children();
                for i in 0..cells.length() {
                    if let Some(cell) = cells.item(i)
                        && let Some(h) = cell.get_attribute("data-header")
                    {
                        widths.set(&table, &h, cell.get_bounding_client_rect().width());
                    }
                }
            }
            drag.set_value(Some((ev.client_x() as f64, th.get_bounding_client_rect().width())));
            let _ = handle.set_pointer_capture(ev.pointer_id());
        }
    };
    let moved = {
        let (table, header) = (table.clone(), header.clone());
        move |ev: leptos::ev::PointerEvent| {
            if let Some((x0, w0)) = drag.get_value() {
                widths.set(&table, &header, w0 + ev.client_x() as f64 - x0);
            }
        }
    };
    let end = move |_| {
        if drag.get_value().is_some() {
            drag.set_value(None);
            widths.save();
        }
    };
    let reset = move |ev: leptos::ev::MouseEvent| {
        ev.stop_propagation();
        widths.reset(&table);
    };

    view! {
        <th class=class style=style title=title data-header=header.clone()>
            {header.clone()}
            <span
                class="col-resize"
                title="Drag to resize · double-click to reset"
                on:pointerdown=down
                on:pointermove=moved
                on:pointerup=end
                on:lostpointercapture=end
                on:click=|ev| ev.stop_propagation()
                on:dblclick=reset
            ></span>
        </th>
    }
}
