use std::collections::{HashMap, HashSet};
use std::time::Duration;

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use subduer_types::{Category, ClusterInfo, ClusterOverview, ForwardInfo, PodDetails, PodHistory, Column, ColumnKind, ContextInfo, ContextList, KindInfo, KindRef, ResourceRow, ResourceTable, events};

use subduer_types::containers::{self, State};

use crate::columns::{ColumnWidths, ResizableTh};
use crate::tauri::{NoArgs, invoke, listen};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DetailTab {
    Details,
    Yaml,
}

/// Seconds between background refreshes of the selected pod's details.
const POD_REFRESH: Duration = Duration::from_secs(15);

/// App-wide reactive state. Every field is a `Copy` signal handle.
#[derive(Clone, Copy)]
pub(crate) struct Store {
    pub(crate) contexts: RwSignal<Vec<ContextInfo>>,
    pub(crate) context: RwSignal<Option<String>>,
    pub(crate) cluster: RwSignal<Option<ClusterInfo>>,
    pub(crate) connecting: RwSignal<bool>,
    pub(crate) error: RwSignal<Option<String>>,
    pub(crate) kind: RwSignal<Option<KindRef>>,
    /// `None` = all namespaces. Kept when switching kinds.
    pub(crate) namespace: RwSignal<Option<String>>,
    pub(crate) kind_filter: RwSignal<String>,
    /// Expanded sidebar sections; all start closed.
    pub(crate) expanded: RwSignal<HashSet<Category>>,
    /// User's sidebar section order (drag and drop), persisted locally.
    pub(crate) section_order: RwSignal<Vec<Category>>,
    /// Section being dragged, and the section it's hovering over.
    pub(crate) dragging: RwSignal<Option<Category>>,
    pub(crate) drop_target: RwSignal<Option<Category>>,
    pub(crate) row_filter: RwSignal<String>,
    pub(crate) table: RwSignal<Option<ResourceTable>>,
    /// Snapshots from watches older than this id are ignored.
    pub(crate) min_watch_id: RwSignal<u64>,
    pub(crate) selected: RwSignal<Option<ResourceRow>>,
    pub(crate) yaml: RwSignal<Option<Result<String, String>>>,
    /// Current time (ms), ticking so ages stay fresh.
    pub(crate) now: RwSignal<f64>,
    /// Context awaiting unlock confirmation.
    pub(crate) unlock_prompt: RwSignal<Option<ContextInfo>>,
    /// Object awaiting delete confirmation.
    pub(crate) delete_prompt: RwSignal<Option<crate::delete::DeleteTarget>>,
    /// Delete to confirm once the unlock prompt succeeds.
    pub(crate) pending_delete: RwSignal<Option<crate::delete::DeleteTarget>>,
    /// Shell to open once the unlock prompt succeeds: (context, target).
    pub(crate) pending_exec: RwSignal<Option<(String, subduer_types::ExecTarget)>>,
    /// Showing the Cluster page instead of a kind's table.
    pub(crate) overview_open: RwSignal<bool>,
    /// Showing the Port Forwards page.
    pub(crate) forwards_open: RwSignal<bool>,
    /// Running port-forwards, kept current by backend events.
    pub(crate) forwards: RwSignal<Vec<ForwardInfo>>,
    pub(crate) forward_prompt: RwSignal<Option<crate::forward::ForwardPrompt>>,
    pub(crate) overview: RwSignal<Option<ClusterOverview>>,
    pub(crate) overview_error: RwSignal<Option<String>>,
    pub(crate) overview_loading: RwSignal<bool>,
    /// When `overview` was fetched (ms).
    pub(crate) overview_at: RwSignal<f64>,
    pub(crate) col_widths: ColumnWidths,
    pub(crate) detail_tab: RwSignal<DetailTab>,
    /// Details of the selected pod.
    pub(crate) pod: RwSignal<Option<Result<PodDetails, String>>>,
    pub(crate) pod_loading: RwSignal<bool>,
    /// Last-hour CPU and memory of the selected pod, refreshed with it.
    pub(crate) pod_history: RwSignal<Option<Result<PodHistory, String>>>,
    pub(crate) history_loading: RwSignal<bool>,
    /// Collapsed detail sections, kept across selections.
    pub(crate) detail_collapsed: RwSignal<HashSet<String>>,
    /// Secret values the user chose to reveal for the selected pod, by
    /// (secret, key). Cleared whenever the selection changes.
    pub(crate) revealed: RwSignal<HashMap<(String, String), Result<String, String>>>,
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
            delete_prompt: RwSignal::new(None),
            pending_delete: RwSignal::new(None),
            pending_exec: RwSignal::new(None),
            overview_open: RwSignal::new(true),
            forwards_open: RwSignal::new(false),
            forwards: RwSignal::new(vec![]),
            forward_prompt: RwSignal::new(None),
            overview: RwSignal::new(None),
            overview_error: RwSignal::new(None),
            overview_loading: RwSignal::new(false),
            overview_at: RwSignal::new(0.0),
            col_widths: ColumnWidths::load(),
            detail_tab: RwSignal::new(DetailTab::Yaml),
            pod: RwSignal::new(None),
            pod_loading: RwSignal::new(false),
            pod_history: RwSignal::new(None),
            history_loading: RwSignal::new(false),
            detail_collapsed: RwSignal::new(HashSet::new()),
            revealed: RwSignal::new(HashMap::new()),
        }
    }

    pub(crate) fn current_context(&self) -> Option<ContextInfo> {
        let name = self.context.get()?;
        self.contexts.get().into_iter().find(|c| c.name == name)
    }

    pub(crate) fn current_context_untracked(&self) -> Option<ContextInfo> {
        let name = self.context.get_untracked()?;
        self.contexts.get_untracked().into_iter().find(|c| c.name == name)
    }

    /// The Node kind is shown with the Cluster page's node table.
    pub(crate) fn node_page(&self) -> bool {
        !self.overview_open.get() && self.kind.with(|k| k.as_ref().is_some_and(|k| k.key() == "core/Node"))
    }

    /// Whether the visible page is built from `cluster_overview` data.
    pub(crate) fn needs_overview(&self) -> bool {
        self.overview_open.get() || self.node_page()
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
        self.overview.set(None);
        self.overview_error.set(None);
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
        listen::<Vec<ForwardInfo>>(subduer_types::forward_events::CHANGED, move |list| store.forwards.set(list)).await;
        crate::forward::load(store);
        store.refresh_contexts().await;
        if let Some(ctx) = store.context.get_untracked() {
            store.connect(ctx).await;
        }
    });

    set_interval(move || store.now.set(js_sys::Date::now()), Duration::from_secs(5));
    crate::overview::start_refresh(store);
    set_interval(
        move || {
            let pod_selected = store.selected.with_untracked(Option::is_some) && selected_is_pod(store);
            if pod_selected && store.detail_tab.get_untracked() == DetailTab::Details {
                crate::pod_detail::load(store, true);
            }
        },
        POD_REFRESH,
    );

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
        if store.needs_overview() {
            store.table.set(None);
            store.selected.set(None);
            if connected {
                spawn_local(async move {
                    let _ = invoke::<_, ()>("stop_watch", &NoArgs {}).await;
                });
            }
            return;
        }
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
                    {move || if store.forwards_open.get() {
                        view! { <crate::forward::ForwardsPage store=store /> }.into_any()
                    } else if store.overview_open.get() {
                        view! { <crate::overview::Overview store=store /> }.into_any()
                    } else {
                        view! { <ResourceView store=store /> }.into_any()
                    }}
                </main>
                <DetailPanel store=store />
            </div>
            <UnlockDialog store=store />
            <crate::delete::DeleteDialog store=store />
            <crate::forward::ForwardDialog store=store />
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
                <select
                    on:change=on_namespace
                    prop:disabled=move || store.cluster.with(Option::is_none) || store.overview_open.get()
                    title=move || if store.overview_open.get() { "The Cluster page is cluster-wide" } else { "" }
                >
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
                store.pending_delete.set(None);
                store.pending_exec.set(None);
            };
            let confirm = {
                let name = name.clone();
                move |_| {
                    typed.set(String::new());
                    store.unlock_prompt.set(None);
                    let name = name.clone();
                    spawn_local(async move {
                        set_unlocked(store, name.clone(), true).await;
                        // A delete that asked for the unlock continues to its own confirmation.
                        if let Some(mut target) = store.pending_delete.get_untracked()
                            && target.context.name == name
                        {
                            target.context.unlocked = true;
                            store.delete_prompt.set(Some(target));
                        }
                        store.pending_delete.set(None);
                        if let Some((context, target)) = store.pending_exec.get_untracked()
                            && context == name
                        {
                            crate::exec::open(store, target);
                        }
                        store.pending_exec.set(None);
                    });
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
    let in_category = move |category: Category| {
        let filter = store.kind_filter.get().to_lowercase();
        // Kinds keep the backend's order within a category.
        store.cluster.with(|c| {
            c.iter()
                .flat_map(|c| &c.kinds)
                .filter(|k| k.category == category && kind_matches(&k.kind, &filter))
                .map(|k| k.kind.clone())
                .collect::<Vec<_>>()
        })
    };
    let sections = move || {
        store
            .section_order
            .get()
            .into_iter()
            .map(|category| (category, in_category(category)))
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
                <div class="top-level-kinds">
                    <Show when=move || kind_matches_text("Cluster", &store.kind_filter.get().to_lowercase())>
                        <button
                            class="kind"
                            class:active=move || store.overview_open.get() && !store.forwards_open.get()
                            on:click=move |_| {
                                store.forwards_open.set(false);
                                store.overview_open.set(true);
                            }
                        ><crate::icons::ClusterIcon /><span class="kind-label">"Cluster"</span></button>
                    </Show>
                    <Show when=move || kind_matches_text("Port Forwards", &store.kind_filter.get().to_lowercase())>
                        <button
                            class="kind"
                            class:active=move || store.forwards_open.get()
                            title="Running port-forwards"
                            on:click=move |_| {
                                store.selected.set(None);
                                store.forwards_open.set(true);
                            }
                        >
                            <crate::icons::ForwardIcon />
                            <span class="kind-label">"Port Forwards"</span>
                            {move || {
                                let n = store.forwards.with(Vec::len);
                                (n > 0).then(|| view! { <span class="count">{n}</span> })
                            }}
                        </button>
                    </Show>
                    {move || kind_list(store, Category::TopLevel, in_category(Category::TopLevel))}
                </div>
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
                                <crate::icons::Chevron open=Signal::derive(open) />
                                <crate::icons::CategoryIcon category=category />
                                <span class="kind-label">{category.label()}</span>
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

fn kind_matches_text(text: &str, filter: &str) -> bool {
    filter.is_empty() || text.to_lowercase().contains(filter)
}

fn kind_matches(k: &KindRef, filter: &str) -> bool {
    filter.is_empty() || k.kind.to_lowercase().contains(filter) || k.plural.contains(filter) || k.group.contains(filter)
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
            let heading = (category.groups_by_api_group() && last_group.as_deref() != Some(&k.group))
                .then(|| {
                    last_group = Some(k.group.clone());
                    view! {
                        <div class="kind-group-title">
                            <crate::icons::GroupIcon group=k.group.clone() />
                            <span class="kind-label">{k.group.clone()}</span>
                        </div>
                    }
                });
            let key = k.key();
            let active = move || {
                !store.overview_open.get() && !store.forwards_open.get() && store.kind.get().is_some_and(|cur| cur.key() == key)
            };
            let label = k.kind.clone();
            let title = format!("{} ({})", k.plural, k.api_version());
            let icon = view! {
                <crate::icons::KindIcon kind=k.clone() custom=category.groups_by_api_group() />
            };
            view! {
                {heading}
                <button
                    class="kind"
                    class:active=active
                    title=title
                    on:click=move |_| {
                        store.overview_open.set(false);
                        store.forwards_open.set(false);
                        store.kind.set(Some(k.clone()));
                    }
                >{icon}<span class="kind-label">{label}</span></button>
            }
        })
        .collect_view()
}

#[component]
fn ResourceView(store: Store) -> impl IntoView {

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
        let count = if store.node_page() {
            store.overview.with(|o| o.as_ref().map(|o| o.nodes.len()))
        } else {
            store.table.get().map(|t| t.rows.len())
        };
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
            {move || store.node_page().then(|| view! { <crate::overview::RefreshStatus store=store /> })}
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
        {move || store.node_page().then(|| store.overview_error.get()).flatten().map(|e| view! {
            <div class="warn">{e}</div>
        })}
        <div class="table-wrap">
            {move || if store.node_page() {
                view! { <crate::overview::NodePage store=store /> }.into_any()
            } else { match store.table.get() {
                None if store.cluster.with(Option::is_some) => view! { <p class="muted pad">"Loading…"</p> }.into_any(),
                None => view! { <p class="muted pad">"Not connected."</p> }.into_any(),
                Some(table) => {
                    let table_id = store.kind.get_untracked().map(|k| k.key()).unwrap_or_default();
                    let single_ns = store.namespace.get().is_some();
                    let headers: Vec<String> = visible(&table.columns, single_ns).map(|(_, c)| c.header.clone()).collect();
                    let style = {
                        let table_id = table_id.clone();
                        move || store.col_widths.table_style(&table_id, &headers)
                    };
                    view! {
                    <table class="resources" class:fixed={let t = table_id.clone(); move || store.col_widths.has_table(&t)} style=style>
                        <thead><tr>
                            {visible(&table.columns, single_ns)
                                .map(|(_, c)| {
                                    let title = if c.kind == ColumnKind::Containers { CONTAINER_LEGEND } else { "" };
                                    view! {
                                        <ResizableTh
                                            widths=store.col_widths
                                            table=table_id.clone()
                                            header=c.header.clone()
                                            class=column_class(c.kind)
                                            title=title
                                        />
                                    }
                                })
                                .collect_view()}
                        </tr></thead>
                        <tbody>
                            <For
                                each=rows
                                key=|r| r.clone()
                                children=move |row| {
                                    let columns = store.table.with_untracked(|t| t.as_ref().map(|t| t.columns.clone()).unwrap_or_default());
                                    let single_ns = store.namespace.get_untracked().is_some();
                                    let id = row.id.clone();
                                    let selected = move || store.selected.with(|s| s.as_ref().is_some_and(|s| s.id == id));
                                    let clicked = row.clone();
                                    let cells = visible(&columns, single_ns)
                                        .filter_map(|(i, c)| row.cells.get(i).map(|v| cell_view(store, c.kind, v.clone())))
                                        .collect_view();
                                    view! {
                                        <tr class:selected=selected on:click=move |_| select_row(store, clicked.clone())>
                                            {cells}
                                        </tr>
                                    }
                                }
                            />
                        </tbody>
                    </table>
                    }.into_any()
                }
            }}}
        </div>
    }
}

/// Columns to display with their cell index. The namespace column is
/// redundant while a single namespace is selected.
fn visible(columns: &[Column], single_namespace: bool) -> impl Iterator<Item = (usize, &Column)> {
    columns
        .iter()
        .enumerate()
        .filter(move |(_, c)| !(single_namespace && c.kind == ColumnKind::Namespace))
}

fn column_class(kind: ColumnKind) -> &'static str {
    match kind {
        ColumnKind::Name => "name",
        ColumnKind::LongText => "long",
        ColumnKind::Number => "num",
        ColumnKind::Age => "age",
        ColumnKind::Namespace | ColumnKind::Text | ColumnKind::Badge | ColumnKind::Containers => "",
    }
}

fn cell_view(store: Store, kind: ColumnKind, value: String) -> AnyView {
    let class = column_class(kind);
    match kind {
        ColumnKind::Age => {
            let age = move || format_age(store.now.get(), &value);
            view! { <td class=class>{age}</td> }.into_any()
        }
        ColumnKind::Containers => view! { <td><ContainerIcons cell=value /></td> }.into_any(),
        ColumnKind::LongText => {
            let title = value.clone();
            view! { <td class=class title=title>{value}</td> }.into_any()
        }
        ColumnKind::Badge => {
            // e.g. "Completed (OOM)" -> badge-completed-oom
            let slug = value
                .to_lowercase()
                .split(|c: char| !c.is_ascii_alphanumeric())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("-");
            let badge = format!("badge badge-{slug}");
            view! { <td><span class=badge>{value}</span></td> }.into_any()
        }
        _ => view! { <td class=class>{value}</td> }.into_any(),
    }
}

/// Explains the container icons (tooltip on the Ready header).
const CONTAINER_LEGEND: &str = "Containers: ✓ running · ◐ starting · ■ stopped · ✕ failed";

/// One icon per container. Each state has its own symbol, so colour is never
/// the only cue: ✓ running, ◐ starting, ■ stopped (exited cleanly), ✕ failed.
#[component]
fn ContainerIcons(cell: String) -> impl IntoView {
    view! {
        <span class="ctr-icons">
            {containers::decode(&cell).into_iter().map(container_icon).collect_view()}
        </span>
    }
}

#[component]
pub(crate) fn ContainerIcon(c: containers::Container) -> impl IntoView {
    container_icon(c)
}

fn container_icon(c: containers::Container) -> impl IntoView {
    let (class, label) = match c.state {
        State::Running => ("ctr running", "running"),
        State::Starting => ("ctr starting", "starting"),
        State::Stopped => ("ctr stopped", "stopped"),
        State::Failed => ("ctr failed", "failed"),
    };
    let kind = if c.init { "init container " } else { "" };
    let title = format!("{kind}{}: {}", c.name, c.detail);
    let aria = format!("{kind}{} {label}", c.name);
    let shape = match c.state {
        State::Running => view! {
            <circle cx="7" cy="7" r="6.5" fill="currentColor" />
            <path d="M4 7.2 L6.1 9.2 L10 5" fill="none" stroke="var(--icon-ink)" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
        }
        .into_any(),
        State::Starting => view! {
            <circle cx="7" cy="7" r="5.6" fill="none" stroke="currentColor" stroke-width="1.6" />
            <path d="M7 1.4 A5.6 5.6 0 0 0 7 12.6 Z" fill="currentColor" />
        }
        .into_any(),
        State::Stopped => view! {
            <circle cx="7" cy="7" r="5.6" fill="none" stroke="currentColor" stroke-width="1.6" />
            <rect x="4.9" y="4.9" width="4.2" height="4.2" rx="0.6" fill="currentColor" />
        }
        .into_any(),
        State::Failed => view! {
            <circle cx="7" cy="7" r="6.5" fill="currentColor" />
            <path d="M4.8 4.8 L9.2 9.2 M9.2 4.8 L4.8 9.2" stroke="var(--icon-ink)" stroke-width="1.8" stroke-linecap="round" />
        }
        .into_any(),
    };
    view! {
        <span class=class title=title role="img" aria-label=aria>
            <svg width="14" height="14" viewBox="0 0 14 14">{shape}</svg>
        </span>
    }
}

fn selected_is_pod(store: Store) -> bool {
    store.kind.with_untracked(|k| k.as_ref().is_some_and(|k| k.key() == "core/Pod"))
}

pub(crate) fn select_row(store: Store, row: ResourceRow) {
    store.yaml.set(None);
    store.pod.set(None);
    store.pod_history.set(None);
    store.revealed.set(HashMap::new());
    store.selected.set(Some(row));
    if selected_is_pod(store) {
        store.detail_tab.set(DetailTab::Details);
        crate::pod_detail::load(store, false);
    } else {
        store.detail_tab.set(DetailTab::Yaml);
        load_yaml(store);
    }
}

fn load_yaml(store: Store) {
    #[derive(Serialize)]
    struct Args {
        kind: KindRef,
        namespace: Option<String>,
        name: String,
    }
    let Some(kind) = store.kind.get_untracked() else { return };
    let Some(row) = store.selected.get_untracked() else { return };
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

/// Starts a port-forward to a Service; hidden for other kinds.
#[component]
fn ServiceForwardButton(store: Store, row: ResourceRow) -> impl IntoView {
    move || {
        store.kind.get().filter(|k| k.key() == "core/Service")?;
        let namespace = row.namespace.clone()?;
        let source = subduer_types::ForwardSource::Service { service: row.name.clone() };
        Some(view! {
            <button
                class="btn small"
                title="Forward a local port to this service"
                on:click=move |_| crate::forward::request(store, namespace.clone(), source.clone())
            >"Forward"</button>
        })
    }
}

/// Opens merged logs for a workload's pods; hidden for other kinds.
#[component]
fn WorkloadLogsButton(store: Store, row: ResourceRow) -> impl IntoView {
    move || {
        let kind = store.kind.get().filter(subduer_types::has_workload_logs)?;
        let namespace = row.namespace.clone()?;
        let title = format!("Open logs from every pod of {} {} in a new window", kind.kind, row.name);
        let target = subduer_types::LogTarget {
            namespace,
            source: subduer_types::LogSource::Workload { kind, name: row.name.clone() },
        };
        Some(view! {
            <button
                class="btn small"
                title=title
                on:click=move |_| crate::logs::open(target.clone(), move |e| store.error.set(Some(e)))
            >"Logs"</button>
        })
    }
}

#[component]
fn DetailPanel(store: Store) -> impl IntoView {
    let close = move |_| {
        store.selected.set(None);
        store.revealed.set(HashMap::new());
    };
    let show_tab = move |tab: DetailTab| {
        store.detail_tab.set(tab);
        if tab == DetailTab::Yaml && store.yaml.with_untracked(Option::is_none) {
            load_yaml(store);
        }
    };
    view! {
        {move || store.selected.get().map(|row| {
            let is_pod = selected_is_pod(store);
            view! {
                <aside class="detail">
                    <div class="detail-header">
                        <div>
                            <div class="detail-title">{row.name.clone()}</div>
                            <div class="muted">{row.namespace.clone().unwrap_or_default()}</div>
                        </div>
                        <div class="detail-actions">
                            <ServiceForwardButton store=store row=row.clone() />
                            <WorkloadLogsButton store=store row=row.clone() />
                            <crate::delete::DeleteButton store=store row=row.clone() />
                            <button class="close" title="Close" on:click=close>"×"</button>
                        </div>
                    </div>
                    {is_pod.then(|| view! {
                        <div class="tabs" role="tablist">
                            <button
                                role="tab"
                                class="tab"
                                class:active=move || store.detail_tab.get() == DetailTab::Details
                                on:click=move |_| show_tab(DetailTab::Details)
                            >"Details"</button>
                            <button
                                role="tab"
                                class="tab"
                                class:active=move || store.detail_tab.get() == DetailTab::Yaml
                                on:click=move |_| show_tab(DetailTab::Yaml)
                            >"YAML"</button>
                            <span class="spacer"></span>
                            {move || (store.detail_tab.get() == DetailTab::Details).then(|| view! {
                                <button
                                    class="btn small"
                                    prop:disabled=move || store.pod_loading.get()
                                    on:click=move |_| crate::pod_detail::load(store, true)
                                >"Refresh"</button>
                            })}
                        </div>
                    })}
                    <div class="detail-body">
                        {move || if store.detail_tab.get() == DetailTab::Details && is_pod {
                            view! { <crate::pod_detail::PodDetailView store=store /> }.into_any()
                        } else {
                            match store.yaml.get() {
                                None => view! { <p class="muted pad">"Loading…"</p> }.into_any(),
                                Some(Ok(yaml)) => view! { <pre class="yaml">{yaml}</pre> }.into_any(),
                                Some(Err(e)) => view! { <p class="warn">{e}</p> }.into_any(),
                            }
                        }}
                    </div>
                </aside>
            }
        })}
    }
}

/// kubectl-style age: 45s, 12m, 5h, 3d, 2y.
pub(crate) fn format_age(now_ms: f64, created: &str) -> String {
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
