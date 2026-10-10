use serde_json::{Value, json};

use super::*;
use crate::testing::{MockApi, kind, list, object, pod};

const PROM: &str = "/api/v1/namespaces/monitoring/services/prometheus-server:9090/proxy/api/v1";

fn service(namespace: &str, name: &str, ports: &[i32], cluster_ip: &str) -> Value {
    let ports: Vec<Value> = ports.iter().map(|p| json!({"port": p})).collect();
    object("v1", "Service", Some(namespace), name, json!({"spec": {"ports": ports, "clusterIP": cluster_ip}}))
}

fn success(data: Value) -> Value {
    json!({"status": "success", "data": data})
}

fn matrix(results: Vec<Value>) -> Value {
    success(json!({"resultType": "matrix", "result": results}))
}

fn vector(results: Vec<Value>) -> Value {
    success(json!({"resultType": "vector", "result": results}))
}

/// A working Prometheus at monitoring/prometheus-server:9090.
fn with_prometheus(api: &MockApi) {
    api.on("GET", "/api/v1/services", list("ServiceList", vec![service("monitoring", "prometheus-server", &[80, 9090], "10.0.0.9")]));
    api.on("GET", &format!("{PROM}/query"), vector(vec![]));
}

/// The decoded `query` parameter of each call to `path`.
fn queries(api: &MockApi, path: &str) -> Vec<String> {
    api.calls_to("GET", path)
        .iter()
        .filter_map(|c| form_urlencoded::parse(c.query.as_bytes()).find(|(k, _)| k == "query").map(|(_, v)| v.into_owned()))
        .collect()
}

fn param(api: &MockApi, path: &str, name: &str) -> Vec<String> {
    api.calls_to("GET", path)
        .iter()
        .filter_map(|c| form_urlencoded::parse(c.query.as_bytes()).find(|(k, _)| k == name).map(|(_, v)| v.into_owned()))
        .collect()
}

fn aligned(start: f64, end: f64, step: f64) {
    assert_eq!(end - start, WINDOW_SECS);
    assert_eq!(end % step, 0.0);
    assert!((unix_now() - end).abs() < step + 5.0);
}

// --- Prometheus discovery -------------------------------------------------

#[tokio::test]
async fn discovery_ranks_skips_and_probes_candidates() {
    let api = MockApi::new();
    let mut no_spec = object("v1", "Service", Some("monitoring"), "prometheus-nospec", json!({}));
    no_spec.as_object_mut().unwrap().remove("spec");
    api.on("GET", "/api/v1/services", list("ServiceList", vec![
        service("monitoring", "kube-prometheus-stack-operator", &[9090], "10.0.0.1"),
        service("monitoring", "prometheus-node-exporter", &[9090], "10.0.0.2"),
        service("monitoring", "prometheus-alertmanager", &[9090], "10.0.0.3"),
        service("shop", "nginx", &[9090], "10.0.0.4"),
        service("monitoring", "prometheus-web", &[80], "10.0.0.5"),
        service("monitoring", "Prometheus-Headless", &[9090], "None"),
        service("monitoring", "prometheus-c", &[9090], "10.0.0.6"),
        service("monitoring", "prometheus-b", &[9090], "10.0.0.7"),
        no_spec,
    ]));
    // prometheus-b doesn't answer (404); prometheus-c does.
    api.on("GET", "/api/v1/namespaces/monitoring/services/prometheus-c:9090/proxy/api/v1/query", vector(vec![]));
    let client = api.client();
    let store = HistoryStore::default();
    let prom = store.prometheus(&client).await.unwrap();
    assert_eq!(prom, &Prometheus { namespace: "monitoring".into(), service: "prometheus-c".into(), port: 9090 });
    let probed: Vec<String> = api.calls().into_iter().filter(|c| c.path.ends_with("/query")).map(|c| c.path).collect();
    assert_eq!(probed, [
        "/api/v1/namespaces/monitoring/services/prometheus-b:9090/proxy/api/v1/query",
        "/api/v1/namespaces/monitoring/services/prometheus-c:9090/proxy/api/v1/query",
    ]);
    assert_eq!(queries(&api, "/api/v1/namespaces/monitoring/services/prometheus-c:9090/proxy/api/v1/query"), ["1"]);
}

#[tokio::test]
async fn headless_service_is_used_last() {
    let api = MockApi::new();
    api.on("GET", "/api/v1/services", list("ServiceList", vec![
        service("monitoring", "prometheus-headless", &[9090], "None"),
        service("monitoring", "prometheus-server", &[9090], "10.0.0.6"),
    ]));
    api.on("GET", "/api/v1/namespaces/monitoring/services/prometheus-headless:9090/proxy/api/v1/query", vector(vec![]));
    let store = HistoryStore::default();
    let prom = store.prometheus(&api.client()).await.unwrap();
    assert_eq!(prom.service, "prometheus-headless");
    assert_eq!(api.calls_to("GET", &format!("{PROM}/query")).len(), 1, "the normal service is tried first");
}

#[tokio::test]
async fn discovery_is_done_once() {
    let api = MockApi::new();
    with_prometheus(&api);
    let client = api.client();
    let store = HistoryStore::default();
    assert!(store.prometheus(&client).await.is_some());
    assert!(store.prometheus(&client).await.is_some());
    assert_eq!(api.calls_to("GET", "/api/v1/services").len(), 1);
    assert_eq!(api.calls_to("GET", &format!("{PROM}/query")).len(), 1);
}

#[tokio::test]
async fn no_prometheus_is_cached_too() {
    let api = MockApi::new();
    api.on_error("GET", "/api/v1/services", 403, "Forbidden", "services is forbidden");
    let client = api.client();
    let store = HistoryStore::default();
    assert!(store.prometheus(&client).await.is_none());
    assert!(store.prometheus(&client).await.is_none());
    assert_eq!(api.calls_to("GET", "/api/v1/services").len(), 1);
}

#[tokio::test]
async fn prometheus_error_status_is_not_usable() {
    let api = MockApi::new();
    api.on("GET", "/api/v1/services", list("ServiceList", vec![service("monitoring", "prometheus-server", &[9090], "10.0.0.9")]));
    api.on("GET", &format!("{PROM}/query"), json!({"status": "error", "errorType": "bad_data", "error": "nope"}));
    assert!(HistoryStore::default().prometheus(&api.client()).await.is_none());
}

#[test]
fn api_path_goes_through_the_service_proxy() {
    let prom = Prometheus { namespace: "monitoring".into(), service: "prom".into(), port: 9090 };
    assert_eq!(prom.api_path("query_range"), "/api/v1/namespaces/monitoring/services/prom:9090/proxy/api/v1/query_range");
}

// --- pod_history ----------------------------------------------------------

#[tokio::test]
async fn pod_history_from_prometheus() {
    let api = MockApi::new();
    with_prometheus(&api);
    let range = format!("{PROM}/query_range");
    api.on_query("GET", &range, "container_cpu_usage", matrix(vec![
        json!({"metric": {"container": "app"}, "values": [[100, "10"], [115, "20"]]}),
        json!({"metric": {"container": "sidecar"}, "values": [[100, "1"], [115, "Inf"]]}),
    ]));
    api.on_query("GET", &range, "container_memory", matrix(vec![
        json!({"metric": {"container": "app"}, "values": [[100, "1000"], [115, "2000"]]}),
        json!({"metric": {"container": "init-only"}, "values": [[100, "5"]]}),
    ]));
    let h = HistoryStore::default().pod_history(&api.client(), "shop", "web-0").await.unwrap();
    assert_eq!(h.source, HistorySource::Prometheus);
    assert_eq!(h.step, STEP_SECS);
    aligned(h.start, h.end, STEP_SECS);
    let names: Vec<&str> = h.containers.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["app", "init-only", "sidecar"]);
    assert_eq!(h.containers[0].usage.cpu, vec![(100.0, 10.0), (115.0, 20.0)]);
    assert!(h.containers[1].usage.cpu.is_empty());
    assert!(h.containers[2].usage.memory.is_empty());
    assert_eq!(h.pod.cpu, vec![(100.0, 11.0), (115.0, 20.0)]);
    assert_eq!(h.pod.memory, vec![(100.0, 1005.0), (115.0, 2000.0)]);

    let qs = queries(&api, &range);
    assert_eq!(qs.len(), 2);
    for q in &qs {
        assert!(q.contains(r#"namespace="shop",pod="web-0",container!="",container!="POD""#), "{q}");
    }
    assert!(qs.iter().any(|q| q.starts_with("sum by (container) (max by (pod, container) (rate(container_cpu_usage_seconds_total{") && q.ends_with("* 1000")));
    assert_eq!(param(&api, &range, "step"), ["15", "15"]);
    assert_eq!(param(&api, &range, "start")[0], h.start.to_string());
    assert_eq!(param(&api, &range, "end")[0], h.end.to_string());
}

fn pod_metrics(name: &str, containers: Value) -> Value {
    object("metrics.k8s.io/v1beta1", "PodMetrics", Some("shop"), name, json!({"containers": containers}))
}

#[tokio::test]
async fn pod_history_falls_back_to_local_samples() {
    let api = MockApi::new();
    // Prometheus is there but knows nothing about the pod.
    with_prometheus(&api);
    api.on("GET", &format!("{PROM}/query_range"), matrix(vec![]));
    api.on("GET", "/api/v1/namespaces/shop/pods/web-0", pod("shop", "web-0", "n1", json!({})));
    api.on("GET", "/apis/metrics.k8s.io/v1beta1/namespaces/shop/pods/web-0", pod_metrics("web-0", json!([
        {"name": "app", "usage": {"cpu": "100m", "memory": "64Mi"}},
        {"name": "sidecar", "usage": {"cpu": "5m", "memory": "1Mi"}},
    ])));
    let client = api.client();
    let store = HistoryStore::default();
    let first = store.pod_history(&client, "shop", "web-0").await.unwrap();
    assert_eq!(first.source, HistorySource::Local);
    aligned(first.start, first.end, STEP_SECS);
    assert_eq!(first.containers.len(), 2);
    assert_eq!(first.containers[0].usage.cpu.len(), 1);
    assert_eq!(first.containers[0].usage.cpu[0].1, 100.0);
    assert_eq!(first.containers[0].usage.memory[0].1, 64.0 * 1024.0 * 1024.0);
    assert_eq!(first.pod.cpu[0].1, 105.0);
    // Each call adds a sample.
    let second = store.pod_history(&client, "shop", "web-0").await.unwrap();
    assert_eq!(second.containers[1].usage.cpu.len(), 2);
}

#[tokio::test]
async fn pod_history_without_prometheus_or_usage() {
    // No Prometheus, no metrics-server, pod not scheduled: an empty local history.
    let api = MockApi::new();
    let mut p = pod("shop", "web-0", "n1", json!({}));
    p["spec"].as_object_mut().unwrap().remove("nodeName");
    api.on("GET", "/api/v1/namespaces/shop/pods/web-0", p);
    let h = HistoryStore::default().pod_history(&api.client(), "shop", "web-0").await.unwrap();
    assert_eq!(h.source, HistorySource::Local);
    assert!(h.containers.is_empty());
    assert!(h.pod.cpu.is_empty() && h.pod.memory.is_empty());
}

#[tokio::test]
async fn pod_history_prometheus_failure_falls_back() {
    // CPU answers, memory fails: Prometheus history is unusable.
    let api = MockApi::new();
    with_prometheus(&api);
    api.on_query("GET", &format!("{PROM}/query_range"), "container_cpu_usage", matrix(vec![
        json!({"metric": {"container": "app"}, "values": [[100, "10"]]}),
    ]));
    api.on("GET", "/api/v1/namespaces/shop/pods/web-0", pod("shop", "web-0", "n1", json!({})));
    api.on("GET", "/api/v1/nodes/n1/proxy/stats/summary", json!({"pods": [
        {"podRef": {"name": "web-0", "namespace": "shop"}, "containers": [
            {"name": "app", "cpu": {"usageNanoCores": 7_000_000.0}, "memory": {"workingSetBytes": 300.0}}]}
    ]}));
    let h = HistoryStore::default().pod_history(&api.client(), "shop", "web-0").await.unwrap();
    assert_eq!(h.source, HistorySource::Local);
    assert_eq!(h.containers[0].usage.cpu[0].1, 7.0);
    assert_eq!(h.containers[0].usage.memory[0].1, 300.0);
}

#[tokio::test]
async fn pod_history_missing_pod_is_an_error() {
    let api = MockApi::new();
    let err = HistoryStore::default().pod_history(&api.client(), "shop", "gone").await.unwrap_err();
    assert!(matches!(err, crate::Error::Kube(kube::Error::Api(ref s)) if s.code == 404), "{err}");
}

// --- workload_history -----------------------------------------------------

fn statefulset_kind() -> KindRef {
    kind("apps", "v1", "StatefulSet", "statefulsets", true)
}

#[tokio::test]
async fn workload_history_from_prometheus() {
    let api = MockApi::new();
    with_prometheus(&api);
    let range = format!("{PROM}/query_range");
    api.on_query("GET", &range, "container_cpu_usage", matrix(vec![json!({"metric": {"container": "db"}, "values": [[100, "30"]]})]));
    api.on_query("GET", &range, "container_memory", matrix(vec![json!({"metric": {"container": "db"}, "values": [[100, "4096"]]})]));
    let h = HistoryStore::default().workload_history(&api.client(), &statefulset_kind(), "shop", "db").await.unwrap();
    assert_eq!(h.source, HistorySource::Prometheus);
    assert_eq!(h.pod.cpu, vec![(100.0, 30.0)]);
    assert_eq!(h.pod.memory, vec![(100.0, 4096.0)]);
    for q in queries(&api, &range) {
        assert!(q.contains(r#"namespace="shop",pod=~"db-[0-9]+","#), "{q}");
    }
    // Prometheus had data: the workload itself isn't read.
    assert!(api.calls_to("GET", "/apis/apps/v1/namespaces/shop/statefulsets/db").is_empty());
}

fn owned_by_db(name: &str, node: &str) -> Value {
    pod("shop", name, node, json!({"metadata": {"labels": {"app": "db"}, "ownerReferences": [
        {"apiVersion": "apps/v1", "kind": "StatefulSet", "name": "db", "uid": "uid-db", "controller": true}]}}))
}

fn fake_statefulset(api: &MockApi) {
    api.on("GET", "/apis/apps/v1/namespaces/shop/statefulsets/db", object("apps/v1", "StatefulSet", Some("shop"), "db", json!({
        "spec": {"selector": {"matchLabels": {"app": "db"}}, "serviceName": "db", "template": {}, "replicas": 2},
        "status": {"replicas": 2}
    })));
    api.on("GET", "/api/v1/namespaces/shop/pods", list("PodList", vec![owned_by_db("db-0", "n1"), owned_by_db("db-1", "n2")]));
}

#[tokio::test]
async fn workload_history_sums_current_pods_locally() {
    let api = MockApi::new();
    with_prometheus(&api);
    api.on("GET", &format!("{PROM}/query_range"), matrix(vec![]));
    fake_statefulset(&api);
    api.on("GET", "/apis/metrics.k8s.io/v1beta1/namespaces/shop/pods", list("PodMetricsList", vec![
        pod_metrics("db-0", json!([{"name": "db", "usage": {"cpu": "100m", "memory": "100"}}, {"name": "backup", "usage": {"cpu": "1m", "memory": "1"}}])),
        pod_metrics("db-1", json!([{"name": "db", "usage": {"cpu": "200m", "memory": "300"}}])),
        // Gone from the workload: not counted.
        pod_metrics("db-9", json!([{"name": "db", "usage": {"cpu": "5000m", "memory": "5000"}}])),
    ]));
    let client = api.client();
    let store = HistoryStore::default();
    let h = store.workload_history(&client, &statefulset_kind(), "shop", "db").await.unwrap();
    assert_eq!(h.source, HistorySource::Local);
    aligned(h.start, h.end, STEP_SECS);
    let db = h.containers.iter().find(|c| c.name == "db").unwrap();
    assert_eq!(db.usage.cpu[0].1, 300.0);
    assert_eq!(db.usage.memory[0].1, 400.0);
    assert_eq!(h.pod.cpu[0].1, 301.0);
    // Samples are kept per workload, apart from a pod of the same name.
    let again = store.workload_history(&client, &statefulset_kind(), "shop", "db").await.unwrap();
    assert_eq!(again.containers.iter().find(|c| c.name == "db").unwrap().usage.cpu.len(), 2);
    assert!(store.local.lock().unwrap().contains_key("StatefulSet:shop/db"));
    let metrics = api.calls_to("GET", "/apis/metrics.k8s.io/v1beta1/namespaces/shop/pods");
    assert!(metrics[0].query.contains("labelSelector=app%3Ddb"), "{}", metrics[0].query);
}

#[tokio::test]
async fn workload_history_without_any_usage() {
    let api = MockApi::new();
    fake_statefulset(&api);
    let h = HistoryStore::default().workload_history(&api.client(), &statefulset_kind(), "shop", "db").await.unwrap();
    assert_eq!(h.source, HistorySource::Local);
    assert!(h.containers.is_empty());
}

#[tokio::test]
async fn workload_history_missing_workload_is_an_error() {
    let api = MockApi::new();
    let err = HistoryStore::default().workload_history(&api.client(), &statefulset_kind(), "shop", "db").await.unwrap_err();
    assert!(matches!(err, crate::Error::Kube(kube::Error::Api(ref s)) if s.code == 404), "{err}");
}

// --- node_usage -----------------------------------------------------------

#[tokio::test]
async fn node_usage_merges_cpu_and_memory() {
    let api = MockApi::new();
    with_prometheus(&api);
    let query = format!("{PROM}/query");
    api.on_query("GET", &query, "container_cpu_usage", vector(vec![
        json!({"metric": {"node": "n1"}, "value": [100, "250"]}),
        json!({"metric": {"node": "n2"}, "value": [100, "NaN"]}),
        json!({"metric": {}, "value": [100, "1"]}),
        json!({"metric": {"node": "n4"}, "value": [100, "oops"]}),
    ]));
    api.on_query("GET", &query, "container_memory", vector(vec![
        json!({"metric": {"node": "n1"}, "value": [100, "1024"]}),
        json!({"metric": {"node": "n3"}, "value": [100, "2048"]}),
    ]));
    let usage = HistoryStore::default().node_usage(&api.client()).await.unwrap();
    assert_eq!(usage.len(), 2);
    assert_eq!(usage["n1"], (250.0, 1024.0));
    assert_eq!(usage["n3"], (0.0, 2048.0));
    let qs = queries(&api, &query);
    assert!(qs.contains(&format!("sum by (node) ({CPU_BY_CONTAINER}) * 1000")), "{qs:?}");
    assert!(qs.contains(&format!("sum by (node) ({MEMORY_BY_CONTAINER})")), "{qs:?}");
}

#[tokio::test]
async fn node_usage_needs_both_queries() {
    let api = MockApi::new();
    with_prometheus(&api);
    // The probe and CPU answer; memory fails.
    api.on_query("GET", &format!("{PROM}/query"), "query=1", vector(vec![]));
    api.on_query("GET", &format!("{PROM}/query"), "container_cpu_usage", vector(vec![]));
    api.on_text("GET", &format!("{PROM}/query"), 503, "unavailable");
    let store = HistoryStore::default();
    let client = api.client();
    assert!(store.prometheus(&client).await.is_some());
    assert!(store.node_usage(&client).await.is_none());

    let api = MockApi::new();
    with_prometheus(&api);
    api.on_query("GET", &format!("{PROM}/query"), "container_memory", json!({"status": "error"}));
    assert!(HistoryStore::default().node_usage(&api.client()).await.is_none());
}

#[tokio::test]
async fn node_usage_without_prometheus() {
    let api = MockApi::new();
    assert!(HistoryStore::default().node_usage(&api.client()).await.is_none());
}

// --- cluster_history ------------------------------------------------------

#[tokio::test]
async fn cluster_history_from_prometheus() {
    let api = MockApi::new();
    with_prometheus(&api);
    let range = format!("{PROM}/query_range");
    api.on_query("GET", &range, "container_cpu_usage", matrix(vec![json!({"metric": {}, "values": [[60, "1500"], [90, "NaN"], [120, "1600"]]})]));
    api.on_query("GET", &range, "container_memory", matrix(vec![json!({"metric": {}, "values": [[60, "8e9"]]})]));
    let h = HistoryStore::default().cluster_history(&api.client()).await.unwrap();
    assert_eq!(h.step, CLUSTER_STEP_SECS);
    aligned(h.start, h.end, CLUSTER_STEP_SECS);
    assert_eq!(h.usage.cpu, vec![(60.0, 1500.0), (120.0, 1600.0)]);
    assert_eq!(h.usage.memory, vec![(60.0, 8e9)]);
    assert_eq!(param(&api, &range, "step"), ["30", "30"]);
    let qs = queries(&api, &range);
    assert!(qs.contains(&format!("sum({CPU_BY_CONTAINER}) * 1000")));
    assert!(qs.contains(&format!("sum({MEMORY_BY_CONTAINER})")));
}

#[tokio::test]
async fn cluster_history_needs_both_series() {
    let api = MockApi::new();
    with_prometheus(&api);
    let range = format!("{PROM}/query_range");
    api.on_query("GET", &range, "container_cpu_usage", matrix(vec![json!({"metric": {}, "values": [[60, "1"]]})]));
    api.on_query("GET", &range, "container_memory", matrix(vec![]));
    assert!(HistoryStore::default().cluster_history(&api.client()).await.is_none());
    assert!(HistoryStore::default().cluster_history(&MockApi::new().client()).await.is_none());
}

// --- assembling series ----------------------------------------------------

#[test]
fn assemble_sums_containers_and_keeps_gaps() {
    let c = |name: &str, cpu: Series, memory: Series| ContainerHistory { name: name.into(), usage: UsageHistory { cpu, memory } };
    let containers = vec![
        // `app` has a gap at 115; `side` restarted and only has 115 onward.
        c("app", vec![(100.0, 1.0), (130.0, 3.0)], vec![(100.0, 10.0), (130.0, 30.0)]),
        c("side", vec![(115.0, 0.5), (130.0, 0.5)], vec![(115.0, 5.0), (130.0000004, 5.0)]),
    ];
    let h = assemble(HistorySource::Local, 0.0, 3600.0, containers.clone());
    assert_eq!((h.source, h.start, h.end, h.step), (HistorySource::Local, 0.0, 3600.0, STEP_SECS));
    assert_eq!(h.pod.cpu, vec![(100.0, 1.0), (115.0, 0.5), (130.0, 3.5)]);
    // Timestamps within a millisecond line up.
    assert_eq!(h.pod.memory, vec![(100.0, 10.0), (115.0, 5.0), (130.0, 35.0)]);
    assert_eq!(h.containers, containers);

    let empty = assemble(HistorySource::Prometheus, 0.0, 3600.0, vec![]);
    assert!(empty.pod.cpu.is_empty() && empty.containers.is_empty());
}

#[test]
fn local_records_without_usage_and_expiry_of_other_pods() {
    let store = HistoryStore::default();
    let sample = Some(HashMap::from([("app".to_owned(), (1.0, 2.0))]));
    store.record("shop/a", 0.0, sample);
    // No usage this time: the series is what was recorded before.
    let out = store.record("shop/a", 10.0, None);
    assert_eq!(out[0].usage.cpu, vec![(0.0, 1.0)]);
    // Asking about another pod much later drops `shop/a`.
    store.record("shop/b", 10.0 + LOCAL_TTL_SECS, None);
    assert!(!store.local.lock().unwrap().contains_key("shop/a"));
    assert!(store.record("shop/a", 10.0 + LOCAL_TTL_SECS, None).is_empty());
}

#[test]
fn parsing_odd_prometheus_results() {
    assert_eq!(single_series(&json!({"result": []})), None);
    assert_eq!(single_series(&json!({})), None);
    assert_eq!(single_series(&json!({"result": [{"metric": {}}]})), None);
    assert_eq!(
        single_series(&json!({"result": [{"values": [[1, "2"], ["x", "3"], [4, 5], [6, "+Inf"], [7, "8"]]}]})),
        Some(vec![(1.0, 2.0), (7.0, 8.0)])
    );
    assert!(instant_by_node(&json!({"result": "nope"})).is_empty());
    assert!(by_container(&json!({})).is_empty());
    assert!(by_container(&json!({"result": [{"metric": {"container": "a"}}]})).is_empty());
    assert!(unix_now() > 1.7e9);
}
