//! Tests for the Node details.

use super::*;
use crate::testing::{MockApi, list, object, pod};
use crate::types::InfoSection;
use serde_json::json;

fn now() -> jiff::Timestamp {
    "2026-10-10T12:00:00Z".parse().unwrap()
}

fn row<'a>(d: &'a ObjectDetails, key: &str) -> Option<&'a str> {
    d.info.iter().find(|r| r.key == key).map(|r| r.value.as_str())
}

fn section<'a>(d: &'a ObjectDetails, id: &str) -> &'a InfoSection {
    d.sections.iter().find(|s| s.id == id).unwrap_or_else(|| panic!("no section {id}"))
}

fn cells(s: &InfoSection) -> Vec<Vec<&str>> {
    s.table.as_ref().unwrap().rows.iter().map(|r| r.cells.iter().map(String::as_str).collect()).collect()
}

fn node_json() -> Value {
    object("v1", "Node", None, "n1", json!({
        "metadata": {"labels": {
            "node-role.kubernetes.io/worker": "", "node.kubernetes.io/instance-type": "m5.large",
            "topology.kubernetes.io/zone": "us-east-1a", "topology.kubernetes.io/region": "us-east-1"}},
        "spec": {"podCIDRs": ["10.0.1.0/24"], "providerID": "aws:///us-east-1a/i-0abc",
                 "taints": [{"key": "dedicated", "value": "batch", "effect": "NoSchedule"},
                            {"key": "node.kubernetes.io/unreachable", "effect": "NoExecute"}]},
        "status": {
            "capacity": {"cpu": "2", "memory": "8Gi", "pods": "29"},
            "allocatable": {"cpu": "1930m", "memory": "7Gi", "pods": "29"},
            "conditions": [{"type": "MemoryPressure", "status": "False"}, {"type": "Ready", "status": "True"}],
            "addresses": [{"type": "InternalIP", "address": "10.0.1.5"}, {"type": "Hostname", "address": "ip-10-0-1-5"}],
            "nodeInfo": {"kubeletVersion": "v1.33.1", "osImage": "Amazon Linux 2023", "kernelVersion": "6.1.0",
                         "containerRuntimeVersion": "containerd://1.7.2", "architecture": "amd64"}}
    }))
}

fn details() -> ObjectDetails {
    crate::config::describe("core/Node", &node_json(), now())
}

#[test]
fn describes_the_node() {
    let d = details();
    assert_eq!(d.status.as_deref(), Some("Ready"));
    assert_eq!(row(&d, "Roles"), Some("worker"));
    assert_eq!(row(&d, "Instance type"), Some("m5.large"));
    assert_eq!(row(&d, "Zone"), Some("us-east-1a · us-east-1"));
    assert_eq!(row(&d, "Internal IP"), Some("10.0.1.5"));
    assert_eq!(row(&d, "Hostname"), Some("ip-10-0-1-5"));
    assert_eq!(row(&d, "Taints"), Some("dedicated=batch:NoSchedule\nnode.kubernetes.io/unreachable:NoExecute"));
    assert_eq!(row(&d, "Kubelet"), Some("v1.33.1"));
    assert_eq!(row(&d, "Container runtime"), Some("containerd://1.7.2"));
    assert_eq!(row(&d, "Pod CIDR"), Some("10.0.1.0/24"));
    assert_eq!(row(&d, "Provider ID"), Some("aws:///us-east-1a/i-0abc"));
    assert_eq!(row(&d, "Scheduling"), None);
    assert_eq!(d.conditions.len(), 2);
}

#[test]
fn statuses() {
    let mut v = node_json();
    v["spec"]["unschedulable"] = json!(true);
    let d = crate::config::describe("core/Node", &v, now());
    assert_eq!(d.status.as_deref(), Some("Cordoned"));
    assert!(row(&d, "Scheduling").is_some());

    v["status"]["conditions"] = json!([{"type": "Ready", "status": "Unknown"}]);
    assert_eq!(node_status(&v), "NotReady");
    v["status"]["conditions"] = json!([]);
    assert_eq!(node_status(&v), "Unknown");

    let bare = object("v1", "Node", None, "n2", json!({}));
    let d = crate::config::describe("core/Node", &bare, now());
    assert_eq!(row(&d, "Taints"), Some("none"));
    assert_eq!(row(&d, "Roles"), None);
}

#[test]
fn other_kinds_are_left_alone() {
    let mut d = details();
    let before = d.info.len();
    describe("core/Namespace", &mut d, &node_json());
    assert_eq!(d.info.len(), before);
}

#[tokio::test]
async fn related_ignores_other_kinds() {
    let api = MockApi::new();
    let mut d = details();
    related(&api.client(), "core/Pod", "n1", &node_json(), &mut d).await.unwrap();
    assert!(d.sections.is_empty());
    assert!(api.calls().is_empty());
}

fn with_requests(cpu: &str, mem: &str) -> Value {
    json!({"spec": {"containers": [{"name": "app", "image": "acme/app:1",
        "resources": {"requests": {"cpu": cpu, "memory": mem}, "limits": {"memory": mem}}}]}})
}

#[tokio::test]
async fn pods_resources_and_events_with_kubelet_stats() {
    let api = MockApi::new();
    let mut crashing = pod("shop", "api-2", "n1", with_requests("250m", "256Mi"));
    crashing["status"]["containerStatuses"][0]["ready"] = json!(false);
    crashing["status"]["containerStatuses"][0]["restartCount"] = json!(4);
    api.on(
        "GET",
        "/api/v1/pods",
        list("PodList", vec![
            crashing,
            pod("kube-system", "proxy-1", "n1", json!({})),
            pod("shop", "api-1", "n1", with_requests("1500m", "1Gi")),
        ]),
    );
    api.on("GET", "/api/v1/nodes/n1/proxy/stats/summary", json!({"pods": [
        {"podRef": {"name": "api-1", "namespace": "shop"}, "containers": [
            {"name": "app", "cpu": {"usageNanoCores": 120_000_000.0}, "memory": {"workingSetBytes": 268435456.0}}]},
        {"podRef": {"name": "proxy-1", "namespace": "kube-system"}, "containers": [
            {"name": "app", "cpu": {"usageNanoCores": 3_000_000.0}, "memory": {"workingSetBytes": 1048576.0}}]}
    ]}));
    api.on("GET", "/api/v1/events", list("EventList", vec![object("v1", "Event", Some("default"), "n1.1", json!({
        "type": "Warning", "reason": "NodeNotReady", "message": "Node is not ready",
        "involvedObject": {"kind": "Node", "name": "n1"}, "lastTimestamp": "2026-10-10T11:00:00Z"
    }))]));

    let mut d = details();
    related(&api.client(), "core/Node", "n1", &node_json(), &mut d).await.unwrap();

    // Only pods still holding resources on this node are asked for.
    let call = &api.calls_to("GET", "/api/v1/pods")[0];
    assert!(call.query.contains("spec.nodeName%3Dn1"), "{}", call.query);
    assert!(call.query.contains("status.phase%21%3DSucceeded"), "{}", call.query);

    let n = d.node.clone().expect("node resources");
    assert_eq!((n.cpu.used, n.cpu.requests, n.cpu.limits, n.cpu.allocatable, n.cpu.capacity), (Some(123.0), 1750.0, 0.0, 1930.0, 2000.0));
    let gib = 1024.0 * 1024.0 * 1024.0;
    assert_eq!((n.memory.requests, n.memory.limits, n.memory.allocatable), (1.25 * gib, 1.25 * gib, 7.0 * gib));
    assert_eq!((n.pods, n.pod_capacity, n.one_pod_hosting), (3, 29, None));
    assert!(d.sections.iter().all(|s| s.id != "node-resources"), "charted, not a table");

    let pods = section(&d, "node-pods");
    let t = pods.table.as_ref().unwrap();
    assert_eq!(t.columns, ["Pod", "Namespace", "Status", "Restarts", "CPU used / req.", "Memory used / req.", "Age"]);
    assert_eq!(t.times, vec![6]);
    // Sorted by namespace, then name; usage only where the kubelet has it.
    let names: Vec<(&str, &str)> = cells(pods).iter().map(|c| (c[1], c[0])).collect();
    assert_eq!(names, [("kube-system", "proxy-1"), ("shop", "api-1"), ("shop", "api-2")]);
    assert_eq!(cells(pods)[1][4..6], ["120m / 1.50 cores", "256.0 MiB / 1.0 GiB"]);
    assert_eq!(cells(pods)[2][3], "4");
    assert_eq!(cells(pods)[2][4], "— / 250m");
    assert_eq!(pods.summary, "3 pods · 1 not ready");
    let link = t.rows[2].link.as_ref().unwrap();
    assert_eq!((link.kind.as_str(), link.namespace.as_deref(), link.name.as_str()), ("core/Pod", Some("shop"), "api-2"));
    assert!(t.rows[2].warn && !t.rows[1].warn);

    assert_eq!(d.events.len(), 1);
    let call = &api.calls_to("GET", "/api/v1/events")[0];
    assert!(call.query.contains("involvedObject.name%3Dn1"), "{}", call.query);
}

#[tokio::test]
async fn usage_from_metrics_server_or_none() {
    let api = MockApi::new();
    api.on("GET", "/api/v1/pods", list("PodList", vec![]));
    api.on("GET", "/apis/metrics.k8s.io/v1beta1/nodes/n1", object("metrics.k8s.io/v1beta1", "NodeMetrics", None, "n1", json!({
        "usage": {"cpu": "250m", "memory": "1Gi"}
    })));
    let mut d = details();
    related(&api.client(), "core/Node", "n1", &node_json(), &mut d).await.unwrap();
    assert_eq!(d.node.as_ref().unwrap().cpu.used, Some(250.0));
    let pods = section(&d, "node-pods");
    assert_eq!(pods.summary, "none");
    assert!(pods.table.as_ref().unwrap().columns.contains(&"CPU requested".to_owned()));

    let api = MockApi::new();
    api.on("GET", "/api/v1/pods", list("PodList", vec![]));
    let mut d = details();
    related(&api.client(), "core/Node", "n1", &node_json(), &mut d).await.unwrap();
    assert_eq!(d.node.unwrap().memory.used, None);
}

#[tokio::test]
async fn failing_pod_list_is_an_error() {
    let api = MockApi::new();
    api.on_error("GET", "/api/v1/pods", 403, "Forbidden", "pods is forbidden");
    let mut d = details();
    assert!(related(&api.client(), "core/Node", "n1", &node_json(), &mut d).await.is_err());
}

#[tokio::test]
async fn fargate_runs_one_pod_per_node_by_design() {
    let api = MockApi::new();
    api.on("GET", "/api/v1/pods", list("PodList", vec![pod("kube-system", "coredns-1", "n1", json!({}))]));
    let mut v = node_json();
    v["metadata"]["labels"] = json!({"eks.amazonaws.com/compute-type": "fargate"});
    v["status"]["allocatable"]["pods"] = json!("1");
    let mut d = crate::config::describe("core/Node", &v, now());
    related(&api.client(), "core/Node", "n1", &v, &mut d).await.unwrap();
    let n = d.node.unwrap();
    assert_eq!((n.pods, n.pod_capacity), (1, 1));
    assert_eq!(n.one_pod_hosting.as_deref(), Some("AWS Fargate"));
}
