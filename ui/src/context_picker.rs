//! The top bar's Context menu: each context with its cloud's logo, and a red
//! sign on production-looking ones. A native `<select>` can't show either,
//! so this is a button and a listbox, worked like one: arrows, Home and End
//! move, Enter or a click picks, Esc or a click elsewhere closes, and typing
//! a letter jumps to the next context starting with it.

use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;
use subduer_types::ContextInfo;
use wasm_bindgen::JsCast;

use crate::app::{Store, guard};
use crate::icons::{ProdIcon, ProviderLogo};

/// The index after `from` for `key` in a list of `len` (`None` if the key
/// doesn't move the highlight).
pub(crate) fn step(key: &str, from: Option<usize>, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let last = len - 1;
    match key {
        "ArrowDown" => Some(from.map_or(0, |i| (i + 1).min(last))),
        "ArrowUp" => Some(from.map_or(last, |i| i.saturating_sub(1))),
        "Home" | "PageUp" => Some(0),
        "End" | "PageDown" => Some(last),
        _ => None,
    }
}

/// The next context after `from` whose name starts with `letter`, wrapping round.
pub(crate) fn jump(names: &[String], from: Option<usize>, letter: char) -> Option<usize> {
    let letter = letter.to_lowercase().to_string();
    let start = from.map_or(0, |i| i + 1);
    (0..names.len()).map(|n| (start + n) % names.len()).find(|&i| names[i].to_lowercase().starts_with(&letter))
}

#[component]
pub fn ContextPicker(store: Store) -> impl IntoView {
    let open = RwSignal::new(false);
    let active = RwSignal::new(None::<usize>);
    let root = NodeRef::<html::Span>::new();
    let list = NodeRef::<html::Ul>::new();

    let selected_index = move || {
        let current = store.context.get_untracked();
        store.contexts.with_untracked(|cs| cs.iter().position(|c| Some(&c.name) == current.as_ref()))
    };
    let show = move || {
        active.set(selected_index().or(Some(0)));
        open.set(true);
    };
    // Picking the context already shown does nothing, as with a native menu.
    // A trace of the current context stops first; then unsaved edits ask.
    let choose = move |ctx: String| {
        open.set(false);
        if store.context.get_untracked().as_deref() == Some(ctx.as_str()) {
            return;
        }
        crate::trace::before_switch(store, ctx.clone(), move || guard(store, move || spawn_local(store.connect(ctx))));
    };

    // Keeps the highlighted row in view as the arrows move it.
    Effect::new(move |_| {
        let (Some(i), Some(list)) = (active.get(), list.get()) else { return };
        if !open.get() {
            return;
        }
        let Some(row) = list.children().item(i as u32).and_then(|r| r.dyn_into::<web_sys::HtmlElement>().ok()) else { return };
        let (top, bottom) = (row.offset_top(), row.offset_top() + row.offset_height());
        if top < list.scroll_top() {
            list.set_scroll_top(top);
        } else if bottom > list.scroll_top() + list.client_height() {
            list.set_scroll_top(bottom - list.client_height());
        }
    });

    // A click anywhere else closes it.
    let outside = window_event_listener(leptos::ev::mousedown, move |ev| {
        if !open.get_untracked() {
            return;
        }
        let inside = ev.target().and_then(|t| t.dyn_into::<web_sys::Node>().ok()).is_some_and(|t| root.get_untracked().is_some_and(|r| r.contains(Some(&t))));
        if !inside {
            open.set(false);
        }
    });
    on_cleanup(move || outside.remove());

    let on_key = move |ev: leptos::ev::KeyboardEvent| {
        let key = ev.key();
        let len = store.contexts.with_untracked(Vec::len);
        if !open.get_untracked() {
            if matches!(key.as_str(), "ArrowDown" | "ArrowUp" | "Enter" | " ") {
                ev.prevent_default();
                show();
            }
            return;
        }
        match key.as_str() {
            "Escape" => {
                // Not the detail panel's or a dialog's Esc.
                ev.prevent_default();
                ev.stop_propagation();
                open.set(false);
            }
            "Tab" => open.set(false),
            "Enter" | " " => {
                ev.prevent_default();
                let name = active.get_untracked().and_then(|i| store.contexts.with_untracked(|cs| cs.get(i).map(|c| c.name.clone())));
                match name {
                    Some(name) => choose(name),
                    None => open.set(false),
                }
            }
            _ => {
                if let Some(i) = step(&key, active.get_untracked(), len) {
                    ev.prevent_default();
                    active.set(Some(i));
                } else if let Some(letter) = key.chars().next().filter(|c| key.chars().count() == 1 && c.is_alphanumeric()) {
                    let names: Vec<String> = store.contexts.with_untracked(|cs| cs.iter().map(|c| c.name.clone()).collect());
                    if let Some(i) = jump(&names, active.get_untracked(), letter) {
                        active.set(Some(i));
                    }
                }
            }
        }
    };

    let shown = move || store.current_context();
    view! {
        <span class="context-picker" node_ref=root>
            <button
                class="context-button"
                class:prod=move || shown().is_some_and(|c| c.prod_like)
                aria-haspopup="listbox"
                aria-expanded=move || open.get().to_string()
                aria-activedescendant=move || open.get().then(|| active.get().map(|i| format!("context-option-{i}"))).flatten()
                // Stays enabled while connecting, so a context that doesn't
                // answer can be left without waiting for it to time out.
                on:click=move |_| if open.get_untracked() { open.set(false) } else { show() }
                on:keydown=on_key
            >
                {move || match shown() {
                    Some(c) => view! { <Entry context=c /> }.into_any(),
                    None => {
                        let label = if store.contexts.with(Vec::is_empty) { "No contexts" } else { "Choose a context" };
                        view! { <span class="context-name muted">{label}</span> }.into_any()
                    }
                }}
                <svg class="context-caret" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4"
                    stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6.5 9.5 12 15l5.5-5.5" /></svg>
            </button>
            <ul class="context-menu" role="listbox" aria-label="Contexts" node_ref=list hidden=move || !open.get()>
                <For
                    each=move || store.contexts.get().into_iter().enumerate()
                    key=|(i, c)| (*i, c.name.clone(), c.provider, c.prod_like)
                    children=move |(i, c)| {
                        let name = StoredValue::new(c.name.clone());
                        let pick = c.name.clone();
                        let selected = move || store.context.with(|ctx| name.with_value(|n| ctx.as_ref() == Some(n)));
                        view! {
                            <li
                                id=format!("context-option-{i}")
                                role="option"
                                class="context-option"
                                class:active=move || active.get() == Some(i)
                                class:selected=selected
                                aria-selected=move || selected().to_string()
                                on:mousemove=move |_| if active.get_untracked() != Some(i) { active.set(Some(i)) }
                                // Not mousedown: that would also close it as an outside click.
                                on:click=move |_| choose(pick.clone())
                            >
                                <Entry context=c />
                            </li>
                        }
                    }
                />
            </ul>
        </span>
    }
}

/// A context's logo, name and (production-looking) sign.
#[component]
fn Entry(context: ContextInfo) -> impl IntoView {
    view! {
        <ProviderLogo provider=context.provider />
        <span class="context-name" title=context.name.clone()>{context.name.clone()}</span>
        {context.prod_like.then(|| view! { <ProdIcon /> })}
    }
}

#[cfg(test)]
#[path = "context_picker_tests.rs"]
mod tests;
