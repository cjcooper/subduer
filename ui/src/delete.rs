//! Deleting the selected object: the header button and the confirmation
//! dialog. The dialog names the object and context; production-like contexts
//! are flagged (their name was typed when changes were allowed), and cascading
//! kinds need the object's name typed out.

use leptos::prelude::*;
use leptos::task::spawn_local;
use subduer_types::{KindRef, ResourceRow};

use crate::app::Store;
use crate::logic::delete::{self as logic, DeleteArgs};
pub use crate::logic::delete::DeleteTarget;
use crate::modal::Modal;
use crate::tauri::invoke;

/// Opens the dialog, or the unlock prompt first if the context is read-only;
/// the dialog then follows once the context is unlocked.
pub fn request_delete(store: Store, kind: KindRef, row: ResourceRow) {
    let Some(context) = store.current_context_untracked() else { return };
    let target = DeleteTarget { context: context.clone(), kind, row };
    if context.unlocked {
        store.delete_prompt.set(Some(target));
    } else {
        store.pending_delete.set(Some(target));
        store.unlock_prompt.set(Some(context));
    }
}

/// Header button for the selected object; hidden when the API doesn't allow
/// deleting this kind.
#[component]
pub fn DeleteButton(store: Store, row: ResourceRow) -> impl IntoView {
    let kind = move || {
        let kind = store.kind.get()?;
        let allowed = store.cluster.with(|c| c.as_ref().is_some_and(|c| logic::allows(&c.kinds, &kind, "delete")));
        allowed.then_some(kind)
    };
    move || {
        kind().map(|kind| {
            let row = row.clone();
            let locked = move || store.current_context().is_some_and(|c| !c.unlocked);
            let title = move || logic::button_title(locked());
            view! {
                <button
                    class="btn small danger-outline"
                    title=title
                    on:click=move |_| request_delete(store, kind.clone(), row.clone())
                >"Delete"</button>
            }
        })
    }
}

#[component]
pub fn DeleteDialog(store: Store) -> impl IntoView {
    let typed = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let close = move || {
        typed.set(String::new());
        error.set(None);
        busy.set(false);
        store.delete_prompt.set(None);
    };

    move || {
        store.delete_prompt.get().map(|target| {
            let DeleteTarget { context, kind, row } = target.clone();
            let must_type = logic::must_type(&target);
            let ok = {
                let name = row.name.clone();
                move || logic::can_confirm(busy.get(), must_type, &typed.get(), &name)
            };
            let confirm = {
                let target = target.clone();
                move |_| {
                    let target = target.clone();
                    busy.set(true);
                    error.set(None);
                    spawn_local(async move {
                        let args = DeleteArgs::new(&target);
                        match invoke::<_, ()>("delete_resource", &args).await {
                            Ok(()) => {
                                if store.selected.with_untracked(|s| s.as_ref().is_some_and(|s| s.id == target.row.id)) {
                                    store.selected.set(None);
                                }
                                close();
                            }
                            Err(e) => {
                                busy.set(false);
                                error.set(Some(e));
                            }
                        }
                    });
                }
            };
            let location = match &row.namespace {
                Some(ns) => view! { " in namespace "<strong>{ns.clone()}</strong> }.into_any(),
                None => ().into_any(),
            };
            view! {
                <Modal on_cancel=move |()| close() busy=busy prod=context.prod_like>
                        <h2>{logic::heading(&kind)}</h2>
                        <p>
                            <strong class="mono">{row.name.clone()}</strong>
                            {location}
                            " on context "<strong>{context.name.clone()}</strong>" will be deleted."
                        </p>
                        {logic::consequence(&kind).map(|c| view! { <p class="warn-text">{c}</p> })}
                        {context.prod_like.then(|| view! {
                            <p>"This context looks like "<strong>"PRODUCTION"</strong>"."</p>
                        })}
                        {must_type.then(|| view! {
                            <p>"Type the name to confirm:"</p>
                            <input
                                class="confirm-input"
                                prop:value=move || typed.get()
                                on:input=move |ev| typed.set(event_target_value(&ev))
                            />
                        })}
                        {move || error.get().map(|e| view! { <p class="warn">{e}</p> })}
                        <div class="modal-actions">
                            <button prop:disabled=move || busy.get() on:click=move |_| close()>"Cancel"</button>
                            <button class="danger" prop:disabled=move || !ok() on:click=confirm.clone()>
                                {move || if busy.get() { "Deleting…" } else { "Delete" }}
                            </button>
                        </div>
                </Modal>
            }
        })
    }
}

#[cfg(test)]
#[path = "delete_tests.rs"]
mod tests_unit;
