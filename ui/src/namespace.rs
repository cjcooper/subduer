//! Namespace actions: New namespace (gated, from the Namespace list's header)
//! and Filter to it (sets the namespace menu to the selected namespace).

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use subduer_types::ResourceRow;

use crate::modal::Modal;
use crate::app::{Store, guard};
use crate::tauri::invoke;

fn on_namespaces(store: Store) -> bool {
    store.kind.with(|k| k.as_ref().is_some_and(|k| k.key() == "core/Namespace"))
}

/// DNS-1123 label: lowercase letters, digits and `-`, starting and ending
/// with a letter or digit, at most 63 characters.
fn valid_name(name: &str) -> bool {
    let ok = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    !name.is_empty() && name.len() <= 63 && name.starts_with(ok) && name.ends_with(ok) && name.chars().all(|c| ok(c) || c == '-')
}

/// "New namespace" in the Namespace list's header, when creating is allowed.
#[component]
pub fn NewNamespaceButton(store: Store) -> impl IntoView {
    let allowed = move || {
        on_namespaces(store)
            && store.cluster.with(|c| {
                c.iter()
                    .flat_map(|c| &c.kinds)
                    .any(|k| k.kind.key() == "core/Namespace" && k.verbs.iter().any(|v| v == "create"))
            })
    };
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
            let locked = store.current_context().is_some_and(|c| !c.unlocked);
            let title = if locked { "Create a namespace… (asks to allow changes first)" } else { "Create a namespace…" };
            view! { <button class="btn small" title=title on:click=open>"New namespace"</button> }
        })
    }
}

#[component]
pub fn NewNamespaceDialog(store: Store) -> impl IntoView {
    #[derive(Serialize)]
    struct Args {
        context: String,
        name: String,
    }
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
        store.table.with(|t| t.as_ref().is_some_and(|t| t.rows.iter().any(|r| r.name == n.trim())))
    };
    let ok = move || valid_name(name.get().trim()) && !exists() && !busy.get();
    let create = move || {
        if !ok() {
            return;
        }
        let Some(ctx) = store.current_context_untracked() else { return };
        let new = name.get_untracked().trim().to_owned();
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            match invoke::<_, ()>("create_namespace", &Args { context: ctx.name, name: new.clone() }).await {
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
            let hint = move || {
                let n = name.get();
                let n = n.trim();
                if n.is_empty() {
                    None
                } else if !valid_name(n) {
                    Some("Use lowercase letters, digits and '-', starting and ending with a letter or digit (63 at most).")
                } else if exists() {
                    Some("A namespace with that name already exists.")
                } else {
                    None
                }
            };
            view! {
                <Modal on_cancel=move |()| close() busy=busy prod=prod>
                        <h2>"New namespace"</h2>
                        <p>"Create a namespace on context "<strong>{ctx_name}</strong>"."</p>
                        {prod.then(|| view! { <p>"This context looks like "<strong>"PRODUCTION"</strong>"."</p> })}
                        <input
                            class="confirm-input mono"
                            placeholder="name, e.g. team-sandbox"
                            spellcheck="false"
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
            let title = format!("Show only objects in {name} in every list");
            view! {
                <button
                    class="btn small"
                    title=title
                    prop:disabled=current
                    on:click=move |_| {
                        let name = name.clone();
                        guard(store, move || store.namespace.set(Some(name)));
                    }
                >{if current { "Filtering to it" } else { "Filter to it" }}</button>
            }
        })
    }
}

#[cfg(test)]
#[path = "namespace_tests.rs"]
mod tests_unit;
