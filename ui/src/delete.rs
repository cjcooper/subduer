//! Deleting the selected object: the header button and the confirmation
//! dialog. The dialog names the object and context; production-like contexts
//! are flagged (their name was typed when changes were allowed), and cascading
//! kinds need the object's name typed out.

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use subduer_types::{ContextInfo, KindRef, ResourceRow};

use crate::modal::Modal;
use crate::app::Store;
use crate::tauri::invoke;

/// What the dialog is about to delete, captured when it opens.
#[derive(Debug, Clone, PartialEq)]
pub struct DeleteTarget {
    pub context: ContextInfo,
    pub kind: KindRef,
    pub row: ResourceRow,
}

/// Kinds whose deletion takes much more with it.
fn cascades(kind: &KindRef) -> bool {
    matches!(
        (kind.group.as_str(), kind.kind.as_str()),
        ("", "Namespace" | "Node" | "PersistentVolume") | ("apiextensions.k8s.io", "CustomResourceDefinition")
    )
}

fn consequence(kind: &KindRef) -> Option<&'static str> {
    Some(match (kind.group.as_str(), kind.kind.as_str()) {
        ("", "Namespace") => "Everything inside the namespace is deleted too.",
        ("apiextensions.k8s.io", "CustomResourceDefinition") => {
            "Every object of this custom type is deleted, across all namespaces."
        }
        ("", "Node") => "The node is removed from the cluster without being drained first.",
        ("", "PersistentVolume" | "PersistentVolumeClaim") => {
            "The stored data may be deleted too, depending on the reclaim policy."
        }
        ("apps", "Deployment" | "StatefulSet" | "DaemonSet" | "ReplicaSet") | ("batch", "Job" | "CronJob") => {
            "Its pods are deleted too."
        }
        ("", "Pod") => "If a controller owns this pod, it will start a replacement.",
        _ => return None,
    })
}

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
        let allowed = store.cluster.with(|c| {
            c.iter()
                .flat_map(|c| &c.kinds)
                .any(|k| k.kind == kind && k.verbs.iter().any(|v| v == "delete"))
        });
        allowed.then_some(kind)
    };
    move || {
        kind().map(|kind| {
            let row = row.clone();
            let locked = move || store.current_context().is_some_and(|c| !c.unlocked);
            let title = move || {
                if locked() {
                    "Delete… (asks to allow changes first: the context is read-only)"
                } else {
                    "Delete…"
                }
            };
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
            let must_type = cascades(&kind);
            let ok = {
                let name = row.name.clone();
                move || !busy.get() && (!must_type || typed.get().trim() == name)
            };
            let confirm = {
                let target = target.clone();
                move |_| {
                    let target = target.clone();
                    busy.set(true);
                    error.set(None);
                    spawn_local(async move {
                        #[derive(Serialize)]
                        struct Args {
                            context: String,
                            kind: KindRef,
                            namespace: Option<String>,
                            name: String,
                        }
                        let args = Args {
                            context: target.context.name.clone(),
                            kind: target.kind.clone(),
                            namespace: target.row.namespace.clone(),
                            name: target.row.name.clone(),
                        };
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
                        <h2>{format!("Delete {}?", kind.kind)}</h2>
                        <p>
                            <strong class="mono">{row.name.clone()}</strong>
                            {location}
                            " on context "<strong>{context.name.clone()}</strong>" will be deleted."
                        </p>
                        {consequence(&kind).map(|c| view! { <p class="warn-text">{c}</p> })}
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
