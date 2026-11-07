//! User-resizable table columns. Widths are kept per table (a kind key, or
//! `nodes`) and per header, and saved in localStorage.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::logic::columns::{self as logic, WidthState};
use crate::prefs::storage;

const KEY: &str = "table.columnWidths";

/// `(table, header) -> width in px`.
#[derive(Clone, Copy)]
pub struct ColumnWidths(RwSignal<WidthState>);

impl ColumnWidths {
    pub fn load() -> Self {
        let saved = storage().and_then(|s| s.get_item(KEY).ok().flatten()).unwrap_or_default();
        Self::from_saved(&saved)
    }

    /// Widths from their saved form (see [`WidthState::from_saved`]).
    pub(crate) fn from_saved(saved: &str) -> Self {
        Self(RwSignal::new(WidthState::from_saved(saved)))
    }

    fn save(&self) {
        let text = self.0.with_untracked(WidthState::saved);
        if let Some(s) = storage() {
            let _ = s.set_item(KEY, &text);
        }
    }

    pub fn has_table(&self, table: &str) -> bool {
        self.0.with(|m| m.has_table(table))
    }

    fn set(&self, table: &str, header: &str, width: f64) {
        self.0.update(|m| m.set(table, header, width));
    }

    fn reset(&self, table: &str) {
        self.0.update(|m| m.forget(table));
        self.save();
    }

    /// See [`WidthState::table_style`].
    pub fn table_style(&self, table: &str, headers: &[String]) -> String {
        self.0.with(|m| m.table_style(table, headers))
    }

    fn cell_style(&self, table: &str, header: &str) -> String {
        self.0.with(|m| m.cell_style(table, header))
    }
}

/// A header cell with a drag handle on its right edge. Double-click the handle
/// to go back to automatic widths for the whole table. With `sorts`, the
/// heading also sorts the table (kept under the same table name).
#[component]
pub fn ResizableTh(
    widths: ColumnWidths,
    table: String,
    header: String,
    #[prop(optional)] class: &'static str,
    #[prop(optional, into)] title: String,
    #[prop(optional)] sorts: Option<crate::sort::Sorts>,
) -> impl IntoView {
    let label = match sorts {
        Some(sorts) => view! { <crate::sort::SortButton sorts=sorts table=table.clone() header=header.clone() /> }.into_any(),
        None => header.clone().into_any(),
    };
    let aria = {
        let (t, h) = (table.clone(), header.clone());
        move || logic::column_aria(sorts.is_some(), sorts.and_then(|s| s.get(&t)), &h)
    };
    // (pointer x, column width) when the drag started.
    let drag = StoredValue::new(None::<(f64, f64)>);
    let style = {
        let (table, header) = (table.clone(), header.clone());
        move || widths.cell_style(&table, &header)
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
                widths.set(&table, &header, logic::dragged_width(w0, x0, ev.client_x() as f64));
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
        <th class=class style=style title=title data-header=header.clone() aria-sort=aria>
            {label}
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
