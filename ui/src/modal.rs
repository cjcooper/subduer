//! The frame every dialog is drawn in (backdrop, card), and Esc.
//!
//! Open dialogs form a stack, per window: the newest is drawn on top and is
//! the one Esc cancels.

use std::cell::{Cell, RefCell};

use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::logic::modal as logic;

struct Open {
    id: u64,
    on_cancel: Callback<()>,
    busy: Signal<bool>,
}

thread_local! {
    static OPEN: RefCell<Vec<Open>> = const { RefCell::new(Vec::new()) };
    static NEXT_ID: Cell<u64> = const { Cell::new(0) };
    static LISTENING: Cell<bool> = const { Cell::new(false) };
}

/// A dialog. Esc runs `on_cancel` (what its Cancel, Close or Keep editing
/// button does), unless `busy` is set: a dialog sets it while its action
/// runs, and disables that button too, so a request in flight isn't hidden.
#[component]
pub fn Modal(
    #[prop(into)] on_cancel: Callback<()>,
    #[prop(into, optional)] busy: Signal<bool>,
    /// Red frame for a production-like context.
    #[prop(optional)] prod: bool,
    #[prop(optional)] wide: bool,
    /// Clicking outside the card cancels too.
    #[prop(optional)] dismiss_outside: bool,
    /// Extra classes on the card, for a dialog's own layout.
    #[prop(optional)] class: &'static str,
    children: Children,
) -> impl IntoView {
    listen_once();
    let id = NEXT_ID.with(|n| n.replace(n.get() + 1));
    let depth = OPEN.with(|o| {
        let mut o = o.borrow_mut();
        o.push(Open { id, on_cancel, busy });
        o.len()
    });
    on_cleanup(move || OPEN.with(|o| o.borrow_mut().retain(|d| d.id != id)));

    let backdrop = NodeRef::<html::Div>::new();
    let outside = move |ev: leptos::ev::MouseEvent| {
        let on_backdrop = ev.target().is_some_and(|t| backdrop.get_untracked().is_some_and(|b| b.is_same_node(t.dyn_ref())));
        if logic::click_cancels(dismiss_outside, on_backdrop, busy.get_untracked()) {
            on_cancel.run(());
        }
    };
    view! {
        // Newer dialogs draw over older ones (the stylesheet's base is 10).
        <div class="modal-backdrop" style:z-index=logic::backdrop_z(depth) node_ref=backdrop on:click=outside>
            <div class=logic::card_class(class) class:prod=prod class:wide=wide>
                {children()}
            </div>
        </div>
    }
}

/// Whether a dialog is open in this window.
pub fn any_open() -> bool {
    OPEN.with(|o| !o.borrow().is_empty())
}

/// One Esc press for us to act on: see [`logic::is_plain_esc`].
pub fn is_plain_esc(ev: &web_sys::KeyboardEvent) -> bool {
    logic::is_plain_esc(&ev.key(), ev.repeat(), ev.default_prevented(), ev.is_composing(), ev.key_code())
}

/// One keydown listener per window, for whichever dialog is on top.
fn listen_once() {
    if LISTENING.with(|l| l.replace(true)) {
        return;
    }
    let handle = window_event_listener(leptos::ev::keydown, |ev| {
        if !is_plain_esc(&ev) {
            return;
        }
        // Copied out: cancelling closes the dialog, whose cleanup edits the stack.
        let Some((on_cancel, busy)) = OPEN.with(|o| o.borrow().last().map(|d| (d.on_cancel, d.busy))) else { return };
        // Handled even while busy, so the detail panel leaves this Esc alone.
        ev.prevent_default();
        if !busy.get_untracked() {
            on_cancel.run(());
        }
    });
    // Lives as long as the window.
    std::mem::forget(handle);
}

/// Tells a request's late result whether its dialog was closed or reopened
/// meanwhile. A dialog calls [`Generation::next`] each time it opens or closes;
/// a request keeps [`Generation::now`] and drops its result unless
/// [`Generation::still`] holds when it lands.
#[derive(Clone, Copy)]
pub struct Generation(StoredValue<u64>);

impl Generation {
    pub fn new() -> Self {
        Self(StoredValue::new(0))
    }

    pub fn next(self) {
        self.0.update_value(|g| *g = logic::next_generation(*g));
    }

    pub fn now(self) -> u64 {
        self.0.get_value()
    }

    pub fn still(self, then: u64) -> bool {
        logic::still(self.now(), then)
    }
}
