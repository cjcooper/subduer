//! The main window's logic: connecting, the sidebar, the resource table,
//! the detail panel and opening objects from links.

use std::collections::HashSet;

use subduer_types::containers::{self, Container, State};
use subduer_types::{
    Category, ClusterInfo, Column, ColumnKind, ContextInfo, ContextList, ContextRemoved, EditTarget, KindInfo, KindRef, LogSource,
    LogTarget, ObjectDetails, ResourceRow, ResourceTable,
};

use crate::logic::prefs::Theme;
use crate::logic::sort::{self, Key};

/// Failed connection checks in a row before the context counts as disconnected.
pub(crate) const MAX_PROBE_FAILURES: u32 = 3;

/// Shown when the kubeconfig's last context goes.
pub(crate) const NO_CONTEXTS_NOTICE: &str = "Your kubeconfig has no contexts left. Add a cluster to carry on.";

/// Explains the container icons (tooltip on the Ready header).
pub(crate) const CONTAINER_LEGEND: &str = "Containers: ✓ running · ◐ starting · ■ stopped · ✕ failed";

pub(crate) fn find_kind(kinds: &[KindInfo], key: &str) -> Option<KindRef> {
    kinds.iter().map(|k| &k.kind).find(|k| k.key() == key).cloned()
}

/// The context called `name`, if it's listed.
pub(crate) fn context_named(contexts: &[ContextInfo], name: Option<&str>) -> Option<ContextInfo> {
    let name = name?;
    contexts.iter().find(|c| c.name == name).cloned()
}

/// Whether `kind` is the one with this key (e.g. `core/Pod`).
pub(crate) fn is_kind(kind: Option<&KindRef>, key: &str) -> bool {
    kind.is_some_and(|k| k.key() == key)
}

/// The Node kind is shown with the Cluster page's node table.
pub(crate) fn is_node_page(overview_open: bool, kind: Option<&KindRef>) -> bool {
    !overview_open && is_kind(kind, "core/Node")
}

/// Pods, workloads and the Config section's kinds get a Details tab besides YAML.
pub(crate) fn has_details(kind: Option<&KindRef>) -> bool {
    is_kind(kind, "core/Pod")
        || kind.is_some_and(|k| subduer_types::has_workload_details(k) || subduer_types::has_object_details(k))
}

/// Whether `kind`'s details are the generic object view.
pub(crate) fn is_object(kind: Option<&KindRef>) -> bool {
    kind.is_some_and(subduer_types::has_object_details)
}

/// The context to connect to when one appears while there were none (added
/// here, or with kubectl): kubectl's current one, else the first.
pub(crate) fn first_context(list: &ContextList) -> Option<String> {
    list.current
        .clone()
        .filter(|c| list.contexts.iter().any(|x| x.name == *c))
        .or_else(|| list.contexts.first().map(|c| c.name.clone()))
}

/// Whether connecting to `context` picks up where a lost connection left off.
pub(crate) fn resumes(disconnected: bool, current: Option<&str>, context: &str) -> bool {
    disconnected && current == Some(context)
}

/// The namespace after connecting: kept when resuming (if the cluster still
/// has it), else the context's default namespace when the cluster has that.
pub(crate) fn namespace_after_connect(
    resume: bool,
    current: Option<String>,
    contexts: &[ContextInfo],
    context: &str,
    namespaces: &[String],
) -> Option<String> {
    if resume && current.as_ref().is_none_or(|ns| namespaces.contains(ns)) {
        return current;
    }
    contexts.iter().find(|c| c.name == context).and_then(|c| c.namespace.clone()).filter(|ns| namespaces.contains(ns))
}

/// The kind after connecting: the selected one if this cluster has it, else Pods.
pub(crate) fn kind_after_connect(current: Option<&KindRef>, kinds: &[KindInfo]) -> Option<KindRef> {
    current.and_then(|k| find_kind(kinds, &k.key())).or_else(|| find_kind(kinds, "core/Pod"))
}

/// Whether a background connection check should run now.
pub(crate) fn should_probe(probing: bool, connecting: bool, connected: bool) -> bool {
    !probing && !connecting && connected
}

/// Whether this many failed checks in a row count as a lost connection.
pub(crate) fn probe_gives_up(failures: u32, connected: bool) -> bool {
    failures >= MAX_PROBE_FAILURES && connected
}

/// Said when this window's context was deleted outside the app.
pub(crate) fn removed_notice(moved: &ContextRemoved) -> String {
    format!("“{}” was removed from your kubeconfig, so this window moved to “{}”.", moved.removed, moved.switch_to)
}

/// Whether a watch's table is current: older watches' snapshots are ignored.
pub(crate) fn takes_table(watch_id: u64, min_watch_id: u64) -> bool {
    watch_id >= min_watch_id
}

/// The row waiting to be selected, once the table lists it.
pub(crate) fn pending_row(table: &ResourceTable, pending: Option<&str>) -> Option<ResourceRow> {
    let id = pending?;
    table.rows.iter().find(|r| r.id == id).cloned()
}

/// Whether an object saved in an edit window is the one shown.
pub(crate) fn edit_shows(selected: Option<&ResourceRow>, kind: Option<&KindRef>, target: &EditTarget) -> bool {
    selected.is_some_and(|s| s.name == target.name && s.namespace == target.namespace) && kind == Some(&target.kind)
}

/// Whether the background refresh reloads the detail panel's Details.
pub(crate) fn refreshes_details(connected: bool, selected: bool, has_details: bool, on_details_tab: bool) -> bool {
    connected && selected && has_details && on_details_tab
}

/// What the main area shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Page {
    Traces,
    Forwards,
    NoClusters,
    Disconnected,
    Overview,
    Resources,
}

pub(crate) fn page(traces_open: bool, forwards_open: bool, no_clusters: bool, disconnected: bool, overview_open: bool) -> Page {
    if traces_open {
        Page::Traces
    } else if forwards_open {
        Page::Forwards
    } else if no_clusters {
        Page::NoClusters
    } else if disconnected {
        Page::Disconnected
    } else if overview_open {
        Page::Overview
    } else {
        Page::Resources
    }
}

pub(crate) fn not_connected_title(context: Option<&str>) -> String {
    format!("Not connected to {}", context.unwrap_or_default())
}

pub(crate) fn connect_label(connecting: bool) -> &'static str {
    if connecting { "Connecting…" } else { "Connect" }
}

/// The namespace filter for a `<select>` value: empty means all namespaces.
pub(crate) fn namespace_choice(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

/// The lock button's class, label and tooltip.
pub(crate) fn lock_look(unlocked: bool, prod: bool) -> (String, &'static str, &'static str) {
    let (class, label, title) = if unlocked {
        ("lock unlocked", "Writable", "Changes are allowed. Click to make read-only.")
    } else {
        ("lock", "Read-only", "Click to allow changes to this context.")
    };
    let class = if prod { format!("{class} prod") } else { class.to_owned() };
    (class, label, title)
}

/// Whether Allow changes can be clicked: a production-like context's name
/// must be typed out.
pub(crate) fn unlock_ok(prod: bool, typed: &str, name: &str) -> bool {
    !prod || typed.trim() == name
}

/// A category's kinds that match the sidebar filter, in the backend's order.
pub(crate) fn kinds_in(cluster: Option<&ClusterInfo>, category: Category, filter: &str) -> Vec<KindRef> {
    let filter = filter.to_lowercase();
    cluster
        .iter()
        .flat_map(|c| &c.kinds)
        .filter(|k| k.category == category && kind_matches(&k.kind, &filter))
        .map(|k| k.kind.clone())
        .collect()
}

/// The class that tints a sidebar section, e.g. `cat-access-control`.
pub(crate) fn category_class(category: Category) -> String {
    format!("cat-{}", category.label().to_lowercase().replace(' ', "-"))
}

pub(crate) fn kind_section_class(category: Category) -> String {
    format!("kind-section {}", category_class(category))
}

/// Whether a fixed sidebar entry's label matches the (lowercased) filter.
pub(crate) fn kind_matches_text(text: &str, filter: &str) -> bool {
    filter.is_empty() || text.to_lowercase().contains(filter)
}

/// Whether a kind matches the (lowercased) filter: its name, plural or group.
pub(crate) fn kind_matches(k: &KindRef, filter: &str) -> bool {
    filter.is_empty() || k.kind.to_lowercase().contains(filter) || k.plural.contains(filter) || k.group.contains(filter)
}

/// Moves `moved` to `target`'s position in `order`; unknown sections leave it as is.
pub(crate) fn reorder(order: &mut Vec<Category>, moved: Category, target: Category) {
    let (Some(from), Some(to)) = (order.iter().position(|c| *c == moved), order.iter().position(|c| *c == target)) else {
        return;
    };
    let item = order.remove(from);
    order.insert(to, item);
}

/// Whether a section shows its kinds: searching shows every match, even in
/// collapsed sections.
pub(crate) fn section_open(filter: &str, expanded: &HashSet<Category>, category: Category) -> bool {
    !filter.is_empty() || expanded.contains(&category)
}

/// Opens a closed section, closes an open one.
pub(crate) fn toggle_section(expanded: &mut HashSet<Category>, category: Category) {
    if !expanded.remove(&category) {
        expanded.insert(category);
    }
}

/// Whether a section is where a dragged (other) section would drop.
pub(crate) fn is_drop_target(drop_target: Option<Category>, dragging: Option<Category>, category: Category) -> bool {
    drop_target == Some(category) && dragging != Some(category)
}

/// Whether the sidebar's Cluster entry is the page shown.
pub(crate) fn cluster_active(overview_open: bool, forwards_open: bool, traces_open: bool) -> bool {
    overview_open && !forwards_open && !traces_open
}

/// Whether the sidebar's Port Forwards entry is the page shown.
pub(crate) fn forwards_active(forwards_open: bool, traces_open: bool) -> bool {
    forwards_open && !traces_open
}

/// Whether a kind's sidebar entry is the page shown.
pub(crate) fn kind_active(overview_open: bool, forwards_open: bool, traces_open: bool, current: Option<&KindRef>, key: &str) -> bool {
    !overview_open && !forwards_open && !traces_open && is_kind(current, key)
}

/// For each kind, whether an API group heading goes before it (`grouped`
/// sections list custom resources under their group).
pub(crate) fn group_headings(grouped: bool, kinds: &[KindRef]) -> Vec<bool> {
    let mut last: Option<&str> = None;
    kinds
        .iter()
        .map(|k| {
            let heading = grouped && last != Some(k.group.as_str());
            if heading {
                last = Some(&k.group);
            }
            heading
        })
        .collect()
}

/// A kind's sidebar tooltip: its plural and API version.
pub(crate) fn kind_title(k: &KindRef) -> String {
    format!("{} ({})", k.plural, k.api_version())
}

/// The headers of a table's columns.
pub(crate) fn column_headers(columns: &[Column]) -> Vec<&str> {
    columns.iter().map(|c| c.header.as_str()).collect()
}

/// The table's rows whose id contains the filter (any case), sorted by
/// column `by`. `parse_time` reads the Age column's times (ms).
pub(crate) fn table_rows(table: ResourceTable, filter: &str, by: Option<(usize, bool)>, parse_time: fn(&str) -> f64) -> Vec<ResourceRow> {
    let filter = filter.to_lowercase();
    let mut rows: Vec<ResourceRow> =
        table.rows.into_iter().filter(|r| filter.is_empty() || r.id.to_lowercase().contains(&filter)).collect();
    let columns = table.columns;
    sort::sort(&mut rows, by, |r, i| cell_key(columns[i].kind, r.cells.get(i).map_or("", String::as_str), parse_time));
    rows
}

/// Where the listed objects are: cluster-wide, one namespace, or all.
pub(crate) fn scope_text(kind: Option<&KindRef>, namespace: Option<&str>) -> String {
    match (kind, namespace) {
        (Some(k), _) if !k.namespaced => "cluster-scoped".to_owned(),
        (_, Some(ns)) => format!("in {ns}"),
        _ => "in all namespaces".to_owned(),
    }
}

/// The watch's error, and whether it stopped retrying.
pub(crate) fn watch_problem(table: &ResourceTable) -> Option<(String, bool)> {
    Some((table.error.clone()?, table.stopped))
}

/// Columns to display with their cell index. The namespace column is
/// redundant while a single namespace is selected.
pub(crate) fn visible(columns: &[Column], single_namespace: bool) -> impl Iterator<Item = (usize, &Column)> {
    columns.iter().enumerate().filter(move |(_, c)| !(single_namespace && c.kind == ColumnKind::Namespace))
}

/// The headers of the [`visible`] columns.
pub(crate) fn visible_headers(columns: &[Column], single_namespace: bool) -> Vec<String> {
    visible(columns, single_namespace).map(|(_, c)| c.header.clone()).collect()
}

/// A column heading's tooltip: the container legend on the containers column.
pub(crate) fn column_title(kind: ColumnKind) -> &'static str {
    if kind == ColumnKind::Containers { CONTAINER_LEGEND } else { "" }
}

/// How a cell sorts: ages youngest first, numbers by value, containers with
/// failures first and then by the share running, the rest as text.
/// `parse_time` reads times (ms; NaN if they don't parse).
pub(crate) fn cell_key(kind: ColumnKind, value: &str, parse_time: fn(&str) -> f64) -> Key {
    match kind {
        ColumnKind::Age => Key::age_ms(parse_time(value)),
        ColumnKind::Number => Key::quantity(value),
        ColumnKind::Containers => {
            let all = containers::decode(value);
            if all.is_empty() {
                return Key::Missing;
            }
            let running = all.iter().filter(|c| c.state == State::Running).count();
            let failed = all.iter().any(|c| c.state == State::Failed);
            Key::num(running as f64 / all.len() as f64 - if failed { 1.0 } else { 0.0 })
        }
        _ => Key::text(value),
    }
}

pub(crate) fn column_class(kind: ColumnKind) -> &'static str {
    match kind {
        ColumnKind::Name => "name",
        ColumnKind::LongText => "long",
        ColumnKind::Number => "num",
        ColumnKind::Age => "age",
        ColumnKind::Namespace | ColumnKind::Text | ColumnKind::Badge | ColumnKind::Containers => "",
    }
}

/// A container icon's class, tooltip and accessible label. Each state has
/// its own symbol, so colour is never the only cue.
pub(crate) fn container_look(c: &Container) -> (&'static str, String, String) {
    let (class, label) = match c.state {
        State::Running => ("ctr running", "running"),
        State::Starting => ("ctr starting", "starting"),
        State::Stopped => ("ctr stopped", "stopped"),
        State::Failed => ("ctr failed", "failed"),
    };
    let kind = if c.init { "init container " } else { "" };
    (class, format!("{kind}{}: {}", c.name, c.detail), format!("{kind}{} {label}", c.name))
}

/// What the Appearance setting does.
pub(crate) fn theme_note(theme: Theme) -> &'static str {
    match theme {
        Theme::Auto => "Follows your system's light or dark appearance.",
        Theme::Light => "Always light.",
        Theme::Dark => "Always dark.",
    }
}

/// What has unsaved edits, for the discard prompt.
pub(crate) fn dirty_what(dirty: Option<String>) -> String {
    dirty.unwrap_or_else(|| "this object".into())
}

/// Whether showing a tab loads the YAML: the YAML tab, not loaded yet.
pub(crate) fn loads_yaml(yaml_tab: bool, yaml_loaded: bool) -> bool {
    yaml_tab && !yaml_loaded
}

/// Whether Esc closes the detail panel: a plain Esc, no dialog open, a
/// selection, and not typing in a text field.
pub(crate) fn esc_closes_panel(plain_esc: bool, dialog_open: bool, selected: bool, in_text_field: bool) -> bool {
    plain_esc && !dialog_open && selected && !in_text_field
}

/// What opening an object from a link does.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum OpenPlan {
    /// The cluster doesn't list the kind: this error.
    Missing(String),
    /// No name: the kind's list, filtered to the namespace.
    List { kind: KindRef, namespace: Option<String> },
    /// One object: its kind's list, the namespace filter widened to the
    /// object's if it hid it, and the row with `id` selected (now, if the
    /// list shown is already that one).
    Select { kind: KindRef, id: String, filter: Option<String>, same_list: bool },
}

/// How to open `name` of kind `kind_key` (in `namespace`, which links don't
/// always know applies) from the list showing `current_kind` filtered to
/// `filter`.
pub(crate) fn open_plan(
    kinds: &[KindInfo],
    kind_key: &str,
    namespace: Option<String>,
    name: String,
    current_kind: Option<&KindRef>,
    filter: Option<String>,
) -> OpenPlan {
    let Some(kind) = find_kind(kinds, kind_key) else {
        return OpenPlan::Missing(format!("{kind_key} isn't listed on this cluster"));
    };
    // Links don't always know whether a kind is namespaced (e.g. Helm manifests).
    let namespace = namespace.filter(|_| kind.namespaced);
    if name.is_empty() {
        return OpenPlan::List { kind, namespace };
    }
    let id = match &namespace {
        Some(ns) => format!("{ns}/{name}"),
        None => name,
    };
    // A namespace filter that hides the object is widened to its namespace.
    let filter = match (&namespace, filter) {
        (Some(ns), Some(f)) if *ns != f => Some(ns.clone()),
        (_, f) => f,
    };
    let same_list = current_kind == Some(&kind) && (filter.is_none() || filter == namespace);
    OpenPlan::Select { kind, id, filter, same_list }
}

/// The controller logs the loaded details point to, if they're for `row`.
pub(crate) fn object_logs(details: Option<&Result<ObjectDetails, String>>, row: &ResourceRow) -> Option<LogTarget> {
    match details {
        Some(Ok(d)) if d.name == row.name && d.namespace == row.namespace => d.logs.clone(),
        _ => None,
    }
}

/// The Controller logs button's tooltip.
pub(crate) fn object_logs_title(target: &LogTarget, name: &str) -> String {
    let controller = match &target.source {
        LogSource::Workload { name, .. } => name,
        LogSource::Container { pod, .. } => pod,
    };
    let filter = target.filter.as_deref().unwrap_or(name);
    format!("Open the logs of {controller} in a new window, filtered to {filter}")
}

/// The Logs button's tooltip for a workload.
pub(crate) fn workload_logs_title(kind: &KindRef, name: &str) -> String {
    format!("Open logs from every pod of {} {name} in a new window", kind.kind)
}

/// kubectl-style age from a parsed time (ms): 45s, 12m, 5h, 3d, 2y; empty
/// when it didn't parse.
pub(crate) fn age_since(now_ms: f64, then_ms: f64) -> String {
    if then_ms.is_nan() {
        return String::new();
    }
    format_span(((now_ms - then_ms) / 1000.0).max(0.0) as u64)
}

/// A length of time the way ages are shown: 45s, 12m, 5h, 3d, 2y.
pub(crate) fn format_span(secs: u64) -> String {
    match secs {
        s if s < 120 => format!("{s}s"),
        s if s < 2 * 3600 => format!("{}m", s / 60),
        s if s < 2 * 86400 => format!("{}h", s / 3600),
        s if s < 365 * 86400 => format!("{}d", s / 86400),
        s => format!("{}y", s / (365 * 86400)),
    }
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
