//! The object Details tab's logic: what each section shows and says, how
//! details tables sort, the access check, and editing a ConfigMap's or
//! Secret's data (the draft, its validation and the diff a save sends).

use serde::Serialize;
use subduer_types::{
    AccessCheck, ClusterInfo, Condition, DataChange, DataEntry, EventSummary, InfoRow, InfoSection, InfoTable, InfoTableRow,
    KindRef, NodeResources, ObjectLink,
};

use crate::sort::Key;
use crate::workload::SaveData;
use crate::workload_detail::healthy;

// ---- Loading ----

/// `object_details`' arguments.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct DetailsArgs {
    pub(crate) kind: KindRef,
    pub(crate) namespace: Option<String>,
    pub(crate) name: String,
}

/// Background refreshes pause while the data is being edited.
pub(crate) fn skip_load(quiet: bool, editing: bool) -> bool {
    quiet && editing
}

/// Nodes chart their usage history too.
pub(crate) fn wants_node_history(kind: &KindRef) -> bool {
    kind.key() == "core/Node"
}

/// Whether a finished load replaces what's shown: only for the object still
/// selected, not once editing has started under a background refresh, and a
/// failed background refresh keeps the last good render.
pub(crate) fn keep_result(quiet: bool, current: bool, editing: bool, failed: bool, have_good: bool) -> bool {
    current && !(quiet && editing) && !(quiet && failed && have_good)
}

// ---- Sections ----

/// The kind's section id.
pub(crate) fn kind_section_id(kind: &str) -> String {
    kind.to_lowercase()
}

/// How the Events section reads: a namespace's lists its recent warnings.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EventsSection {
    pub(crate) id: &'static str,
    pub(crate) title: &'static str,
    pub(crate) empty: &'static str,
    pub(crate) summary: String,
}

/// The Events section, for namespaced objects, namespaces and nodes (whose
/// events are found in any namespace).
pub(crate) fn events_section(kind: &str, namespace: Option<&str>, events: &[EventSummary]) -> Option<EventsSection> {
    let in_namespace = kind == "Namespace";
    if !(namespace.is_some() || in_namespace || kind == "Node") {
        return None;
    }
    let warnings = events.iter().filter(|e| e.type_ == "Warning").count();
    let summary = match (events.len(), warnings) {
        (0, _) => "none".to_owned(),
        (n, 0) => n.to_string(),
        (n, w) => format!("{n} · ⚠ {w}"),
    };
    Some(if in_namespace {
        EventsSection { id: "ns-warnings", title: "Recent warnings", empty: "No warnings in it.", summary }
    } else {
        EventsSection { id: "events", title: "Events", empty: "No events for it.", summary }
    })
}

/// ServiceAccounts get the Check access section.
pub(crate) fn checks_access(kind: &str) -> bool {
    kind == "ServiceAccount"
}

/// A node's Resources summary: its pod count, or the one-pod-per-node
/// hosting that makes the count meaningless.
pub(crate) fn node_summary(n: &NodeResources) -> String {
    match &n.one_pod_hosting {
        None => format!("{} of {} pods", n.pods, n.pod_capacity),
        Some(h) => h.clone(),
    }
}

/// Why a one-pod-per-node node's pod count isn't shown.
pub(crate) fn hosting_note(hosting: &str) -> String {
    format!("Pod count not shown: {hosting} runs one pod per node, so the node is always full by design.")
}

/// The pods meter's value text.
pub(crate) fn fmt_count(v: f64) -> String {
    format!("{v:.0}")
}

/// `5m ago · 2026-…`.
pub(crate) fn ago_text(age: &str, time: &str) -> String {
    format!("{age} ago · {time}")
}

/// A label chip.
pub(crate) fn label_chip(key: &str, value: &str) -> String {
    format!("{key}={value}")
}

/// "3", or "3 · 1 unhealthy".
pub(crate) fn conditions_summary(conditions: &[Condition]) -> String {
    match (conditions.len(), conditions.iter().filter(|c| !healthy(c)).count()) {
        (n, 0) => n.to_string(),
        (n, u) => format!("{n} · {u} unhealthy"),
    }
}

// ---- Info rows and tables ----

/// How an info row's value is shown.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RowValue {
    /// A time, shown relative to now beside the time itself.
    Time,
    /// Several lines.
    Block,
    Url(String),
    Link(ObjectLink),
    Plain,
}

/// `parses`: whether the value reads as a time.
pub(crate) fn row_value(r: &InfoRow, parses: bool) -> RowValue {
    if r.time && parses {
        RowValue::Time
    } else if r.value.contains('\n') {
        RowValue::Block
    } else if let Some(url) = &r.url {
        RowValue::Url(url.clone())
    } else if let Some(link) = &r.link {
        RowValue::Link(link.clone())
    } else {
        RowValue::Plain
    }
}

/// `5m ago · 2026-…` or `in 3h · 2026-…`.
pub(crate) fn time_text(relative: &str, value: &str) -> String {
    format!("{relative} · {value}")
}

/// A link's tooltip: `Open Deployment web`.
pub(crate) fn link_title(link: &ObjectLink) -> String {
    format!("Open {} {}", link.kind.rsplit('/').next().unwrap_or_default(), link.name)
}

/// A section with neither rows nor table rows shows its empty note.
pub(crate) fn section_empty(s: &InfoSection) -> bool {
    s.rows.is_empty() && s.table.as_ref().is_none_or(|t| t.rows.is_empty())
}

/// The heading of the bar column (quota used of hard, disk used, …).
pub(crate) const FILL_HEADER: &str = "Used";

/// Sorts are kept per section, so a section's order survives refreshes.
pub(crate) fn sort_table(section: &str) -> String {
    format!("detail:{section}")
}

/// Whether any row has a bar.
pub(crate) fn has_fill(t: &InfoTable) -> bool {
    t.rows.iter().any(|r| r.fill.is_some())
}

/// The sortable headings: the columns, then the bar's.
pub(crate) fn sort_headers(t: &InfoTable) -> Vec<String> {
    let mut headers = t.columns.clone();
    headers.push(FILL_HEADER.to_owned());
    headers
}

/// The class of column `i`'s heading.
pub(crate) fn head_class(t: &InfoTable, i: usize) -> &'static str {
    if t.numeric.contains(&i) { "num" } else { "" }
}

/// Whether cell `i` shows as an age; `parses`: whether it reads as a time.
pub(crate) fn time_cell(t: &InfoTable, i: usize, parses: bool) -> bool {
    t.times.contains(&i) && parses
}

/// A bar: its percentage, whether it warns or is full, its tooltip and style.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Fill {
    pub(crate) warning: bool,
    pub(crate) critical: bool,
    pub(crate) title: String,
    pub(crate) style: String,
}

pub(crate) fn fill(r: &InfoTableRow) -> Option<Fill> {
    let p = (r.fill? * 100.0).round();
    Some(Fill { warning: r.warn && p < 100.0, critical: p >= 100.0, title: format!("{p}% used"), style: format!("width:{p}%") })
}

/// A linked row's tooltip.
pub(crate) fn row_title(r: &InfoTableRow) -> Option<String> {
    r.link.as_ref().map(|l| format!("Open {}", l.name))
}

/// How a cell of a details table sorts: times as ages, numeric columns by
/// value, the bar by its share, the rest as text.
pub(crate) fn info_key(t: &InfoTable, r: &InfoTableRow, i: usize) -> Key {
    if i == t.columns.len() {
        return Key::opt(r.fill);
    }
    let cell = r.cells.get(i).map_or("", String::as_str);
    if t.times.contains(&i) {
        Key::age(cell)
    } else if t.numeric.contains(&i) {
        Key::quantity(cell)
    } else {
        Key::text(cell)
    }
}

// ---- Check access ----

pub(crate) const VERBS: [&str; 9] = ["get", "list", "watch", "create", "update", "patch", "delete", "deletecollection", "*"];

/// `check_access`' arguments.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct AccessArgs {
    pub(crate) account_namespace: String,
    pub(crate) account: String,
    pub(crate) verb: String,
    pub(crate) resource: String,
    pub(crate) namespace: String,
    pub(crate) name: String,
}

/// A check needs a resource.
pub(crate) fn can_ask(resource: &str) -> bool {
    !resource.trim().is_empty()
}

/// The answer's badge class, word and reason.
pub(crate) fn access_answer(c: &AccessCheck) -> (&'static str, &'static str, String) {
    let (class, text) = if c.allowed { ("badge badge-ready", "Allowed") } else { ("badge badge-warning", "Denied") };
    let reason = if c.reason.is_empty() && !c.allowed { "No rule allows it.".to_owned() } else { c.reason.clone() };
    (class, text, reason)
}

// ---- Data: view, edit, save ----

/// `open_url`' arguments.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct UrlArgs {
    pub(crate) url: String,
}

/// `secret_values`' arguments.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SecretArgs {
    pub(crate) namespace: String,
    pub(crate) name: String,
}

/// One entry of the draft while editing.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Draft {
    pub(crate) id: u32,
    pub(crate) key: String,
    pub(crate) value: String,
    pub(crate) binary: bool,
    pub(crate) size: usize,
}

impl Draft {
    /// A new, empty key.
    pub(crate) fn new(id: u32) -> Self {
        Self { id, key: String::new(), value: String::new(), binary: false, size: 0 }
    }
}

/// Key names the API accepts in ConfigMaps and Secrets.
pub(crate) fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 253
        && key != "."
        && key != ".."
        && key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// A key typed so far that the API won't take (empty is just unfinished).
pub(crate) fn bad_key(key: &str) -> bool {
    !key.is_empty() && !valid_key(key)
}

#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct SaveSummary {
    pub(crate) added: Vec<String>,
    pub(crate) changed: Vec<String>,
    pub(crate) removed: Vec<String>,
}

/// What a save would change (`None`: nothing yet), or why it can't be saved.
pub(crate) type Pending = Result<Option<(Vec<DataChange>, SaveSummary)>, String>;

pub(crate) fn diff(original: &[(String, Option<String>)], draft: &[Draft]) -> Pending {
    let mut seen = std::collections::HashSet::new();
    for d in draft {
        if !valid_key(&d.key) {
            return Err(if d.key.is_empty() {
                "Every key needs a name.".to_owned()
            } else {
                format!("“{}” isn't a valid key: use letters, digits, '-', '_' and '.'.", d.key)
            });
        }
        if !seen.insert(d.key.as_str()) {
            return Err(format!("“{}” is used twice.", d.key));
        }
    }
    let mut changes = Vec::new();
    let mut summary = SaveSummary::default();
    for (key, _) in original {
        if !seen.contains(key.as_str()) {
            changes.push(DataChange { key: key.clone(), value: None });
            summary.removed.push(key.clone());
        }
    }
    for d in draft.iter().filter(|d| !d.binary) {
        match original.iter().find(|(k, _)| *k == d.key) {
            Some((_, Some(old))) if *old == d.value => {}
            Some(_) => {
                changes.push(DataChange { key: d.key.clone(), value: Some(d.value.clone()) });
                summary.changed.push(d.key.clone());
            }
            None => {
                changes.push(DataChange { key: d.key.clone(), value: Some(d.value.clone()) });
                summary.added.push(d.key.clone());
            }
        }
    }
    Ok((!changes.is_empty()).then_some((changes, summary)))
}

/// A ConfigMap's text values as the edit's starting point.
pub(crate) fn data_values(entries: &[DataEntry]) -> Vec<(String, Option<String>)> {
    entries.iter().map(|e| (e.key.clone(), e.value.clone())).collect()
}

/// The draft for `entries` given their `values`; a key without a text value
/// is binary and kept as is.
pub(crate) fn drafts(entries: &[DataEntry], values: &[(String, Option<String>)]) -> Vec<Draft> {
    entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let value = values.iter().find(|(k, _)| *k == e.key).and_then(|(_, v)| v.clone());
            Draft { id: i as u32, key: e.key.clone(), binary: value.is_none(), value: value.unwrap_or_default(), size: e.size }
        })
        .collect()
}

/// A draft entry's field, or empty once it's gone.
pub(crate) fn draft_field(draft: &[Draft], id: u32, pick: fn(&Draft) -> String) -> String {
    draft.iter().find(|x| x.id == id).map(pick).unwrap_or_default()
}

/// Changes the draft entry `id`, if it's still there.
pub(crate) fn update_draft(draft: &mut [Draft], id: u32, update: impl Fn(&mut Draft)) {
    if let Some(x) = draft.iter_mut().find(|x| x.id == id) {
        update(x);
    }
}

/// Drops the draft entry `id`.
pub(crate) fn remove_draft(draft: &mut Vec<Draft>, id: u32) {
    draft.retain(|x| x.id != id);
}

/// A value's text area grows with it: 2 to 16 rows.
pub(crate) fn value_rows(value: &str) -> String {
    (value.lines().count() + 1).clamp(2, 16).to_string()
}

pub(crate) fn size_text(bytes: usize) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{:.1} KiB", b as f64 / 1024.0),
        b => format!("{:.1} MiB", b as f64 / 1024.0 / 1024.0),
    }
}

/// An entry's size line: `12 B`, or `3.0 KiB · binary`.
pub(crate) fn size_line(size: usize, binary: bool) -> String {
    format!("{}{}", size_text(size), if binary { " · binary" } else { "" })
}

/// A binary entry while editing.
pub(crate) fn binary_note(size: usize) -> String {
    format!("Binary value, {}: kept as is.", size_text(size))
}

/// Whether the API allows patching `kind` on the cluster.
pub(crate) fn can_patch(kind: Option<&KindRef>, cluster: Option<&ClusterInfo>) -> bool {
    kind.is_some_and(|k| {
        cluster.iter().flat_map(|c| &c.kinds).any(|i| i.kind == *k && i.verbs.iter().any(|v| v == "patch"))
    })
}

/// Whether the Edit button shows.
pub(crate) fn editable(read_only: Option<&str>, can_patch: bool) -> bool {
    read_only.is_none() && can_patch
}

/// Whether to say the user's access doesn't allow changes.
pub(crate) fn no_access(read_only: Option<&str>, can_patch: bool) -> bool {
    !can_patch && read_only.is_none()
}

/// The Edit button's label: a Secret's values show once editing starts.
pub(crate) fn edit_label(secret: bool) -> &'static str {
    if secret { "Edit (shows values)" } else { "Edit" }
}

/// What the unsaved-edits guard names: `Secret db-creds`.
pub(crate) fn dirty_label(secret: bool, name: &str) -> String {
    format!("{} {name}", if secret { "Secret" } else { "ConfigMap" })
}

/// Anything typed counts as unsaved, including a draft that can't be saved yet.
pub(crate) fn dirty(editing: bool, pending: &Pending, label: &str) -> Option<String> {
    (editing && !matches!(pending, Ok(None))).then(|| label.to_owned())
}

/// Whether Save… is enabled.
pub(crate) fn can_save(pending: &Pending) -> bool {
    matches!(pending, Ok(Some(_)))
}

/// The save request for the pending changes, if there are any to save.
pub(crate) fn save_request(kind: Option<KindRef>, pending: Pending, resource_version: &str) -> Option<(KindRef, SaveData)> {
    let (Some(kind), Ok(Some((changes, summary)))) = (kind, pending) else { return None };
    let save = SaveData {
        resource_version: resource_version.to_owned(),
        changes,
        added: summary.added,
        changed: summary.changed,
        removed: summary.removed,
    };
    Some((kind, save))
}

/// The Data section's summary: `empty`, `1 key`, `3 keys`.
pub(crate) fn data_summary(count: usize) -> String {
    if count == 0 { "empty".to_owned() } else { format!("{count} {}", if count == 1 { "key" } else { "keys" }) }
}

/// Under the editor: why it can't be saved, or how much would be.
pub(crate) fn pending_text(pending: &Pending) -> Result<String, String> {
    match pending {
        Err(e) => Err(e.clone()),
        Ok(None) => Ok("No changes yet.".to_owned()),
        Ok(Some((_, s))) => {
            let n = s.added.len() + s.changed.len() + s.removed.len();
            Ok(format!("{n} {} to save.", if n == 1 { "change" } else { "changes" }))
        }
    }
}

#[cfg(test)]
#[path = "object_detail_tests.rs"]
mod tests;
