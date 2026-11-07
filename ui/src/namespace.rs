//! Namespace actions: New namespace (gated, from the Namespace list's header)
//! and Filter to it (sets the namespace menu to the selected namespace).

use leptos::prelude::*;
use leptos::task::spawn_local;
use subduer_types::ResourceRow;

use crate::app::{Store, guard};
use crate::logic::namespace::{self as logic, CreateArgs};
use crate::modal::Modal;
use crate::tauri::invoke;

fn on_namespaces(store: Store) -> bool {
    store.kind.with(|k| logic::is_namespaces(k.as_ref()))
}

/// "New namespace" in the Namespace list's header, when creating is allowed.
#[component]
pub fn NewNamespaceButton(store: Store) -> impl IntoView {
    let allowed = move || on_namespaces(store) && store.cluster.with(|c| c.as_ref().is_some_and(|c| logic::may_create(&c.kinds)));
    let open = move |_| {
        let Some(ctx) = store.current_context_untracked() else { return };
        if ctx.unlocked {
            store.new_namespace.set(true);
        } else {
            store.pending_new_namespace.set(true);
            store.unlock_prompt.set(Some(ctx));
        }
    };
    move || {
        allowed().then(|| {
            let title = logic::new_title(store.current_context().is_some_and(|c| !c.unlocked));
            view! { <button class="btn small" title=title on:click=open>"New namespace"</button> }
        })
    }
}

#[component]
pub fn NewNamespaceDialog(store: Store) -> impl IntoView {
    let name = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let close = move || {
        name.set(String::new());
        error.set(None);
        busy.set(false);
        store.new_namespace.set(false);
    };
    let exists = move || {
        let n = name.get();
        store.table.with(|t| t.as_ref().is_some_and(|t| logic::exists(&t.rows, &n)))
    };
    let ok = move || logic::can_create(&name.get(), exists(), busy.get());
    let create = move || {
        if !ok() {
            return;
        }
        let Some(ctx) = store.current_context_untracked() else { return };
        let new = name.get_untracked().trim().to_owned();
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            match invoke::<_, ()>("create_namespace", &CreateArgs { context: ctx.name, name: new.clone() }).await {
                Ok(()) => {
                    close();
                    // Select it once the watch lists it.
                    store.pending_select.set(Some(new));
                }
                Err(e) => {
                    busy.set(false);
                    error.set(Some(e));
                }
            }
        });
    };
    move || {
        store.new_namespace.get().then(|| {
            let context = store.current_context_untracked();
            let prod = context.as_ref().is_some_and(|c| c.prod_like);
            let ctx_name = context.map(|c| c.name).unwrap_or_default();
            let hint = move || logic::hint(&name.get(), exists());
            view! {
                <Modal on_cancel=move |()| close() busy=busy prod=prod>
                        <h2>"New namespace"</h2>
                        <p>"Create a namespace on context "<strong>{ctx_name}</strong>"."</p>
                        {prod.then(|| view! { <p>"This context looks like "<strong>"PRODUCTION"</strong>"."</p> })}
                        <input
                            class="confirm-input mono"
                            placeholder="name, e.g. team-sandbox"
                            autofocus
                            prop:value=move || name.get()
                            on:input=move |ev| name.set(event_target_value(&ev))
                            on:keydown=move |ev| if ev.key() == "Enter" { create() }
                        />
                        {move || hint().map(|h| view! { <p class="warn-text">{h}</p> })}
                        {move || error.get().map(|e| view! { <p class="warn">{e}</p> })}
                        <div class="modal-actions">
                            <button prop:disabled=move || busy.get() on:click=move |_| close()>"Cancel"</button>
                            <button class="primary" prop:disabled=move || !ok() on:click=move |_| create()>
                                {move || if busy.get() { "Creating…" } else { "Create" }}
                            </button>
                        </div>
                </Modal>
            }
        })
    }
}

/// On a selected namespace: sets the namespace menu to it.
#[component]
pub fn FilterButton(store: Store, row: ResourceRow) -> impl IntoView {
    move || {
        on_namespaces(store).then(|| {
            let name = row.name.clone();
            let current = store.namespace.get().as_deref() == Some(name.as_str());
            let (title, label) = logic::filter_button(&name, current);
            view! {
                <button
                    class="btn small"
                    title=title
                    prop:disabled=current
                    on:click=move |_| {
                        let name = name.clone();
                        guard(store, move || store.namespace.set(Some(name)));
                    }
                >{label}</button>
            }
        })
    }
}
