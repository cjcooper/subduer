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
use crate::logic::context_picker::{self as logic, KeyAction};

#[component]
pub fn ContextPicker(store: Store) -> impl IntoView {
    let open = RwSignal::new(false);
    let active = RwSignal::new(None::<usize>);
    let root = NodeRef::<html::Span>::new();
    let list = NodeRef::<html::Ul>::new();

    let show = move || {
        let current = store.context.get_untracked();
        active.set(store.contexts.with_untracked(|cs| logic::opening_index(cs, current.as_deref())));
        open.set(true);
    };
    // Picking the context already shown does nothing, as with a native menu.
    // A trace of the current context stops first; then unsaved edits ask.
    let choose = move |ctx: String| {
        open.set(false);
        if !logic::is_switch(store.context.get_untracked().as_deref(), &ctx) {
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
        if let Some(to) = logic::scroll_into_view(top, bottom, list.scroll_top(), list.client_height()) {
            list.set_scroll_top(to);
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
        let names: Vec<String> = store.contexts.with_untracked(|cs| cs.iter().map(|c| c.name.clone()).collect());
        match logic::key_action(&ev.key(), open.get_untracked(), active.get_untracked(), &names) {
            KeyAction::Ignore => {}
            KeyAction::Open => {
                ev.prevent_default();
                show();
            }
            KeyAction::Escape => {
                // Not the detail panel's or a dialog's Esc.
                ev.prevent_default();
                ev.stop_propagation();
                open.set(false);
            }
            KeyAction::Tab => open.set(false),
            KeyAction::Pick(index) => {
                ev.prevent_default();
                match index.map(|i| names[i].clone()) {
                    Some(name) => choose(name),
                    None => open.set(false),
                }
            }
            KeyAction::Highlight { index, prevent } => {
                if prevent {
                    ev.prevent_default();
                }
                active.set(Some(index));
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
                aria-activedescendant=move || logic::active_descendant(open.get(), active.get())
                // Stays enabled while connecting, so a context that doesn't
                // answer can be left without waiting for it to time out.
                on:click=move |_| if open.get_untracked() { open.set(false) } else { show() }
                on:keydown=on_key
            >
                {move || match shown() {
                    Some(c) => view! { <Entry context=c /> }.into_any(),
                    None => {
                        let label = logic::empty_label(store.contexts.with(Vec::is_empty));
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
                                id=logic::option_id(i)
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
