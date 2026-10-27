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
use crate::modal::{Generation, Modal};
use crate::tauri::{NoArgs, invoke};

/// The names in order after moving `name` to index `to`, or `None` if that
/// leaves its file's run of rows (or changes nothing).
pub(crate) fn moved(list: &[ManagedContext], name: &str, to: usize) -> Option<Vec<String>> {
    let from = list.iter().position(|c| c.name == name)?;
    let file = &list[from].file;
    let first = list.iter().position(|c| c.file == *file)?;
    let last = list.iter().rposition(|c| c.file == *file)?;
    if to == from || to < first || to > last {
        return None;
    }
    let mut names: Vec<String> = list.iter().map(|c| c.name.clone()).collect();
    let n = names.remove(from);
    names.insert(to, n);
    Some(names)
}

/// Where a row dropped on row `target` (in its lower half if `after`) ends up.
pub(crate) fn drop_index(from: usize, target: usize, after: bool) -> usize {
    let at = if after { target + 1 } else { target };
    if at > from { at - 1 } else { at }
}

/// What removing `name` takes with it: its cluster and user, unless another
/// listed context uses them too.
pub(crate) fn goes_with(list: &[ManagedContext], name: &str) -> (Option<String>, Option<String>) {
    let Some(c) = list.iter().find(|c| c.name == name) else { return (None, None) };
    let alone = |field: fn(&ManagedContext) -> &str| {
        let value = field(c);
        (!value.is_empty() && !list.iter().any(|o| o.name != name && field(o) == value)).then(|| value.to_owned())
    };
    (alone(|c| &c.cluster), alone(|c| &c.user))
}

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
                l.contexts.sort_by_key(|c| order.iter().position(|o| *o == c.name));
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
                    Ok(r) => notice.set(Some(removed_text(&r))),
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
        let same_file = move || {
            let file = file.clone();
            dragging.with(|d| d.as_ref().is_some_and(|d| contexts().iter().any(|x| x.name == *d && x.file == file)))
        };
        let same_file_drop = same_file.clone();
        let server = if c.server.is_empty() { "No cluster entry".to_owned() } else { c.server.clone() };
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
                        f64::from(ev.client_y()) > r.top() + r.height() / 2.0
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
                        aria-label=format!("Production: {name}")
                        title=move || if prod() { "Production: changes need extra confirmation. Click to unmark." } else { "Not production. Click to mark it as production." }
                        prop:disabled=move || busy.get()
                        on:click=move |_| set_production(key.get_value(), !prod())>
                        <ProdIcon />
                    </button>
                    <button class="btn small icon" title="Move up" aria-label=format!("Move {name} up")
                        prop:disabled=move || busy.get() || index == first
                        on:click=move |_| move_to(up.clone(), index.wrapping_sub(1))>"↑"</button>
                    <button class="btn small icon" title="Move down" aria-label=format!("Move {name} down")
                        prop:disabled=move || busy.get() || index == last
                        on:click=move |_| move_to(down.clone(), index + 1)>"↓"</button>
                    <button class="btn small" prop:disabled=move || busy.get() || in_use()
                        title=move || if here() {
                            "This window is on it: switch to another context first"
                        } else if elsewhere() {
                            "Another window is on it: switch that window to another context first"
                        } else {
                            ""
                        }
                        on:click=move |_| { error.set(None); confirm.set(Some(ask.clone())) }>"Remove"</button>
                </div>
                {move || asking().then(|| {
                    let go = go.clone();
                    let also: Vec<String> = [cluster_goes.clone().map(|c| format!("cluster {c:?}")), user_goes.clone().map(|u| format!("user {u:?}"))]
                        .into_iter().flatten().collect();
                    let also = match also.len() {
                        0 => "Its cluster and user stay: other contexts use them.".to_owned(),
                        1 => format!("Its {} goes too: nothing else uses it.", also[0]),
                        _ => format!("Its {} go too: nothing else uses them.", also.join(" and ")),
                    };
                    view! {
                        <div class="cluster-confirm" role="alert">
                            <span>{format!("Remove {:?} from the kubeconfig? {also} kubectl and other tools lose it too.", go)}</span>
                            <span class="cluster-confirm-actions">
                                <button class="btn small" prop:disabled=move || busy.get() on:click=move |_| confirm.set(None)>"Keep"</button>
                                // Another window may switch to it meanwhile.
                                <button class="btn small danger" prop:disabled=move || busy.get() || in_use() on:click=move |_| remove(go.clone())>
                                    {move || if busy.get() { "Removing…" } else { "Remove" }}
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
                    let several = l.files.iter().filter(|f| l.contexts.iter().any(|c| c.file == **f)).count() > 1;
                    let mut items = vec![];
                    for (i, c) in l.contexts.iter().enumerate() {
                        let first = l.contexts.iter().position(|x| x.file == c.file).unwrap_or(i);
                        let last = l.contexts.iter().rposition(|x| x.file == c.file).unwrap_or(i);
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

fn removed_text(r: &RemovedContext) -> String {
    let also: Vec<String> = [r.cluster.as_ref().map(|c| format!("cluster {c:?}")), r.user.as_ref().map(|u| format!("user {u:?}"))]
        .into_iter()
        .flatten()
        .collect();
    if also.is_empty() { format!("Removed {:?}.", r.context) } else { format!("Removed {:?}, with its {}.", r.context, also.join(" and ")) }
}

#[cfg(test)]
#[path = "clusters_tests.rs"]
mod tests;
