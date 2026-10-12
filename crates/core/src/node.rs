//! The Details tab of a Node: its state, where it runs, addresses, taints and
//! system info; what its pods ask for and use against what it can hold (the
//! last hour's usage comes from `HistoryStore::node_history`); the
//! pods on it, each opening its own Details; and its events.

use k8s_openapi::api::core::v1::{Node, Pod};
use kube::api::{Api, ListParams};
use kube::{Client, ResourceExt};
use serde_json::Value;

use crate::config::{plural, str_at, table_section};
use crate::overview::{PodResources, node_summary, pod_resources};
use crate::types::{InfoRow, InfoTable, InfoTableRow, NodeResources, ObjectDetails, ObjectLink};
use crate::Result;

/// `Ready`, `NotReady` or `Unknown`; `Cordoned` when ready but closed to new pods.
pub(crate) fn node_status(v: &Value) -> String {
    let ready = v["status"]["conditions"]
        .as_array()
        .and_then(|c| c.iter().find(|c| c["type"] == "Ready"))
        .map(|c| c["status"] == "True");
    match ready {
        Some(true) if v["spec"]["unschedulable"] == true => "Cordoned",
        Some(true) => "Ready",
        Some(false) => "NotReady",
        None => "Unknown",
    }
    .to_owned()
}

/// Fills in what comes from the Node object itself.
pub(crate) fn describe(key: &str, d: &mut ObjectDetails, v: &Value) {
    if key != "core/Node" {
        return;
    }
    d.status = Some(node_status(v));
    let (spec, status, labels) = (&v["spec"], &v["status"], &v["metadata"]["labels"]);
    if let Ok(node) = serde_json::from_value::<Node>(v.clone()) {
        let s = node_summary(&node, 0, PodResources::default(), None);
        if !s.roles.is_empty() {
            d.info.push(InfoRow::new("Roles", s.roles.join(", ")));
        }
        if let Some(t) = s.instance_type {
            d.info.push(InfoRow::new("Instance type", t));
        }
    }
    let place: Vec<String> = ["topology.kubernetes.io/zone", "topology.kubernetes.io/region"]
        .iter()
        .filter_map(|k| str_at(labels, k))
        .collect();
    if !place.is_empty() {
        d.info.push(InfoRow::new("Zone", place.join(" · ")));
    }
    if spec["unschedulable"] == true {
        d.info.push(InfoRow::new("Scheduling", "cordoned: no new pods are placed here"));
    }
    for a in status["addresses"].as_array().into_iter().flatten() {
        if let (Some(t), Some(addr)) = (str_at(a, "type"), str_at(a, "address")) {
            d.info.push(InfoRow::new(address_label(&t), addr).mono());
        }
    }
    let taints: Vec<String> = spec["taints"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| {
            let key = str_at(t, "key").unwrap_or_default();
            let value = str_at(t, "value").filter(|v| !v.is_empty()).map(|v| format!("={v}")).unwrap_or_default();
            format!("{key}{value}:{}", str_at(t, "effect").unwrap_or_default())
        })
        .collect();
    d.info.push(if taints.is_empty() {
        InfoRow::new("Taints", "none")
    } else {
        InfoRow::new("Taints", taints.join("\n")).mono()
    });
    let info = &status["nodeInfo"];
    for (label, field) in [
        ("Kubelet", "kubeletVersion"),
        ("OS", "osImage"),
        ("Kernel", "kernelVersion"),
        ("Container runtime", "containerRuntimeVersion"),
        ("Architecture", "architecture"),
    ] {
        if let Some(value) = str_at(info, field).filter(|v| !v.is_empty()) {
            d.info.push(InfoRow::new(label, value));
        }
    }
    let cidrs: Vec<String> =
        spec["podCIDRs"].as_array().into_iter().flatten().filter_map(|c| c.as_str().map(str::to_owned)).collect();
    if !cidrs.is_empty() {
        d.info.push(InfoRow::new("Pod CIDR", cidrs.join(", ")).mono());
    }
    if let Some(id) = str_at(spec, "providerID") {
        d.info.push(InfoRow::new("Provider ID", id).mono());
    }
}

fn address_label(t: &str) -> String {
    match t {
        "InternalIP" => "Internal IP".into(),
        "ExternalIP" => "External IP".into(),
        "InternalDNS" => "Internal DNS".into(),
        "ExternalDNS" => "External DNS".into(),
        "Hostname" => "Hostname".into(),
        other => other.into(),
    }
}

pub(crate) fn cpu_text(m: f64) -> String {
    if m >= 1000.0 { format!("{:.2} cores", m / 1000.0) } else { format!("{m:.0}m") }
}

/// Adds what needs other objects: resources, the pods on it and its events.
pub(crate) async fn related(client: &Client, key: &str, name: &str, v: &Value, d: &mut ObjectDetails) -> Result<()> {
    if key != "core/Node" {
        return Ok(());
    }
    // Pods still holding resources here; finished ones are left out, like `kubectl describe node`.
    let fields = format!("spec.nodeName={name},status.phase!=Succeeded,status.phase!=Failed");
    let fields_ref = format!("involvedObject.kind=Node,involvedObject.name={name}");
    let (pods_api, lp) = (Api::<Pod>::all(client.clone()), ListParams::default().fields(&fields));
    let (pods, summary, events) = tokio::join!(
        pods_api.list(&lp),
        crate::usage::kubelet_summary(client, name),
        crate::pod_details::list_events(client, "", Some(&fields_ref), |_| Some(None)),
    );
    let mut pods = pods?.items;
    pods.sort_by_key(|p| (p.namespace(), p.name_any()));

    // Usage: the kubelet's stats give the node and each pod; metrics-server the node only.
    let used = match &summary {
        Some(s) => Some(crate::usage::node_total(s)),
        None => node_metrics(client, name).await,
    };

    // Resources, charted at the top of the tab.
    let asked = pods.iter().filter_map(|p| p.spec.as_ref()).map(pod_resources).fold(PodResources::default(), |a, r| {
        PodResources {
            cpu_requests: a.cpu_requests + r.cpu_requests,
            cpu_limits: a.cpu_limits + r.cpu_limits,
            memory_requests: a.memory_requests + r.memory_requests,
            memory_limits: a.memory_limits + r.memory_limits,
        }
    });
    if let Ok(node) = serde_json::from_value::<Node>(v.clone()) {
        let s = node_summary(&node, pods.len(), asked, used);
        let one_pod_hosting = (s.instance_type.as_deref() == Some("fargate")).then(|| "AWS Fargate".to_owned());
        d.node = Some(NodeResources { cpu: s.cpu, memory: s.memory, pods: s.pods, pod_capacity: s.pod_capacity, one_pod_hosting });
    }

    // The pods on it. CPU and memory read "used / requested" when the
    // kubelet's stats are available, else just what's requested.
    let with_usage = summary.is_some();
    let both = |used: Option<f64>, asked: f64, fmt: &dyn Fn(f64) -> String| match (with_usage, used) {
        (false, _) => fmt(asked),
        (true, Some(u)) => format!("{} / {}", fmt(u), fmt(asked)),
        (true, None) => format!("— / {}", fmt(asked)),
    };
    let rows: Vec<InfoTableRow> = pods
        .iter()
        .map(|p| {
            let (ns, pod_name) = (p.namespace().unwrap_or_default(), p.name_any());
            let data = serde_json::to_value(p).unwrap_or_default();
            let status = crate::table::pod_status(&data, p.metadata.deletion_timestamp.is_some());
            let statuses = p.status.as_ref().and_then(|s| s.container_statuses.as_ref());
            let restarts: i32 = statuses.into_iter().flatten().map(|c| c.restart_count).sum();
            let res = p.spec.as_ref().map(pod_resources).unwrap_or_default();
            let used = summary
                .as_ref()
                .and_then(|s| crate::usage::pod_usage(s, &ns, &pod_name))
                .map(|c| c.values().fold((0.0, 0.0), |a, u| (a.0 + u.0, a.1 + u.1)));
            let mut row = InfoTableRow::new(vec![
                pod_name.clone(),
                ns.clone(),
                status.clone(),
                restarts.to_string(),
                both(used.map(|u| u.0), res.cpu_requests, &cpu_text),
                both(used.map(|u| u.1), res.memory_requests, &crate::storage::bytes_text),
                p.creation_timestamp().map(|t| t.0.to_string()).unwrap_or_default(),
            ]);
            row.link = Some(ObjectLink { kind: "core/Pod".into(), namespace: Some(ns), name: pod_name });
            row.warn = !matches!(status.as_str(), "Running" | "Completed") || !pod_ready(p);
            row
        })
        .collect();
    let (cpu, memory) = if with_usage { ("CPU used / req.", "Memory used / req.") } else { ("CPU requested", "Memory requested") };
    let columns = ["Pod", "Namespace", "Status", "Restarts", cpu, memory, "Age"].map(String::from).to_vec();
    let (numeric, times) = (vec![3, 4, 5], vec![6]);
    let not_ready = rows.iter().filter(|r| r.warn).count();
    let mut section = table_section(
        "node-pods",
        "Pods",
        InfoTable { columns, numeric, times, rows, ..Default::default() },
        "No pods are running on it.",
    );
    if !pods.is_empty() {
        section.summary = match not_ready {
            0 => plural(pods.len(), "pod"),
            n => format!("{} · {n} not ready", plural(pods.len(), "pod")),
        };
    }
    d.sections.push(section);

    d.events = events;
    Ok(())
}

/// Every container ready (pods without statuses yet aren't).
fn pod_ready(p: &Pod) -> bool {
    p.status
        .as_ref()
        .and_then(|s| s.container_statuses.as_ref())
        .is_some_and(|c| !c.is_empty() && c.iter().all(|c| c.ready))
}

/// One node's (cpu millicores, memory bytes) from metrics-server.
pub(crate) async fn node_metrics(client: &Client, name: &str) -> Option<(f64, f64)> {
    let m = crate::usage::metrics_get(client, &format!("nodes/{name}"), None).await?;
    let usage = &m["usage"];
    Some((
        usage["cpu"].as_str().map_or(0.0, crate::quantity::millicores),
        usage["memory"].as_str().map_or(0.0, crate::quantity::bytes),
    ))
}

#[cfg(test)]
#[path = "node_tests.rs"]
mod tests;
