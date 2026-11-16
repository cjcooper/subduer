//! Resizable detail panes: a divider between a table and the pane beside it.
//! Drag it (or focus it and use the arrow keys) to resize; double-click to go
//! back to the default width. Widths are kept per pane in localStorage.
//!
//! The divider goes right before the pane, so its previous sibling is the
//! table and its next sibling the pane.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::logic::pane::{self as logic, PaneWidths};
use crate::prefs::storage;

const KEY: &str = "layout.paneWidths";

fn load_all() -> PaneWidths {
    PaneWidths::from_saved(&storage().and_then(|s| s.get_item(KEY).ok().flatten()).unwrap_or_default())
}

/// One pane's width: `None` until the user resizes it.
#[derive(Clone, Copy)]
pub struct PaneWidth {
    pane: &'static str,
    width: RwSignal<Option<f64>>,
}

impl PaneWidth {
    /// The saved width of `pane` (a short fixed name such as `detail`).
    pub fn load(pane: &'static str) -> Self {
        Self { pane, width: RwSignal::new(load_all().get(pane)) }
    }

    /// The pane's `style` attribute (reactive).
    pub fn style(&self) -> String {
        logic::pane_style(self.width.get())
    }

    /// Saves this pane's width alongside the others' (re-read, so another
    /// window's changes to other panes are kept).
    fn save(&self) {
        let mut all = load_all();
        all.put(self.pane, self.width.get_untracked());
        if let Some(s) = storage() {
            let _ = s.set_item(KEY, &all.saved());
        }
    }
}

/// The pane's current width and the room the pane and table share, measured
/// from the divider's siblings.
fn measure(divider: &web_sys::Element) -> Option<(f64, f64)> {
    let table = divider.previous_element_sibling()?.get_bounding_client_rect();
    let pane = divider.next_element_sibling()?.get_bounding_client_rect();
    Some((pane.width(), pane.right() - table.left()))
}

fn element(target: Option<web_sys::EventTarget>) -> Option<web_sys::Element> {
    target.and_then(|t| t.dyn_into::<web_sys::Element>().ok())
}

/// The divider. `label` names the pane for screen readers.
#[component]
pub fn PaneResizer(pane: PaneWidth, label: &'static str) -> impl IntoView {
    // (pointer x, pane width, room) when the drag started.
    let drag = StoredValue::new(None::<(f64, f64, f64)>);
    let down = move |ev: leptos::ev::PointerEvent| {
        if ev.button() != 0 {
            return;
        }
        let Some(handle) = element(ev.current_target()) else { return };
        let Some((width, room)) = measure(&handle) else { return };
        ev.prevent_default();
        drag.set_value(Some((ev.client_x() as f64, width, room)));
        let _ = handle.set_pointer_capture(ev.pointer_id());
    };
    let moved = move |ev: leptos::ev::PointerEvent| {
        // A click without a move leaves the default width alone.
        if let Some((x0, w0, room)) = drag.get_value()
            && ev.client_x() as f64 != x0
        {
            let w = logic::clamp_width(logic::dragged_width(w0, x0, ev.client_x() as f64), room);
            pane.width.set(Some(w));
        }
    };
    let end = move |_| {
        if drag.get_value().is_some() {
            drag.set_value(None);
            pane.save();
        }
    };
    let key = move |ev: leptos::ev::KeyboardEvent| {
        let Some(change) = logic::key_change(&ev.key()) else { return };
        let Some((width, room)) = element(ev.current_target()).as_ref().and_then(measure) else { return };
        ev.prevent_default();
        pane.width.set(Some(logic::clamp_width(width + change, room)));
        pane.save();
    };
    let reset = move |_| {
        pane.width.set(None);
        pane.save();
    };

    view! {
        <div
            class="pane-resizer"
            role="separator"
            aria-orientation="vertical"
            aria-label=format!("Resize {label}")
            tabindex="0"
            title="Drag to resize · double-click to reset"
            on:pointerdown=down
            on:pointermove=moved
            on:pointerup=end
            on:lostpointercapture=end
            on:keydown=key
            on:dblclick=reset
        ></div>
    }
}
