//! Turns objects into table rows, with extra columns for common kinds.

use kube::ResourceExt;
use kube::api::DynamicObject;
use serde_json::Value;

use crate::types::containers::{self, Container, State};
use crate::types::{Column, ColumnKind, KindRef, ResourceRow};

fn col(header: &str, kind: ColumnKind) -> Column {
    Column {
        header: header.to_owned(),
        kind,
    }
}

fn is_event(kind: &KindRef) -> bool {
    kind.group.is_empty() && kind.kind == "Event"
}

/// Kind-specific columns shown between Name/Namespace and Age.
fn extra_columns(kind: &KindRef) -> Vec<Column> {
    use ColumnKind::{Badge, Number, Text};
    let cols: &[(&str, ColumnKind)] = match (kind.group.as_str(), kind.kind.as_str()) {
        ("", "Pod") => &[
            ("Init", ColumnKind::Containers),
            ("Ready", ColumnKind::Containers),
            ("Status", Badge), ("Restarts", Number), ("Node", Text)],
        ("apps", "Deployment") => &[("Ready", Text), ("Up-to-date", Number), ("Available", Number)],
        ("apps", "StatefulSet") => &[("Ready", Text)],
        ("apps", "DaemonSet") => &[("Desired", Number), ("Ready", Number), ("Available", Number)],
        ("apps", "ReplicaSet") => &[("Desired", Number), ("Ready", Number)],
        ("", "Service") => &[("Type", Text), ("Cluster-IP", Text), ("Ports", Text)],
        ("", "Node") => &[("Status", Badge), ("Version", Text)],
        ("batch", "Job") => &[("Succeeded", Number), ("Failed", Number)],
        ("batch", "CronJob") => &[("Schedule", Text), ("Suspend", Text), ("Last Schedule", ColumnKind::Age)],
        _ => &[],
    };
    cols.iter().map(|(h, k)| col(h, *k)).collect()
}

/// Full column layout for a kind; matches the cells produced by [`row`].
pub fn columns(kind: &KindRef) -> Vec<Column> {
    use ColumnKind::*;
    if is_event(kind) {
        return vec![
            col("Type", Badge),
            col("Message", LongText),
            col("Namespace", Namespace),
            col("Involved Object", Text),
            col("Source", Text),
            col("Count", Number),
            col("Age", Age),
            col("Last Seen", Age),
        ];
    }
    let mut cols = vec![col("Name", Name)];
    if kind.namespaced {
        cols.push(col("Namespace", Namespace));
    }
    cols.extend(extra_columns(kind));
    cols.push(col("Age", Age));
    cols
}

pub fn row(kind: &KindRef, obj: &DynamicObject) -> ResourceRow {
    let name = obj.name_any();
    let namespace = obj.namespace();
    let created = obj.creation_timestamp().map(|t| t.0.to_string()).unwrap_or_default();
    let cells = if is_event(kind) {
        event_cells(obj, &created)
    } else {
        let mut cells = vec![name.clone()];
        if kind.namespaced {
            cells.push(namespace.clone().unwrap_or_default());
        }
        cells.extend(cells_for(kind, obj));
        cells.push(created);
        cells
    };
    ResourceRow {
        id: row_id(namespace.as_deref(), &name),
        name,
        namespace,
        cells,
    }
}

/// Puts rows in display order. Rows arrive sorted by `namespace/name`;
/// events are shown most recent first.
pub fn sort_rows(kind: &KindRef, rows: &mut [ResourceRow]) {
    if is_event(kind) {
        const LAST_SEEN: usize = 7;
        rows.sort_by(|a, b| b.cells[LAST_SEEN].cmp(&a.cells[LAST_SEEN]));
    }
}

/// Core v1 events. Newer producers fill `series`/`eventTime`/`reporting*`
/// instead of `count`/`*Timestamp`/`source`, so fall back to those.
fn event_cells(obj: &DynamicObject, created: &str) -> Vec<String> {
    let data = &obj.data;
    let involved = &data["involvedObject"];
    let object = match (involved["kind"].as_str(), involved["name"].as_str()) {
        (Some(kind), Some(name)) => format!("{kind}/{name}"),
        (_, Some(name)) => name.to_owned(),
        _ => String::new(),
    };
    let source = data["source"]["component"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| data["reportingComponent"].as_str().filter(|s| !s.is_empty()))
        .map(|component| match data["source"]["host"].as_str() {
            Some(host) if !host.is_empty() => format!("{component}, {host}"),
            _ => component.to_owned(),
        })
        .unwrap_or_default();
    let count = data["count"]
        .as_i64()
        .or_else(|| data["series"]["count"].as_i64())
        .unwrap_or(1);
    let first_seen = first_str(&[&data["firstTimestamp"], &data["eventTime"]]).unwrap_or(created);
    let last_seen = first_str(&[&data["lastTimestamp"], &data["series"]["lastObservedTime"], &data["eventTime"]])
        .unwrap_or(first_seen);
    vec![
        text(&data["type"]),
        text(&data["message"]).trim().to_owned(),
        obj.namespace().unwrap_or_default(),
        object,
        source,
        count.to_string(),
        first_seen.to_owned(),
        last_seen.to_owned(),
    ]
}

pub(crate) fn first_str<'a>(values: &[&'a Value]) -> Option<&'a str> {
    values.iter().find_map(|v| v.as_str().filter(|s| !s.is_empty()))
}

pub fn row_id(namespace: Option<&str>, name: &str) -> String {
    match namespace {
        Some(ns) => format!("{ns}/{name}"),
        None => name.to_owned(),
    }
}

/// Values for [`extra_columns`].
fn cells_for(kind: &KindRef, obj: &DynamicObject) -> Vec<String> {
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
    let restarts: i64 = statuses.iter().map(|c| int(&c["restartCount"])).sum();

    let (init, app): (Vec<_>, Vec<_>) = container_states(data).into_iter().partition(|c| c.init);
    vec![
        containers::encode(&init),
        containers::encode(&app),
        pod_status(data, terminating),
        restarts.to_string(),
        text(&data["spec"]["nodeName"]),
    ]
}

/// Waiting reasons that mean a container is on its way up rather than stuck.
const STARTING_REASONS: &[&str] = &["ContainerCreating", "PodInitializing"];

/// State of each init container, then each app container, in spec order.
pub(crate) fn container_states(data: &Value) -> Vec<Container> {
    let spec = &data["spec"];
    let status = &data["status"];
    let mut all = states_of(&spec["initContainers"], &status["initContainerStatuses"], true);
    all.extend(states_of(&spec["containers"], &status["containerStatuses"], false));
    all
}

fn states_of(specs: &Value, statuses: &Value, init: bool) -> Vec<Container> {
    let statuses = statuses.as_array().map(Vec::as_slice).unwrap_or_default();
    specs
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|spec| {
            let (state, detail) = match statuses.iter().find(|s| s["name"] == spec["name"]) {
                None => (State::Starting, "Pending".to_owned()),
                Some(s) if init => init_container_state(s),
                Some(s) => container_state(s),
            };
            Container {
                name: text(&spec["name"]),
                state,
                detail,
                init,
            }
        })
        .collect()
}

/// Init containers have no readiness of their own: running is the expected
/// state, and exiting 0 is success.
fn init_container_state(status: &Value) -> (State, String) {
    if status["state"]["running"].is_object() {
        return (State::Running, "Running".to_owned());
    }
    container_state(status)
}

fn container_state(status: &Value) -> (State, String) {
    let state = &status["state"];
    if state["running"].is_object() {
        return if status["ready"].as_bool() == Some(true) {
            (State::Running, "Running".to_owned())
        } else {
            (State::Starting, "Running, not ready".to_owned())
        };
    }
    if state["waiting"].is_object() {
        let reason = state["waiting"]["reason"].as_str().unwrap_or("Waiting");
        return if STARTING_REASONS.contains(&reason) {
            (State::Starting, reason.to_owned())
        } else {
            (State::Failed, reason.to_owned())
        };
    }
    if state["terminated"].is_object() {
        let terminated = &state["terminated"];
        let reason = terminated_reason(terminated);
        let detail = format!("{reason} (exit {})", int(&terminated["exitCode"]));
        return if int(&terminated["exitCode"]) == 0 {
            (State::Stopped, detail)
        } else {
            (State::Failed, detail)
        };
    }
    (State::Starting, "Unknown".to_owned())
}

/// Pod status, following kubectl's rules with one change: a container that
/// exited 0 counts as completed even when containerd reports `OOMKilled`.
/// That happens when a child process was OOM-killed but the main process
/// still succeeded; it shows as `Completed (OOM)` so it stays visible.
pub(crate) fn pod_status(data: &Value, terminating: bool) -> String {
    let status = &data["status"];
    let mut reason = status["reason"]
        .as_str()
        .filter(|r| !r.is_empty())
        .or_else(|| status["phase"].as_str())
        .unwrap_or_default()
        .to_owned();

    // Init containers: report the first one that hasn't finished successfully.
    let inits = status["initContainerStatuses"].as_array().map(Vec::as_slice).unwrap_or_default();
    for (i, c) in inits.iter().enumerate() {
        let state = &c["state"];
        let terminated = &state["terminated"];
        if terminated.is_object() {
            if int(&terminated["exitCode"]) == 0 {
                continue;
            }
            return format!("Init:{}", terminated_reason(terminated));
        }
        if let Some(waiting) = state["waiting"]["reason"].as_str().filter(|r| !r.is_empty() && *r != "PodInitializing") {
            return format!("Init:{waiting}");
        }
        return format!("Init:{i}/{}", inits.len());
    }

    let statuses = status["containerStatuses"].as_array().map(Vec::as_slice).unwrap_or_default();
    let container_reason = statuses.iter().find_map(|c| {
        let state = &c["state"];
        let waiting = state["waiting"]["reason"].as_str().filter(|r| !r.is_empty());
        waiting
            .map(str::to_owned)
            .or_else(|| state["terminated"].is_object().then(|| terminated_reason(&state["terminated"])))
    });
    if let Some(r) = container_reason {
        reason = r;
    }
    let any_running = statuses
        .iter()
        .any(|c| c["ready"].as_bool() == Some(true) && c["state"]["running"].is_object());
    if reason == "Completed" && any_running {
        reason = "Running".to_owned();
    }

    if terminating {
        if status["reason"] == "NodeLost" { "Unknown".to_owned() } else { "Terminating".to_owned() }
    } else {
        reason
    }
}

fn terminated_reason(terminated: &Value) -> String {
    let exit_code = int(&terminated["exitCode"]);
    match terminated["reason"].as_str().filter(|r| !r.is_empty()) {
        Some("OOMKilled") if exit_code == 0 => "Completed (OOM)".to_owned(),
        Some(reason) => reason.to_owned(),
        None => match int(&terminated["signal"]) {
            0 => format!("ExitCode:{exit_code}"),
            signal => format!("Signal:{signal}"),
        },
    }
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

pub(crate) fn int(v: &Value) -> i64 {
    v.as_i64().unwrap_or(0)
}

pub(crate) fn text(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event_kind() -> KindRef {
        KindRef {
            group: String::new(),
            version: "v1".into(),
            kind: "Event".into(),
            plural: "events".into(),
            namespaced: true,
        }
    }

    fn event(json: serde_json::Value) -> DynamicObject {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn legacy_event_fields() {
        let obj = event(serde_json::json!({
            "apiVersion": "v1", "kind": "Event",
            "metadata": {"name": "api.1", "namespace": "shop", "creationTimestamp": "2026-10-01T00:00:00Z"},
            "type": "Warning",
            "message": "Back-off restarting failed container\n",
            "involvedObject": {"kind": "Pod", "name": "api-7d9"},
            "source": {"component": "kubelet", "host": "node-1"},
            "count": 12,
            "firstTimestamp": "2026-10-01T00:00:05Z",
            "lastTimestamp": "2026-10-01T01:00:00Z"
        }));
        let row = row(&event_kind(), &obj);
        assert_eq!(row.cells.len(), columns(&event_kind()).len());
        assert_eq!(
            row.cells,
            [
                "Warning",
                "Back-off restarting failed container",
                "shop",
                "Pod/api-7d9",
                "kubelet, node-1",
                "12",
                "2026-10-01T00:00:05Z",
                "2026-10-01T01:00:00Z"
            ]
        );
    }

    #[test]
    fn new_style_event_fields() {
        let obj = event(serde_json::json!({
            "apiVersion": "v1", "kind": "Event",
            "metadata": {"name": "web.2", "namespace": "shop", "creationTimestamp": "2026-10-01T00:00:00Z"},
            "type": "Normal",
            "message": "Scaled up",
            "involvedObject": {"kind": "Deployment", "name": "web"},
            "reportingComponent": "deployment-controller",
            "eventTime": "2026-10-01T00:00:01.000000Z",
            "series": {"count": 3, "lastObservedTime": "2026-10-01T00:10:00.000000Z"}
        }));
        let cells = row(&event_kind(), &obj).cells;
        assert_eq!(cells[4], "deployment-controller");
        assert_eq!(cells[5], "3");
        assert_eq!(cells[6], "2026-10-01T00:00:01.000000Z");
        assert_eq!(cells[7], "2026-10-01T00:10:00.000000Z");
    }

    fn pod_kind() -> KindRef {
        KindRef {
            kind: "Pod".into(),
            plural: "pods".into(),
            ..event_kind()
        }
    }

    fn pod_status_of(status: serde_json::Value) -> String {
        let obj = event(serde_json::json!({
            "apiVersion": "v1", "kind": "Pod",
            "metadata": {"name": "p", "namespace": "ns"},
            "spec": {"containers": [{"name": "main"}]},
            "status": status
        }));
        row(&pod_kind(), &obj).cells[4].clone()
    }

    #[test]
    fn pod_oom_child_but_exit_zero_is_completed() {
        // Real case: migration job whose script survived an OOM-killed child.
        let status = pod_status_of(serde_json::json!({
            "phase": "Succeeded",
            "containerStatuses": [{"name": "main", "ready": false, "restartCount": 0,
                "state": {"terminated": {"exitCode": 0, "reason": "OOMKilled"}}}]
        }));
        assert_eq!(status, "Completed (OOM)");
    }

    #[test]
    fn pod_statuses_match_kubectl() {
        let terminated = |code: i64, reason: Option<&str>| {
            let mut t = serde_json::json!({"exitCode": code});
            if let Some(r) = reason {
                t["reason"] = r.into();
            }
            pod_status_of(serde_json::json!({
                "phase": "Failed",
                "containerStatuses": [{"name": "main", "state": {"terminated": t}}]
            }))
        };
        assert_eq!(terminated(137, Some("OOMKilled")), "OOMKilled");
        assert_eq!(terminated(0, Some("Completed")), "Completed");
        assert_eq!(terminated(2, None), "ExitCode:2");
        assert_eq!(
            pod_status_of(serde_json::json!({
                "phase": "Running",
                "containerStatuses": [{"name": "main", "state": {"waiting": {"reason": "CrashLoopBackOff"}}}]
            })),
            "CrashLoopBackOff"
        );
        assert_eq!(
            pod_status_of(serde_json::json!({
                "phase": "Pending",
                "initContainerStatuses": [
                    {"name": "a", "state": {"terminated": {"exitCode": 0, "reason": "Completed"}}},
                    {"name": "b", "state": {"running": {}}}
                ]
            })),
            "Init:1/2"
        );
        assert_eq!(
            pod_status_of(serde_json::json!({"phase": "Failed", "reason": "Evicted"})),
            "Evicted"
        );
    }

    fn pod_containers(status: serde_json::Value) -> Vec<(State, String)> {
        let obj = event(serde_json::json!({
            "apiVersion": "v1", "kind": "Pod",
            "metadata": {"name": "p", "namespace": "ns"},
            "spec": {"containers": [{"name": "app"}, {"name": "proxy"}, {"name": "job"}, {"name": "new"}]},
            "status": status
        }));
        containers::decode(&row(&pod_kind(), &obj).cells[3])
            .into_iter()
            .map(|c| (c.state, c.name))
            .collect()
    }

    #[test]
    fn container_icons() {
        let states = pod_containers(serde_json::json!({
            "phase": "Running",
            "containerStatuses": [
                {"name": "proxy", "ready": false, "state": {"waiting": {"reason": "CrashLoopBackOff"}}},
                {"name": "app", "ready": true, "state": {"running": {}}},
                {"name": "job", "ready": false, "state": {"terminated": {"exitCode": 0, "reason": "Completed"}}},
                {"name": "new", "ready": false, "state": {"waiting": {"reason": "ContainerCreating"}}}
            ]
        }));
        assert_eq!(
            states,
            [
                (State::Running, "app".to_owned()),
                (State::Failed, "proxy".to_owned()),
                (State::Stopped, "job".to_owned()),
                (State::Starting, "new".to_owned()),
            ]
        );
        // Running but failing readiness counts as starting; no status yet too.
        let states = pod_containers(serde_json::json!({
            "phase": "Pending",
            "containerStatuses": [{"name": "app", "ready": false, "state": {"running": {}}}]
        }));
        assert_eq!(states[0].0, State::Starting);
        assert_eq!(states[1].0, State::Starting);
    }

    #[test]
    fn init_containers_in_their_own_column() {
        let obj = event(serde_json::json!({
            "apiVersion": "v1", "kind": "Pod",
            "metadata": {"name": "p", "namespace": "ns"},
            "spec": {
                "initContainers": [{"name": "migrate"}, {"name": "warm"}, {"name": "wait"}],
                "containers": [{"name": "app"}]
            },
            "status": {
                "phase": "Pending",
                "initContainerStatuses": [
                    {"name": "migrate", "ready": false, "state": {"terminated": {"exitCode": 0, "reason": "Completed"}}},
                    {"name": "warm", "ready": false, "state": {"running": {}}},
                    {"name": "wait", "ready": false, "state": {"waiting": {"reason": "PodInitializing"}}}
                ],
                "containerStatuses": [{"name": "app", "ready": false, "state": {"waiting": {"reason": "PodInitializing"}}}]
            }
        }));
        let cells = row(&pod_kind(), &obj).cells;
        let got: Vec<_> = [&cells[2], &cells[3]]
            .into_iter()
            .flat_map(|cell| containers::decode(cell))
            .map(|c| (c.name, c.state, c.init))
            .collect();
        // Init column holds only init containers, Ready only app containers.
        assert!(containers::decode(&cells[2]).iter().all(|c| c.init));
        assert!(containers::decode(&cells[3]).iter().all(|c| !c.init));
        assert_eq!(
            got,
            [
                ("migrate".to_owned(), State::Stopped, true),
                ("warm".to_owned(), State::Running, true),
                ("wait".to_owned(), State::Starting, true),
                ("app".to_owned(), State::Starting, false),
            ]
        );
    }

    #[test]
    fn events_sorted_most_recent_first() {
        let mut rows: Vec<ResourceRow> = ["2026-10-01T01:00:00Z", "2026-10-01T03:00:00Z", "2026-10-01T02:00:00Z"]
            .iter()
            .map(|t| ResourceRow {
                id: t.to_string(),
                name: String::new(),
                namespace: None,
                cells: vec![String::new(); 7].into_iter().chain([t.to_string()]).collect(),
            })
            .collect();
        sort_rows(&event_kind(), &mut rows);
        let order: Vec<_> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(order, ["2026-10-01T03:00:00Z", "2026-10-01T02:00:00Z", "2026-10-01T01:00:00Z"]);
    }
}
