//! Turns objects into table rows, with extra columns for common kinds.

use kube::ResourceExt;
use kube::api::DynamicObject;
use serde_json::Value;

use crate::types::{KindRef, ResourceRow};

/// Extra column headers for a kind (Name / Namespace / Age are always shown).
pub fn columns(kind: &KindRef) -> Vec<String> {
    let cols: &[&str] = match (kind.group.as_str(), kind.kind.as_str()) {
        ("", "Pod") => &["Ready", "Status", "Restarts", "Node"],
        ("apps", "Deployment") => &["Ready", "Up-to-date", "Available"],
        ("apps", "StatefulSet") => &["Ready"],
        ("apps", "DaemonSet") => &["Desired", "Ready", "Available"],
        ("apps", "ReplicaSet") => &["Desired", "Ready"],
        ("", "Service") => &["Type", "Cluster-IP", "Ports"],
        ("", "Node") => &["Status", "Version"],
        ("batch", "Job") => &["Succeeded", "Failed"],
        ("batch", "CronJob") => &["Schedule", "Suspend", "Last Schedule"],
        _ => &[],
    };
    cols.iter().map(|c| c.to_string()).collect()
}

pub fn row(kind: &KindRef, obj: &DynamicObject) -> ResourceRow {
    let name = obj.name_any();
    let namespace = obj.namespace();
    ResourceRow {
        id: row_id(namespace.as_deref(), &name),
        cells: cells(kind, obj),
        created: obj.creation_timestamp().map(|t| t.0.to_string()),
        name,
        namespace,
    }
}

pub fn row_id(namespace: Option<&str>, name: &str) -> String {
    match namespace {
        Some(ns) => format!("{ns}/{name}"),
        None => name.to_owned(),
    }
}

fn cells(kind: &KindRef, obj: &DynamicObject) -> Vec<String> {
    let data = &obj.data;
    let spec = &data["spec"];
    let status = &data["status"];
    match (kind.group.as_str(), kind.kind.as_str()) {
        ("", "Pod") => pod_cells(data, obj.metadata.deletion_timestamp.is_some()),
        ("apps", "Deployment") => vec![
            format!("{}/{}", int(&status["readyReplicas"]), int(&spec["replicas"])),
            int(&status["updatedReplicas"]).to_string(),
            int(&status["availableReplicas"]).to_string(),
        ],
        ("apps", "StatefulSet") => vec![format!("{}/{}", int(&status["readyReplicas"]), int(&spec["replicas"]))],
        ("apps", "DaemonSet") => vec![
            int(&status["desiredNumberScheduled"]).to_string(),
            int(&status["numberReady"]).to_string(),
            int(&status["numberAvailable"]).to_string(),
        ],
        ("apps", "ReplicaSet") => vec![int(&spec["replicas"]).to_string(), int(&status["readyReplicas"]).to_string()],
        ("", "Service") => vec![
            text(&spec["type"]),
            text(&spec["clusterIP"]),
            spec["ports"]
                .as_array()
                .map(|ports| {
                    ports
                        .iter()
                        .map(|p| format!("{}/{}", int(&p["port"]), p["protocol"].as_str().unwrap_or("TCP")))
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default(),
        ],
        ("", "Node") => vec![node_status(data), text(&status["nodeInfo"]["kubeletVersion"])],
        ("batch", "Job") => vec![int(&status["succeeded"]).to_string(), int(&status["failed"]).to_string()],
        ("batch", "CronJob") => vec![
            text(&spec["schedule"]),
            spec["suspend"].as_bool().unwrap_or(false).to_string(),
            text(&status["lastScheduleTime"]),
        ],
        _ => vec![],
    }
}

fn pod_cells(data: &Value, terminating: bool) -> Vec<String> {
    let status = &data["status"];
    let statuses = status["containerStatuses"].as_array().map(Vec::as_slice).unwrap_or_default();
    let total = data["spec"]["containers"].as_array().map_or(0, Vec::len);
    let ready = statuses.iter().filter(|c| c["ready"].as_bool() == Some(true)).count();
    let restarts: i64 = statuses.iter().map(|c| int(&c["restartCount"])).sum();

    // Like kubectl: a waiting/terminated reason beats the phase.
    let reason = statuses
        .iter()
        .find_map(|c| {
            c["state"]["waiting"]["reason"]
                .as_str()
                .or_else(|| c["state"]["terminated"]["reason"].as_str())
        })
        .map(str::to_owned);
    let phase = if terminating {
        "Terminating".to_owned()
    } else {
        reason.unwrap_or_else(|| text(&status["phase"]))
    };

    vec![
        format!("{ready}/{total}"),
        phase,
        restarts.to_string(),
        text(&data["spec"]["nodeName"]),
    ]
}

fn node_status(data: &Value) -> String {
    let ready = data["status"]["conditions"]
        .as_array()
        .and_then(|conds| conds.iter().find(|c| c["type"] == "Ready"))
        .map(|c| c["status"] == "True");
    let mut status = match ready {
        Some(true) => "Ready".to_owned(),
        Some(false) => "NotReady".to_owned(),
        None => "Unknown".to_owned(),
    };
    if data["spec"]["unschedulable"].as_bool() == Some(true) {
        status.push_str(",SchedulingDisabled");
    }
    status
}

fn int(v: &Value) -> i64 {
    v.as_i64().unwrap_or(0)
}

fn text(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_owned()
}
