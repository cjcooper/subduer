//! File › Manage Clusters… (the menu is the only way in): the kubeconfig's
//! contexts, in the order the Context menu shows them. Drag a row, or use its
//! arrows, to move it (within its own file), remove one, or add one (Add
//! Cluster…, in [`crate::add_cluster`]). Every change is written to the
//! kubeconfig straight away, and every window's Context menu follows it.

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use subduer_types::{ManagedContext, ManagedContexts, RemovedContext};
use wasm_bindgen::JsCast;

use crate::app::Store;
use crate::icons::{ProdIcon, ProviderLogo};
use crate::logic::clusters::{self as logic, drop_index, goes_with, moved};
use crate::modal::{Generation, Modal};
use crate::tauri::{NoArgs, invoke};

#[component]
pub fn ManageClustersDialog(store: Store) -> impl IntoView {
    let list = RwSignal::new(None::<Result<ManagedContexts, String>>);
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let notice = RwSignal::new(None::<String>);
    let confirm = RwSignal::new(None::<String>);
    let dragging = RwSignal::new(None::<String>);
    let drop_at = RwSignal::new(None::<(String, bool)>);
    let opened = Generation::new();

    let load = move || {
        let at = opened.now();
        spawn_local(async move {
            let result = invoke::<_, ManagedContexts>("managed_contexts", &NoArgs {}).await;
            if opened.still(at) {
                list.set(Some(result));
            }
        });
    };
    Effect::new(move |was_open: Option<bool>| {
        let open = store.clusters_open.get();
        if open && was_open != Some(true) {
            opened.next();
        }
        // Reloaded whenever the Context menu is (a change here or in another window).
        store.contexts.track();
        if open {
            untrack(load);
        }
        open
    });
    let close = move || {
        opened.next();
        list.set(None);
        busy.set(false);
        error.set(None);
        notice.set(None);
        confirm.set(None);
        dragging.set(None);
        drop_at.set(None);
        store.clusters_open.set(false);
    };
    let contexts = move || list.with(|l| l.as_ref().and_then(|l| l.as_ref().ok()).map(|l| l.contexts.clone()).unwrap_or_default());

    let move_to = move |name: String, to: usize| {
        if busy.get_untracked() {
            return;
        }
        let Some(order) = moved(&contexts(), &name, to) else { return };
        #[derive(Serialize)]
        struct Args {
            order: Vec<String>,
        }
        // Shown moved straight away; put back from the file if the write fails.
        list.update(|l| {
            if let Some(Ok(l)) = l {
                logic::reorder(&mut l.contexts, &order);
            }
        });
        busy.set(true);
        error.set(None);
        notice.set(None);
        let at = opened.now();
        spawn_local(async move {
            let result = invoke::<_, ()>("reorder_contexts", &Args { order }).await;
            if opened.still(at) {
                busy.set(false);
                if let Err(e) = result {
                    error.set(Some(e));
                    load();
                }
            }
        });
    };
    let remove = move |name: String| {
        #[derive(Serialize)]
        struct Args {
            context: String,
        }
        busy.set(true);
        error.set(None);
        notice.set(None);
        let at = opened.now();
        spawn_local(async move {
            let result = invoke::<_, RemovedContext>("remove_context", &Args { context: name }).await;
            if opened.still(at) {
                busy.set(false);
                confirm.set(None);
                match result {
                    Ok(r) => notice.set(Some(logic::removed_text(&r))),
                    Err(e) => error.set(Some(e)),
                }
                load();
            }
        });
    };
    let set_production = move |context: String, production: bool| {
        #[derive(Serialize)]
        struct Args {
            context: String,
            production: bool,
        }
        error.set(None);
        spawn_local(async move {
            // The Context menu (and this list) follow when the backend says the contexts changed.
            if let Err(e) = invoke::<_, ()>("set_production", &Args { context, production }).await {
                error.set(Some(e));
            }
        });
    };
    let end_drag = move || {
        dragging.set(None);
        drop_at.set(None);
    };

    let row = move |c: ManagedContext, index: usize, count_in_file: (usize, usize)| {
        let name = c.name.clone();
        let (first, last) = count_in_file;
        // Shared by the row's closures, which are then Copy.
        let key = StoredValue::new(name.clone());
        let is = move |other: Option<&str>| key.with_value(|k| other == Some(k.as_str()));
        let is_dragging = move || dragging.with(|d| is(d.as_deref()));
        let target = move || drop_at.with(|d| d.as_ref().filter(|(t, _)| is(Some(t))).map(|(_, after)| *after));
        let here = move || store.context.with(|c| is(c.as_deref()));
        let elsewhere = move || store.in_use_elsewhere.with(|names| names.iter().any(|n| is(Some(n))));
        let in_use = move || here() || elsewhere();
        let current = move || list.with(|l| l.as_ref().and_then(|l| l.as_ref().ok()).is_some_and(|l| is(l.current.as_deref())));
        let prod = move || store.contexts.with(|cs| cs.iter().any(|x| x.prod_like && is(Some(&x.name))));
        let asking = move || confirm.with(|c| is(c.as_deref()));
        let file = c.file.clone();
        let (up, down, drag, over, start, ask, go) = (name.clone(), name.clone(), name.clone(), name.clone(), name.clone(), name.clone(), name.clone());
        let same_file = move || dragging.with(|d| logic::drag_in_file(&contexts(), d.as_deref(), &file));
        let same_file_drop = same_file.clone();
        let server = logic::server_label(&c.server);
        let (cluster_goes, user_goes) = goes_with(&contexts(), &name);
        view! {
            <li
                class="cluster-row"
                class:dragging=is_dragging
                class:drop-before=move || target() == Some(false)
                class:drop-after=move || target() == Some(true)
                on:dragover=move |ev: web_sys::DragEvent| {
                    if !same_file() {
                        return;
                    }
                    ev.prevent_default();
                    // Over the row's lower half: the moved row goes after it.
                    let after = ev.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()).is_some_and(|row| {
                        let r = row.get_bounding_client_rect();
                        logic::drops_after(f64::from(ev.client_y()), r.top(), r.height())
                    });
                    drop_at.set(Some((over.clone(), after)));
                }
                on:drop=move |ev: web_sys::DragEvent| {
                    ev.prevent_default();
                    let after = drop_at.get_untracked().is_some_and(|(_, a)| a);
                    if let Some(moving) = dragging.get_untracked().filter(|_| same_file_drop()) {
                        let list = contexts();
                        if let Some(from) = list.iter().position(|x| x.name == moving) {
                            move_to(moving, drop_index(from, index, after));
                        }
                    }
                    end_drag();
                }
            >
                <span
                    class="cluster-grip"
                    draggable=move || (!busy.get()).to_string()
                    title="Drag to reorder"
                    aria-hidden="true"
                    on:dragstart=move |ev: web_sys::DragEvent| {
                        // WebKit only starts a drag when data is set.
                        if let Some(dt) = ev.data_transfer() {
                            dt.set_effect_allowed("move");
                            let _ = dt.set_data("text/plain", &start);
                        }
                        dragging.set(Some(drag.clone()));
                    }
                    on:dragend=move |_| end_drag()
                >
                    "⠿"
                </span>
                <ProviderLogo provider=c.provider />
                <div class="cluster-text">
                    <div class="cluster-name">
                        <span class="mono">{name.clone()}</span>
                        {move || current().then(|| view! { <span class="badge" title="kubectl's current-context">"kubectl current"</span> })}
                        {move || here().then(|| view! { <span class="badge badge-running">"this window"</span> })}
                        {move || elsewhere().then(|| view! { <span class="badge badge-running">"another window"</span> })}
                    </div>
                    <div class="muted small mono cluster-server">{server}</div>
                </div>
                <div class="cluster-actions">
                    <button class="btn small icon prod-toggle" class:on=prod
                        aria-pressed=move || prod().to_string()
                        aria-label=logic::prod_label(&name)
                        title=move || logic::prod_title(prod())
                        prop:disabled=move || busy.get()
                        on:click=move |_| set_production(key.get_value(), !prod())>
                        <ProdIcon />
                    </button>
                    <button class="btn small icon" title="Move up" aria-label=logic::move_label(&name, true)
                        prop:disabled=move || busy.get() || index == first
                        on:click=move |_| move_to(up.clone(), index.wrapping_sub(1))>"↑"</button>
                    <button class="btn small icon" title="Move down" aria-label=logic::move_label(&name, false)
                        prop:disabled=move || busy.get() || index == last
                        on:click=move |_| move_to(down.clone(), index + 1)>"↓"</button>
                    <button class="btn small" prop:disabled=move || busy.get() || in_use()
                        title=move || logic::remove_title(here(), elsewhere())
                        on:click=move |_| { error.set(None); confirm.set(Some(ask.clone())) }>"Remove"</button>
                </div>
                {move || asking().then(|| {
                    let go = go.clone();
                    let question = logic::confirm_text(&go, cluster_goes.as_deref(), user_goes.as_deref());
                    view! {
                        <div class="cluster-confirm" role="alert">
                            <span>{question}</span>
                            <span class="cluster-confirm-actions">
                                <button class="btn small" prop:disabled=move || busy.get() on:click=move |_| confirm.set(None)>"Keep"</button>
                                // Another window may switch to it meanwhile.
                                <button class="btn small danger" prop:disabled=move || busy.get() || in_use() on:click=move |_| remove(go.clone())>
                                    {move || logic::remove_label(busy.get())}
                                </button>
                            </span>
                        </div>
                    }
                })}
            </li>
        }
    };

    move || {
        store.clusters_open.get().then(|| {
            let body = move || match list.get() {
                None => view! { <p class="muted">"Reading the kubeconfig…"</p> }.into_any(),
                Some(Err(e)) => view! { <pre class="warn add-cluster-error">{e}</pre> }.into_any(),
                Some(Ok(l)) if l.contexts.is_empty() => {
                    view! { <p class="muted cluster-empty">"No clusters in your kubeconfig yet. Add one to get started."</p> }.into_any()
                }
                Some(Ok(l)) => {
                    let several = logic::several_files(&l);
                    let mut items = vec![];
                    for (i, c) in l.contexts.iter().enumerate() {
                        let (first, last) = logic::file_bounds(&l.contexts, i);
                        if several && i == first {
                            items.push(view! { <li class="cluster-file muted small mono">{c.file.clone()}</li> }.into_any());
                        }
                        items.push(row(c.clone(), i, (first, last)).into_any());
                    }
                    view! {
                        {several.then(|| view! { <p class="muted small">"Your kubeconfig is several files: a cluster moves only among the others in its file."</p> })}
                        <ul class="cluster-list">{items}</ul>
                    }
                    .into_any()
                }
            };
            view! {
                <Modal on_cancel=move |()| close() busy=busy wide=true class="manage-clusters">
                    <h2>"Manage Clusters"</h2>
                    <p class="muted small cluster-intro">
                        "The Context menu lists these in this order. Changes are saved to your kubeconfig straight away, so kubectl and other tools see them too."
                    </p>
                    {body}
                    {move || notice.get().map(|n| view! { <p class="cluster-notice small">{n}</p> })}
                    {move || error.get().map(|e| view! { <pre class="warn add-cluster-error">{e}</pre> })}
                    <div class="modal-actions">
                        <button class="manage-add" prop:disabled=move || busy.get() on:click=move |_| store.add_cluster_open.set(true)>"Add Cluster…"</button>
                        <span class="spacer"></span>
                        <button class="primary" prop:disabled=move || busy.get() on:click=move |_| close()>"Done"</button>
                    </div>
                </Modal>
            }
        })
    }
}
