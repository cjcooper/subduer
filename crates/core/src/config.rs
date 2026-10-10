//! The Details tab of the Config section's kinds: ConfigMaps and Secrets
//! (data, and the pods using them), HPAs, PDBs, quotas and limit ranges,
//! priority and runtime classes, leases, admission webhooks and validating
//! admission policies.
//!
//! Objects are read as JSON through the discovered API version, so clusters
//! serving older versions (HPA v1, policies in v1beta1) work too. Secret
//! values are never read here: only their sizes, and whether they're text.

use std::collections::BTreeSet;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
use kube::api::{Api, ListParams};
use kube::{Client, ResourceExt};
use serde_json::Value;

use crate::cluster::dynamic_api;
use crate::types::{
    DataEntry, DataSet, InfoRow, InfoSection, InfoTable, InfoTableRow, KindRef, ObjectDetails, ObjectLink,
    has_object_details,
};
use crate::workload::{conditions, selector_string};
use crate::{Error, Result};

pub async fn object_details(client: &Client, kind: &KindRef, namespace: Option<&str>, name: &str) -> Result<ObjectDetails> {
    if crate::helm::is_release(kind) {
        return crate::helm::release_details(client, namespace.unwrap_or_default(), name).await;
    }
    if !has_object_details(kind) {
        return Err(Error::Message(format!("no details view for {}", kind.kind)));
    }
    let object = dynamic_api(client, kind, namespace).get(name).await?;
    let v = serde_json::to_value(&object).map_err(|e| Error::Message(e.to_string()))?;
    let mut d = describe(&kind.key(), &v, jiff::Timestamp::now());
    let ns = namespace.unwrap_or_default();
    match kind.key().as_str() {
        "core/ConfigMap" => d.sections.push(used_by(client, ns, name, false).await?),
        "core/Secret" => d.sections.push(used_by(client, ns, name, true).await?),
        "policy/PodDisruptionBudget" => d.sections.push(pdb_pods(client, ns, &v).await?),
        "admissionregistration.k8s.io/ValidatingAdmissionPolicy" => {
            d.sections.push(policy_bindings(client, kind, name).await?);
        }
        key => {
            crate::network::related(client, key, ns, name, &v, &mut d).await?;
            crate::storage::related(client, key, ns, name, &v, &mut d).await?;
            crate::rbac::related(client, key, ns, name, &v, &mut d).await?;
            crate::namespace::related(client, key, name, &mut d).await?;
        }
    }
    if let Some(ns) = namespace {
        let fields = format!("involvedObject.kind={},involvedObject.name={name}", kind.kind);
        d.events = crate::pod_details::list_events(client, ns, Some(&fields), |_| Some(None)).await;
    }
    Ok(d)
}

/// Everything that comes from the object itself.
pub(crate) fn describe(key: &str, v: &Value, now: jiff::Timestamp) -> ObjectDetails {
    let meta = &v["metadata"];
    let pairs = |m: &Value| -> Vec<(String, String)> {
        m.as_object()
            .into_iter()
            .flatten()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned()))
            .collect()
    };
    let mut d = ObjectDetails {
        kind: str_at(v, "kind").unwrap_or_default(),
        name: str_at(meta, "name").unwrap_or_default(),
        namespace: str_at(meta, "namespace"),
        status: None,
        info: Vec::new(),
        created: str_at(meta, "creationTimestamp"),
        labels: pairs(&meta["labels"]),
        annotations: pairs(&meta["annotations"])
            .into_iter()
            .filter(|(k, _)| k != "kubectl.kubernetes.io/last-applied-configuration")
            .collect(),
        conditions: conditions(v["status"]["conditions"].as_array().map(Vec::as_slice)),
        data: None,
        sections: Vec::new(),
        events: Vec::new(),
    };
    match key {
        "core/ConfigMap" => config_map(&mut d, v),
        "core/Secret" => secret(&mut d, v),
        "autoscaling/HorizontalPodAutoscaler" => hpa(&mut d, v),
        "policy/PodDisruptionBudget" => pdb(&mut d, v),
        "core/ResourceQuota" => quota(&mut d, v),
        "core/LimitRange" => limit_range(&mut d, v),
        "scheduling.k8s.io/PriorityClass" => priority_class(&mut d, v),
        "node.k8s.io/RuntimeClass" => runtime_class(&mut d, v),
        "coordination.k8s.io/Lease" => lease(&mut d, v, now),
        "admissionregistration.k8s.io/MutatingWebhookConfiguration" => webhooks(&mut d, v, true),
        "admissionregistration.k8s.io/ValidatingWebhookConfiguration" => webhooks(&mut d, v, false),
        "admissionregistration.k8s.io/ValidatingAdmissionPolicy" => policy(&mut d, v),
        "admissionregistration.k8s.io/ValidatingAdmissionPolicyBinding" => binding(&mut d, v),
        key => {
            crate::network::describe(key, &mut d, v);
            crate::storage::describe(key, &mut d, v);
            crate::rbac::describe(key, &mut d, v);
            crate::namespace::describe(key, &mut d, v);
        }
    }
    // Owner rows last, like workloads.
    if let Some(owner) = v["metadata"]["ownerReferences"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|o| o["controller"] == true)
    {
        let (kind, name) = (str_at(owner, "kind").unwrap_or_default(), str_at(owner, "name").unwrap_or_default());
        d.info.push(InfoRow::new("Controlled by", format!("{kind}/{name}")));
    }
    d
}

// ---- ConfigMap and Secret ----

fn config_map(d: &mut ObjectDetails, v: &Value) {
    let mut entries: Vec<DataEntry> = v["data"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, val)| {
            let text = val.as_str().unwrap_or_default().to_owned();
            DataEntry { key: k.clone(), size: text.len(), value: Some(text), binary: false }
        })
        .chain(v["binaryData"].as_object().into_iter().flatten().map(|(k, val)| DataEntry {
            key: k.clone(),
            value: None,
            size: decoded_len(val.as_str().unwrap_or_default()),
            binary: true,
        }))
        .collect();
    entries.sort_by(|a, b| a.key.cmp(&b.key));
    data_rows(d, &entries);
    let immutable = v["immutable"] == true;
    if immutable {
        d.info.push(InfoRow::new("Immutable", "yes"));
    }
    let read_only = immutable.then(|| "Immutable: it can't be changed, only deleted and created again.".to_owned());
    d.data = Some(DataSet {
        secret: false,
        resource_version: str_at(&v["metadata"], "resourceVersion").unwrap_or_default(),
        read_only,
        warning: managed_warning(v),
        entries,
    });
}

fn secret(d: &mut ObjectDetails, v: &Value) {
    let type_ = str_at(v, "type").unwrap_or_else(|| "Opaque".to_owned());
    d.info.push(InfoRow::new("Type", type_.clone()).mono());
    let annotations = &v["metadata"]["annotations"];
    if let Some(sa) = str_at(annotations, "kubernetes.io/service-account.name") {
        let link = ObjectLink { kind: "core/ServiceAccount".into(), namespace: d.namespace.clone(), name: sa.clone() };
        d.info.push(InfoRow::new("Service account", sa).link(link));
    }
    // Sizes and whether each value is text; the values themselves are dropped here.
    let mut entries: Vec<DataEntry> = v["data"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, val)| {
            let bytes = STANDARD.decode(val.as_str().unwrap_or_default()).unwrap_or_default();
            DataEntry { key: k.clone(), value: None, size: bytes.len(), binary: std::str::from_utf8(&bytes).is_err() }
        })
        .collect();
    entries.sort_by(|a, b| a.key.cmp(&b.key));
    data_rows(d, &entries);
    let immutable = v["immutable"] == true;
    if immutable {
        d.info.push(InfoRow::new("Immutable", "yes"));
    }
    let read_only = if immutable {
        Some("Immutable: it can't be changed, only deleted and created again.".to_owned())
    } else if type_.starts_with("helm.sh/release") {
        Some("A Helm release record: change it with helm, not here.".to_owned())
    } else {
        None
    };
    let warning = managed_warning(v).or_else(|| {
        (type_ == "kubernetes.io/service-account-token")
            .then(|| "A service account token, filled in by Kubernetes; edits may be overwritten.".to_owned())
    });
    d.data = Some(DataSet {
        secret: true,
        resource_version: str_at(&v["metadata"], "resourceVersion").unwrap_or_default(),
        read_only,
        warning,
        entries,
    });
}

fn data_rows(d: &mut ObjectDetails, entries: &[DataEntry]) {
    let total: usize = entries.iter().map(|e| e.size).sum();
    d.info.push(InfoRow::new("Keys", entries.len().to_string()));
    d.info.push(InfoRow::new("Size", size_text(total)));
}

/// Who may undo edits made here.
fn managed_warning(v: &Value) -> Option<String> {
    let meta = &v["metadata"];
    if str_at(&meta["labels"], "app.kubernetes.io/managed-by").as_deref() == Some("Helm") {
        let release = str_at(&meta["annotations"], "meta.helm.sh/release-name");
        let release = release.map(|r| format!(" (release {r})")).unwrap_or_default();
        return Some(format!("Managed by Helm{release}: the next helm upgrade puts the chart's values back."));
    }
    if str_at(meta, "name").as_deref() == Some("kube-root-ca.crt") {
        return Some("Kept in sync by Kubernetes: edits are overwritten.".to_owned());
    }
    let owner = meta["ownerReferences"].as_array().into_iter().flatten().find(|o| o["controller"] == true)?;
    Some(format!(
        "Controlled by {} {}, which may overwrite edits.",
        str_at(owner, "kind").unwrap_or_default(),
        str_at(owner, "name").unwrap_or_default()
    ))
}

/// The pods in the namespace that use a ConfigMap or Secret, and how.
async fn used_by(client: &Client, namespace: &str, name: &str, secret: bool) -> Result<InfoSection> {
    let pods = Api::<Pod>::namespaced(client.clone(), namespace).list(&ListParams::default()).await?;
    let mut rows: Vec<InfoTableRow> = pods
        .items
        .iter()
        .filter_map(|p| {
            let how = uses(p, name, secret);
            if how.is_empty() {
                return None;
            }
            let owner = p
                .owner_references()
                .iter()
                .find(|o| o.controller == Some(true))
                .map_or_else(|| "—".to_owned(), |o| format!("{}/{}", o.kind, o.name));
            let mut row = InfoTableRow::new(vec![p.name_any(), owner, how.join(", ")]);
            row.link = Some(ObjectLink { kind: "core/Pod".into(), namespace: Some(namespace.to_owned()), name: p.name_any() });
            Some(row)
        })
        .collect();
    rows.sort_by(|a, b| a.cells.cmp(&b.cells));
    let summary = if rows.is_empty() { "none".to_owned() } else { plural(rows.len(), "pod") };
    Ok(InfoSection {
        id: "used-by".into(),
        title: "Used by".into(),
        summary,
        rows: Vec::new(),
        table: Some(InfoTable { columns: vec!["Pod".into(), "Controlled by".into(), "As".into()], rows, ..Default::default() }),
        empty: Some("No pods in this namespace use it.".into()),
    })
}

/// How a pod uses the ConfigMap or Secret `name`: `env`, `envFrom`, `volume`, `image pull`.
fn uses(pod: &Pod, name: &str, secret: bool) -> Vec<&'static str> {
    let Some(spec) = &pod.spec else { return Vec::new() };
    let mut how = BTreeSet::new();
    let containers = spec.containers.iter().chain(spec.init_containers.iter().flatten());
    for c in containers {
        for e in c.env.iter().flatten() {
            let Some(from) = &e.value_from else { continue };
            let hit = if secret {
                from.secret_key_ref.as_ref().is_some_and(|r| r.name == name)
            } else {
                from.config_map_key_ref.as_ref().is_some_and(|r| r.name == name)
            };
            if hit {
                how.insert("env");
            }
        }
        for e in c.env_from.iter().flatten() {
            let hit = if secret {
                e.secret_ref.as_ref().is_some_and(|r| r.name == name)
            } else {
                e.config_map_ref.as_ref().is_some_and(|r| r.name == name)
            };
            if hit {
                how.insert("envFrom");
            }
        }
    }
    for vol in spec.volumes.iter().flatten() {
        let direct = if secret {
            vol.secret.as_ref().is_some_and(|s| s.secret_name.as_deref() == Some(name))
        } else {
            vol.config_map.as_ref().is_some_and(|c| c.name == name)
        };
        let projected = vol.projected.iter().flat_map(|p| p.sources.iter().flatten()).any(|s| {
            if secret {
                s.secret.as_ref().is_some_and(|r| r.name == name)
            } else {
                s.config_map.as_ref().is_some_and(|r| r.name == name)
            }
        });
        if direct || projected {
            how.insert("volume");
        }
    }
    if secret && spec.image_pull_secrets.iter().flatten().any(|r| r.name == name) {
        how.insert("image pull");
    }
    how.into_iter().collect()
}

// ---- HorizontalPodAutoscaler ----

fn hpa(d: &mut ObjectDetails, v: &Value) {
    let (spec, status) = (&v["spec"], &v["status"]);
    let target = &spec["scaleTargetRef"];
    if let (Some(kind), Some(name)) = (str_at(target, "kind"), str_at(target, "name")) {
        let group = str_at(target, "apiVersion")
            .and_then(|a| a.split_once('/').map(|(g, _)| g.to_owned()))
            .unwrap_or_else(|| "core".to_owned());
        let link = ObjectLink { kind: format!("{group}/{kind}"), namespace: d.namespace.clone(), name: name.clone() };
        d.info.push(InfoRow::new("Scales", format!("{kind} {name}")).link(link));
    }
    let (current, desired) = (int_at(status, "currentReplicas"), int_at(status, "desiredReplicas"));
    d.info.push(InfoRow::new(
        "Replicas",
        format!("{} now · {} wanted", current.unwrap_or(0), desired.unwrap_or(0)),
    ));
    d.info.push(InfoRow::new("Min replicas", int_at(spec, "minReplicas").unwrap_or(1).to_string()));
    d.info.push(InfoRow::new("Max replicas", int_at(spec, "maxReplicas").unwrap_or(0).to_string()));
    if let Some(t) = str_at(status, "lastScaleTime") {
        d.info.push(InfoRow::new("Last scaled", t).time());
    }

    let mut rows = Vec::new();
    // autoscaling/v1 has a single CPU target.
    if let Some(target) = int_at(spec, "targetCPUUtilizationPercentage") {
        let now = int_at(status, "currentCPUUtilizationPercentage").map_or_else(|| "—".to_owned(), |c| format!("{c}%"));
        rows.push(InfoTableRow::new(vec!["cpu".into(), now, format!("{target}% of request")]));
    }
    let current_metrics: Vec<&Value> = status["currentMetrics"].as_array().into_iter().flatten().collect();
    for m in spec["metrics"].as_array().into_iter().flatten() {
        let type_ = str_at(m, "type").unwrap_or_default();
        let (label, name, target) = metric(m, &type_);
        let current = current_metrics
            .iter()
            .find(|c| str_at(c, "type").as_deref() == Some(type_.as_str()) && metric(c, &type_).1 == name)
            .map_or_else(|| "—".to_owned(), |c| metric_value(&metric_body(c, &type_)["current"]));
        rows.push(InfoTableRow::new(vec![label, current, target]));
    }
    let n = rows.len();
    d.sections.push(InfoSection {
        id: "metrics".into(),
        title: "Metrics".into(),
        summary: if n == 0 { "none".into() } else { n.to_string() },
        rows: Vec::new(),
        table: Some(InfoTable {
            columns: vec!["Metric".into(), "Now".into(), "Target".into()],
            numeric: vec![1, 2],
            rows,
            ..Default::default()
        }),
        empty: Some("No metrics: it won't scale.".into()),
    });

    let behavior = &spec["behavior"];
    let mut rows = Vec::new();
    for (key, label) in [("scaleUp", "Scale up"), ("scaleDown", "Scale down")] {
        let b = &behavior[key];
        if b.is_null() {
            continue;
        }
        let mut parts = Vec::new();
        if let Some(s) = int_at(b, "stabilizationWindowSeconds") {
            parts.push(format!("waits {s}s"));
        }
        let policies: Vec<String> = b["policies"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| {
                let value = int_at(p, "value").unwrap_or(0);
                let unit = match (str_at(p, "type").as_deref(), value) {
                    (Some("Percent"), _) => "%",
                    (_, 1) => " pod",
                    _ => " pods",
                };
                format!("{value}{unit} per {}s", int_at(p, "periodSeconds").unwrap_or(0))
            })
            .collect();
        if !policies.is_empty() {
            let select = str_at(b, "selectPolicy").unwrap_or_else(|| "Max".into());
            parts.push(format!("{} ({select})", policies.join(", ")));
        }
        if !parts.is_empty() {
            rows.push(InfoRow::new(label, parts.join(" · ")));
        }
    }
    if !rows.is_empty() {
        d.sections.push(InfoSection {
            id: "behavior".into(),
            title: "Behavior".into(),
            summary: String::new(),
            rows,
            table: None,
            empty: None,
        });
    }

    let holds = |t: &str, s: &str| d.conditions.iter().any(|c| c.type_ == t && c.status == s);
    d.status = Some(
        if holds("AbleToScale", "False") {
            "Unable to scale"
        } else if holds("ScalingActive", "False") {
            "Inactive"
        } else if holds("ScalingLimited", "True") {
            "At limit"
        } else {
            "Active"
        }
        .to_owned(),
    );
}

/// The metric's spec or status body for its type, e.g. `m.resource`.
fn metric_body<'a>(m: &'a Value, type_: &str) -> &'a Value {
    match type_ {
        "Resource" => &m["resource"],
        "ContainerResource" => &m["containerResource"],
        "Pods" => &m["pods"],
        "Object" => &m["object"],
        "External" => &m["external"],
        _ => &Value::Null,
    }
}

/// (label, identity for matching status to spec, target text).
fn metric(m: &Value, type_: &str) -> (String, String, String) {
    let body = metric_body(m, type_);
    let name = match type_ {
        "Resource" => str_at(body, "name").unwrap_or_default(),
        "ContainerResource" => format!(
            "{} ({})",
            str_at(body, "name").unwrap_or_default(),
            str_at(body, "container").unwrap_or_default()
        ),
        "Object" => format!(
            "{} on {}/{}",
            str_at(&body["metric"], "name").unwrap_or_default(),
            str_at(&body["describedObject"], "kind").unwrap_or_default(),
            str_at(&body["describedObject"], "name").unwrap_or_default()
        ),
        _ => str_at(&body["metric"], "name").unwrap_or_default(),
    };
    let label = match type_ {
        "Pods" => format!("{name} (per pod)"),
        "External" => format!("{name} (external)"),
        _ => name.clone(),
    };
    let target = &body["target"];
    let target = match str_at(target, "type").as_deref() {
        Some("Utilization") => format!("{}% of request", int_at(target, "averageUtilization").unwrap_or(0)),
        Some("AverageValue") => format!("{} average", str_at(target, "averageValue").unwrap_or_default()),
        Some("Value") => str_at(target, "value").unwrap_or_default(),
        _ => "—".to_owned(),
    };
    (label, name, target)
}

fn metric_value(current: &Value) -> String {
    match (int_at(current, "averageUtilization"), str_at(current, "averageValue"), str_at(current, "value")) {
        (Some(u), Some(avg), _) => format!("{u}% ({avg})"),
        (Some(u), None, _) => format!("{u}%"),
        (None, Some(avg), _) => format!("{avg} average"),
        (None, None, Some(v)) => v,
        _ => "—".to_owned(),
    }
}

// ---- PodDisruptionBudget ----

fn pdb(d: &mut ObjectDetails, v: &Value) {
    let (spec, status) = (&v["spec"], &v["status"]);
    if let Some(min) = str_at(spec, "minAvailable") {
        d.info.push(InfoRow::new("Min available", min));
    }
    if let Some(max) = str_at(spec, "maxUnavailable") {
        d.info.push(InfoRow::new("Max unavailable", max));
    }
    let healthy = int_at(status, "currentHealthy").unwrap_or(0);
    let needed = int_at(status, "desiredHealthy").unwrap_or(0);
    let expected = int_at(status, "expectedPods").unwrap_or(0);
    let allowed = int_at(status, "disruptionsAllowed").unwrap_or(0);
    d.info.push(InfoRow::new("Healthy", format!("{healthy} of {expected} pods · {needed} needed")));
    d.info.push(InfoRow::new("Evictions allowed", allowed.to_string()));
    if let Some(selector) = label_selector(&spec["selector"]) {
        d.info.push(InfoRow::new("Selector", selector).mono());
    }
    if let Some(policy) = str_at(spec, "unhealthyPodEvictionPolicy") {
        d.info.push(InfoRow::new("Unhealthy pods", policy));
    }
    d.status = Some(
        if expected == 0 {
            "No pods"
        } else if healthy < needed {
            "Unhealthy"
        } else if allowed == 0 {
            "Blocking"
        } else {
            "Allowing"
        }
        .to_owned(),
    );
}

async fn pdb_pods(client: &Client, namespace: &str, v: &Value) -> Result<InfoSection> {
    let selector = label_selector(&v["spec"]["selector"]);
    let pods = match &selector {
        // An empty selector covers every pod in the namespace.
        Some(s) => ListParams::default().labels(s),
        None => ListParams::default(),
    };
    let pods = Api::<Pod>::namespaced(client.clone(), namespace).list(&pods).await?;
    let rows: Vec<InfoTableRow> = pods
        .items
        .iter()
        .map(|p| {
            let data = serde_json::to_value(p).unwrap_or_default();
            let status = crate::table::pod_status(&data, p.metadata.deletion_timestamp.is_some());
            let node = p.spec.as_ref().and_then(|s| s.node_name.clone()).unwrap_or_default();
            let mut row = InfoTableRow::new(vec![p.name_any(), status, node]);
            row.link = Some(ObjectLink { kind: "core/Pod".into(), namespace: Some(namespace.to_owned()), name: p.name_any() });
            row
        })
        .collect();
    Ok(InfoSection {
        id: "pdb-pods".into(),
        title: "Pods".into(),
        summary: if rows.is_empty() { "none".into() } else { rows.len().to_string() },
        rows: Vec::new(),
        table: Some(InfoTable { columns: vec!["Pod".into(), "Status".into(), "Node".into()], rows, ..Default::default() }),
        empty: Some("No pods match its selector.".into()),
    })
}

// ---- ResourceQuota and LimitRange ----

fn quota(d: &mut ObjectDetails, v: &Value) {
    let (spec, status) = (&v["spec"], &v["status"]);
    let hard = if status["hard"].is_object() { &status["hard"] } else { &spec["hard"] };
    let mut at_limit = 0;
    let mut near = 0;
    let rows: Vec<InfoTableRow> = hard
        .as_object()
        .into_iter()
        .flatten()
        .map(|(resource, limit)| {
            let limit = limit.as_str().unwrap_or_default().to_owned();
            let used = str_at(&status["used"], resource).unwrap_or_else(|| "0".to_owned());
            let fill = match (crate::quantity::parse(&used), crate::quantity::parse(&limit)) {
                (Some(u), Some(h)) if h > 0.0 => Some(u / h),
                (Some(u), Some(_)) => Some(if u > 0.0 { 1.0 } else { 0.0 }),
                _ => None,
            };
            let full = fill.is_some_and(|f| f >= 1.0);
            at_limit += usize::from(full);
            near += usize::from(fill.is_some_and(|f| (0.9..1.0).contains(&f)));
            let mut row = InfoTableRow::new(vec![resource.clone(), used, limit]);
            row.fill = fill.map(|f| f.min(1.0));
            row.warn = fill.is_some_and(|f| f >= 0.9);
            row
        })
        .collect();
    let scopes: Vec<String> = spec["scopes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(str::to_owned))
        .collect();
    if !scopes.is_empty() {
        d.info.push(InfoRow::new("Scopes", scopes.join(", ")));
    }
    for e in spec["scopeSelector"]["matchExpressions"].as_array().into_iter().flatten() {
        let values: Vec<&str> = e["values"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        d.info.push(InfoRow::new(
            "Scope selector",
            format!(
                "{} {} {}",
                str_at(e, "scopeName").unwrap_or_default(),
                str_at(e, "operator").unwrap_or_default(),
                values.join(",")
            )
            .trim_end()
            .to_owned(),
        ));
    }
    let n = rows.len();
    d.status = Some(
        if at_limit > 0 {
            "At limit"
        } else if near > 0 {
            "Near limit"
        } else {
            "Within limits"
        }
        .to_owned(),
    );
    let summary = match at_limit {
        0 => plural(n, "resource"),
        a => format!("{} · {a} at limit", plural(n, "resource")),
    };
    d.sections.push(InfoSection {
        id: "quota".into(),
        title: "Usage".into(),
        summary,
        rows: Vec::new(),
        table: Some(InfoTable {
            times: Vec::new(),
            columns: vec!["Resource".into(), "Used".into(), "Hard".into()],
            numeric: vec![1, 2],
            mono: vec![0],
            rows,
        }),
        empty: Some("No limits set.".into()),
    });
}

fn limit_range(d: &mut ObjectDetails, v: &Value) {
    let mut rows = Vec::new();
    let mut types = Vec::new();
    for l in v["spec"]["limits"].as_array().into_iter().flatten() {
        let type_ = str_at(l, "type").unwrap_or_default();
        types.push(type_.clone());
        let fields = ["min", "max", "defaultRequest", "default", "maxLimitRequestRatio"];
        let resources: BTreeSet<&String> = fields.iter().flat_map(|f| l[*f].as_object().into_iter().flatten().map(|(k, _)| k)).collect();
        for r in resources {
            let mut cells = vec![type_.clone(), r.clone()];
            cells.extend(fields.iter().map(|f| str_at(&l[*f], r).unwrap_or_else(|| "—".into())));
            rows.push(InfoTableRow::new(cells));
        }
    }
    d.info.push(InfoRow::new("Applies to", if types.is_empty() { "nothing".to_owned() } else { types.join(", ") }));
    let n = rows.len();
    d.sections.push(InfoSection {
        id: "limits".into(),
        title: "Limits".into(),
        summary: n.to_string(),
        rows: Vec::new(),
        table: Some(InfoTable {
            columns: ["Type", "Resource", "Min", "Max", "Default request", "Default limit", "Max limit/request"]
                .map(String::from)
                .to_vec(),
            numeric: vec![2, 3, 4, 5, 6],
            rows,
            ..Default::default()
        }),
        empty: Some("No limits set.".into()),
    });
}

// ---- PriorityClass, RuntimeClass, Lease ----

fn priority_class(d: &mut ObjectDetails, v: &Value) {
    d.info.push(InfoRow::new("Value", int_at(v, "value").unwrap_or(0).to_string()));
    d.info.push(InfoRow::new("Default for pods", if v["globalDefault"] == true { "yes" } else { "no" }));
    d.info.push(InfoRow::new(
        "Preemption",
        str_at(v, "preemptionPolicy").unwrap_or_else(|| "PreemptLowerPriority".into()),
    ));
    if let Some(text) = str_at(v, "description") {
        d.info.push(InfoRow::new("Description", text));
    }
    if d.name.starts_with("system-") {
        d.info.push(InfoRow::new("Built in", "yes: reserved for cluster components"));
    }
}

fn runtime_class(d: &mut ObjectDetails, v: &Value) {
    d.info.push(InfoRow::new("Handler", str_at(v, "handler").unwrap_or_default()).mono());
    let overhead: Vec<String> = v["overhead"]["podFixed"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, q)| format!("{k} {}", q.as_str().unwrap_or_default()))
        .collect();
    if !overhead.is_empty() {
        d.info.push(InfoRow::new("Pod overhead", overhead.join(" · ")));
    }
    let selector: Vec<String> = v["scheduling"]["nodeSelector"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, val)| format!("{k}={}", val.as_str().unwrap_or_default()))
        .collect();
    if !selector.is_empty() {
        d.info.push(InfoRow::new("Node selector", selector.join(",")).mono());
    }
    let tolerations = v["scheduling"]["tolerations"].as_array().map_or(0, Vec::len);
    if tolerations > 0 {
        d.info.push(InfoRow::new("Tolerations", tolerations.to_string()));
    }
}

fn lease(d: &mut ObjectDetails, v: &Value, now: jiff::Timestamp) {
    let spec = &v["spec"];
    let holder = str_at(spec, "holderIdentity").filter(|h| !h.is_empty());
    let duration = int_at(spec, "leaseDurationSeconds");
    let renewed = str_at(spec, "renewTime");
    d.info.push(InfoRow::new("Holder", holder.clone().unwrap_or_else(|| "nobody".into())).mono());
    if let Some(s) = duration {
        d.info.push(InfoRow::new("Duration", format!("{s}s")));
    }
    if let Some(t) = str_at(spec, "acquireTime") {
        d.info.push(InfoRow::new("Acquired", t).time());
    }
    if let Some(t) = &renewed {
        d.info.push(InfoRow::new("Renewed", t.clone()).time());
    }
    if let Some(n) = int_at(spec, "leaseTransitions") {
        d.info.push(InfoRow::new("Changed hands", format!("{n} times")));
    }
    if let Some(s) = str_at(spec, "strategy") {
        d.info.push(InfoRow::new("Strategy", s));
    }
    if let Some(p) = str_at(spec, "preferredHolder") {
        d.info.push(InfoRow::new("Preferred holder", p).mono());
    }
    let expired = match (renewed.and_then(|t| t.parse::<jiff::Timestamp>().ok()), duration) {
        (Some(t), Some(s)) => t + jiff::SignedDuration::from_secs(s) < now,
        _ => false,
    };
    d.status = Some(
        match (holder.is_some(), expired) {
            (false, _) => "Free",
            (true, true) => "Expired",
            (true, false) => "Held",
        }
        .to_owned(),
    );
}

// ---- Admission webhooks and policies ----

fn webhooks(d: &mut ObjectDetails, v: &Value, mutating: bool) {
    let hooks: Vec<&Value> = v["webhooks"].as_array().into_iter().flatten().collect();
    d.info.push(InfoRow::new("Webhooks", hooks.len().to_string()));
    for w in hooks {
        let name = str_at(w, "name").unwrap_or_default();
        let config = &w["clientConfig"];
        let mut rows = Vec::new();
        if let Some(url) = str_at(config, "url") {
            rows.push(InfoRow::new("Calls", url).mono());
        } else if let Some(svc) = config["service"].as_object().map(|_| &config["service"]) {
            let (ns, svc_name) = (str_at(svc, "namespace").unwrap_or_default(), str_at(svc, "name").unwrap_or_default());
            let port = int_at(svc, "port").unwrap_or(443);
            let path = str_at(svc, "path").unwrap_or_default();
            let link = ObjectLink { kind: "core/Service".into(), namespace: Some(ns.clone()), name: svc_name.clone() };
            rows.push(InfoRow::new("Calls", format!("service {ns}/{svc_name}:{port}{path}")).mono().link(link));
        }
        let failure = str_at(w, "failurePolicy").unwrap_or_else(|| "Fail".into());
        let timeout = int_at(w, "timeoutSeconds").unwrap_or(10);
        rows.push(InfoRow::new(
            "If it fails",
            if failure == "Ignore" { "Ignore: requests go through" } else { "Fail: requests are rejected" },
        ));
        rows.push(InfoRow::new("Timeout", format!("{timeout}s")));
        if let Some(s) = str_at(w, "sideEffects") {
            rows.push(InfoRow::new("Side effects", s));
        }
        rows.push(InfoRow::new("Match policy", str_at(w, "matchPolicy").unwrap_or_else(|| "Equivalent".into())));
        if mutating {
            rows.push(InfoRow::new(
                "Reinvocation",
                str_at(w, "reinvocationPolicy").unwrap_or_else(|| "Never".into()),
            ));
        }
        selector_rows(&mut rows, w);
        for c in w["matchConditions"].as_array().into_iter().flatten() {
            rows.push(InfoRow::new(
                format!("Condition {}", str_at(c, "name").unwrap_or_default()),
                str_at(c, "expression").unwrap_or_default(),
            ).mono());
        }
        let rules = rules_table(w["rules"].as_array().map(Vec::as_slice).unwrap_or_default());
        d.sections.push(InfoSection {
            id: format!("webhook:{name}"),
            title: name,
            summary: format!("{failure} · {}", plural(rules.rows.len(), "rule")),
            rows,
            table: Some(rules),
            empty: None,
        });
    }
}

fn policy(d: &mut ObjectDetails, v: &Value) {
    let spec = &v["spec"];
    d.info.push(InfoRow::new(
        "If it fails",
        if str_at(spec, "failurePolicy").as_deref() == Some("Ignore") {
            "Ignore: requests go through"
        } else {
            "Fail: requests are rejected"
        },
    ));
    if let Some(kind) = str_at(&spec["paramKind"], "kind") {
        let api = str_at(&spec["paramKind"], "apiVersion").unwrap_or_default();
        d.info.push(InfoRow::new("Parameters", format!("{kind} ({api})")));
    }
    let constraints = &spec["matchConstraints"];
    if let Some(p) = str_at(constraints, "matchPolicy") {
        d.info.push(InfoRow::new("Match policy", p));
    }
    selector_rows(&mut d.info, constraints);

    let rules = rules_table(constraints["resourceRules"].as_array().map(Vec::as_slice).unwrap_or_default());
    d.sections.push(table_section("resources", "Applies to", rules, "No resource rules."));
    let excluded = rules_table(constraints["excludeResourceRules"].as_array().map(Vec::as_slice).unwrap_or_default());
    if !excluded.rows.is_empty() {
        d.sections.push(table_section("excluded", "Excludes", excluded, ""));
    }
    let named = |list: &Value, a: &str, b: &str| -> Vec<InfoTableRow> {
        list.as_array()
            .into_iter()
            .flatten()
            .map(|x| InfoTableRow::new(vec![str_at(x, a).unwrap_or_default(), str_at(x, b).unwrap_or_default()]))
            .collect()
    };
    let validations: Vec<InfoTableRow> = spec["validations"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|x| {
            let message = str_at(x, "message")
                .or_else(|| str_at(x, "messageExpression").map(|m| format!("= {m}")))
                .unwrap_or_default();
            InfoTableRow::new(vec![str_at(x, "expression").unwrap_or_default(), message, str_at(x, "reason").unwrap_or_default()])
        })
        .collect();
    let table = |columns: &[&str], rows: Vec<InfoTableRow>| InfoTable {
        columns: columns.iter().map(|c| (*c).to_owned()).collect(),
        mono: vec![0, 1].into_iter().filter(|i| *i == 0 || columns.len() == 2).collect(),
        rows,
        ..Default::default()
    };
    d.sections.push(table_section("validations", "Validations", table(&["Expression", "Message", "Reason"], validations), "No validations."));
    let variables = named(&spec["variables"], "name", "expression");
    if !variables.is_empty() {
        d.sections.push(table_section("variables", "Variables", table(&["Name", "Expression"], variables), ""));
    }
    let conditions = named(&spec["matchConditions"], "name", "expression");
    if !conditions.is_empty() {
        d.sections.push(table_section("match-conditions", "Match conditions", table(&["Name", "Expression"], conditions), ""));
    }
    let audit = named(&spec["auditAnnotations"], "key", "valueExpression");
    if !audit.is_empty() {
        d.sections.push(table_section("audit", "Audit annotations", table(&["Key", "Value expression"], audit), ""));
    }
    let warnings = named(&v["status"]["typeChecking"]["expressionWarnings"], "fieldRef", "warning");
    if !warnings.is_empty() {
        d.status = Some("Type warnings".into());
        let mut t = table(&["Field", "Warning"], warnings);
        t.mono = vec![0];
        d.sections.push(table_section("type-warnings", "Type warnings", t, ""));
    }
}

async fn policy_bindings(client: &Client, policy_kind: &KindRef, name: &str) -> Result<InfoSection> {
    let kind = KindRef {
        kind: "ValidatingAdmissionPolicyBinding".into(),
        plural: "validatingadmissionpolicybindings".into(),
        ..policy_kind.clone()
    };
    let list = dynamic_api(client, &kind, None).list(&ListParams::default()).await?;
    let rows = list
        .items
        .iter()
        .filter(|b| b.data["spec"]["policyName"].as_str() == Some(name))
        .map(|b| {
            let actions: Vec<&str> =
                b.data["spec"]["validationActions"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            let mut row = InfoTableRow::new(vec![b.name_any(), actions.join(", ")]);
            row.link = Some(ObjectLink { kind: kind.key(), namespace: None, name: b.name_any() });
            row
        })
        .collect();
    let table = InfoTable { columns: vec!["Binding".into(), "Actions".into()], rows, ..Default::default() };
    let mut section = table_section("bindings", "Bindings", table, "Not bound: it isn't enforced anywhere.");
    if section.table.as_ref().is_some_and(|t| t.rows.is_empty()) {
        section.summary = "none · not enforced".into();
    }
    Ok(section)
}

fn binding(d: &mut ObjectDetails, v: &Value) {
    let spec = &v["spec"];
    if let Some(policy) = str_at(spec, "policyName") {
        let link = ObjectLink {
            kind: "admissionregistration.k8s.io/ValidatingAdmissionPolicy".into(),
            namespace: None,
            name: policy.clone(),
        };
        d.info.push(InfoRow::new("Policy", policy).link(link));
    }
    let actions: Vec<&str> = spec["validationActions"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    d.info.push(InfoRow::new("On failure", actions.join(", ")));
    let param = &spec["paramRef"];
    if param.is_object() {
        let mut text = Vec::new();
        if let Some(n) = str_at(param, "name") {
            text.push(format!("name {n}"));
        }
        if let Some(n) = str_at(param, "namespace") {
            text.push(format!("namespace {n}"));
        }
        if let Some(s) = label_selector(&param["selector"]) {
            text.push(format!("selector {s}"));
        }
        if let Some(a) = str_at(param, "parameterNotFoundAction") {
            text.push(format!("if missing: {a}"));
        }
        d.info.push(InfoRow::new("Parameters", text.join(" · ")));
    }
    let resources = &spec["matchResources"];
    selector_rows(&mut d.info, resources);
    let rules = rules_table(resources["resourceRules"].as_array().map(Vec::as_slice).unwrap_or_default());
    if !rules.rows.is_empty() {
        d.sections.push(table_section("resources", "Narrowed to", rules, ""));
    }
}

/// Namespace and object selector rows; an empty selector matches everything.
fn selector_rows(rows: &mut Vec<InfoRow>, v: &Value) {
    for (key, label) in [("namespaceSelector", "Namespaces"), ("objectSelector", "Objects")] {
        if v[key].is_object() {
            let text = label_selector(&v[key]).unwrap_or_else(|| "all".into());
            rows.push(InfoRow::new(label, text).mono());
        }
    }
}

/// Operations, groups, versions, resources and scope of admission rules.
fn rules_table(rules: &[Value]) -> InfoTable {
    let list = |v: &Value, key: &str| -> String {
        let items: Vec<&str> = v[key].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        let items: Vec<&str> = items.into_iter().map(|i| if i.is_empty() { "core" } else { i }).collect();
        if items.is_empty() { "—".to_owned() } else { items.join(", ") }
    };
    InfoTable {
        columns: ["Operations", "Groups", "Versions", "Resources", "Scope"].map(String::from).to_vec(),
        mono: vec![3],
        rows: rules
            .iter()
            .map(|r| {
                InfoTableRow::new(vec![
                    list(r, "operations"),
                    list(r, "apiGroups"),
                    list(r, "apiVersions"),
                    list(r, "resources"),
                    str_at(r, "scope").unwrap_or_else(|| "*".into()),
                ])
            })
            .collect(),
        ..Default::default()
    }
}

pub(crate) fn table_section(id: &str, title: &str, table: InfoTable, empty: &str) -> InfoSection {
    InfoSection {
        id: id.into(),
        title: title.into(),
        summary: if table.rows.is_empty() { "none".into() } else { table.rows.len().to_string() },
        rows: Vec::new(),
        table: Some(table),
        empty: (!empty.is_empty()).then(|| empty.to_owned()),
    }
}

// ---- helpers ----

/// A string, number or bool at `key`, as text.
pub(crate) fn str_at(v: &Value, key: &str) -> Option<String> {
    match &v[key] {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

pub(crate) fn int_at(v: &Value, key: &str) -> Option<i64> {
    v[key].as_i64()
}

/// kubectl selector syntax, or `None` for a missing or empty selector.
pub(crate) fn label_selector(v: &Value) -> Option<String> {
    let selector: LabelSelector = serde_json::from_value(v.clone()).ok()?;
    Some(selector_string(&selector)).filter(|s| !s.is_empty())
}

fn decoded_len(b64: &str) -> usize {
    let padding = b64.bytes().rev().take_while(|b| *b == b'=').count();
    (b64.len() / 4 * 3).saturating_sub(padding)
}

fn size_text(bytes: usize) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{:.1} KiB", b as f64 / 1024.0),
        b => format!("{:.1} MiB", b as f64 / 1024.0 / 1024.0),
    }
}

pub(crate) fn plural(n: usize, one: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {one}s") }
}

/// Key names the API accepts in ConfigMaps and Secrets.
pub fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 253
        && key != "."
        && key != ".."
        && key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// The merge patch for a data save. Values of Secrets are base64-encoded;
/// a removed ConfigMap key is cleared from `binaryData` too.
pub(crate) fn data_patch(secret: bool, resource_version: &str, changes: &[crate::types::DataChange]) -> Value {
    let mut data = serde_json::Map::new();
    let mut binary = serde_json::Map::new();
    for c in changes {
        let value = match &c.value {
            Some(v) if secret => Value::String(STANDARD.encode(v)),
            Some(v) => Value::String(v.clone()),
            None => {
                if !secret {
                    binary.insert(c.key.clone(), Value::Null);
                }
                Value::Null
            }
        };
        data.insert(c.key.clone(), value);
    }
    let mut patch = serde_json::json!({ "metadata": { "resourceVersion": resource_version }, "data": data });
    if !binary.is_empty() {
        patch["binaryData"] = Value::Object(binary);
    }
    patch
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn now() -> jiff::Timestamp {
        "2026-10-10T12:00:00Z".parse().unwrap()
    }

    fn row<'a>(d: &'a ObjectDetails, key: &str) -> Option<&'a str> {
        d.info.iter().find(|r| r.key == key).map(|r| r.value.as_str())
    }

    #[test]
    fn config_map_data_and_warnings() {
        let v = json!({
            "kind": "ConfigMap",
            "metadata": {"name": "app", "namespace": "shop", "resourceVersion": "42",
                         "labels": {"app.kubernetes.io/managed-by": "Helm"},
                         "annotations": {"meta.helm.sh/release-name": "shop", "kubectl.kubernetes.io/last-applied-configuration": "{}"}},
            "data": {"b.yaml": "x: 1\n", "a": "hello"},
            "binaryData": {"logo.png": "iVBORw0KGgo="}
        });
        let d = describe("core/ConfigMap", &v, now());
        let data = d.data.clone().unwrap();
        assert_eq!(data.entries.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(), ["a", "b.yaml", "logo.png"]);
        assert_eq!(data.entries[0].value.as_deref(), Some("hello"));
        assert_eq!((data.entries[2].binary, data.entries[2].size, data.entries[2].value.clone()), (true, 8, None));
        assert_eq!(data.resource_version, "42");
        assert!(data.warning.unwrap().contains("release shop"));
        assert_eq!(data.read_only, None);
        assert!(d.annotations.iter().all(|(k, _)| !k.contains("last-applied")));
        assert_eq!(row(&d, "Keys"), Some("3"));

        let immutable = json!({"kind": "ConfigMap", "metadata": {"name": "x"}, "immutable": true, "data": {}});
        assert!(describe("core/ConfigMap", &immutable, now()).data.unwrap().read_only.is_some());
    }

    #[test]
    fn secrets_never_carry_values() {
        let v = json!({
            "kind": "Secret", "type": "kubernetes.io/tls",
            "metadata": {"name": "tls", "namespace": "shop", "resourceVersion": "7"},
            "data": {"tls.crt": STANDARD.encode("-----BEGIN CERTIFICATE-----"), "raw": STANDARD.encode([0xff, 0xfe])}
        });
        let d = describe("core/Secret", &v, now());
        let data = d.data.clone().unwrap();
        assert!(data.secret);
        assert!(data.entries.iter().all(|e| e.value.is_none()));
        let raw = data.entries.iter().find(|e| e.key == "raw").unwrap();
        assert_eq!((raw.binary, raw.size), (true, 2));
        assert_eq!(row(&d, "Type"), Some("kubernetes.io/tls"));
        assert!(!serde_json::to_string(&data.entries).unwrap().contains("BEGIN"));

        let helm = json!({"kind": "Secret", "type": "helm.sh/release.v1", "metadata": {"name": "sh.helm.release.v1.shop.v3"}, "data": {}});
        assert!(describe("core/Secret", &helm, now()).data.unwrap().read_only.unwrap().contains("Helm"));
    }

    #[test]
    fn pods_using_a_config_map_or_secret() {
        let pod: Pod = serde_json::from_value(json!({
            "metadata": {"name": "api-1"},
            "spec": {
                "containers": [{"name": "api",
                    "env": [{"name": "A", "valueFrom": {"configMapKeyRef": {"name": "app", "key": "a"}}},
                            {"name": "P", "valueFrom": {"secretKeyRef": {"name": "db", "key": "password"}}}],
                    "envFrom": [{"secretRef": {"name": "api-env"}}]}],
                "volumes": [{"name": "cfg", "projected": {"sources": [{"configMap": {"name": "app"}}]}},
                            {"name": "tls", "secret": {"secretName": "db"}}],
                "imagePullSecrets": [{"name": "regcred"}]
            }
        }))
        .unwrap();
        assert_eq!(uses(&pod, "app", false), ["env", "volume"]);
        assert_eq!(uses(&pod, "db", true), ["env", "volume"]);
        assert_eq!(uses(&pod, "api-env", true), ["envFrom"]);
        assert_eq!(uses(&pod, "regcred", true), ["image pull"]);
        assert!(uses(&pod, "app", true).is_empty());
    }

    #[test]
    fn hpa_metrics_and_status() {
        let v = json!({
            "kind": "HorizontalPodAutoscaler",
            "metadata": {"name": "api", "namespace": "shop"},
            "spec": {"scaleTargetRef": {"apiVersion": "apps/v1", "kind": "Deployment", "name": "api"},
                     "minReplicas": 2, "maxReplicas": 10,
                     "metrics": [{"type": "Resource", "resource": {"name": "cpu", "target": {"type": "Utilization", "averageUtilization": 70}}},
                                 {"type": "Pods", "pods": {"metric": {"name": "rps"}, "target": {"type": "AverageValue", "averageValue": "100"}}}],
                     "behavior": {"scaleDown": {"stabilizationWindowSeconds": 300, "policies": [{"type": "Percent", "value": 50, "periodSeconds": 60}]}}},
            "status": {"currentReplicas": 10, "desiredReplicas": 10,
                       "currentMetrics": [{"type": "Resource", "resource": {"name": "cpu", "current": {"averageUtilization": 93, "averageValue": "465m"}}}],
                       "conditions": [{"type": "AbleToScale", "status": "True"}, {"type": "ScalingLimited", "status": "True", "reason": "TooManyReplicas"}]}
        });
        let d = describe("autoscaling/HorizontalPodAutoscaler", &v, now());
        assert_eq!(d.status.as_deref(), Some("At limit"));
        assert_eq!(d.info[0].link.as_ref().unwrap().kind, "apps/Deployment");
        assert_eq!(row(&d, "Min replicas"), Some("2"));
        let metrics = d.sections[0].table.as_ref().unwrap();
        assert_eq!(metrics.rows[0].cells, ["cpu", "93% (465m)", "70% of request"]);
        assert_eq!(metrics.rows[1].cells, ["rps (per pod)", "—", "100 average"]);
        assert_eq!(d.sections[1].rows[0].value, "waits 300s · 50% per 60s (Max)");
    }

    #[test]
    fn pdb_quota_and_lease_status() {
        let pdb = json!({"kind": "PodDisruptionBudget", "metadata": {"name": "api"},
            "spec": {"minAvailable": 2, "selector": {"matchLabels": {"app": "api"}}},
            "status": {"currentHealthy": 2, "desiredHealthy": 2, "expectedPods": 2, "disruptionsAllowed": 0}});
        let d = describe("policy/PodDisruptionBudget", &pdb, now());
        assert_eq!(d.status.as_deref(), Some("Blocking"));
        assert_eq!(row(&d, "Selector"), Some("app=api"));

        let quota = json!({"kind": "ResourceQuota", "metadata": {"name": "q"},
            "status": {"hard": {"pods": "10", "requests.memory": "1Gi"}, "used": {"pods": "10", "requests.memory": "512Mi"}}});
        let d = describe("core/ResourceQuota", &quota, now());
        assert_eq!(d.status.as_deref(), Some("At limit"));
        let rows = &d.sections[0].table.as_ref().unwrap().rows;
        assert_eq!((rows[0].cells[0].as_str(), rows[0].fill, rows[0].warn), ("pods", Some(1.0), true));
        assert_eq!((rows[1].fill, rows[1].warn), (Some(0.5), false));

        let lease = |renew: &str| {
            json!({"kind": "Lease", "metadata": {"name": "l"},
                   "spec": {"holderIdentity": "node-1", "leaseDurationSeconds": 40, "renewTime": renew}})
        };
        assert_eq!(describe("coordination.k8s.io/Lease", &lease("2026-10-10T11:59:50.000000Z"), now()).status.as_deref(), Some("Held"));
        assert_eq!(describe("coordination.k8s.io/Lease", &lease("2026-10-10T11:00:00.000000Z"), now()).status.as_deref(), Some("Expired"));
    }

    #[test]
    fn webhook_sections() {
        let v = json!({"kind": "ValidatingWebhookConfiguration", "metadata": {"name": "checks"},
            "webhooks": [{"name": "check.acme.io", "failurePolicy": "Ignore", "sideEffects": "None",
                "clientConfig": {"service": {"namespace": "acme", "name": "checker", "path": "/validate"}},
                "namespaceSelector": {},
                "rules": [{"operations": ["CREATE", "UPDATE"], "apiGroups": [""], "apiVersions": ["v1"], "resources": ["pods"]}]}]});
        let d = describe("admissionregistration.k8s.io/ValidatingWebhookConfiguration", &v, now());
        let s = &d.sections[0];
        assert_eq!((s.id.as_str(), s.summary.as_str()), ("webhook:check.acme.io", "Ignore · 1 rule"));
        assert_eq!(s.rows[0].value, "service acme/checker:443/validate");
        assert!(s.rows.iter().any(|r| r.key == "Namespaces" && r.value == "all"));
        assert_eq!(s.table.as_ref().unwrap().rows[0].cells, ["CREATE, UPDATE", "core", "v1", "pods", "*"]);
    }

    #[test]
    fn save_patches_and_keys() {
        use crate::types::DataChange;
        let changes = [
            DataChange { key: "a".into(), value: Some("1".into()) },
            DataChange { key: "old".into(), value: None },
        ];
        assert_eq!(
            data_patch(false, "9", &changes),
            json!({"metadata": {"resourceVersion": "9"}, "data": {"a": "1", "old": null}, "binaryData": {"old": null}})
        );
        assert_eq!(
            data_patch(true, "9", &changes),
            json!({"metadata": {"resourceVersion": "9"}, "data": {"a": "MQ==", "old": null}})
        );
        assert!(valid_key("app.config_v2-x"));
        assert!(!valid_key("a/b") && !valid_key("") && !valid_key("..") && !valid_key("a b"));
    }
}
