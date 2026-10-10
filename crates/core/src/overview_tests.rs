use serde_json::{Value, json};

use super::*;
use crate::testing::{MockApi, list, object, pod};

const MI: f64 = 1024.0 * 1024.0;
const GI: f64 = 1024.0 * MI;
const PROM: &str = "/api/v1/namespaces/monitoring/services/prometheus-server:9090/proxy/api/v1";

fn node_info(kubelet: &str) -> Value {
    json!({"architecture": "amd64", "bootID": "", "containerRuntimeVersion": "containerd://1.7", "kernelVersion": "6.1",
           "kubeProxyVersion": "", "kubeletVersion": kubelet, "machineID": "", "operatingSystem": "linux",
           "osImage": "Linux", "systemUUID": ""})
}

fn nodes() -> Vec<Value> {
    vec![
        object("v1", "Node", None, "n2", json!({
            "metadata": {"labels": {"eks.amazonaws.com/compute-type": "fargate", "node.kubernetes.io/instance-type": "m5.large"}},
            "status": {"capacity": {"cpu": "2", "memory": "4Gi"}, "allocatable": {"cpu": "2", "memory": "4Gi", "pods": "1"},
                       "conditions": [{"type": "Ready", "status": "False"}], "nodeInfo": node_info("v1.31.0-eks")}
        })),
        object("v1", "Node", None, "n1", json!({
            "metadata": {"labels": {
                "node-role.kubernetes.io/worker": "", "node-role.kubernetes.io/control-plane": "",
                "node-role.kubernetes.io/": "", "node.kubernetes.io/instance-type": "m6i.large",
                "beta.kubernetes.io/instance-type": "old"}},
            "spec": {"unschedulable": true},
            "status": {"capacity": {"cpu": "4", "memory": "16Gi", "pods": "110"},
                       "allocatable": {"cpu": "3900m", "memory": "15Gi", "pods": "110"},
                       "conditions": [{"type": "MemoryPressure", "status": "False"}, {"type": "Ready", "status": "True"}],
                       "nodeInfo": node_info("v1.31.2")}
        })),
        // No status at all, and only the beta instance-type label.
        object("v1", "Node", None, "n3", json!({"metadata": {"labels": {"beta.kubernetes.io/instance-type": "t3.small"}}})),
    ]
}

fn resources(requests: Value, limits: Value) -> Value {
    json!({"spec": {"containers": [{"name": "app", "image": "acme/app:1", "resources": {"requests": requests, "limits": limits}}]}})
}

fn with_phase(mut p: Value, phase: &str) -> Value {
    p["status"]["phase"] = json!(phase);
    p
}

fn pods() -> Vec<Value> {
    let mut pending = pod("acme", "queued", "", resources(json!({"cpu": "100m", "memory": "4Gi"}), json!({})));
    pending["spec"].as_object_mut().unwrap().remove("nodeName");
    let mut no_status = pod("acme", "fresh", "n1", json!({}));
    no_status.as_object_mut().unwrap().remove("status");
    vec![
        pod("shop", "web-1", "n1", resources(json!({"cpu": "500m", "memory": "1Gi"}), json!({"cpu": "1", "memory": "2Gi"}))),
        pod("shop", "web-2", "n1", resources(json!({"cpu": "250m", "memory": "512Mi"}), json!({}))),
        with_phase(pod("acme", "job-done", "n2", resources(json!({"cpu": "8", "memory": "8Gi"}), json!({}))), "Succeeded"),
        with_phase(pod("acme", "job-failed", "n2", resources(json!({"cpu": "8", "memory": "8Gi"}), json!({}))), "Failed"),
        with_phase(pending, "Pending"),
        // Unknown phase, still holds its (zero) requests on n1.
        no_status,
        with_phase(pod("acme", "odd", "n1", json!({})), "Evicted"),
    ]
}

fn timestamp(ago_secs: i64) -> String {
    (jiff::Timestamp::now() - jiff::SignedDuration::from_secs(ago_secs)).strftime("%Y-%m-%dT%H:%M:%SZ").to_string()
}

fn event(name: &str, extra: Value) -> Value {
    let mut e = object("v1", "Event", Some("shop"), name, json!({"involvedObject": {}, "type": "Warning"}));
    crate::testing::merge(&mut e, extra);
    e
}

/// Nodes, pods, version and warning events; no metrics of any kind.
fn fake_cluster(api: &MockApi) {
    api.on("GET", "/api/v1/nodes", list("NodeList", nodes()));
    api.on("GET", "/api/v1/pods", list("PodList", pods()));
    api.on("GET", "/version", json!({"major": "1", "minor": "31", "gitVersion": "v1.31.2", "gitCommit": "abc",
        "gitTreeState": "clean", "buildDate": "2026-01-01T00:00:00Z", "goVersion": "go1.22", "compiler": "gc",
        "platform": "linux/amd64"}));
    api.on("GET", "/api/v1/events", list("EventList", vec![
        event("recent", json!({"lastTimestamp": timestamp(600)})),
        event("old", json!({"lastTimestamp": timestamp(7200)})),
        event("series", json!({"series": {"count": 3, "lastObservedTime": timestamp(60).replace('Z', ".000000Z")}})),
        event("new-api", json!({"eventTime": timestamp(30).replace('Z', ".000000Z")})),
        event("timeless", json!({})),
    ]));
}

fn node<'a>(o: &'a ClusterOverview, name: &str) -> &'a NodeSummary {
    o.nodes.iter().find(|n| n.name == name).unwrap()
}

#[tokio::test]
async fn overview_with_metrics_server() {
    let api = MockApi::new();
    fake_cluster(&api);
    api.on("GET", "/apis/metrics.k8s.io/v1beta1/nodes", list("NodeMetricsList", vec![
        object("metrics.k8s.io/v1beta1", "NodeMetrics", None, "n1", json!({"usage": {"cpu": "1200m", "memory": "2Gi"}})),
        object("metrics.k8s.io/v1beta1", "NodeMetrics", None, "n2", json!({"usage": {"cpu": "100m"}})),
    ]));
    let o = cluster_overview(&api.client(), &HistoryStore::default()).await.unwrap();

    assert_eq!(o.server_version.as_deref(), Some("v1.31.2"));
    assert_eq!(o.warnings_last_hour, Some(3));
    assert!(o.metrics_available);
    assert_eq!(o.usage_source, Some(UsageSource::MetricsServer));
    assert_eq!(o.history, None);
    let c = &o.pods;
    assert_eq!((c.running, c.pending, c.succeeded, c.failed, c.unknown), (2, 1, 1, 1, 2));

    // Nodes sorted by name.
    let names: Vec<&str> = o.nodes.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, ["n1", "n2", "n3"]);
    let n1 = node(&o, "n1");
    assert!(n1.ready && !n1.schedulable);
    assert_eq!(n1.roles, ["control-plane", "worker"]);
    assert_eq!(n1.instance_type.as_deref(), Some("m6i.large"));
    assert_eq!(n1.kubelet_version, "v1.31.2");
    assert_eq!(n1.created.as_deref(), Some("2026-10-01T00:00:00Z"));
    assert_eq!((n1.pods, n1.pod_capacity), (4, 110));
    assert_eq!(n1.cpu, ResourceTotals { capacity: 4000.0, allocatable: 3900.0, requests: 750.0, limits: 1000.0, used: Some(1200.0) });
    assert_eq!(n1.memory, ResourceTotals { capacity: 16.0 * GI, allocatable: 15.0 * GI, requests: 1.5 * GI, limits: 2.0 * GI, used: Some(2.0 * GI) });

    let n2 = node(&o, "n2");
    assert!(!n2.ready && n2.schedulable);
    assert_eq!(n2.instance_type.as_deref(), Some("fargate"));
    // Finished pods don't count against the node.
    assert_eq!((n2.pods, n2.pod_capacity), (0, 1));
    assert_eq!((n2.cpu.requests, n2.cpu.used, n2.memory.used), (0.0, Some(100.0), Some(0.0)));

    let n3 = node(&o, "n3");
    assert!(!n3.ready && n3.roles.is_empty());
    assert_eq!(n3.instance_type.as_deref(), Some("t3.small"));
    assert_eq!((n3.kubelet_version.as_str(), n3.pod_capacity, n3.cpu.capacity, n3.cpu.used), ("", 0, 0.0, None));

    // Cluster totals add the nodes up; missing usage counts as zero.
    assert_eq!(o.cpu, ResourceTotals { capacity: 6000.0, allocatable: 5900.0, requests: 750.0, limits: 1000.0, used: Some(1300.0) });
    assert_eq!(o.memory.capacity, 20.0 * GI);
    assert_eq!(o.memory.used, Some(2.0 * GI));

    // Namespaces by memory requests, largest first; the pending pod counts.
    let ns: Vec<(&str, usize, f64, f64)> = o.namespaces.iter().map(|n| (n.namespace.as_str(), n.pods, n.cpu, n.memory)).collect();
    assert_eq!(ns, [("acme", 3, 100.0, 4.0 * GI), ("shop", 2, 750.0, 1.5 * GI)]);

    // Cheap list calls, and warnings only.
    assert!(api.calls_to("GET", "/api/v1/pods")[0].query.contains("resourceVersion=0"));
    assert!(api.calls_to("GET", "/api/v1/events")[0].query.contains("fieldSelector=type%3DWarning"));
    // metrics-server was enough: no kubelet calls.
    assert!(api.calls().iter().all(|c| !c.path.contains("/proxy/stats/summary")));
}

fn kubelet_summary(api: &MockApi, node: &str, cpu_nanos: f64, memory: f64) {
    api.on("GET", &format!("/api/v1/nodes/{node}/proxy/stats/summary"), json!({"pods": [
        {"podRef": {"name": "web-1", "namespace": "shop"}, "containers": [
            {"name": "app", "cpu": {"usageNanoCores": cpu_nanos}, "memory": {"workingSetBytes": memory}}]}
    ]}));
}

#[tokio::test]
async fn overview_with_prometheus_and_kubelet_fill_in() {
    let api = MockApi::new();
    fake_cluster(&api);
    api.on("GET", "/api/v1/services", list("ServiceList", vec![
        object("v1", "Service", Some("monitoring"), "prometheus-server", json!({"spec": {"ports": [{"port": 9090}], "clusterIP": "10.0.0.9"}})),
    ]));
    let query = format!("{PROM}/query");
    api.on("GET", &query, json!({"status": "success", "data": {"resultType": "scalar", "result": [0, "1"]}}));
    // Prometheus only knows n2 (idle by our count); n1 comes from its kubelet.
    api.on_query("GET", &query, "container_cpu_usage", json!({"status": "success", "data": {"result": [
        {"metric": {"node": "n2"}, "value": [0, "50"]}]}}));
    api.on_query("GET", &query, "container_memory", json!({"status": "success", "data": {"result": [
        {"metric": {"node": "n2"}, "value": [0, "100"]}]}}));
    let range = format!("{PROM}/query_range");
    api.on("GET", &range, json!({"status": "success", "data": {"result": [{"metric": {}, "values": [[60, "42"]]}]}}));
    kubelet_summary(&api, "n1", 300_000_000.0, MI);

    let o = cluster_overview(&api.client(), &HistoryStore::default()).await.unwrap();
    assert_eq!(o.usage_source, Some(UsageSource::Prometheus));
    assert!(o.metrics_available);
    assert_eq!(node(&o, "n1").cpu.used, Some(300.0));
    assert_eq!(node(&o, "n1").memory.used, Some(MI));
    assert_eq!((node(&o, "n2").cpu.used, node(&o, "n2").memory.used), (Some(50.0), Some(100.0)));
    assert_eq!(node(&o, "n3").cpu.used, Some(0.0));
    assert_eq!(o.cpu.used, Some(350.0));
    let history = o.history.unwrap();
    assert_eq!(history.usage.cpu, vec![(60.0, 42.0)]);
    // Only busy nodes Prometheus didn't cover are asked.
    let kubelet: Vec<String> = api.calls().into_iter().filter(|c| c.path.ends_with("/stats/summary")).map(|c| c.path).collect();
    assert_eq!(kubelet, ["/api/v1/nodes/n1/proxy/stats/summary"]);
}

#[tokio::test]
async fn overview_with_kubelet_only() {
    let api = MockApi::new();
    fake_cluster(&api);
    kubelet_summary(&api, "n1", 2_000_000.0, 512.0);
    let o = cluster_overview(&api.client(), &HistoryStore::default()).await.unwrap();
    assert_eq!(o.usage_source, Some(UsageSource::Kubelet));
    assert_eq!(node(&o, "n1").cpu.used, Some(2.0));
    assert_eq!(node(&o, "n2").cpu.used, Some(0.0));
    assert_eq!(o.memory.used, Some(512.0));
}

#[tokio::test]
async fn overview_without_any_usage() {
    let api = MockApi::new();
    api.on("GET", "/api/v1/nodes", list("NodeList", nodes()));
    api.on("GET", "/api/v1/pods", list("PodList", pods()));
    // No version, events, metrics-server, Prometheus or kubelet stats.
    let o = cluster_overview(&api.client(), &HistoryStore::default()).await.unwrap();
    assert_eq!(o.server_version, None);
    assert_eq!(o.warnings_last_hour, None);
    assert!(!o.metrics_available);
    assert_eq!(o.usage_source, None);
    assert_eq!(o.cpu.used, None);
    assert!(o.nodes.iter().all(|n| n.cpu.used.is_none() && n.memory.used.is_none()));
    assert_eq!(o.cpu.requests, 750.0);
}

#[tokio::test]
async fn overview_with_only_idle_nodes_needs_no_usage_source() {
    // Nothing runs anywhere: no node needs asking, so all are idle.
    let api = MockApi::new();
    api.on("GET", "/api/v1/nodes", list("NodeList", nodes()));
    api.on("GET", "/api/v1/pods", list("PodList", vec![]));
    let o = cluster_overview(&api.client(), &HistoryStore::default()).await.unwrap();
    assert_eq!(o.usage_source, Some(UsageSource::Kubelet));
    assert_eq!(o.cpu.used, Some(0.0));
    assert!(o.namespaces.is_empty());
    assert_eq!(o.pods, PodCounts::default());
}

#[tokio::test]
async fn overview_fails_without_nodes_or_pods() {
    let api = MockApi::new();
    api.on("GET", "/api/v1/pods", list("PodList", vec![]));
    api.on_error("GET", "/api/v1/nodes", 403, "Forbidden", "nodes is forbidden");
    let err = cluster_overview(&api.client(), &HistoryStore::default()).await.unwrap_err();
    assert_eq!(err.user_message(), "nodes is forbidden");

    let api = MockApi::new();
    api.on("GET", "/api/v1/nodes", list("NodeList", vec![]));
    assert!(cluster_overview(&api.client(), &HistoryStore::default()).await.is_err());
}

#[test]
fn pod_resources_without_any_resources() {
    let spec: PodSpec = serde_json::from_value(json!({"containers": [{"name": "app"}]})).unwrap();
    assert_eq!(pod_resources(&spec), PodResources::default());
}

#[test]
fn pod_resources_overhead_and_limits() {
    let spec: PodSpec = serde_json::from_value(json!({
        "containers": [
            {"name": "a", "resources": {"requests": {"cpu": "100m"}, "limits": {"cpu": "200m", "memory": "1Gi"}}},
            {"name": "b", "resources": {"limits": {"cpu": "300m"}}}
        ],
        "initContainers": [
            {"name": "i1", "resources": {"limits": {"cpu": "2", "memory": "100Mi"}}},
            {"name": "i2", "resources": {"requests": {"memory": "3Gi"}}}
        ],
        "overhead": {"cpu": "10m"}
    }))
    .unwrap();
    let r = pod_resources(&spec);
    assert_eq!(r, PodResources { cpu_requests: 110.0, cpu_limits: 2010.0, memory_requests: 3.0 * GI, memory_limits: GI });
    assert_eq!(add(r, r).cpu_limits, 4020.0);
}
