//! The Cluster page: capacity, usage and health at a glance.

use std::time::Duration;

use leptos::prelude::*;
use leptos::task::spawn_local;
use subduer_types::{ClusterHistory, ClusterOverview, NodeSummary, PodCounts, ResourceRow, ResourceTotals, Series, UsageSource};

use crate::app::{Store, format_age, select_row};
use crate::chart::{ChartData, LineChart, Scale};
use crate::columns::ResizableTh;
use crate::logic::overview as logic;
pub use crate::logic::overview::{fmt_bytes, fmt_cpu};
use crate::tauri::{NoArgs, invoke};

const REFRESH: Duration = Duration::from_secs(30);

/// Fetches when a page that needs it opens or the cluster changes, then every
/// 30s while one is open, until [`logic::MAX_FAILURES`] fetches in a row fail.
pub fn start_refresh(store: Store) {
    Effect::new(move |_| {
        if store.needs_overview() && store.cluster.with(Option::is_some) {
            refresh(store, false);
        }
    });
    set_interval(
        move || {
            let polling = logic::keep_polling(store.overview_failures.get_untracked());
            if polling && untrack(|| store.needs_overview() && store.cluster.with(Option::is_some)) {
                refresh(store, true);
            }
        },
        REFRESH,
    );
}

fn refresh(store: Store, force: bool) {
    let age = js_sys::Date::now() - store.overview_at.get_untracked();
    if !logic::needs_fetch(store.overview_loading.get_untracked(), store.overview.with_untracked(Option::is_some), age, force) {
        return;
    }
    store.overview_loading.set(true);
    let generation = store.generation.get_untracked();
    spawn_local(async move {
        let result = invoke::<_, ClusterOverview>("cluster_overview", &NoArgs {}).await;
        // Drop the result if the user connected again meanwhile (another
        // cluster, or the same one after a dropped VPN); connect resets loading.
        if store.generation.get_untracked() != generation {
            return;
        }
        match result {
            Ok(data) => {
                store.overview_failures.set(0);
                store.overview.set(Some(data));
                store.overview_error.set(None);
                store.overview_at.set(js_sys::Date::now());
            }
            Err(e) => {
                store.overview_failures.update(|n| *n += 1);
                let (message, gave_up) = logic::failure(store.overview_failures.get_untracked(), e);
                store.overview_error.set(Some(message));
                if gave_up {
                    store.gave_up();
                }
            }
        }
        store.overview_loading.set(false);
    });
}

#[component]
pub fn Overview(store: Store) -> impl IntoView {
    let subtitle = move || {
        let context = store.context.get().unwrap_or_default();
        logic::subtitle(context, store.overview.with(|o| o.as_ref().and_then(|o| o.server_version.clone())))
    };
    view! {
        <div class="resource-header">
            {move || store.current_context().map(|c| view! { <crate::icons::ProviderLogo provider=c.provider /> })}
            <h1>"Cluster"</h1>
            <span class="muted">{subtitle}</span>
            <span class="spacer"></span>
            <RefreshStatus store=store />
        </div>
        {move || store.overview_error.get().map(|e| view! { <div class="warn">{e}</div> })}
        <div class="overview" class:stale=move || store.overview_loading.get() && store.overview.with(Option::is_some)>
            {move || match store.overview.get() {
                None => view! { <p class="muted pad">"Loading…"</p> }.into_any(),
                Some(o) => {
                    let namespaces = store.cluster.with(|c| c.as_ref().map_or(0, |c| c.namespaces.len()));
                    view! { <OverviewBody o=o namespaces=namespaces store=store /> }.into_any()
                }
            }}
        </div>
    }
}

/// "Updated Ns ago" plus a Refresh button for overview-backed pages.
#[component]
pub fn RefreshStatus(store: Store) -> impl IntoView {
    let updated = move || logic::updated(store.overview_at.get(), store.now.get());
    view! {
        <span class="muted">{updated}</span>
        <button
            class="btn"
            prop:disabled=move || store.overview_loading.get()
            on:click=move |_| {
                store.overview_failures.set(0);
                refresh(store, true);
            }
        >{move || if store.overview_loading.get() { "Refreshing…" } else { "Refresh" }}</button>
    }
}

/// The Node kind's listing: the Cluster page's node table, filterable by name,
/// with rows that open the YAML panel.
#[component]
pub fn NodePage(store: Store) -> impl IntoView {
    view! {
        <div class:stale=move || store.overview_loading.get() && store.overview.with(Option::is_some) class="fade">
            {move || {
                let filter = store.row_filter.get();
                match store.overview.get() {
                    None => view! { <p class="muted pad">"Loading…"</p> }.into_any(),
                    Some(o) => {
                        let nodes = logic::filter_nodes(o.nodes, &filter);
                        view! { <NodeTable nodes=nodes metrics=o.metrics_available store=store selectable=true /> }.into_any()
                    }
                }
            }}
        </div>
    }
}

#[component]
fn OverviewBody(o: ClusterOverview, namespaces: usize, store: Store) -> impl IntoView {
    let t = logic::tiles(&o);
    let node_count = o.nodes.len();
    let pods = o.pods;
    let metrics = o.metrics_available;
    let source = o.usage_source;
    let history = o.history.clone();
    let by_cpu = logic::by_cpu(&o.namespaces);

    view! {
        <div class="kpi-row">
            <StatTile label="Nodes" value=t.nodes note=t.nodes_note tone=t.nodes_tone />
            <StatTile label="Running pods" value=t.pods note=t.pods_note tone=t.pods_tone />
            <StatTile label="Namespaces" value=namespaces.to_string() note=String::new() />
            <StatTile label="Warning events, last hour" value=t.warnings note=t.warnings_note tone=t.warnings_tone />
        </div>

        <div class="card-row">
            <ResourceCard
                title="Memory available"
                res="res-memory"
                hero=true
                totals=o.memory
                nodes=node_count
                source=source
                history=history.clone().map(|h| (h.clone(), h.usage.memory))
                fmt=fmt_bytes
                scale=Scale::Binary
            />
            <ResourceCard
                title="CPU available"
                res="res-cpu"
                hero=false
                totals=o.cpu
                nodes=node_count
                source=source
                history=history.map(|h| (h.clone(), h.usage.cpu))
                fmt=fmt_cpu
                scale=Scale::Decimal
            />
        </div>

        <section class="card">
            <h2 class="card-title">"Pods by status"</h2>
            <PodStatusBar pods=pods />
        </section>

        <div class="card-row">
            <section class="card res-memory">
                <h2 class="card-title"><span class="res-dot"></span>"Memory requested by namespace"</h2>
                <NamespaceBars rows=logic::namespace_rows(&o.namespaces, |n| n.memory) fmt=fmt_bytes />
            </section>
            <section class="card res-cpu">
                <h2 class="card-title"><span class="res-dot"></span>"CPU requested by namespace"</h2>
                <NamespaceBars rows=logic::namespace_rows(&by_cpu, |n| n.cpu) fmt=fmt_cpu />
            </section>
        </div>

        <section class="card">
            <h2 class="card-title">"Nodes"</h2>
            <NodeTable nodes=o.nodes metrics=metrics store=store selectable=false />
        </section>
    }
}

#[component]
fn StatTile(
    label: &'static str,
    value: String,
    note: String,
    /// `good`, `warning` or `critical` tints the tile; empty leaves it plain.
    #[prop(optional)]
    tone: &'static str,
) -> impl IntoView {
    view! {
        <div class=format!("stat-tile {tone}")>
            <div class="stat-label">{label}</div>
            <div class="stat-value">{value}</div>
            <div class="stat-note">{note}</div>
        </div>
    }
}

/// Available capacity with used / requested / limits meters against it, and
/// the last hour of usage when Prometheus has it.
#[component]
fn ResourceCard(
    title: &'static str,
    /// `res-cpu` or `res-memory`: the card's colour.
    res: &'static str,
    hero: bool,
    totals: ResourceTotals,
    nodes: usize,
    source: Option<UsageSource>,
    history: Option<(ClusterHistory, Series)>,
    fmt: fn(f64) -> String,
    scale: Scale,
) -> impl IntoView {
    let alloc = totals.allocatable;
    let sub = logic::card_subtitle(nodes, totals.capacity, fmt);
    let used = match totals.used {
        Some(used) => view! { <Meter label="Used" value=used total=alloc fmt=fmt /> }.into_any(),
        None => view! {
            <div class="meter-row">
                <span class="meter-label">"Used"</span>
                <span class="meter-value muted">"not available"</span>
            </div>
        }
        .into_any(),
    };
    view! {
        <section class=format!("card {res}")>
            <h2 class="card-title"><span class="res-dot"></span>{title}</h2>
            <div class=if hero { "hero" } else { "stat-value" }>{fmt(alloc)}</div>
            <div class="muted">{sub}</div>
            <div class="meters">
                {used}
                <Meter label="Requested" value=totals.requests total=alloc fmt=fmt />
                <Meter label="Limits" value=totals.limits total=alloc fmt=fmt />
            </div>
            {history.map(|(h, points)| {
                // No request/allocatable lines: at cluster scale they dwarf usage
                // and flatten the trend; the meters above carry them.
                let peak = logic::peak(&points, fmt);
                let data = ChartData { start: h.start, end: h.end, step: h.step, points, refs: vec![] };
                view! {
                    <div class="card-chart">
                        <div class="meter-row">
                            <span class="meter-label">"Used, last hour"</span>
                            <span class="meter-value">{peak}</span>
                        </div>
                        <LineChart label=title data=Signal::derive(move || Some(data.clone())) fmt=fmt scale=scale />
                    </div>
                }
            })}
            <p class="note">{logic::usage_note(source)}</p>
        </section>
    }
}

#[component]
pub(crate) fn Meter(label: &'static str, value: f64, total: f64, fmt: fn(f64) -> String) -> impl IntoView {
    let m = logic::meter(label, value, total, fmt);
    view! {
        <div class="meter-row">
            <span class="meter-label">{label}</span>
            <span class="meter-value">{m.text}</span>
        </div>
        <div class="meter" title=m.title>
            <div class=m.fill_class style=m.width></div>
        </div>
    }
}

#[component]
fn PodStatusBar(pods: PodCounts) -> impl IntoView {
    let total = pods.total();
    let segments = logic::pod_segments(pods);
    if total == 0 {
        return view! { <p class="muted">"No pods."</p> }.into_any();
    }
    let bar = segments
        .iter()
        .filter(|(_, _, n)| *n > 0)
        .map(|(label, class, n)| {
            let (style, title) = logic::segment(label, *n, total);
            view! { <div class=format!("segment {class}") style=style title=title></div> }
        })
        .collect_view();
    let legend = segments
        .into_iter()
        .map(|(label, class, n)| {
            view! {
                <span class="legend-item">
                    <span class=format!("swatch {class}")></span>
                    {label}
                    <strong>{n}</strong>
                </span>
            }
        })
        .collect_view();
    view! {
        <div class="stacked-bar">{bar}</div>
        <div class="legend">{legend}<span class="legend-total muted">{format!("{total} total")}</span></div>
    }
    .into_any()
}

#[component]
fn NamespaceBars(rows: Vec<(String, f64, usize)>, fmt: fn(f64) -> String) -> impl IntoView {
    let max = logic::bar_max(&rows);
    if rows.is_empty() {
        return view! { <p class="muted">"No requests set."</p> }.into_any();
    }
    rows.into_iter()
        .map(|(name, value, pods)| {
            let (width, title) = logic::bar(&name, value, pods, max, fmt);
            view! {
                <div class="hbar-row" title=title>
                    <span class="hbar-label">{name}</span>
                    <span class="hbar-track">
                        <span class="hbar" style=width></span>
                        <span class="hbar-value">{fmt(value)}</span>
                    </span>
                </div>
            }
        })
        .collect_view()
        .into_any()
}

#[component]
fn NodeTable(nodes: Vec<NodeSummary>, metrics: bool, store: Store, selectable: bool) -> impl IntoView {
    const TABLE: &str = "nodes";
    let headers = logic::node_headers(metrics);
    let names: Vec<&'static str> = headers.iter().map(|(h, _)| *h).collect();
    let rows = move || {
        let mut nodes = nodes.clone();
        let by = store.sorts.by(TABLE, &names);
        crate::sort::sort(&mut nodes, by, |n, i| logic::node_key(n, names[i]));
        nodes
        .into_iter()
        .map(|n| {
            let (status, badge) = logic::node_status(&n);
            let created = n.created.clone().unwrap_or_default();
            let age = move || format_age(store.now.get(), &created);
            let name = n.name.clone();
            let selected = {
                let name = name.clone();
                move || selectable && store.selected.with(|s| s.as_ref().is_some_and(|s| s.id == name))
            };
            let click = move |_| {
                if selectable {
                    select_row(
                        store,
                        ResourceRow {
                            id: name.clone(),
                            name: name.clone(),
                            namespace: None,
                            cells: vec![],
                        },
                    );
                }
            };
            view! {
                <tr class:selected=selected class:clickable=selectable on:click=click>
                    <td class="name">{n.name.clone()}</td>
                    <td><span class=badge>{status}</span></td>
                    <td>{n.instance_type.clone().unwrap_or_default()}</td>
                    <td class="num">{logic::node_pods(&n)}</td>
                    <td class="res-cpu"><MiniMeter value=n.cpu.requests total=n.cpu.allocatable /></td>
                    <td class="res-memory"><MiniMeter value=n.memory.requests total=n.memory.allocatable /></td>
                    {metrics.then(|| view! {
                        <td class="res-cpu"><MiniMeter value=n.cpu.used.unwrap_or(0.0) total=n.cpu.allocatable /></td>
                        <td class="res-memory"><MiniMeter value=n.memory.used.unwrap_or(0.0) total=n.memory.allocatable /></td>
                    })}
                    <td class="num">{fmt_cpu(n.cpu.allocatable)}</td>
                    <td class="num">{fmt_bytes(n.memory.allocatable)}</td>
                    <td>{n.kubelet_version.clone()}</td>
                    <td class="age">{age}</td>
                </tr>
            }
        })
        .collect_view()
    };
    let names: Vec<String> = headers.iter().map(|(h, _)| h.to_string()).collect();
    let style = move || store.col_widths.table_style(TABLE, &names);
    view! {
        <div class="table-scroll">
            <table class="resources" class:fixed=move || store.col_widths.has_table(TABLE) style=style>
                <thead><tr>
                    {headers.iter().map(|(h, class)| view! {
                        <ResizableTh widths=store.col_widths table=TABLE.to_owned() header=h.to_string() class=*class sorts=store.sorts />
                    }).collect_view()}
                </tr></thead>
                <tbody>{rows}</tbody>
            </table>
        </div>
    }
}

#[component]
fn MiniMeter(value: f64, total: f64) -> impl IntoView {
    let (fill, width, text) = logic::mini_meter(value, total);
    view! {
        <span class="mini-meter">
            <span class="meter small"><span class=fill style=width></span></span>
            <span class="mini-value">{text}</span>
        </span>
    }
}
