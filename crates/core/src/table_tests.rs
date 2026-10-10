use super::*;
use crate::testing::{kind, object, pod};
use serde_json::json;

fn dynamic(v: Value) -> DynamicObject {
    serde_json::from_value(v).unwrap()
}

fn headers(k: &KindRef) -> Vec<String> {
    columns(k).into_iter().map(|c| c.header).collect()
}

/// Every row has exactly one cell per column.
fn checked_row(k: &KindRef, obj: Value) -> ResourceRow {
    let row = row(k, &dynamic(obj));
    assert_eq!(row.cells.len(), columns(k).len(), "{}", k.kind);
    row
}

#[test]
fn columns_per_kind() {
    let deploy = kind("apps", "v1", "Deployment", "deployments", true);
    assert_eq!(headers(&deploy), ["Name", "Namespace", "Ready", "Up-to-date", "Available", "Age"]);
    let node = kind("", "v1", "Node", "nodes", false);
    assert_eq!(headers(&node), ["Name", "Status", "Version", "Age"]);
    let cm = kind("", "v1", "ConfigMap", "configmaps", true);
    assert_eq!(headers(&cm), ["Name", "Namespace", "Age"]);
    let ns = kind("", "v1", "Namespace", "namespaces", false);
    assert_eq!(headers(&ns), ["Name", "Age"]);
    // An "Event" outside the core group is an ordinary kind.
    let ev2 = kind("events.k8s.io", "v1", "Event", "events", true);
    assert_eq!(headers(&ev2), ["Name", "Namespace", "Age"]);
    let cron = kind("batch", "v1", "CronJob", "cronjobs", true);
    let cols = columns(&cron);
    assert_eq!(cols[4].kind, ColumnKind::Age);
    assert_eq!(cols[4].header, "Last Schedule");
}

#[test]
fn workload_cells() {
    let deploy = kind("apps", "v1", "Deployment", "deployments", true);
    let row = checked_row(&deploy, object("apps/v1", "Deployment", Some("shop"), "web", json!({
        "spec": {"replicas": 3},
        "status": {"readyReplicas": 2, "updatedReplicas": 3, "availableReplicas": 1}
    })));
    assert_eq!(row.id, "shop/web");
    assert_eq!(row.namespace.as_deref(), Some("shop"));
    assert_eq!(row.cells, ["web", "shop", "2/3", "3", "1", "2026-10-01T00:00:00Z"]);

    let sts = kind("apps", "v1", "StatefulSet", "statefulsets", true);
    let row = checked_row(&sts, object("apps/v1", "StatefulSet", Some("shop"), "db", json!({"spec": {"replicas": 2}})));
    assert_eq!(row.cells[2], "0/2");

    let ds = kind("apps", "v1", "DaemonSet", "daemonsets", true);
    let row = checked_row(&ds, object("apps/v1", "DaemonSet", Some("shop"), "agent", json!({
        "status": {"desiredNumberScheduled": 5, "numberReady": 4, "numberAvailable": 3}
    })));
    assert_eq!(&row.cells[2..5], ["5", "4", "3"]);

    let rs = kind("apps", "v1", "ReplicaSet", "replicasets", true);
    let row = checked_row(&rs, object("apps/v1", "ReplicaSet", Some("shop"), "web-1", json!({
        "spec": {"replicas": 2}, "status": {"readyReplicas": 1}
    })));
    assert_eq!(&row.cells[2..4], ["2", "1"]);

    let job = kind("batch", "v1", "Job", "jobs", true);
    let row = checked_row(&job, object("batch/v1", "Job", Some("shop"), "once", json!({
        "status": {"succeeded": 1, "failed": 2}
    })));
    assert_eq!(&row.cells[2..4], ["1", "2"]);

    let cron = kind("batch", "v1", "CronJob", "cronjobs", true);
    let row = checked_row(&cron, object("batch/v1", "CronJob", Some("shop"), "nightly", json!({
        "spec": {"schedule": "0 3 * * *", "suspend": true},
        "status": {"lastScheduleTime": "2026-10-09T03:00:00Z"}
    })));
    assert_eq!(&row.cells[2..5], ["0 3 * * *", "true", "2026-10-09T03:00:00Z"]);
    let row = checked_row(&cron, object("batch/v1", "CronJob", Some("shop"), "hourly", json!({"spec": {"schedule": "@hourly"}})));
    assert_eq!(&row.cells[2..5], ["@hourly", "false", ""]);
}

#[test]
fn service_cells() {
    let svc = kind("", "v1", "Service", "services", true);
    let row = checked_row(&svc, object("v1", "Service", Some("shop"), "web", json!({
        "spec": {"type": "ClusterIP", "clusterIP": "10.0.0.1",
                 "ports": [{"port": 80}, {"port": 53, "protocol": "UDP"}]}
    })));
    assert_eq!(&row.cells[2..5], ["ClusterIP", "10.0.0.1", "80/TCP,53/UDP"]);
    let row = checked_row(&svc, object("v1", "Service", Some("shop"), "bare", json!({"spec": {}})));
    assert_eq!(&row.cells[2..5], ["", "", ""]);
}

#[test]
fn node_cells() {
    let node = kind("", "v1", "Node", "nodes", false);
    let ready = |status: &str, unschedulable: bool| {
        let row = checked_row(&node, object("v1", "Node", None, "node-1", json!({
            "spec": {"unschedulable": unschedulable},
            "status": {"conditions": [{"type": "MemoryPressure", "status": "False"}, {"type": "Ready", "status": status}],
                       "nodeInfo": {"kubeletVersion": "v1.31.2"}}
        })));
        assert_eq!(row.id, "node-1");
        assert_eq!(row.namespace, None);
        assert_eq!(row.cells[2], "v1.31.2");
        row.cells[1].clone()
    };
    assert_eq!(ready("True", false), "Ready");
    assert_eq!(ready("False", false), "NotReady");
    assert_eq!(ready("True", true), "Ready,SchedulingDisabled");
    let row = checked_row(&node, object("v1", "Node", None, "node-2", json!({})));
    assert_eq!(row.cells[1], "Unknown");
}

#[test]
fn other_kinds_have_no_extra_cells() {
    let cm = kind("", "v1", "ConfigMap", "configmaps", true);
    let row = checked_row(&cm, object("v1", "ConfigMap", Some("shop"), "app", json!({})));
    assert_eq!(row.cells, ["app", "shop", "2026-10-01T00:00:00Z"]);
    // No creation timestamp: an empty Age cell.
    let row = checked_row(&cm, json!({"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": "x", "namespace": "shop"}}));
    assert_eq!(row.cells[2], "");
}

fn event_kind() -> KindRef {
    kind("", "v1", "Event", "events", true)
}

fn event_cells_of(extra: Value) -> Vec<String> {
    let mut v = json!({"apiVersion": "v1", "kind": "Event",
        "metadata": {"name": "e.1", "namespace": "shop", "creationTimestamp": "2026-10-01T00:00:00Z"}});
    crate::testing::merge(&mut v, extra);
    checked_row(&event_kind(), v).cells
}

#[test]
fn event_fallbacks() {
    // Nothing set: count 1, times fall back to creation, no object or source.
    let cells = event_cells_of(json!({}));
    assert_eq!(cells, ["", "", "shop", "", "", "1", "2026-10-01T00:00:00Z", "2026-10-01T00:00:00Z"]);
    // Involved object without kind; source component without host; empty
    // legacy fields fall through to the newer ones.
    let cells = event_cells_of(json!({
        "involvedObject": {"name": "web"},
        "source": {"component": "", "host": ""},
        "reportingComponent": "kubelet",
        "firstTimestamp": "",
        "eventTime": "2026-10-01T00:00:02Z"
    }));
    assert_eq!(cells[3], "web");
    assert_eq!(cells[4], "kubelet");
    assert_eq!(cells[6], "2026-10-01T00:00:02Z");
    assert_eq!(cells[7], "2026-10-01T00:00:02Z");
    // Only a first time: last seen is the first time.
    let cells = event_cells_of(json!({"firstTimestamp": "2026-10-01T00:00:03Z", "source": {"component": "sched"}}));
    assert_eq!(cells[4], "sched");
    assert_eq!(cells[7], "2026-10-01T00:00:03Z");
    // Involved object with only a kind is blank.
    let cells = event_cells_of(json!({"involvedObject": {"kind": "Pod"}}));
    assert_eq!(cells[3], "");
}

#[test]
fn sort_rows_leaves_other_kinds_alone() {
    let cm = kind("", "v1", "ConfigMap", "configmaps", true);
    let mk = |id: &str| ResourceRow { id: id.into(), name: id.into(), namespace: None, cells: vec![] };
    let mut rows = vec![mk("b"), mk("a")];
    sort_rows(&cm, &mut rows);
    assert_eq!(rows[0].id, "b");
}

#[test]
fn helpers() {
    assert_eq!(row_id(Some("shop"), "web"), "shop/web");
    assert_eq!(row_id(None, "node-1"), "node-1");
    assert_eq!(first_str(&[&json!(""), &json!(3), &json!("x")]), Some("x"));
    assert_eq!(first_str(&[&Value::Null]), None);
    assert_eq!(int(&json!("7")), 0);
    assert_eq!(text(&json!(7)), "");
}

fn pod_kind() -> KindRef {
    kind("", "v1", "Pod", "pods", true)
}

fn status_of(status: Value, deleting: bool) -> String {
    let mut extra = json!({"status": status});
    if deleting {
        extra["metadata"] = json!({"deletionTimestamp": "2026-10-01T00:01:00Z"});
    }
    let mut v = object("v1", "Pod", Some("shop"), "web", json!({"spec": {"containers": [{"name": "app"}]}}));
    // Replace status wholesale.
    v["status"] = Value::Null;
    crate::testing::merge(&mut v, extra);
    checked_row(&pod_kind(), v).cells[4].clone()
}

#[test]
fn pod_row_cells() {
    let row = checked_row(&pod_kind(), pod("shop", "web-1", "node-1", json!({
        "status": {"containerStatuses": [
            {"name": "app", "ready": true, "restartCount": 3, "image": "acme/app:1", "imageID": "", "state": {"running": {}}}]}
    })));
    assert_eq!(row.cells[0], "web-1");
    assert_eq!(row.cells[4], "Running");
    assert_eq!(row.cells[5], "3");
    assert_eq!(row.cells[6], "node-1");
    assert!(containers::decode(&row.cells[2]).is_empty());
}

#[test]
fn pod_status_rules() {
    // Failed init container.
    assert_eq!(
        status_of(json!({"phase": "Pending", "initContainerStatuses": [
            {"name": "migrate", "state": {"terminated": {"exitCode": 1, "reason": "Error"}}}]}), false),
        "Init:Error"
    );
    // Init container waiting with a reason.
    assert_eq!(
        status_of(json!({"phase": "Pending", "initContainerStatuses": [
            {"name": "migrate", "state": {"waiting": {"reason": "ImagePullBackOff"}}}]}), false),
        "Init:ImagePullBackOff"
    );
    // PodInitializing counts as progress.
    assert_eq!(
        status_of(json!({"phase": "Pending", "initContainerStatuses": [
            {"name": "migrate", "state": {"waiting": {"reason": "PodInitializing"}}}]}), false),
        "Init:0/1"
    );
    // All inits done: app container state decides.
    assert_eq!(
        status_of(json!({"phase": "Running",
            "initContainerStatuses": [{"name": "migrate", "state": {"terminated": {"exitCode": 0}}}],
            "containerStatuses": [{"name": "app", "ready": true, "state": {"running": {}}}]}), false),
        "Running"
    );
    // One completed sidecar alongside a running ready container still shows Running.
    assert_eq!(
        status_of(json!({"phase": "Running", "containerStatuses": [
            {"name": "job", "state": {"terminated": {"exitCode": 0, "reason": "Completed"}}},
            {"name": "app", "ready": true, "state": {"running": {}}}]}), false),
        "Running"
    );
    // Killed by a signal, no reason.
    assert_eq!(
        status_of(json!({"phase": "Failed", "containerStatuses": [
            {"name": "app", "state": {"terminated": {"exitCode": 137, "signal": 9}}}]}), false),
        "Signal:9"
    );
    // Terminating, and node lost.
    assert_eq!(status_of(json!({"phase": "Running"}), true), "Terminating");
    assert_eq!(status_of(json!({"phase": "Running", "reason": "NodeLost"}), true), "Unknown");
    // Empty reason falls back to phase; nothing at all is empty.
    assert_eq!(status_of(json!({"phase": "Pending", "reason": ""}), false), "Pending");
    assert_eq!(status_of(json!({}), false), "");
}

#[test]
fn container_state_details() {
    let states = |statuses: Value, init: bool| -> Vec<(State, String)> {
        let spec = if init {
            json!({"initContainers": [{"name": "a"}], "containers": []})
        } else {
            json!({"containers": [{"name": "a"}]})
        };
        let key = if init { "initContainerStatuses" } else { "containerStatuses" };
        let data = json!({"spec": spec, "status": {key: statuses}});
        container_states(&data).into_iter().map(|c| (c.state, c.detail)).collect()
    };
    assert_eq!(states(json!([{"name": "a", "state": {"waiting": {}}}]), false), [(State::Failed, "Waiting".into())]);
    assert_eq!(states(json!([{"name": "a", "state": {}}]), false), [(State::Starting, "Unknown".into())]);
    assert_eq!(
        states(json!([{"name": "a", "state": {"terminated": {"exitCode": 2, "reason": "Error"}}}]), false),
        [(State::Failed, "Error (exit 2)".into())]
    );
    assert_eq!(states(json!([]), false), [(State::Starting, "Pending".into())]);
    // Init containers: running is fine even when not ready; others as usual.
    assert_eq!(states(json!([{"name": "a", "ready": false, "state": {"running": {}}}]), true), [(State::Running, "Running".into())]);
    assert_eq!(
        states(json!([{"name": "a", "state": {"terminated": {"exitCode": 0, "reason": "Completed"}}}]), true),
        [(State::Stopped, "Completed (exit 0)".into())]
    );
    assert!(container_states(&json!({})).is_empty());
}

fn status_with_inits(init_specs: Value, status: Value) -> String {
    let v = json!({"apiVersion": "v1", "kind": "Pod", "metadata": {"name": "web", "namespace": "shop"},
        "spec": {"initContainers": init_specs, "containers": [{"name": "app"}]}, "status": status});
    checked_row(&pod_kind(), v).cells[4].clone()
}

#[test]
fn running_native_sidecars_count_as_initialised() {
    let specs = json!([{"name": "migrate"}, {"name": "proxy", "restartPolicy": "Always"}]);
    let status = |proxy_state: Value| json!({"phase": "Running",
        "initContainerStatuses": [
            {"name": "migrate", "state": {"terminated": {"exitCode": 0, "reason": "Completed"}}},
            {"name": "proxy", "ready": true, "state": proxy_state}],
        "containerStatuses": [{"name": "app", "ready": true, "state": {"running": {}}}]});
    // Healthy sidecar: the pod is Running, not Init:1/2.
    assert_eq!(status_with_inits(specs.clone(), status(json!({"running": {}}))), "Running");
    // A crashing sidecar still surfaces.
    assert_eq!(
        status_with_inits(specs.clone(), status(json!({"waiting": {"reason": "CrashLoopBackOff"}}))),
        "Init:CrashLoopBackOff"
    );
    // An ordinary init container that is running is still initialising.
    let ordinary = json!([{"name": "migrate"}, {"name": "proxy"}]);
    assert_eq!(status_with_inits(ordinary, status(json!({"running": {}}))), "Init:1/2");
    // Sidecar running, but a later ordinary init container still running.
    let later = json!([{"name": "proxy", "restartPolicy": "Always"}, {"name": "migrate"}]);
    let st = json!({"phase": "Pending", "initContainerStatuses": [
        {"name": "proxy", "state": {"running": {}}},
        {"name": "migrate", "state": {"running": {}}}]});
    assert_eq!(status_with_inits(later, st), "Init:1/2");
}
