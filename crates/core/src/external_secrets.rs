//! The Details tab of External Secrets Operator kinds (`external-secrets.io`)
//! and its generators (`generators.external-secrets.io`): what each one
//! syncs, from where and how often, its stores and the objects using them,
//! and the operator's controller, whose logs explain failures. Built on
//! `config`'s generic shape. Also Force sync, the `force-sync` annotation.
//!
//! Nothing here reads secret values: ExternalSecrets and stores only name
//! remote keys and references, the target Secret is read by metadata only,
//! and literal values in generators (a Fake's `data`) and generator state are
//! shown as key names.

use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::Secret;
use kube::api::{Api, ListParams, Patch, PatchParams};
use kube::core::DynamicObject;
use kube::Client;
use serde_json::{Value, json};

use crate::cluster::dynamic_api;
use crate::config::{label_selector, plural, str_at, table_section};
use crate::safety::SafetyGate;
use crate::types::{
    InfoRow, InfoSection, InfoTable, InfoTableRow, KindRef, LogSource, LogTarget, ObjectDetails, ObjectLink,
    can_force_sync,
};
use crate::{Error, Result};

pub(crate) const GROUP: &str = "external-secrets.io";
pub(crate) const GENERATORS: &str = "generators.external-secrets.io";
/// The annotation ExternalSecrets refresh on when it changes; a
/// ClusterExternalSecret copies it to its ExternalSecrets.
pub const FORCE_SYNC: &str = "force-sync";
/// The controller Deployment's label in the official Helm chart (its webhook
/// and cert controller have their own names).
const CONTROLLER_LABEL: &str = "app.kubernetes.io/name=external-secrets";

/// Fills in what comes from the object itself.
pub(crate) fn describe(key: &str, d: &mut ObjectDetails, v: &Value) {
    match key {
        "external-secrets.io/ExternalSecret" => external_secret(d, v),
        "external-secrets.io/ClusterExternalSecret" => cluster_external_secret(d, v),
        "external-secrets.io/SecretStore" | "external-secrets.io/ClusterSecretStore" => store(d, v),
        "external-secrets.io/PushSecret" => push_secret(d, v),
        "external-secrets.io/ClusterPushSecret" => cluster_push_secret(d, v),
        "generators.external-secrets.io/ClusterGenerator" => cluster_generator(d, v),
        "generators.external-secrets.io/GeneratorState" => generator_state(d, v),
        k if k.starts_with("generators.external-secrets.io/") => generator(d, &v["spec"], is_literal_data(k)),
        k if k.starts_with("external-secrets.io/") => ready_status(d, v, "Ready", "Not ready"),
        _ => {}
    }
}

/// Adds what needs other objects: the target Secret, the objects a store or
/// generator serves, a cluster kind's per-namespace objects, the controller
/// (for Logs) and, for cluster-scoped kinds, events from any namespace.
pub(crate) async fn related(
    client: &Client,
    kind: &KindRef,
    namespace: &str,
    name: &str,
    v: &Value,
    d: &mut ObjectDetails,
) -> Result<()> {
    match kind.key().as_str() {
        "external-secrets.io/ExternalSecret" => target_secret(client, namespace, name, v, d).await,
        "external-secrets.io/ClusterExternalSecret" => {
            let items = list_or_note(external_secrets(client, None).await, "ExternalSecrets", d);
            d.sections.push(provisioned(v, &items, "ClusterExternalSecret", name, "ExternalSecret"));
        }
        "external-secrets.io/SecretStore" | "external-secrets.io/ClusterSecretStore" => {
            let cluster = kind.kind == "ClusterSecretStore";
            let items = list_or_note(external_secrets(client, (!cluster).then_some(namespace)).await, "ExternalSecrets", d);
            d.sections.push(store_users(&items, &kind.kind, name));
        }
        "external-secrets.io/ClusterPushSecret" => {
            let items = list_or_note(push_secrets(client, None).await, "PushSecrets", d);
            d.sections.push(provisioned(v, &items, "ClusterPushSecret", name, "PushSecret"));
        }
        key if key.starts_with("generators.external-secrets.io/") && kind.kind != "GeneratorState" => {
            let ns = (kind.namespaced).then_some(namespace);
            let items = list_or_note(external_secrets(client, ns).await, "ExternalSecrets", d);
            d.sections.push(generator_users(&items, &kind.kind, name));
        }
        _ => {}
    }
    controller(client, CONTROLLER_LABEL, name, d).await;
    if !kind.namespaced {
        let fields = format!("involvedObject.kind={},involvedObject.name={name}", kind.kind);
        d.events = crate::pod_details::list_events(client, "", Some(&fields), |_| Some(None)).await;
    }
    Ok(())
}

// ---- ExternalSecret ----

fn external_secret(d: &mut ObjectDetails, v: &Value) {
    let (spec, status) = (&v["spec"], &v["status"]);
    d.status = Some(sync_status(v));
    store_row(d, &spec["secretStoreRef"]);
    let target = str_at(&spec["target"], "name").unwrap_or_else(|| d.name.clone());
    d.info.push(InfoRow::new("Target secret", target.clone()).mono().link(link("core/Secret", d.namespace.as_deref(), &target)));
    target_rows(d, &spec["target"]);
    refresh_rows(d, spec, status, v);
    d.sections.push(data_section(&spec["data"], d.namespace.as_deref()));
    if let Some(s) = data_from_section(&spec["dataFrom"], d.namespace.as_deref()) {
        d.sections.push(s);
    }
}

/// `Synced`, `Sync failed`, `Secret missing`, … from the Ready condition.
fn sync_status(v: &Value) -> String {
    let Some(ready) = condition(v, "Ready") else { return "Pending".into() };
    match (str_at(ready, "status").as_deref(), str_at(ready, "reason").as_deref()) {
        (Some("True"), _) => "Synced".into(),
        (_, Some("SecretSyncedError" | "ResourceSyncedError")) => "Sync failed".into(),
        (_, Some("SecretDeleted" | "ResourceDeleted")) => "Deleted".into(),
        (_, Some("SecretMissing" | "ResourceMissing")) => "Secret missing".into(),
        (_, Some("SecretOwnedByOther")) => "Owned by other".into(),
        (Some("Unknown"), _) => "Pending".into(),
        _ => "Not ready".into(),
    }
}

fn condition<'a>(v: &'a Value, type_: &str) -> Option<&'a Value> {
    v["status"]["conditions"].as_array()?.iter().find(|c| c["type"] == type_)
}

/// The badge from the Ready condition, for kinds with nothing more specific.
fn ready_status(d: &mut ObjectDetails, v: &Value, ready: &str, not_ready: &str) {
    d.status = Some(match condition(v, "Ready").and_then(|c| str_at(c, "status")).as_deref() {
        Some("True") => ready.into(),
        Some("False") => not_ready.into(),
        _ => "Pending".into(),
    });
}

fn store_row(d: &mut ObjectDetails, store_ref: &Value) {
    let Some(name) = str_at(store_ref, "name") else { return };
    let kind = str_at(store_ref, "kind").unwrap_or_else(|| "SecretStore".into());
    let ns = if kind == "ClusterSecretStore" { None } else { d.namespace.clone() };
    let link = ObjectLink { kind: format!("{GROUP}/{kind}"), namespace: ns, name: name.clone() };
    d.info.push(InfoRow::new("Store", format!("{kind}/{name}")).link(link));
}

fn target_rows(d: &mut ObjectDetails, target: &Value) {
    let creation = str_at(target, "creationPolicy").unwrap_or_else(|| "Owner".into());
    let creation_text = match creation.as_str() {
        "Owner" => "Owner: creates the Secret and owns it (deleted with this)",
        "Orphan" => "Orphan: creates the Secret but doesn't own it (kept when this is deleted)",
        "Merge" => "Merge: adds its keys to a Secret that must already exist",
        "None" => "None: doesn't write a Secret",
        _ => "",
    };
    d.info.push(InfoRow::new("Creation", if creation_text.is_empty() { creation } else { creation_text.into() }));
    if let Some(deletion) = str_at(target, "deletionPolicy") {
        let text = match deletion.as_str() {
            "Retain" => "Retain: the Secret is kept when the remote secret is gone",
            "Delete" => "Delete: the Secret is deleted when the remote secret is gone",
            "Merge" => "Merge: only its keys are removed when the remote secret is gone",
            _ => "",
        };
        d.info.push(InfoRow::new("Deletion", if text.is_empty() { deletion } else { text.into() }));
    }
    if target["immutable"] == true {
        d.info.push(InfoRow::new("Immutable", "yes: the Secret is written once"));
    }
    let template = &target["template"];
    if template.is_object() {
        let keys = template["data"].as_object().map_or(0, |m| m.len());
        let from = template["templateFrom"].as_array().map_or(0, Vec::len);
        let mut text = format!("engine {}", str_at(template, "engineVersion").unwrap_or_else(|| "v2".into()));
        if let Some(t) = str_at(template, "type") {
            text.push_str(&format!(" · type {t}"));
        }
        if keys > 0 {
            text.push_str(&format!(" · {}", plural(keys, "key")));
        }
        if from > 0 {
            text.push_str(&format!(" · {from} from ConfigMaps/Secrets"));
        }
        d.info.push(InfoRow::new("Template", text));
    }
}

/// Refresh policy and interval, last refresh, the next one, and a pending
/// Force sync.
fn refresh_rows(d: &mut ObjectDetails, spec: &Value, status: &Value, v: &Value) {
    let policy = str_at(spec, "refreshPolicy");
    let interval = str_at(spec, "refreshInterval").unwrap_or_else(|| "1h".into());
    let seconds = go_duration(&interval);
    match policy.as_deref() {
        Some("CreatedOnce") => d.info.push(InfoRow::new("Refresh policy", "CreatedOnce: fetched once, never refreshed")),
        Some("OnChange") => d.info.push(InfoRow::new("Refresh policy", "OnChange: refreshed only when this object changes")),
        Some(p) => d.info.push(InfoRow::new("Refresh policy", p.to_owned())),
        None => {}
    }
    let periodic = matches!(policy.as_deref(), None | Some("Periodic"));
    if periodic {
        let text = match seconds {
            Some(0) => format!("{interval}: fetched once, then never refreshed"),
            _ => interval.clone(),
        };
        d.info.push(InfoRow::new("Refresh interval", text));
    }
    let last = str_at(status, "refreshTime");
    if let Some(t) = &last {
        d.info.push(InfoRow::new("Last refresh", t.clone()).time());
    }
    if periodic
        && let (Some(t), Some(s)) = (&last, seconds.filter(|s| *s > 0))
        && let Ok(at) = t.parse::<jiff::Timestamp>()
        && let Ok(next) = at.checked_add(jiff::SignedDuration::from_secs(s))
    {
        d.info.push(InfoRow::new("Next refresh", next.to_string()).time());
    }
    forced_row(d, v);
}

/// When Force sync was last asked for: the `force-sync` annotation, which is
/// usually Unix seconds.
fn forced_row(d: &mut ObjectDetails, v: &Value) {
    let Some(forced) = str_at(&v["metadata"]["annotations"], FORCE_SYNC) else { return };
    match forced.parse::<i64>().ok().and_then(|s| jiff::Timestamp::from_second(s).ok()) {
        Some(t) => d.info.push(InfoRow::new("Forced sync", t.to_string()).time()),
        None => d.info.push(InfoRow::new("Forced sync", forced).mono()),
    }
}

/// A Go duration (`1h`, `1h30m`, `15m0s`, `500ms`, `0`) in whole seconds.
pub(crate) fn go_duration(s: &str) -> Option<i64> {
    let s = s.trim();
    if s == "0" {
        return Some(0);
    }
    let (mut total, mut rest) = (0f64, s);
    if rest.is_empty() {
        return None;
    }
    while !rest.is_empty() {
        let digits = rest.find(|c: char| !(c.is_ascii_digit() || c == '.')).filter(|n| *n > 0)?;
        let n: f64 = rest[..digits].parse().ok()?;
        rest = &rest[digits..];
        let unit = rest.find(|c: char| c.is_ascii_digit() || c == '.').unwrap_or(rest.len());
        let scale = match &rest[..unit] {
            "h" => 3600.0,
            "m" => 60.0,
            "s" => 1.0,
            "ms" => 0.001,
            "us" | "µs" => 0.000_001,
            "ns" => 0.000_000_001,
            _ => return None,
        };
        total += n * scale;
        rest = &rest[unit..];
    }
    Some(total as i64)
}

fn data_section(data: &Value, namespace: Option<&str>) -> InfoSection {
    let rows: Vec<InfoTableRow> = data
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| {
            let r = &e["remoteRef"];
            let mut row = InfoTableRow::new(vec![
                str_at(e, "secretKey").unwrap_or_default(),
                str_at(r, "key").unwrap_or_default(),
                str_at(r, "property").unwrap_or_default(),
                str_at(r, "version").unwrap_or_default(),
                source_text(&e["sourceRef"]),
            ]);
            row.link = source_link(&e["sourceRef"], namespace);
            row
        })
        .collect();
    let table = drop_empty_columns(
        InfoTable {
            columns: ["Secret key", "Remote key", "Property", "Version", "From"].map(String::from).to_vec(),
            mono: vec![0, 1, 2],
            rows,
            ..Default::default()
        },
        &[2, 3, 4],
    );
    table_section("es-data", "Data", table, "No single keys: everything comes from Data from.")
}

fn data_from_section(data_from: &Value, namespace: Option<&str>) -> Option<InfoSection> {
    let items = data_from.as_array().filter(|a| !a.is_empty())?;
    let rows = items
        .iter()
        .map(|e| {
            let (how, what) = if let Some(key) = str_at(&e["extract"], "key") {
                let property = str_at(&e["extract"], "property").map(|p| format!(" · {p}")).unwrap_or_default();
                ("Extract", format!("{key}{property}"))
            } else if e["find"].is_object() {
                let f = &e["find"];
                let mut parts = Vec::new();
                if let Some(p) = str_at(f, "path") {
                    parts.push(format!("path {p}"));
                }
                if let Some(r) = str_at(&f["name"], "regexp") {
                    parts.push(format!("name ~ {r}"));
                }
                for (k, t) in f["tags"].as_object().into_iter().flatten() {
                    parts.push(format!("tag {k}={}", t.as_str().unwrap_or_default()));
                }
                ("Find", parts.join(", "))
            } else if e["sourceRef"]["generatorRef"].is_object() {
                ("Generate", String::new())
            } else {
                ("", String::new())
            };
            let rewrites = e["rewrite"].as_array().map_or(0, Vec::len);
            let mut row = InfoTableRow::new(vec![
                how.into(),
                what,
                if rewrites > 0 { plural(rewrites, "rewrite") } else { String::new() },
                source_text(&e["sourceRef"]),
            ]);
            row.link = source_link(&e["sourceRef"], namespace);
            row
        })
        .collect();
    let table = InfoTable { columns: ["How", "What", "Rewrite", "From"].map(String::from).to_vec(), mono: vec![1], rows, ..Default::default() };
    let table = drop_empty_columns(table, &[2, 3]);
    Some(table_section("es-data-from", "Data from", table, ""))
}

/// Leaves out the `optional` columns no row fills (Version, From, …), keeping
/// the mono indexes in step. Tables without rows keep every column.
pub(crate) fn drop_empty_columns(mut table: InfoTable, optional: &[usize]) -> InfoTable {
    if table.rows.is_empty() {
        return table;
    }
    let drop: Vec<usize> = optional
        .iter()
        .copied()
        .filter(|&c| table.rows.iter().all(|r| r.cells.get(c).is_none_or(String::is_empty)))
        .collect();
    let keep = |c: usize| !drop.contains(&c);
    let shift = |c: usize| c - drop.iter().filter(|&&d| d < c).count();
    table.columns = table.columns.into_iter().enumerate().filter(|(i, _)| keep(*i)).map(|(_, c)| c).collect();
    for row in &mut table.rows {
        row.cells = std::mem::take(&mut row.cells).into_iter().enumerate().filter(|(i, _)| keep(*i)).map(|(_, c)| c).collect();
    }
    for cols in [&mut table.mono, &mut table.numeric, &mut table.times] {
        *cols = cols.iter().copied().filter(|&c| keep(c)).map(shift).collect();
    }
    table
}

/// A data entry's own store or generator, overriding the ExternalSecret's store.
fn source_text(source: &Value) -> String {
    if let Some(name) = str_at(&source["storeRef"], "name") {
        return format!("{}/{name}", str_at(&source["storeRef"], "kind").unwrap_or_else(|| "SecretStore".into()));
    }
    if let Some(name) = str_at(&source["generatorRef"], "name") {
        return format!("{}/{name}", str_at(&source["generatorRef"], "kind").unwrap_or_default());
    }
    String::new()
}

fn source_link(source: &Value, namespace: Option<&str>) -> Option<ObjectLink> {
    if let Some(name) = str_at(&source["storeRef"], "name") {
        let kind = str_at(&source["storeRef"], "kind").unwrap_or_else(|| "SecretStore".into());
        let ns = (kind != "ClusterSecretStore").then(|| namespace.map(str::to_owned)).flatten();
        return Some(ObjectLink { kind: format!("{GROUP}/{kind}"), namespace: ns, name });
    }
    let g = &source["generatorRef"];
    let name = str_at(g, "name")?;
    let kind = str_at(g, "kind")?;
    let ns = (kind != "ClusterGenerator").then(|| namespace.map(str::to_owned)).flatten();
    Some(ObjectLink { kind: format!("{GENERATORS}/{kind}"), namespace: ns, name })
}

/// Whether the target Secret exists (metadata only: no data is read).
async fn target_secret(client: &Client, namespace: &str, name: &str, v: &Value, d: &mut ObjectDetails) {
    if str_at(&v["spec"]["target"], "creationPolicy").as_deref() == Some("None") {
        return;
    }
    let target = str_at(&v["spec"]["target"], "name").unwrap_or_else(|| name.to_owned());
    let found = Api::<Secret>::namespaced(client.clone(), namespace).get_metadata_opt(&target).await;
    if let Ok(None) = found
        && let Some(row) = d.info.iter_mut().find(|r| r.key == "Target secret")
    {
        row.value = format!("{target} (missing)");
        row.link = None;
    }
}

// ---- ClusterExternalSecret and ClusterPushSecret ----

fn cluster_external_secret(d: &mut ObjectDetails, v: &Value) {
    let spec = &v["spec"];
    let es_name = str_at(spec, "externalSecretName").unwrap_or_else(|| d.name.clone());
    d.info.push(InfoRow::new("ExternalSecret name", es_name).mono());
    namespaces_rows(d, spec);
    let es = &spec["externalSecretSpec"];
    store_row(d, &es["secretStoreRef"]);
    let target = str_at(&es["target"], "name").unwrap_or_else(|| str_at(spec, "externalSecretName").unwrap_or_else(|| d.name.clone()));
    d.info.push(InfoRow::new("Target secret", target).mono());
    target_rows(d, &es["target"]);
    let es_interval = str_at(es, "refreshInterval").unwrap_or_else(|| "1h".into());
    d.info.push(InfoRow::new("Refresh interval", format!("{es_interval} (each ExternalSecret)")));
    if let Some(t) = str_at(spec, "refreshTime").or_else(|| str_at(spec, "refreshInterval")) {
        d.info.push(InfoRow::new("Namespaces checked", format!("every {t}")));
    }
    forced_row(d, v);
    d.status = Some(cluster_status(v));
    d.sections.push(data_section(&es["data"], None));
    if let Some(s) = data_from_section(&es["dataFrom"], None) {
        d.sections.push(s);
    }
}

fn cluster_push_secret(d: &mut ObjectDetails, v: &Value) {
    let spec = &v["spec"];
    let ps_name = str_at(spec, "pushSecretName").unwrap_or_else(|| d.name.clone());
    d.info.push(InfoRow::new("PushSecret name", ps_name).mono());
    namespaces_rows(d, spec);
    push_rows(d, &spec["pushSecretSpec"], None);
    if let Some(t) = str_at(spec, "refreshTime").or_else(|| str_at(spec, "refreshInterval")) {
        d.info.push(InfoRow::new("Namespaces checked", format!("every {t}")));
    }
    d.status = Some(cluster_status(v));
}

/// Which namespaces a cluster kind writes into, in words.
fn namespaces_rows(d: &mut ObjectDetails, spec: &Value) {
    let mut parts: Vec<String> = Vec::new();
    let names: Vec<String> = strings(&spec["namespaces"]);
    if !names.is_empty() {
        parts.push(names.join(", "));
    }
    let selectors = spec["namespaceSelectors"].as_array().cloned().unwrap_or_default();
    for s in selectors.iter().chain(spec["namespaceSelector"].is_object().then_some(&spec["namespaceSelector"])) {
        parts.push(match label_selector(s) {
            Some(sel) => format!("labelled {sel}"),
            None => "every namespace".into(),
        });
    }
    let text = if parts.is_empty() { "none selected".to_owned() } else { parts.join("; ") };
    d.info.push(InfoRow::new("Namespaces", text).mono());
}

/// `Ready`, `Partly failed`, `Failed` or `Not ready`.
fn cluster_status(v: &Value) -> String {
    let failed = v["status"]["failedNamespaces"].as_array().map_or(0, Vec::len);
    let provisioned = v["status"]["provisionedNamespaces"].as_array().map_or(0, Vec::len);
    match (failed, provisioned) {
        (0, _) => match condition(v, "Ready").and_then(|c| str_at(c, "status")).as_deref() {
            Some("True") => "Ready".into(),
            Some(_) => "Not ready".into(),
            None => "Pending".into(),
        },
        (_, 0) => "Failed".into(),
        _ => "Partly failed".into(),
    }
}

/// One row per namespace: its object (owned by `owner`), that object's
/// status, and the namespaces that failed with why.
fn provisioned(v: &Value, items: &[Value], owner_kind: &str, owner: &str, child_kind: &str) -> InfoSection {
    let mut rows: Vec<InfoTableRow> = items
        .iter()
        .filter(|o| {
            o["metadata"]["ownerReferences"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|r| r["kind"] == owner_kind && r["name"] == owner)
        })
        .map(|o| {
            let (ns, name) = (str_at(&o["metadata"], "namespace").unwrap_or_default(), str_at(&o["metadata"], "name").unwrap_or_default());
            let status = sync_status(o);
            let mut row = InfoTableRow::new(vec![ns.clone(), name.clone(), status.clone(), str_at(&o["status"], "refreshTime").unwrap_or_default()]);
            row.link = Some(link(&format!("{GROUP}/{child_kind}"), Some(&ns), &name));
            row.warn = status != "Synced";
            row
        })
        .collect();
    for f in v["status"]["failedNamespaces"].as_array().into_iter().flatten() {
        let mut row = InfoTableRow::new(vec![
            str_at(f, "namespace").unwrap_or_default(),
            String::new(),
            format!("Failed: {}", str_at(f, "reason").unwrap_or_default()),
            String::new(),
        ]);
        row.warn = true;
        rows.push(row);
    }
    rows.sort_by(|a, b| a.cells.cmp(&b.cells));
    let table = InfoTable {
        columns: vec!["Namespace".into(), child_kind.into(), "Status".into(), "Last refresh".into()],
        mono: vec![1],
        times: vec![3],
        rows,
        ..Default::default()
    };
    table_section(&format!("{}-children", child_kind.to_lowercase()), &format!("{child_kind}s"), table, "None made yet: no namespace matches.")
}

// ---- SecretStore and ClusterSecretStore ----

fn store(d: &mut ObjectDetails, v: &Value) {
    let (spec, status) = (&v["spec"], &v["status"]);
    d.status = Some(match condition(v, "Ready") {
        Some(c) if c["status"] == "True" => "Valid".into(),
        Some(c) if str_at(c, "reason").as_deref() == Some("InvalidProviderConfig") || c["status"] == "False" => "Invalid".into(),
        _ => "Unknown".into(),
    });
    let (provider, config) = spec["provider"]
        .as_object()
        .and_then(|m| m.iter().next())
        .map(|(k, v)| (k.clone(), v.clone()))
        .unwrap_or_default();
    let mut text = provider_name(&provider, &config);
    if let Some(region) = str_at(&config, "region") {
        text.push_str(&format!(" ({region})"));
    }
    d.info.push(InfoRow::new("Provider", text));
    if let Some(c) = str_at(status, "capabilities") {
        let what = match c.as_str() {
            "ReadOnly" => "ReadOnly: ExternalSecrets can read",
            "WriteOnly" => "WriteOnly: PushSecrets can write",
            "ReadWrite" => "ReadWrite: read by ExternalSecrets, written by PushSecrets",
            _ => "",
        };
        d.info.push(InfoRow::new("Capabilities", if what.is_empty() { c } else { what.into() }));
    }
    if let Some(c) = str_at(spec, "controller") {
        d.info.push(InfoRow::new("Controller class", c).mono());
    }
    if let Some(i) = str_at(spec, "refreshInterval") {
        d.info.push(InfoRow::new("Checked every", format!("{i}s")));
    }
    let retry = &spec["retrySettings"];
    if retry.is_object() {
        let n = str_at(retry, "maxRetries").unwrap_or_else(|| "default".into());
        let every = str_at(retry, "retryInterval").unwrap_or_default();
        d.info.push(InfoRow::new("Retries", if every.is_empty() { n } else { format!("{n}, every {every}") }));
    }
    // Who may use a ClusterSecretStore.
    if v["kind"] == "ClusterSecretStore" {
        let mut parts = Vec::new();
        for c in spec["conditions"].as_array().into_iter().flatten() {
            let names = strings(&c["namespaces"]);
            if !names.is_empty() {
                parts.push(names.join(", "));
            }
            for r in strings(&c["namespaceRegexes"]) {
                parts.push(format!("matching {r}"));
            }
            if let Some(sel) = label_selector(&c["namespaceSelector"]) {
                parts.push(format!("labelled {sel}"));
            }
        }
        d.info.push(InfoRow::new("Usable from", if parts.is_empty() { "every namespace".into() } else { parts.join("; ") }).mono());
    }
    let mut rows = Vec::new();
    leaf_rows(&config, "", d.namespace.as_deref(), false, &mut rows);
    if !rows.is_empty() {
        d.sections.push(InfoSection {
            id: "store-provider".into(),
            title: format!("Provider: {provider}"),
            summary: plural(rows.len(), "setting"),
            rows,
            table: None,
            empty: None,
        });
    }
}

fn provider_name(provider: &str, config: &Value) -> String {
    match provider {
        "aws" => match str_at(config, "service").as_deref() {
            Some("ParameterStore") => "AWS Parameter Store".into(),
            _ => "AWS Secrets Manager".into(),
        },
        "vault" => "HashiCorp Vault".into(),
        "gcpsm" => "Google Secret Manager".into(),
        "azurekv" => "Azure Key Vault".into(),
        "kubernetes" => "Kubernetes".into(),
        "onepassword" | "onepasswordSDK" => "1Password".into(),
        "doppler" => "Doppler".into(),
        "bitwardensecretsmanager" => "Bitwarden Secrets Manager".into(),
        "akeyless" => "Akeyless".into(),
        "ibm" => "IBM Secrets Manager".into(),
        "infisical" => "Infisical".into(),
        "webhook" => "Webhook".into(),
        "fake" => "Fake (test values)".into(),
        "" => "—".into(),
        other => other.to_owned(),
    }
}

/// The objects using a store: ExternalSecrets naming it, or naming it for one
/// data entry.
fn store_users(items: &[Value], store_kind: &str, store: &str) -> InfoSection {
    let uses = |r: &Value| str_at(r, "name").as_deref() == Some(store) && str_at(r, "kind").unwrap_or_else(|| "SecretStore".into()) == store_kind;
    let mut rows: Vec<InfoTableRow> = items
        .iter()
        .filter(|o| {
            let spec = &o["spec"];
            uses(&spec["secretStoreRef"])
                || ["data", "dataFrom"]
                    .iter()
                    .flat_map(|k| spec[*k].as_array().into_iter().flatten())
                    .any(|e| uses(&e["sourceRef"]["storeRef"]))
        })
        .map(es_row)
        .collect();
    rows.sort_by(|a, b| a.cells.cmp(&b.cells));
    users_section("store-users", rows, "No ExternalSecrets use it.")
}

fn es_row(o: &Value) -> InfoTableRow {
    let (ns, name) = (str_at(&o["metadata"], "namespace").unwrap_or_default(), str_at(&o["metadata"], "name").unwrap_or_default());
    let status = sync_status(o);
    let mut row = InfoTableRow::new(vec![ns.clone(), name.clone(), status.clone(), str_at(&o["status"], "refreshTime").unwrap_or_default()]);
    row.link = Some(link(&format!("{GROUP}/ExternalSecret"), Some(&ns), &name));
    row.warn = status != "Synced";
    row
}

fn users_section(id: &str, rows: Vec<InfoTableRow>, empty: &str) -> InfoSection {
    let table = InfoTable {
        columns: ["Namespace", "ExternalSecret", "Status", "Last refresh"].map(String::from).to_vec(),
        mono: vec![1],
        times: vec![3],
        rows,
        ..Default::default()
    };
    table_section(id, "Used by", table, empty)
}

// ---- PushSecret ----

fn push_secret(d: &mut ObjectDetails, v: &Value) {
    d.status = Some(match condition(v, "Ready") {
        Some(c) if c["status"] == "True" => "Synced".into(),
        Some(c) if c["status"] == "False" => "Sync failed".into(),
        _ => "Pending".into(),
    });
    let ns = d.namespace.clone();
    push_rows(d, &v["spec"], ns.as_deref());
    if let Some(t) = str_at(&v["status"], "refreshTime") {
        d.info.push(InfoRow::new("Last refresh", t).time());
    }
    let mut rows = Vec::new();
    for (store, keys) in v["status"]["syncedPushSecrets"].as_object().into_iter().flatten() {
        for key in keys.as_object().into_iter().flatten().map(|(k, _)| k) {
            rows.push(InfoTableRow::new(vec![store.clone(), key.clone()]));
        }
    }
    let table = InfoTable { columns: vec!["Store".into(), "Remote key".into()], mono: vec![1], rows, ..Default::default() };
    d.sections.push(table_section("push-synced", "Pushed", table, "Nothing pushed yet."));
}

/// A PushSecret's (or a ClusterPushSecret's template's) source, stores and
/// policies, and its data table.
fn push_rows(d: &mut ObjectDetails, spec: &Value, namespace: Option<&str>) {
    let selector = &spec["selector"];
    if let Some(name) = str_at(&selector["secret"], "name") {
        let row = InfoRow::new("Source secret", name.clone()).mono();
        d.info.push(match namespace {
            Some(_) => row.link(link("core/Secret", namespace, &name)),
            None => row,
        });
    } else if let Some(sel) = label_selector(&selector["secret"]["selector"]) {
        d.info.push(InfoRow::new("Source secrets", format!("labelled {sel}")).mono());
    } else if let Some(g) = str_at(&selector["generatorRef"], "name") {
        d.info.push(InfoRow::new("Source", format!("{}/{g}", str_at(&selector["generatorRef"], "kind").unwrap_or_default())));
    }
    for r in spec["secretStoreRefs"].as_array().into_iter().flatten() {
        if str_at(r, "name").is_some() {
            store_row(d, r);
        } else if let Some(sel) = label_selector(&r["labelSelector"]) {
            d.info.push(InfoRow::new("Stores", format!("{} labelled {sel}", str_at(r, "kind").unwrap_or_else(|| "SecretStore".into()))).mono());
        }
    }
    let update = str_at(spec, "updatePolicy").unwrap_or_else(|| "Replace".into());
    d.info.push(InfoRow::new("Update", match update.as_str() {
        "Replace" => "Replace: overwrites the remote value".into(),
        "IfNotExists" => "IfNotExists: only writes keys that don't exist yet".into(),
        _ => update,
    }));
    let deletion = str_at(spec, "deletionPolicy").unwrap_or_else(|| "None".into());
    d.info.push(InfoRow::new("Deletion", match deletion.as_str() {
        "Delete" => "Delete: remote keys are deleted with this".into(),
        "None" => "None: remote keys are kept when this is deleted".into(),
        _ => deletion,
    }));
    d.info.push(InfoRow::new("Refresh interval", str_at(spec, "refreshInterval").unwrap_or_else(|| "1h".into())));
    let rows: Vec<InfoTableRow> = spec["data"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| {
            let m = &e["match"];
            InfoTableRow::new(vec![
                str_at(m, "secretKey").unwrap_or_else(|| "(whole secret)".into()),
                str_at(&m["remoteRef"], "remoteKey").unwrap_or_default(),
                str_at(&m["remoteRef"], "property").unwrap_or_default(),
            ])
        })
        .collect();
    let table = InfoTable { columns: ["Secret key", "Remote key", "Property"].map(String::from).to_vec(), mono: vec![0, 1, 2], rows, ..Default::default() };
    d.sections.push(table_section("push-data", "Data", table, "No keys listed."));
}

// ---- Generators ----

/// Generators whose `data` holds literal values rather than settings.
fn is_literal_data(key: &str) -> bool {
    key == "generators.external-secrets.io/Fake"
}

fn generator(d: &mut ObjectDetails, spec: &Value, literal_data: bool) {
    let mut rows = Vec::new();
    leaf_rows(spec, "", d.namespace.as_deref(), literal_data, &mut rows);
    d.info.extend(rows);
}

fn cluster_generator(d: &mut ObjectDetails, v: &Value) {
    let spec = &v["spec"];
    let kind = str_at(spec, "kind").unwrap_or_default();
    d.info.push(InfoRow::new("Generates", kind.clone()));
    let settings = spec["generator"].as_object().and_then(|m| m.values().next()).cloned().unwrap_or_default();
    generator(d, &settings, kind == "Fake");
}

fn generator_state(d: &mut ObjectDetails, v: &Value) {
    let spec = &v["spec"];
    let r = &spec["resource"];
    if let Some(kind) = str_at(r, "kind") {
        d.info.push(InfoRow::new("Generator", format!("{kind}/{}", str_at(&r["metadata"], "name").or_else(|| str_at(r, "name")).unwrap_or_default())));
    }
    if let Some(t) = str_at(spec, "garbageCollectionDeadline") {
        d.info.push(InfoRow::new("Removed after", t).time());
    }
    // The state can hold generated credentials: key names only.
    let keys: Vec<String> = spec["state"].as_object().into_iter().flatten().map(|(k, _)| k.clone()).collect();
    if !keys.is_empty() {
        d.info.push(InfoRow::new("State keys", keys.join(", ")).mono());
    }
}

/// ExternalSecrets that take values from this generator.
fn generator_users(items: &[Value], kind: &str, name: &str) -> InfoSection {
    let mut rows: Vec<InfoTableRow> = items
        .iter()
        .filter(|o| {
            ["data", "dataFrom"]
                .iter()
                .flat_map(|k| o["spec"][*k].as_array().into_iter().flatten())
                .any(|e| {
                    let g = &e["sourceRef"]["generatorRef"];
                    g["kind"] == kind && g["name"] == name
                })
        })
        .map(es_row)
        .collect();
    rows.sort_by(|a, b| a.cells.cmp(&b.cells));
    users_section("generator-users", rows, "No ExternalSecrets use it.")
}

/// One row per scalar setting, keyed by its path (`auth.jwt.serviceAccountRef.name`).
/// References to Secrets and ServiceAccounts link to them; with
/// `literal_data`, values under `data` are left out (only their keys shown).
fn leaf_rows(v: &Value, path: &str, namespace: Option<&str>, literal_data: bool, out: &mut Vec<InfoRow>) {
    match v {
        Value::Object(m) => {
            if literal_data && path == "data" {
                let keys: Vec<&str> = m.keys().map(String::as_str).collect();
                out.push(InfoRow::new("data", format!("{} (values not shown)", keys.join(", "))).mono());
                return;
            }
            for (k, child) in m {
                let p = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
                leaf_rows(child, &p, namespace, literal_data, out);
            }
            ref_link(m, path, namespace, out);
        }
        Value::Array(a) => {
            if a.iter().all(|x| !x.is_object() && !x.is_array()) {
                let items: Vec<String> = a.iter().filter_map(scalar).collect();
                out.push(InfoRow::new(path, items.join(", ")).mono());
            } else {
                for (i, child) in a.iter().enumerate() {
                    leaf_rows(child, &format!("{path}[{i}]"), namespace, literal_data, out);
                }
            }
        }
        other => {
            if let Some(s) = scalar(other) {
                out.push(InfoRow::new(path, s).mono());
            }
        }
    }
}

/// Links the `name` row of a `…secretRef`/`…SecretRef` or `serviceAccountRef`.
fn ref_link(m: &serde_json::Map<String, Value>, path: &str, namespace: Option<&str>, out: &mut [InfoRow]) {
    let last = path.rsplit('.').next().unwrap_or_default();
    let kind = if last.eq_ignore_ascii_case("secretRef") || last.ends_with("SecretRef") || last.ends_with("secretRef") {
        "core/Secret"
    } else if last == "serviceAccountRef" {
        "core/ServiceAccount"
    } else {
        return;
    };
    let Some(name) = m.get("name").and_then(Value::as_str) else { return };
    let ns = m.get("namespace").and_then(Value::as_str).or(namespace);
    let Some(ns) = ns else { return };
    let key = format!("{path}.name");
    if let Some(row) = out.iter_mut().rev().find(|r| r.key == key) {
        row.link = Some(link(kind, Some(ns), name));
    }
}

fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

// ---- lookups ----

/// ExternalSecrets in `namespace` (or everywhere), as JSON. Tries the v1 API,
/// then v1beta1 for older operators.
pub(crate) async fn external_secrets(client: &Client, namespace: Option<&str>) -> Result<Vec<Value>> {
    list_any(client, "ExternalSecret", "externalsecrets", &["v1", "v1beta1"], namespace).await
}

async fn push_secrets(client: &Client, namespace: Option<&str>) -> Result<Vec<Value>> {
    list_any(client, "PushSecret", "pushsecrets", &["v1alpha1", "v1"], namespace).await
}

async fn list_any(client: &Client, kind: &str, plural: &str, versions: &[&str], namespace: Option<&str>) -> Result<Vec<Value>> {
    // What's returned if every version is missing (or none is given).
    let mut last = Error::Message(format!("no {kind} API"));
    for version in versions {
        let k = es_kind(kind, plural, version);
        match dynamic_api(client, &k, namespace).list(&ListParams::default()).await {
            Ok(list) => return Ok(list.items.iter().filter_map(|o: &DynamicObject| serde_json::to_value(o).ok()).collect()),
            Err(kube::Error::Api(s)) if s.code == 404 => last = kube::Error::Api(s).into(),
            Err(e) => return Err(e.into()),
        }
    }
    Err(last)
}

fn es_kind(kind: &str, plural: &str, version: &str) -> KindRef {
    KindRef { group: GROUP.into(), version: version.into(), kind: kind.into(), plural: plural.into(), namespaced: true }
}

/// The list, or none with a note on the kind's section saying why.
pub(crate) fn list_or_note(result: Result<Vec<Value>>, what: &str, d: &mut ObjectDetails) -> Vec<Value> {
    result.unwrap_or_else(|e| {
        d.info.push(InfoRow::new("Note", format!("couldn't list {what}: {}", e.user_message())));
        Vec::new()
    })
}

/// The operator's controller Deployment (the first labelled `label`), linked
/// and offered as Logs filtered to this object's name. Skipped quietly if it
/// can't be listed. Returns its namespace.
pub(crate) async fn controller(client: &Client, label: &str, name: &str, d: &mut ObjectDetails) -> Option<String> {
    let lp = ListParams::default().labels(label);
    let list = Api::<Deployment>::all(client.clone()).list_metadata(&lp).await.ok()?;
    let m = &list.items.first()?.metadata;
    let (ns, deploy) = (m.namespace.clone()?, m.name.clone()?);
    d.info.push(InfoRow::new("Controller", format!("{ns}/{deploy}")).link(link("apps/Deployment", Some(&ns), &deploy)));
    let kind = KindRef { group: "apps".into(), version: "v1".into(), kind: "Deployment".into(), plural: "deployments".into(), namespaced: true };
    d.logs = Some(LogTarget { namespace: ns.clone(), source: LogSource::Workload { kind, name: deploy }, filter: Some(name.to_owned()) });
    Some(ns)
}

pub(crate) fn link(kind: &str, namespace: Option<&str>, name: &str) -> ObjectLink {
    ObjectLink { kind: kind.into(), namespace: namespace.map(str::to_owned), name: name.into() }
}

pub(crate) fn strings(v: &Value) -> Vec<String> {
    v.as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_owned)).collect()
}

// ---- Force sync ----

/// Asks External Secrets to fetch now, like `kubectl annotate es <name>
/// force-sync=$(date +%s) --overwrite`: a changed annotation makes the
/// ExternalSecret refresh. A ClusterExternalSecret copies the annotation to
/// its ExternalSecrets; they're annotated directly too, for operators that
/// don't. Returns how many ExternalSecrets were annotated.
pub async fn force_sync(
    client: &Client,
    gate: &SafetyGate,
    context: &str,
    kind: &KindRef,
    namespace: Option<&str>,
    name: &str,
    dry_run: bool,
) -> Result<usize> {
    if !can_force_sync(kind) {
        return Err(Error::Message(format!("{} can't be force-synced", kind.kind)));
    }
    if !dry_run {
        gate.ensure_writable(context)?;
    }
    let now = jiff::Timestamp::now().as_second().to_string();
    let patch = json!({ "metadata": { "annotations": { FORCE_SYNC: now } } });
    let params = PatchParams { dry_run, ..PatchParams::default() };
    let object = dynamic_api(client, kind, namespace).patch(name, &params, &Patch::Merge(&patch)).await?;
    let mut count = 0;
    if kind.kind == "ExternalSecret" {
        count = 1;
    } else {
        // Spec and status (all that's read here) are in the object's data.
        let v = object.data;
        let child = str_at(&v["spec"], "externalSecretName")
            .or_else(|| str_at(&v["status"], "externalSecretName"))
            .unwrap_or_else(|| name.to_owned());
        let es = es_kind("ExternalSecret", "externalsecrets", &kind.version);
        let mut failed = Vec::new();
        for ns in strings(&v["status"]["provisionedNamespaces"]) {
            match dynamic_api(client, &es, Some(&ns)).patch(&child, &params, &Patch::Merge(&patch)).await {
                Ok(_) => count += 1,
                Err(e) => failed.push(format!("{ns}: {}", Error::from(e).user_message())),
            }
        }
        if !failed.is_empty() {
            tracing::warn!(%context, %name, ?failed, "force sync: some ExternalSecrets not annotated");
            return Err(Error::Message(format!(
                "The ClusterExternalSecret was annotated, and passes it on to its ExternalSecrets, but {} of them couldn't be annotated directly: {}",
                failed.len(),
                failed.join("; ")
            )));
        }
    }
    tracing::info!(%context, kind = %kind.key(), ?namespace, %name, count, dry_run, "force sync requested");
    Ok(count)
}

#[cfg(test)]
#[path = "external_secrets_tests.rs"]
mod coverage_tests;
