use std::collections::HashSet;
use std::time::Duration;

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use subduer_types::{Category, ClusterInfo, ContextInfo, ContextList, KindInfo, KindRef, ResourceRow, ResourceTable, events};

use crate::tauri::{NoArgs, invoke, listen};

/// App-wide reactive state. Every field is a `Copy` signal handle.
#[derive(Clone, Copy)]
struct Store {
    contexts: RwSignal<Vec<ContextInfo>>,
    context: RwSignal<Option<String>>,
    cluster: RwSignal<Option<ClusterInfo>>,
    connecting: RwSignal<bool>,
    error: RwSignal<Option<String>>,
    kind: RwSignal<Option<KindRef>>,
    /// `None` = all namespaces. Kept when switching kinds.
    namespace: RwSignal<Option<String>>,
    kind_filter: RwSignal<String>,
    /// Expanded sidebar sections; all start closed.
    expanded: RwSignal<HashSet<Category>>,
    /// User's sidebar section order (drag and drop), persisted locally.
    section_order: RwSignal<Vec<Category>>,
    /// Section being dragged, and the section it's hovering over.
    dragging: RwSignal<Option<Category>>,
    drop_target: RwSignal<Option<Category>>,
    row_filter: RwSignal<String>,
    table: RwSignal<Option<ResourceTable>>,
    /// Snapshots from watches older than this id are ignored.
    min_watch_id: RwSignal<u64>,
    selected: RwSignal<Option<ResourceRow>>,
    yaml: RwSignal<Option<Result<String, String>>>,
    /// Current time (ms), ticking so ages stay fresh.
    now: RwSignal<f64>,
    /// Context awaiting unlock confirmation.
    unlock_prompt: RwSignal<Option<ContextInfo>>,
}

impl Store {
    fn new() -> Self {
        Self {
            contexts: RwSignal::new(vec![]),
            context: RwSignal::new(None),
            cluster: RwSignal::new(None),
            connecting: RwSignal::new(false),
            error: RwSignal::new(None),
            kind: RwSignal::new(None),
            namespace: RwSignal::new(None),
            kind_filter: RwSignal::new(String::new()),
            expanded: RwSignal::new(HashSet::new()),
            section_order: RwSignal::new(crate::prefs::load_section_order()),
            dragging: RwSignal::new(None),
            drop_target: RwSignal::new(None),
            row_filter: RwSignal::new(String::new()),
            table: RwSignal::new(None),
            min_watch_id: RwSignal::new(0),
            selected: RwSignal::new(None),
            yaml: RwSignal::new(None),
            now: RwSignal::new(js_sys::Date::now()),
            unlock_prompt: RwSignal::new(None),
        }
    }

    fn current_context(&self) -> Option<ContextInfo> {
        let name = self.context.get()?;
        self.contexts.get().into_iter().find(|c| c.name == name)
    }

    async fn refresh_contexts(self) {
        match invoke::<_, ContextList>("list_contexts", &NoArgs {}).await {
            Ok(list) => {
                if self.context.get_untracked().is_none() {
                    self.context.set(list.current.clone());
                }
                self.contexts.set(list.contexts);
            }
            Err(e) => self.error.set(Some(e)),
        }
    }

    async fn connect(self, context: String) {
        #[derive(Serialize)]
        struct Args<'a> {
            context: &'a str,
        }
        self.connecting.set(true);
        self.error.set(None);
        self.cluster.set(None);
        self.table.set(None);
        self.selected.set(None);
        self.context.set(Some(context.clone()));
        match invoke::<_, ClusterInfo>("connect", &Args { context: &context }).await {
            Ok(info) => {
                // Start in the context's default namespace when it has one.
                let default_ns = self
                    .contexts
                    .get_untracked()
                    .into_iter()
                    .find(|c| c.name == context)
                    .and_then(|c| c.namespace)
                    .filter(|ns| info.namespaces.contains(ns));
                self.namespace.set(default_ns);
                // Keep the selected kind if this cluster has it, else show Pods.
                let keep = self.kind.get_untracked().and_then(|k| find_kind(&info.kinds, &k.key()));
                self.kind.set(keep.or_else(|| find_kind(&info.kinds, "core/Pod")));
                self.cluster.set(Some(info));
            }
            Err(e) => self.error.set(Some(format!("connect to {context}: {e}"))),
        }
        self.connecting.set(false);
    }
}

fn find_kind(kinds: &[KindInfo], key: &str) -> Option<KindRef> {
    kinds.iter().map(|k| &k.kind).find(|k| k.key() == key).cloned()
}

#[component]
pub fn App() -> impl IntoView {
    let store = Store::new();

    spawn_local(async move {
        listen::<ResourceTable>(events::RESOURCE_TABLE, move |table| {
            if table.watch_id >= store.min_watch_id.get_untracked() {
                store.min_watch_id.set(table.watch_id);
                store.table.set(Some(table));
            }
        })
        .await;
        store.refresh_contexts().await;
        if let Some(ctx) = store.context.get_untracked() {
            store.connect(ctx).await;
        }
    });

    set_interval(move || store.now.set(js_sys::Date::now()), Duration::from_secs(5));

    // (Re)start the watch whenever the cluster, kind or namespace changes.
    Effect::new(move |_| {
        #[derive(Serialize)]
        struct Args {
            kind: KindRef,
            namespace: Option<String>,
        }
        let connected = store.cluster.with(Option::is_some);
        let kind = store.kind.get();
        let namespace = store.namespace.get();
        let Some(kind) = kind.filter(|_| connected) else { return };
        store.table.set(None);
        store.selected.set(None);
        // Backend ids are sequential; anything older than the next one is stale.
        store.min_watch_id.update(|id| *id += 1);
        spawn_local(async move {
            match invoke::<_, u64>("watch_resources", &Args { kind, namespace }).await {
                Ok(id) => store.min_watch_id.update(|min| *min = (*min).max(id)),
                Err(e) => store.error.set(Some(e)),
            }
        });
    });

    view! {
        <div class="app">
            <TopBar store=store />
            <div class="body">
                <Sidebar store=store />
                <main class="main">
                    <ResourceView store=store />
                </main>
                <DetailPanel store=store />
            </div>
            <UnlockDialog store=store />
            {move || store.error.get().map(|e| view! {
                <div class="error-bar" on:click=move |_| store.error.set(None)>{e}<span class="dismiss">"×"</span></div>
            })}
        </div>
    }
}

#[component]
fn TopBar(store: Store) -> impl IntoView {
    let on_context = move |ev: leptos::ev::Event| {
        let ctx = event_target_value(&ev);
        spawn_local(store.connect(ctx));
    };
    let on_namespace = move |ev: leptos::ev::Event| {
        let ns = event_target_value(&ev);
        store.namespace.set((!ns.is_empty()).then_some(ns));
    };

    view! {
        <header class="topbar">
            <span class="brand">"k8s-subduer"</span>
            <label class="field">
                "Context"
                <select on:change=on_context prop:disabled=move || store.connecting.get()>
                    <For
                        each=move || store.contexts.get()
                        key=|c| c.name.clone()
                        children=move |c| {
                            let name = c.name.clone();
                            let selected = move || store.context.get().as_deref() == Some(name.as_str());
                            let label = if c.prod_like { format!("{} ⚠", c.name) } else { c.name.clone() };
                            view! { <option value=c.name.clone() selected=selected>{label}</option> }
                        }
                    />
                </select>
            </label>
            <label class="field">
                "Namespace"
                <select on:change=on_namespace prop:disabled=move || store.cluster.with(Option::is_none)>
                    <option value="" selected=move || store.namespace.get().is_none()>"All namespaces"</option>
                    {move || {
                        store.cluster.get().map(|c| c.namespaces).unwrap_or_default().into_iter().map(|ns| {
                            let value = ns.clone();
                            let label = ns.clone();
                            let selected = move || store.namespace.get().as_deref() == Some(value.as_str());
                            view! { <option value=ns selected=selected>{label}</option> }
                        }).collect_view()
                    }}
                </select>
            </label>
            <span class="spacer"></span>
            {move || store.connecting.get().then(|| view! { <span class="muted">"Connecting…"</span> })}
            <LockToggle store=store />
        </header>
    }
}

async fn set_unlocked(store: Store, context: String, unlocked: bool) {
    #[derive(Serialize)]
    struct Args {
        context: String,
        unlocked: bool,
    }
    if let Err(e) = invoke::<_, ()>("set_unlocked", &Args { context, unlocked }).await {
        store.error.set(Some(e));
    }
    store.refresh_contexts().await;
}

#[component]
fn LockToggle(store: Store) -> impl IntoView {
    let toggle = move |_| {
        let Some(ctx) = store.current_context() else { return };
        if ctx.unlocked {
            spawn_local(set_unlocked(store, ctx.name, false));
        } else {
            store.unlock_prompt.set(Some(ctx));
        }
    };

    view! {
        {move || store.current_context().map(|ctx| {
            let (class, label, title) = if ctx.unlocked {
                ("lock unlocked", "🔓 Writable", "Changes are allowed. Click to make read-only.")
            } else {
                ("lock", "🔒 Read-only", "Click to allow changes to this context.")
            };
            let class = if ctx.prod_like { format!("{class} prod") } else { class.to_owned() };
            view! { <button class=class title=title on:click=toggle>{label}</button> }
        })}
    }
}

/// Confirms unlocking; production-like contexts must have their name typed out.
#[component]
fn UnlockDialog(store: Store) -> impl IntoView {
    let typed = RwSignal::new(String::new());
    view! {
        {move || store.unlock_prompt.get().map(|ctx| {
            let name = ctx.name.clone();
            let prod = ctx.prod_like;
            let ok = {
                let name = name.clone();
                move || !prod || typed.get().trim() == name
            };
            let cancel = move |_| {
                typed.set(String::new());
                store.unlock_prompt.set(None);
            };
            let confirm = {
                let name = name.clone();
                move |_| {
                    typed.set(String::new());
                    store.unlock_prompt.set(None);
                    spawn_local(set_unlocked(store, name.clone(), true));
                }
            };
            view! {
                <div class="modal-backdrop">
                    <div class="modal" class:prod=prod>
                        <h2>"Allow changes?"</h2>
                        {if prod {
                            view! {
                                <p><strong>{name.clone()}</strong>" looks like "<strong>"PRODUCTION"</strong>"."</p>
                                <p>"Type the context name to allow changes:"</p>
                                <input
                                    class="confirm-input"
                                    prop:value=move || typed.get()
                                    on:input=move |ev| typed.set(event_target_value(&ev))
                                />
                            }.into_any()
                        } else {
                            view! { <p>"Context "<strong>{name.clone()}</strong>" will become writable for this session."</p> }.into_any()
                        }}
                        <div class="modal-actions">
                            <button on:click=cancel>"Cancel"</button>
                            <button class="danger" prop:disabled=move || !ok() on:click=confirm>"Allow changes"</button>
                        </div>
                    </div>
                </div>
            }
        })}
    }
}

#[component]
fn Sidebar(store: Store) -> impl IntoView {
    // Kinds arrive from the backend already in section order.
    let sections = move || {
        let filter = store.kind_filter.get().to_lowercase();
        let matches = |k: &KindRef| {
            filter.is_empty()
                || k.kind.to_lowercase().contains(&filter)
                || k.plural.contains(&filter)
                || k.group.contains(&filter)
        };
        let kinds = store.cluster.get().map(|c| c.kinds).unwrap_or_default();
        // Kinds within a section keep the backend's order.
        store
            .section_order
            .get()
            .into_iter()
            .map(|category| {
                let in_section = kinds
                    .iter()
                    .filter(|k| k.category == category && matches(&k.kind))
                    .map(|k| k.kind.clone())
                    .collect::<Vec<_>>();
                (category, in_section)
            })
            .filter(|(_, kinds)| !kinds.is_empty())
            .collect::<Vec<_>>()
    };

    view! {
        <nav class="sidebar">
            <input
                class="search"
                type="search"
                placeholder="Filter kinds…"
                prop:value=move || store.kind_filter.get()
                on:input=move |ev| store.kind_filter.set(event_target_value(&ev))
            />
            <div class="kinds">
                {move || sections().into_iter().map(|(category, kinds)| {
                    // Searching shows every match, even in collapsed sections.
                    let open = move || {
                        !store.kind_filter.with(String::is_empty)
                            || store.expanded.with(|c| c.contains(&category))
                    };
                    let toggle = move |_| store.expanded.update(|c| {
                        if !c.remove(&category) {
                            c.insert(category);
                        }
                    });
                    let count = kinds.len();
                    let is_dragging = move || store.dragging.get() == Some(category);
                    let is_target = move || {
                        store.drop_target.get() == Some(category) && store.dragging.get() != Some(category)
                    };
                    view! {
                        <div
                            class="kind-section"
                            class:dragging=is_dragging
                            class:drop-target=is_target
                            on:dragover=move |ev: web_sys::DragEvent| {
                                if store.dragging.get_untracked().is_some() {
                                    ev.prevent_default();
                                    store.drop_target.set(Some(category));
                                }
                            }
                            on:drop=move |ev: web_sys::DragEvent| {
                                ev.prevent_default();
                                if let Some(moved) = store.dragging.get_untracked() {
                                    move_section(store, moved, category);
                                }
                                end_drag(store);
                            }
                        >
                            <button
                                class="kind-section-title"
                                draggable="true"
                                title="Drag to reorder"
                                on:click=toggle
                                on:dragstart=move |ev: web_sys::DragEvent| {
                                    // WebKit only starts a drag when data is set.
                                    if let Some(dt) = ev.data_transfer() {
                                        dt.set_effect_allowed("move");
                                        let _ = dt.set_data("text/plain", category.label());
                                    }
                                    store.dragging.set(Some(category));
                                }
                                on:dragend=move |_| end_drag(store)
                            >
                                <span class="chevron" class:open=open>"▸"</span>
                                {category.label()}
                                <span class="section-count">{count}</span>
                            </button>
                            <Show when=open>
                                {kind_list(store, category, kinds.clone())}
                            </Show>
                        </div>
                    }
                }).collect_view()}
            </div>
        </nav>
    }
}

/// Moves `moved` to where `target` is, shifting the sections in between.
fn move_section(store: Store, moved: Category, target: Category) {
    if moved == target {
        return;
    }
    store.section_order.update(|order| {
        let (Some(from), Some(to)) = (
            order.iter().position(|c| *c == moved),
            order.iter().position(|c| *c == target),
        ) else {
            return;
        };
        let item = order.remove(from);
        order.insert(to, item);
    });
    crate::prefs::save_section_order(&store.section_order.get_untracked());
}

fn end_drag(store: Store) {
    store.dragging.set(None);
    store.drop_target.set(None);
}

/// Kind buttons for one section. Custom resources get a sub-heading per API group.
fn kind_list(store: Store, category: Category, kinds: Vec<KindRef>) -> impl IntoView {
    let mut last_group: Option<String> = None;
    kinds
        .into_iter()
        .map(|k| {
            let heading = (category == Category::CustomResources && last_group.as_deref() != Some(&k.group))
                .then(|| {
                    last_group = Some(k.group.clone());
                    view! { <div class="kind-group-title">{k.group.clone()}</div> }
                });
            let key = k.key();
            let active = move || store.kind.get().is_some_and(|cur| cur.key() == key);
            let label = k.kind.clone();
            let title = format!("{} ({})", k.plural, k.api_version());
            view! {
                {heading}
                <button
                    class="kind"
                    class:active=active
                    title=title
                    on:click=move |_| store.kind.set(Some(k.clone()))
                >{label}</button>
            }
        })
        .collect_view()
}

#[component]
fn ResourceView(store: Store) -> impl IntoView {
    let show_namespace =
        move || store.kind.get().is_some_and(|k| k.namespaced) && store.namespace.get().is_none();

    let rows = move || {
        let filter = store.row_filter.get().to_lowercase();
        store
            .table
            .get()
            .map(|t| t.rows)
            .unwrap_or_default()
            .into_iter()
            .filter(|r| filter.is_empty() || r.id.to_lowercase().contains(&filter))
            .collect::<Vec<_>>()
    };

    let header = move || {
        let kind = store.kind.get();
        let title = kind.as_ref().map(|k| k.kind.clone()).unwrap_or_default();
        let scope = match (&kind, store.namespace.get()) {
            (Some(k), _) if !k.namespaced => "cluster-scoped".to_owned(),
            (_, Some(ns)) => format!("in {ns}"),
            _ => "in all namespaces".to_owned(),
        };
        let count = store.table.get().map(|t| t.rows.len());
        view! {
            <h1>{title}</h1>
            <span class="muted">{scope}</span>
            {count.map(|n| view! { <span class="count">{n}</span> })}
        }
    };

    view! {
        <div class="resource-header">
            {header}
            <span class="spacer"></span>
            <input
                class="search"
                type="search"
                placeholder="Filter by name…"
                prop:value=move || store.row_filter.get()
                on:input=move |ev| store.row_filter.set(event_target_value(&ev))
            />
        </div>
        {move || store.table.get().and_then(|t| t.error).map(|e| view! {
            <div class="warn">"Watch error (retrying): " {e}</div>
        })}
        <div class="table-wrap">
            {move || match store.table.get() {
                None if store.cluster.with(Option::is_some) => view! { <p class="muted pad">"Loading…"</p> }.into_any(),
                None => view! { <p class="muted pad">"Not connected."</p> }.into_any(),
                Some(table) => view! {
                    <table class="resources">
                        <thead><tr>
                            <th>"Name"</th>
                            {show_namespace().then(|| view! { <th>"Namespace"</th> })}
                            {table.columns.iter().map(|c| view! { <th>{c.clone()}</th> }).collect_view()}
                            <th>"Age"</th>
                        </tr></thead>
                        <tbody>
                            <For
                                each=rows
                                key=|r| r.clone()
                                children=move |row| {
                                    let id = row.id.clone();
                                    let selected = move || store.selected.with(|s| s.as_ref().is_some_and(|s| s.id == id));
                                    let created = row.created.clone();
                                    let age = move || created.as_deref().map(|c| format_age(store.now.get(), c)).unwrap_or_default();
                                    let ns = row.namespace.clone().unwrap_or_default();
                                    let clicked = row.clone();
                                    view! {
                                        <tr class:selected=selected on:click=move |_| select_row(store, clicked.clone())>
                                            <td class="name">{row.name.clone()}</td>
                                            {show_namespace().then(|| view! { <td>{ns}</td> })}
                                            {row.cells.iter().map(|c| view! { <td>{c.clone()}</td> }).collect_view()}
                                            <td class="age">{age}</td>
                                        </tr>
                                    }
                                }
                            />
                        </tbody>
                    </table>
                }.into_any(),
            }}
        </div>
    }
}

fn select_row(store: Store, row: ResourceRow) {
    #[derive(Serialize)]
    struct Args {
        kind: KindRef,
        namespace: Option<String>,
        name: String,
    }
    let Some(kind) = store.kind.get_untracked() else { return };
    store.yaml.set(None);
    store.selected.set(Some(row.clone()));
    spawn_local(async move {
        let args = Args {
            kind,
            namespace: row.namespace.clone(),
            name: row.name.clone(),
        };
        let result = invoke::<_, String>("get_yaml", &args).await;
        // Ignore the response if the user has moved on.
        if store.selected.with_untracked(|s| s.as_ref().is_some_and(|s| s.id == row.id)) {
            store.yaml.set(Some(result));
        }
    });
}

#[component]
fn DetailPanel(store: Store) -> impl IntoView {
    view! {
        {move || store.selected.get().map(|row| view! {
            <aside class="detail">
                <div class="detail-header">
                    <div>
                        <div class="detail-title">{row.name.clone()}</div>
                        <div class="muted">{row.namespace.clone().unwrap_or_default()}</div>
                    </div>
                    <button class="close" on:click=move |_| store.selected.set(None)>"×"</button>
                </div>
                {move || match store.yaml.get() {
                    None => view! { <p class="muted pad">"Loading…"</p> }.into_any(),
                    Some(Ok(yaml)) => view! { <pre class="yaml">{yaml}</pre> }.into_any(),
                    Some(Err(e)) => view! { <p class="warn">{e}</p> }.into_any(),
                }}
            </aside>
        })}
    }
}

/// kubectl-style age: 45s, 12m, 5h, 3d, 2y.
fn format_age(now_ms: f64, created: &str) -> String {
    let then = js_sys::Date::parse(created);
    if then.is_nan() {
        return String::new();
    }
    let secs = ((now_ms - then) / 1000.0).max(0.0) as u64;
    match secs {
        s if s < 120 => format!("{s}s"),
        s if s < 2 * 3600 => format!("{}m", s / 60),
        s if s < 2 * 86400 => format!("{}h", s / 3600),
        s if s < 365 * 86400 => format!("{}d", s / 86400),
        s => format!("{}y", s / (365 * 86400)),
    }
}
