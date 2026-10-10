//! The Cluster page: capacity, usage and health at a glance.

use std::time::Duration;

use leptos::prelude::*;
use leptos::task::spawn_local;
use subduer_types::{
    ClusterHistory, ClusterOverview, NamespaceRequests, NodeSummary, PodCounts, ResourceRow, ResourceTotals, Series, UsageSource,
};

use crate::app::{Store, format_age, select_row};
use crate::chart::{ChartData, LineChart, Scale};
use crate::columns::ResizableTh;
use crate::tauri::{NoArgs, invoke};

const REFRESH: Duration = Duration::from_secs(30);
/// Bars shown per namespace chart; the rest fold into "Other".
const TOP_NAMESPACES: usize = 8;

/// Data younger than this is reused when switching between the Cluster and Node pages.
const FRESH_MS: f64 = 10_000.0;

/// Fetches when a page that needs it opens or the cluster changes, then every
/// 30s while one is open.
pub fn start_refresh(store: Store) {
    Effect::new(move |_| {
        if store.needs_overview() && store.cluster.with(Option::is_some) {
            refresh(store, false);
        }
    });
    set_interval(
        move || {
            if untrack(|| store.needs_overview() && store.cluster.with(Option::is_some)) {
                refresh(store, true);
            }
        },
        REFRESH,
    );
}

fn refresh(store: Store, force: bool) {
    if store.overview_loading.get_untracked() {
        return;
    }
    let fresh = store.overview.with_untracked(Option::is_some)
        && js_sys::Date::now() - store.overview_at.get_untracked() < FRESH_MS;
    if fresh && !force {
        return;
    }
    store.overview_loading.set(true);
    let context = store.context.get_untracked();
    spawn_local(async move {
        let result = invoke::<_, ClusterOverview>("cluster_overview", &NoArgs {}).await;
        // Drop the result if the user switched clusters meanwhile.
        if store.context.get_untracked() == context {
            match result {
                Ok(data) => {
                    store.overview.set(Some(data));
                    store.overview_error.set(None);
                    store.overview_at.set(js_sys::Date::now());
                }
                Err(e) => store.overview_error.set(Some(e)),
            }
        }
        store.overview_loading.set(false);
    });
}

#[component]
pub fn Overview(store: Store) -> impl IntoView {
    let subtitle = move || {
        let context = store.context.get().unwrap_or_default();
        match store.overview.with(|o| o.as_ref().and_then(|o| o.server_version.clone())) {
            Some(v) => format!("{context} · {v}"),
            None => context,
        }
    };
    view! {
        <div class="resource-header">
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
    let updated = move || {
        let at = store.overview_at.get();
        (at > 0.0).then(|| format!("Updated {} ago", age_ms(store.now.get() - at)))
    };
    view! {
        <span class="muted">{updated}</span>
        <button
            class="btn"
            prop:disabled=move || store.overview_loading.get()
            on:click=move |_| refresh(store, true)
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
                let filter = store.row_filter.get().to_lowercase();
                match store.overview.get() {
                    None => view! { <p class="muted pad">"Loading…"</p> }.into_any(),
                    Some(o) => {
                        let nodes = o
                            .nodes
                            .into_iter()
                            .filter(|n| filter.is_empty() || n.name.to_lowercase().contains(&filter))
                            .collect();
                        view! { <NodeTable nodes=nodes metrics=o.metrics_available store=store selectable=true /> }.into_any()
                    }
                }
            }}
        </div>
    }
}

#[component]
fn OverviewBody(o: ClusterOverview, namespaces: usize, store: Store) -> impl IntoView {
    let ready = o.nodes.iter().filter(|n| n.ready).count();
    let node_count = o.nodes.len();
    let nodes_note = if ready == node_count {
        "all ready".to_owned()
    } else {
        format!("⚠ {} not ready", node_count - ready)
    };
    let pods = o.pods;
    let pods_note = format!("{} pending · {} failed", pods.pending, pods.failed);
    let (warnings, warnings_note) = match o.warnings_last_hour {
        Some(0) => ("0".to_owned(), "none".to_owned()),
        Some(n) => (n.to_string(), "⚠ see Events".to_owned()),
        None => ("–".to_owned(), "can't read events".to_owned()),
    };
    let metrics = o.metrics_available;
    let source = o.usage_source;
    let history = o.history.clone();
    let mut by_cpu = o.namespaces.clone();
    by_cpu.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));

    view! {
        <div class="kpi-row">
            <StatTile label="Nodes" value=node_count.to_string() note=nodes_note />
            <StatTile label="Running pods" value=pods.running.to_string() note=pods_note />
            <StatTile label="Namespaces" value=namespaces.to_string() note=String::new() />
            <StatTile label="Warning events, last hour" value=warnings note=warnings_note />
        </div>

        <div class="card-row">
            <ResourceCard
                title="Memory available"
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
            <section class="card">
                <h2 class="card-title">"Memory requested by namespace"</h2>
                <NamespaceBars rows=namespace_rows(&o.namespaces, |n| n.memory) fmt=fmt_bytes />
            </section>
            <section class="card">
                <h2 class="card-title">"CPU requested by namespace"</h2>
                <NamespaceBars rows=namespace_rows(&by_cpu, |n| n.cpu) fmt=fmt_cpu />
            </section>
        </div>

        <section class="card">
            <h2 class="card-title">"Nodes"</h2>
            <NodeTable nodes=o.nodes metrics=metrics store=store selectable=false />
        </section>
    }
}

#[component]
fn StatTile(label: &'static str, value: String, note: String) -> impl IntoView {
    view! {
        <div class="stat-tile">
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
    hero: bool,
    totals: ResourceTotals,
    nodes: usize,
    source: Option<UsageSource>,
    history: Option<(ClusterHistory, Series)>,
    fmt: fn(f64) -> String,
    scale: Scale,
) -> impl IntoView {
    let alloc = totals.allocatable;
    let sub = format!("allocatable across {nodes} nodes · {} capacity", fmt(totals.capacity));
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
        <section class="card">
            <h2 class="card-title">{title}</h2>
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
                let peak = points.iter().map(|p| p.1).fold(0.0, f64::max);
                let data = ChartData { start: h.start, end: h.end, step: h.step, points, refs: vec![] };
                view! {
                    <div class="card-chart">
                        <div class="meter-row">
                            <span class="meter-label">"Used, last hour"</span>
                            <span class="meter-value">{format!("peak {}", fmt(peak))}</span>
                        </div>
                        <LineChart label=title data=Signal::derive(move || Some(data.clone())) fmt=fmt scale=scale />
                    </div>
                }
            })}
            <p class="note">{usage_note(source)}</p>
        </section>
    }
}

fn usage_note(source: Option<UsageSource>) -> &'static str {
    match source {
        Some(UsageSource::MetricsServer) => "Usage from metrics-server.",
        Some(UsageSource::Prometheus) => {
            "Usage from Prometheus (container working set), with kubelet stats for nodes it doesn't cover."
        }
        Some(UsageSource::Kubelet) => "Usage from kubelet stats on each node. No metrics-server or Prometheus found.",
        None => "Live usage needs metrics-server, Prometheus, or access to kubelet stats (nodes/proxy).",
    }
}

/// Severity class and marker for a fraction of capacity. Colour is never the
/// only signal: warning levels also get an icon in the value text.
fn severity(fraction: f64) -> (&'static str, &'static str) {
    if fraction >= 0.9 {
        ("critical", "⚠ ")
    } else if fraction >= 0.75 {
        ("warning", "⚠ ")
    } else {
        ("normal", "")
    }
}

#[component]
fn Meter(label: &'static str, value: f64, total: f64, fmt: fn(f64) -> String) -> impl IntoView {
    let fraction = if total > 0.0 { value / total } else { 0.0 };
    let (sev, icon) = severity(fraction);
    let over = if fraction > 1.0 { " · overcommitted" } else { "" };
    let text = format!("{icon}{} · {}{over}", fmt(value), pct(fraction));
    let width = format!("width: {:.1}%", (fraction * 100.0).clamp(0.0, 100.0));
    view! {
        <div class="meter-row">
            <span class="meter-label">{label}</span>
            <span class="meter-value">{text}</span>
        </div>
        <div class="meter" title=format!("{label}: {} of {}", fmt(value), fmt(total))>
            <div class=format!("meter-fill {sev}") style=width></div>
        </div>
    }
}

#[component]
fn PodStatusBar(pods: PodCounts) -> impl IntoView {
    let total = pods.total();
    let segments: Vec<(&str, &str, usize)> = vec![
        ("Running", "good", pods.running),
        ("Pending", "warning", pods.pending),
        ("Failed", "critical", pods.failed),
        ("Unknown", "serious", pods.unknown),
        ("Succeeded", "done", pods.succeeded),
    ];
    if total == 0 {
        return view! { <p class="muted">"No pods."</p> }.into_any();
    }
    let bar = segments
        .iter()
        .filter(|(_, _, n)| *n > 0)
        .map(|(label, class, n)| {
            let style = format!("flex-grow: {n}");
            let title = format!("{label}: {n} ({})", pct(*n as f64 / total as f64));
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

/// Top namespaces by `value`, with the rest folded into one "Other" bar.
fn namespace_rows(sorted: &[NamespaceRequests], value: fn(&NamespaceRequests) -> f64) -> Vec<(String, f64, usize)> {
    let mut rows: Vec<(String, f64, usize)> = sorted
        .iter()
        .filter(|n| value(n) > 0.0)
        .take(TOP_NAMESPACES)
        .map(|n| (n.namespace.clone(), value(n), n.pods))
        .collect();
    let rest: Vec<&NamespaceRequests> = sorted.iter().filter(|n| value(n) > 0.0).skip(TOP_NAMESPACES).collect();
    if !rest.is_empty() {
        let label = format!("Other ({} namespaces)", rest.len());
        rows.push((label, rest.iter().map(|n| value(n)).sum(), rest.iter().map(|n| n.pods).sum()));
    }
    rows
}

#[component]
fn NamespaceBars(rows: Vec<(String, f64, usize)>, fmt: fn(f64) -> String) -> impl IntoView {
    let max = rows.iter().map(|r| r.1).fold(0.0, f64::max);
    if rows.is_empty() {
        return view! { <p class="muted">"No requests set."</p> }.into_any();
    }
    rows.into_iter()
        .map(|(name, value, pods)| {
            let width = format!("width: {:.1}%", if max > 0.0 { value / max * 100.0 } else { 0.0 });
            let title = format!("{name}: {} across {pods} pods", fmt(value));
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
    let rows = nodes
        .into_iter()
        .map(|n| {
            let mut status = if n.ready { "Ready".to_owned() } else { "NotReady".to_owned() };
            if !n.schedulable {
                status.push_str(" · Cordoned");
            }
            let badge = match (n.ready, n.schedulable) {
                (false, _) => "badge badge-notready",
                (true, false) => "badge badge-pending",
                (true, true) => "badge badge-ready",
            };
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
                    <td class="num">{format!("{}/{}", n.pods, n.pod_capacity)}</td>
                    <td><MiniMeter value=n.cpu.requests total=n.cpu.allocatable /></td>
                    <td><MiniMeter value=n.memory.requests total=n.memory.allocatable /></td>
                    {metrics.then(|| view! {
                        <td><MiniMeter value=n.cpu.used.unwrap_or(0.0) total=n.cpu.allocatable /></td>
                        <td><MiniMeter value=n.memory.used.unwrap_or(0.0) total=n.memory.allocatable /></td>
                    })}
                    <td class="num">{fmt_cpu(n.cpu.allocatable)}</td>
                    <td class="num">{fmt_bytes(n.memory.allocatable)}</td>
                    <td>{n.kubelet_version.clone()}</td>
                    <td class="age">{age}</td>
                </tr>
            }
        })
        .collect_view();
    const TABLE: &str = "nodes";
    let mut headers: Vec<(&str, &'static str)> = vec![
        ("Name", ""),
        ("Status", ""),
        ("Type", ""),
        ("Pods", "num"),
        ("CPU requested", ""),
        ("Memory requested", ""),
    ];
    if metrics {
        headers.extend([("CPU used", ""), ("Memory used", "")]);
    }
    headers.extend([("CPU", "num"), ("Memory", "num"), ("Version", ""), ("Age", "")]);
    let names: Vec<String> = headers.iter().map(|(h, _)| h.to_string()).collect();
    let style = move || store.col_widths.table_style(TABLE, &names);
    view! {
        <div class="table-scroll">
            <table class="resources" class:fixed=move || store.col_widths.has_table(TABLE) style=style>
                <thead><tr>
                    {headers.iter().map(|(h, class)| view! {
                        <ResizableTh widths=store.col_widths table=TABLE.to_owned() header=h.to_string() class=*class />
                    }).collect_view()}
                </tr></thead>
                <tbody>{rows}</tbody>
            </table>
        </div>
    }
}

#[component]
fn MiniMeter(value: f64, total: f64) -> impl IntoView {
    let fraction = if total > 0.0 { value / total } else { 0.0 };
    let (sev, icon) = severity(fraction);
    let width = format!("width: {:.1}%", (fraction * 100.0).clamp(0.0, 100.0));
    view! {
        <span class="mini-meter">
            <span class="meter small"><span class=format!("meter-fill {sev}") style=width></span></span>
            <span class="mini-value">{format!("{icon}{}", pct(fraction))}</span>
        </span>
    }
}

fn pct(fraction: f64) -> String {
    let p = fraction * 100.0;
    if p > 0.0 && p < 1.0 { "<1%".to_owned() } else { format!("{p:.0}%") }
}

pub fn fmt_bytes(b: f64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = b;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{v:.0} B") } else { format!("{v:.1} {}", UNITS[unit]) }
}

pub fn fmt_cpu(millicores: f64) -> String {
    if millicores > 0.0 && millicores < 10.0 {
        format!("{millicores:.1}m")
    } else if millicores < 1000.0 {
        format!("{millicores:.0}m")
    } else {
        let cores = millicores / 1000.0;
        match cores {
            1.0 => "1 core".to_owned(),
            c if c.fract() == 0.0 => format!("{c:.0} cores"),
            c => format!("{c:.1} cores"),
        }
    }
}

fn age_ms(ms: f64) -> String {
    let secs = (ms / 1000.0).max(0.0) as u64;
    if secs < 60 { format!("{secs}s") } else { format!("{}m", secs / 60) }
}
