//! Coverage for pod details, secret values, usage and events, against the
//! fake API server.

use super::*;
use crate::testing::{MockApi, list, object, pod};
use crate::types::containers::State;
use serde_json::json;

const POD: &str = "/api/v1/namespaces/shop/pods/api-1";
const METRICS: &str = "/apis/metrics.k8s.io/v1beta1/namespaces/shop/pods";
const EVENTS: &str = "/api/v1/namespaces/shop/events";

fn metrics(name: &str, containers: Value) -> Value {
    object("metrics.k8s.io/v1beta1", "PodMetrics", Some("shop"), name, json!({
        "timestamp": "2026-10-10T12:00:00Z", "window": "30s", "containers": containers
    }))
}

fn event(name: &str, extra: Value) -> Value {
    object("v1", "Event", Some("shop"), name, merge_into(json!({
        "involvedObject": {"kind": "Pod", "name": "api-1", "namespace": "shop", "uid": "uid-api-1"},
        "type": "Normal", "reason": "Started", "message": "Started container app",
        "source": {"component": "kubelet"}
    }), extra))
}

fn merge_into(mut a: Value, b: Value) -> Value {
    crate::testing::merge(&mut a, b);
    a
}

/// A pod with an init container, a sidecar and two app containers using
/// every kind of env source.
fn busy_pod() -> Value {
    pod("shop", "api-1", "n1", json!({
        "metadata": {
            "labels": {"app": "api"},
            "ownerReferences": [
                {"apiVersion": "v1", "kind": "Node", "name": "n1", "uid": "x"},
                {"apiVersion": "apps/v1", "kind": "ReplicaSet", "name": "api-7d9f", "uid": "y", "controller": true}
            ]
        },
        "spec": {
            "serviceAccountName": "api", "restartPolicy": "Always", "priorityClassName": "high",
            "initContainers": [
                {"name": "migrate", "image": "acme/migrate:1",
                 "resources": {"requests": {"cpu": "1", "memory": "1Gi"}}},
                {"name": "proxy", "image": "acme/proxy:1", "restartPolicy": "Always",
                 "resources": {"requests": {"cpu": "50m", "memory": "32Mi"}, "limits": {"cpu": "100m", "memory": "64Mi"}}}
            ],
            "containers": [
                {"name": "app", "image": "acme/app:1",
                 "ports": [{"name": "http", "containerPort": 8080}, {"containerPort": 9090, "protocol": "UDP"}],
                 "resources": {"requests": {"cpu": "250m", "memory": "128Mi"}, "limits": {"cpu": "500m", "memory": "256Mi"}},
                 "env": [
                     {"name": "PLAIN", "value": "1"},
                     {"name": "EMPTY"},
                     {"name": "MODE", "valueFrom": {"configMapKeyRef": {"name": "cfg", "key": "mode"}}},
                     {"name": "MISSING", "valueFrom": {"configMapKeyRef": {"name": "cfg", "key": "nope"}}},
                     {"name": "LOCKED", "valueFrom": {"configMapKeyRef": {"name": "locked-cfg", "key": "a"}}},
                     {"name": "DB_PASS", "valueFrom": {"secretKeyRef": {"name": "db", "key": "password"}}},
                     {"name": "POD_IP", "valueFrom": {"fieldRef": {"fieldPath": "status.podIP"}}},
                     {"name": "CPU", "valueFrom": {"resourceFieldRef": {"resource": "limits.cpu"}}}
                 ],
                 "envFrom": [
                     {"configMapRef": {"name": "cfg"}, "prefix": "CFG_"},
                     {"configMapRef": {"name": "locked-cfg"}},
                     {"secretRef": {"name": "api-keys"}},
                     {"secretRef": {"name": "locked-secret"}}
                 ]},
                {"name": "worker", "image": "acme/worker:1",
                 "resources": {"requests": {"cpu": "100m"}}}
            ],
            "ephemeralContainers": [
                {"name": "subduer-capture-1", "image": "acme/netshoot:1", "command": ["sleep", "90"]},
                {"name": "debugger", "image": "acme/netshoot:1"}
            ]
        },
        "status": {
            "phase": "Running", "podIP": "10.0.0.5", "hostIP": "192.168.1.10", "qosClass": "Burstable",
            "initContainerStatuses": [
                {"name": "migrate", "ready": false, "restartCount": 0, "image": "acme/migrate:1", "imageID": "",
                 "state": {"terminated": {"exitCode": 0, "reason": "Completed", "finishedAt": "2026-10-10T11:00:00Z"}}},
                {"name": "proxy", "ready": true, "restartCount": 1, "image": "acme/proxy:1", "imageID": "",
                 "state": {"running": {"startedAt": "2026-10-10T11:00:01Z"}}}
            ],
            "containerStatuses": [
                {"name": "app", "ready": true, "restartCount": 2, "image": "acme/app:1", "imageID": "",
                 "state": {"running": {"startedAt": "2026-10-10T11:00:05Z"}},
                 "lastState": {"terminated": {"exitCode": 137, "reason": "OOMKilled", "finishedAt": "2026-10-10T11:00:04Z"}}},
                {"name": "worker", "ready": false, "restartCount": 3, "image": "acme/worker:1", "imageID": "",
                 "state": {"waiting": {"reason": "CrashLoopBackOff"}},
                 "lastState": {"terminated": {"exitCode": 1, "finishedAt": "2026-10-10T11:00:03Z"}}}
            ],
            "ephemeralContainerStatuses": [
                {"name": "subduer-capture-1", "ready": false, "restartCount": 0, "image": "acme/netshoot:1", "imageID": "acme/netshoot@sha256:1",
                 "state": {"running": {"startedAt": "2026-10-10T12:00:00Z"}}}
            ]
        }
    }))
}

#[test]
fn a_restarted_ephemeral_container_says_how_it_last_ended() {
    let pod = json!({
        "spec": {"ephemeralContainers": [{"name": "debugger", "image": "busybox"}, {"name": "probe", "image": "busybox"}]},
        "status": {"ephemeralContainerStatuses": [
            {"name": "debugger", "restartCount": 1, "state": {"running": {"startedAt": "2026-10-10T12:05:00Z"}},
             "lastState": {"terminated": {"exitCode": 2, "reason": "Error", "finishedAt": "2026-10-10T12:04:00Z"}}},
            {"name": "probe", "state": {"running": {}}, "lastState": {"terminated": {"exitCode": 0, "finishedAt": "2026-10-10T12:00:00Z"}}}
        ]}
    });
    let c = ephemeral_containers(&pod);
    assert_eq!((c[0].restarts, c[0].last_termination.as_deref()), (1, Some("Error (exit 2) at 2026-10-10T12:04:00Z")));
    assert_eq!(c[1].last_termination.as_deref(), Some("Terminated (exit 0) at 2026-10-10T12:00:00Z"));
}

fn fake_sources(api: &MockApi) {
    api.on("GET", "/api/v1/namespaces/shop/configmaps/cfg", object("v1", "ConfigMap", Some("shop"), "cfg", json!({"data": {"mode": "fast", "level": "debug"}})));
    api.on_error("GET", "/api/v1/namespaces/shop/configmaps/locked-cfg", 403, "Forbidden", "configmaps \"locked-cfg\" is forbidden");
    api.on("GET", "/api/v1/namespaces/shop/secrets/api-keys", object("v1", "Secret", Some("shop"), "api-keys", json!({"data": {"TOKEN": "ZmFrZQ==", "OTHER": "ZmFrZQ=="}})));
    api.on_error("GET", "/api/v1/namespaces/shop/secrets/locked-secret", 403, "Forbidden", "secrets \"locked-secret\" is forbidden");
}

fn fake_node(api: &MockApi) {
    api.on("GET", "/api/v1/nodes/n1", object("v1", "Node", None, "n1", json!({"status": {"allocatable": {"cpu": "4", "memory": "8Gi"}}})));
}

#[tokio::test]
async fn pod_details_with_metrics_server() {
    let api = MockApi::new();
    api.on("GET", POD, busy_pod());
    fake_sources(&api);
    fake_node(&api);
    api.on("GET", &format!("{METRICS}/api-1"), metrics("api-1", json!([
        {"name": "proxy", "usage": {"cpu": "5m", "memory": "10Mi"}},
        {"name": "app", "usage": {"cpu": "120m", "memory": "100Mi"}},
        {"name": "worker", "usage": {}}
    ])));
    api.on("GET", EVENTS, list("EventList", vec![
        event("e-old", json!({"lastTimestamp": "2026-10-10T11:00:00Z", "firstTimestamp": "2026-10-10T10:00:00Z", "count": 3})),
        event("e-new", json!({"lastTimestamp": "2026-10-10T11:30:00Z", "type": "Warning", "reason": "BackOff"})),
        // Same name, earlier pod: different uid.
        event("e-stale", json!({"involvedObject": {"uid": "uid-older"}, "lastTimestamp": "2026-10-10T11:59:00Z"})),
    ]));

    let d = pod_details(&api.client(), "shop", "api-1").await.unwrap();
    assert_eq!(d.name, "api-1");
    assert_eq!(d.namespace, "shop");
    // Status follows the table's rules for the same pod.
    let data = serde_json::to_value(serde_json::from_value::<Pod>(busy_pod()).unwrap()).unwrap();
    assert_eq!(d.status, crate::table::pod_status(&data, false));
    assert_eq!(d.node.as_deref(), Some("n1"));
    assert_eq!(d.pod_ip.as_deref(), Some("10.0.0.5"));
    assert_eq!(d.host_ip.as_deref(), Some("192.168.1.10"));
    assert_eq!(d.qos_class.as_deref(), Some("Burstable"));
    assert_eq!(d.service_account.as_deref(), Some("api"));
    // The controller wins over the first owner.
    assert_eq!(d.owner.as_deref(), Some("ReplicaSet/api-7d9f"));
    assert_eq!(d.restart_policy.as_deref(), Some("Always"));
    assert_eq!(d.priority_class.as_deref(), Some("high"));
    assert_eq!(d.created.as_deref(), Some("2026-10-01T00:00:00Z"));
    assert_eq!(d.labels, [("app".to_owned(), "api".to_owned())]);
    assert!(d.metrics_available);

    // Requests: max(init 1000m, apps 50+250+100) = 1000m; worker has no limit, so no pod limit.
    assert_eq!(d.cpu.requests, Some(1000.0));
    assert_eq!(d.cpu.limits, None);
    assert_eq!(d.cpu.used, Some(125.0));
    assert_eq!(d.cpu.node, Some(4000.0));
    assert_eq!(d.memory.used, Some(110.0 * 1024.0 * 1024.0));
    assert_eq!(d.memory.node, Some(8.0 * 1024f64.powi(3)));

    let names: Vec<_> = d.containers.iter().map(|c| (c.name.as_str(), c.init, c.sidecar, c.ephemeral)).collect();
    assert_eq!(names, [
        ("migrate", true, false, false),
        ("proxy", true, true, false),
        ("app", false, false, false),
        ("worker", false, false, false),
        ("subduer-capture-1", false, false, true),
        ("debugger", false, false, true),
    ]);
    // Ephemeral containers: image and state only; running is their normal state.
    let capture = &d.containers[4];
    assert_eq!((capture.image.as_str(), capture.state, capture.started.as_deref()), ("acme/netshoot:1", State::Running, Some("2026-10-10T12:00:00Z")));
    assert_eq!((capture.cpu.requests, capture.ports.len(), capture.env.len()), (None, 0, 0));
    assert_eq!((d.containers[5].state, d.containers[5].detail.as_str()), (State::Starting, "Pending"));
    let migrate = &d.containers[0];
    assert_eq!(migrate.state, State::Stopped);
    assert_eq!(migrate.cpu.used, None);
    assert_eq!(migrate.last_termination, None);
    let proxy = &d.containers[1];
    assert_eq!(proxy.restarts, 1);
    assert_eq!(proxy.cpu.used, Some(5.0));
    assert_eq!(proxy.cpu.limits, Some(100.0));
    let app = &d.containers[2];
    assert!(app.ready);
    assert_eq!(app.restarts, 2);
    assert_eq!(app.image, "acme/app:1");
    assert_eq!(app.state, State::Running);
    assert_eq!(app.started.as_deref(), Some("2026-10-10T11:00:05Z"));
    assert_eq!(app.last_termination.as_deref(), Some("OOMKilled (exit 137) at 2026-10-10T11:00:04Z"));
    assert_eq!(app.ports, ["http 8080/TCP", "9090/UDP"]);
    assert_eq!(app.cpu.requests, Some(250.0));
    assert_eq!(app.cpu.limits, Some(500.0));
    assert_eq!(app.cpu.used, Some(120.0));
    assert_eq!(app.cpu.node, Some(4000.0));
    assert_eq!(app.memory.requests, Some(128.0 * 1024.0 * 1024.0));
    let worker = &d.containers[3];
    assert_eq!(worker.state, State::Failed);
    assert_eq!(worker.detail, "CrashLoopBackOff");
    assert_eq!(worker.started, None);
    assert_eq!(worker.last_termination.as_deref(), Some("Terminated (exit 1) at 2026-10-10T11:00:03Z"));
    assert_eq!(worker.cpu.used, Some(0.0));
    assert_eq!(worker.memory.limits, None);

    let env: Vec<_> = app.env.iter().map(|v| (v.name.as_str(), &v.value, v.from.as_deref())).collect();
    assert_eq!(env, [
        ("PLAIN", &EnvValue::Literal("1".into()), None),
        ("EMPTY", &EnvValue::Literal(String::new()), None),
        ("MODE", &EnvValue::ConfigMap { name: "cfg".into(), key: "mode".into(), value: Some("fast".into()) }, None),
        ("MISSING", &EnvValue::ConfigMap { name: "cfg".into(), key: "nope".into(), value: None }, None),
        ("LOCKED", &EnvValue::ConfigMap { name: "locked-cfg".into(), key: "a".into(), value: None }, None),
        ("DB_PASS", &EnvValue::Secret { name: "db".into(), key: "password".into() }, None),
        ("POD_IP", &EnvValue::Field("status.podIP".into()), None),
        ("CPU", &EnvValue::Resource("limits.cpu".into()), None),
        ("CFG_level", &EnvValue::ConfigMap { name: "cfg".into(), key: "level".into(), value: Some("debug".into()) }, Some("ConfigMap cfg")),
        ("CFG_mode", &EnvValue::ConfigMap { name: "cfg".into(), key: "mode".into(), value: Some("fast".into()) }, Some("ConfigMap cfg")),
        ("", &EnvValue::Unreadable { source: "ConfigMap locked-cfg".into(), error: env_error(&app.env[10]) }, Some("ConfigMap locked-cfg")),
        ("OTHER", &EnvValue::Secret { name: "api-keys".into(), key: "OTHER".into() }, Some("Secret api-keys")),
        ("TOKEN", &EnvValue::Secret { name: "api-keys".into(), key: "TOKEN".into() }, Some("Secret api-keys")),
        ("", &EnvValue::Unreadable { source: "Secret locked-secret".into(), error: env_error(&app.env[13]) }, Some("Secret locked-secret")),
    ]);
    assert!(env_error(&app.env[10]).contains("forbidden"));
    assert!(env_error(&app.env[13]).contains("forbidden"));
    // A secretKeyRef is shown by name only; that Secret is never read.
    assert!(api.calls_to("GET", "/api/v1/namespaces/shop/secrets/db").is_empty());

    // Newest first, and the earlier pod's event is dropped.
    let events: Vec<_> = d.events.iter().map(|e| (e.reason.as_str(), e.last_seen.as_str())).collect();
    assert_eq!(events, [("BackOff", "2026-10-10T11:30:00Z"), ("Started", "2026-10-10T11:00:00Z")]);
    assert_eq!(d.events[1].count, 3);
    assert_eq!(d.events[1].first_seen, "2026-10-10T10:00:00Z");
    assert_eq!(d.events[1].source, "kubelet");
    assert_eq!(d.events[1].object, None);
    let call = &api.calls_to("GET", EVENTS)[0];
    assert!(call.query.contains("fieldSelector="), "{}", call.query);
    assert!(call.query.contains("involvedObject.name%3Dapi-1"), "{}", call.query);
    // metrics-server answered, so the kubelet wasn't asked.
    assert!(api.calls_to("GET", "/api/v1/nodes/n1/proxy/stats/summary").is_empty());
}

fn env_error(v: &EnvVar) -> String {
    match &v.value {
        EnvValue::Unreadable { error, .. } => error.clone(),
        other => panic!("not unreadable: {other:?}"),
    }
}

#[tokio::test]
async fn pod_details_falls_back_to_the_kubelet() {
    let api = MockApi::new();
    api.on("GET", POD, pod("shop", "api-1", "n1", json!({
        "spec": {"containers": [{"name": "app", "image": "acme/app:1",
            "resources": {"requests": {"cpu": "100m", "memory": "64Mi"}, "limits": {"cpu": "200m", "memory": "128Mi"}}}],
            // Has no limits of its own, and mustn't hide the pod's.
            "ephemeralContainers": [{"name": "subduer-capture-1", "image": "acme/netshoot:1"}]}
    })));
    api.on("GET", "/api/v1/nodes/n1/proxy/stats/summary", json!({"pods": [
        {"podRef": {"name": "api-1", "namespace": "shop"}, "containers": [
            {"name": "app", "cpu": {"usageNanoCores": 30e6}, "memory": {"workingSetBytes": 1000.0}}]}
    ]}));
    // A node without allocatable figures.
    api.on("GET", "/api/v1/nodes/n1", object("v1", "Node", None, "n1", json!({"status": {}})));

    let d = pod_details(&api.client(), "shop", "api-1").await.unwrap();
    assert!(api.calls_to("GET", &format!("{METRICS}/api-1")).len() == 1);
    assert!(d.metrics_available);
    assert_eq!(d.status, "Running");
    assert_eq!(d.owner, None);
    assert_eq!(d.cpu.requests, Some(100.0));
    assert_eq!(d.cpu.limits, Some(200.0));
    assert_eq!(d.memory.limits, Some(128.0 * 1024.0 * 1024.0));
    assert_eq!(d.cpu.used, Some(30.0));
    assert_eq!(d.memory.used, Some(1000.0));
    assert_eq!(d.cpu.node, None);
    assert_eq!(d.containers[0].cpu.used, Some(30.0));
    assert!(d.containers[0].env.is_empty());
    // Events unavailable (404) → none.
    assert!(d.events.is_empty());
}

#[tokio::test]
async fn pod_details_without_any_metrics_or_node() {
    let api = MockApi::new();
    let mut p = pod("shop", "api-1", "n1", json!({
        "metadata": {"deletionTimestamp": "2026-10-10T12:00:00Z",
                     "ownerReferences": [{"apiVersion": "v1", "kind": "Node", "name": "n1", "uid": "x"}]},
        "status": {"phase": "Pending"}
    }));
    p["spec"].as_object_mut().unwrap().remove("nodeName");
    p["status"].as_object_mut().unwrap().remove("containerStatuses");
    api.on("GET", POD, p);

    let d = pod_details(&api.client(), "shop", "api-1").await.unwrap();
    assert_eq!(d.status, "Terminating");
    assert_eq!(d.node, None);
    // Non-controller owners still show.
    assert_eq!(d.owner.as_deref(), Some("Node/n1"));
    assert!(!d.metrics_available);
    assert_eq!(d.cpu.used, None);
    assert_eq!(d.memory.used, None);
    assert_eq!(d.cpu.requests, None);
    assert_eq!(d.cpu.limits, None);
    let c = &d.containers[0];
    assert!(!c.ready);
    assert_eq!(c.restarts, 0);
    assert_eq!(c.state, State::Starting);
    assert_eq!(c.detail, "Pending");
    // With no node, neither the kubelet nor the Node is asked for.
    assert!(api.calls().iter().all(|c| !c.path.starts_with("/api/v1/nodes")));
}

#[tokio::test]
async fn pod_details_missing_pod_is_an_error() {
    let api = MockApi::new();
    let err = pod_details(&api.client(), "shop", "gone").await.unwrap_err();
    assert!(matches!(err, Error::Kube(kube::Error::Api(ref s)) if s.code == 404), "{err:?}");
}

#[tokio::test]
async fn secret_value_decodes_text_and_describes_binary() {
    let api = MockApi::new();
    api.on("GET", "/api/v1/namespaces/shop/secrets/db", object("v1", "Secret", Some("shop"), "db", json!({
        "data": {"password": "ZmFrZS1wYXNz", "cert": "/9j/AA=="}
    })));
    let client = api.client();
    assert_eq!(secret_value(&client, "shop", "db", "password").await.unwrap(), "fake-pass");
    assert_eq!(secret_value(&client, "shop", "db", "cert").await.unwrap(), "<binary, 4 bytes>");
    let err = secret_value(&client, "shop", "db", "nope").await.unwrap_err();
    assert!(matches!(&err, Error::NotFound(m) if m == "key \"nope\" in Secret db"), "{err:?}");
    let err = secret_value(&client, "shop", "missing", "password").await.unwrap_err();
    assert!(matches!(err, Error::Kube(_)));
}

#[tokio::test]
async fn secret_value_of_a_secret_without_data() {
    let api = MockApi::new();
    api.on("GET", "/api/v1/namespaces/shop/secrets/empty", object("v1", "Secret", Some("shop"), "empty", json!({})));
    let client = api.client();
    assert!(matches!(secret_value(&client, "shop", "empty", "a").await, Err(Error::NotFound(_))));
    assert!(secret_values(&client, "shop", "empty").await.unwrap().is_empty());
}

#[tokio::test]
async fn secret_values_marks_binary_values() {
    let api = MockApi::new();
    api.on("GET", "/api/v1/namespaces/shop/secrets/db", object("v1", "Secret", Some("shop"), "db", json!({
        "data": {"user": "YWNtZQ==", "cert": "/9j/AA=="}
    })));
    let client = api.client();
    let values = secret_values(&client, "shop", "db").await.unwrap();
    assert_eq!(values, [("cert".to_owned(), None), ("user".to_owned(), Some("acme".to_owned()))]);
    assert!(secret_values(&client, "shop", "missing").await.is_err());
}

#[tokio::test]
async fn namespace_usage_sums_every_pod() {
    let api = MockApi::new();
    api.on("GET", METRICS, list("PodMetricsList", vec![
        metrics("a", json!([{"name": "app", "usage": {"cpu": "100m", "memory": "1Ki"}},
                            {"name": "side", "usage": {"cpu": "50000000n", "memory": "1Ki"}}])),
        metrics("b", json!([{"name": "app", "usage": {"cpu": "1", "memory": "2Ki"}}])),
        // No containers array: skipped.
        object("metrics.k8s.io/v1beta1", "PodMetrics", Some("shop"), "c", json!({})),
    ]));
    assert_eq!(namespace_usage(&api.client(), "shop").await, Some((1150.0, 4096.0)));

    let none = MockApi::new();
    assert_eq!(namespace_usage(&none.client(), "shop").await, None);
}

#[tokio::test]
async fn pods_usage_from_metrics_server() {
    let api = MockApi::new();
    api.on_query("GET", METRICS, "labelSelector=app%3Dapi", list("PodMetricsList", vec![
        metrics("api-1", json!([{"name": "app", "usage": {"cpu": "10m", "memory": "100"}}])),
        object("metrics.k8s.io/v1beta1", "PodMetrics", Some("shop"), "api-2", json!({})),
    ]));
    let pods = vec![("api-1".to_owned(), Some("n1".to_owned()))];
    let usage = pods_usage(&api.client(), "shop", "app=api", &pods).await.unwrap();
    assert_eq!(usage.len(), 1);
    assert_eq!(usage["api-1"]["app"], (10.0, 100.0));
    assert!(api.calls().iter().all(|c| !c.path.contains("stats/summary")));
}

#[tokio::test]
async fn pods_usage_falls_back_to_kubelets() {
    let api = MockApi::new();
    let summary = |pods: Value| json!({"pods": pods});
    let c = |cpu: f64, mem: f64| json!([{"name": "app", "cpu": {"usageNanoCores": cpu}, "memory": {"workingSetBytes": mem}}]);
    api.on("GET", "/api/v1/nodes/n1/proxy/stats/summary", summary(json!([
        {"podRef": {"name": "api-1", "namespace": "shop"}, "containers": c(1e6, 1.0)},
        {"podRef": {"name": "api-2", "namespace": "shop"}, "containers": c(2e6, 2.0)},
        {"podRef": {"name": "other", "namespace": "shop"}, "containers": c(9e6, 9.0)}
    ])));
    api.on_error("GET", "/api/v1/nodes/n2/proxy/stats/summary", 503, "ServiceUnavailable", "down");
    let pods = vec![
        ("api-1".to_owned(), Some("n1".to_owned())),
        ("api-2".to_owned(), Some("n1".to_owned())),
        ("api-3".to_owned(), Some("n2".to_owned())),
        ("api-4".to_owned(), None),
        // On a readable node but not in its summary yet.
        ("api-5".to_owned(), Some("n1".to_owned())),
    ];
    let usage = pods_usage(&api.client(), "shop", "app=api", &pods).await.unwrap();
    let mut names: Vec<_> = usage.keys().cloned().collect();
    names.sort();
    assert_eq!(names, ["api-1", "api-2"]);
    assert_eq!(usage["api-2"]["app"], (2.0, 2.0));
    // One request per node, however many pods it runs.
    assert_eq!(api.calls_to("GET", "/api/v1/nodes/n1/proxy/stats/summary").len(), 1);
}

#[tokio::test]
async fn pods_usage_without_any_source() {
    let api = MockApi::new();
    let pods = vec![("api-1".to_owned(), Some("n1".to_owned())), ("api-2".to_owned(), None)];
    assert!(pods_usage(&api.client(), "shop", "app=api", &pods).await.is_none());
    assert!(pods_usage(&api.client(), "shop", "app=api", &[]).await.is_none());
}

#[tokio::test]
async fn pod_usage_sources() {
    let api = MockApi::new();
    let client = api.client();
    // Neither metrics-server nor a node.
    assert!(pod_usage(&client, "shop", "api-1", None).await.is_none());
    // Kubelet can't be read either.
    assert!(pod_usage(&client, "shop", "api-1", Some("n1")).await.is_none());
    // Metrics without containers fall through to the kubelet.
    api.on("GET", &format!("{METRICS}/api-1"), object("metrics.k8s.io/v1beta1", "PodMetrics", Some("shop"), "api-1", json!({})));
    api.on("GET", "/api/v1/nodes/n1/proxy/stats/summary", json!({"pods": [
        {"podRef": {"name": "api-1", "namespace": "shop"}, "containers": [{"name": "app", "cpu": {"usageNanoCores": 4e6}, "memory": {"workingSetBytes": 5.0}}]}
    ]}));
    assert_eq!(pod_usage(&client, "shop", "api-1", Some("n1")).await.unwrap()["app"], (4.0, 5.0));
}

#[tokio::test]
async fn node_allocatable_needs_both_figures() {
    let api = MockApi::new();
    api.on("GET", "/api/v1/nodes/n1", object("v1", "Node", None, "n1", json!({"status": {"allocatable": {"cpu": "2"}}})));
    fake_node_named(&api, "n2");
    let client = api.client();
    assert_eq!(node_allocatable(&client, None).await, None);
    assert_eq!(node_allocatable(&client, Some("n1")).await, None);
    assert_eq!(node_allocatable(&client, Some("gone")).await, None);
    assert_eq!(node_allocatable(&client, Some("n2")).await, Some((500.0, 1024.0)));
}

fn fake_node_named(api: &MockApi, name: &str) {
    api.on("GET", &format!("/api/v1/nodes/{name}"), object("v1", "Node", None, name, json!({"status": {"allocatable": {"cpu": "500m", "memory": "1Ki"}}})));
}

#[tokio::test]
async fn list_events_filters_labels_and_sorts() {
    let api = MockApi::new();
    api.on("GET", EVENTS, list("EventList", vec![
        // events.k8s.io-style: eventTime + series, reportingComponent.
        event("series", json!({"source": null, "reportingComponent": "acme-controller", "eventTime": "2026-10-10T09:00:00.000000Z",
                               "series": {"count": 7, "lastObservedTime": "2026-10-10T11:45:00.000000Z"},
                               "message": "  padded\n"})),
        // Only a creation time.
        event("bare", json!({"source": null})),
        event("drop", json!({"lastTimestamp": "2026-10-10T11:50:00Z"})),
        event("other-kind", json!({"involvedObject": {"kind": "ReplicaSet", "name": "api-7d9f"},
                                   "lastTimestamp": "2026-10-10T11:40:00Z"})),
    ]));
    let client = api.client();
    let events = list_events(&client, "shop", Some("type=Warning"), |e| match e.metadata.name.as_deref() {
        Some("drop") => None,
        Some("other-kind") => Some(Some("ReplicaSet api-7d9f".to_owned())),
        _ => Some(None),
    })
    .await;
    let order: Vec<_> = events.iter().map(|e| e.last_seen.as_str()).collect();
    assert_eq!(order, ["2026-10-10T11:45:00.000000Z", "2026-10-10T11:40:00Z", "2026-10-01T00:00:00Z"]);
    let series = &events[0];
    assert_eq!(series.count, 7);
    assert_eq!(series.first_seen, "2026-10-10T09:00:00.000000Z");
    assert_eq!(series.source, "acme-controller");
    assert_eq!(series.message, "padded");
    assert_eq!(events[1].object.as_deref(), Some("ReplicaSet api-7d9f"));
    let bare = &events[2];
    assert_eq!(bare.first_seen, "2026-10-01T00:00:00Z");
    assert_eq!(bare.count, 1);
    assert_eq!(bare.source, "");
    assert!(api.calls_to("GET", EVENTS)[0].query.contains("fieldSelector=type%3DWarning"));

    // No field selector: none sent.
    let all = list_events(&client, "shop", None, |_| Some(None)).await;
    assert_eq!(all.len(), 4);
    assert!(!api.calls_to("GET", EVENTS)[1].query.contains("fieldSelector"));
}

#[tokio::test]
async fn list_events_forbidden_is_empty() {
    let api = MockApi::new();
    api.on_error("GET", EVENTS, 403, "Forbidden", "events is forbidden");
    assert!(list_events(&api.client(), "shop", None, |_| Some(None)).await.is_empty());
}

#[tokio::test]
async fn pod_events_without_uid_keep_all() {
    let api = MockApi::new();
    api.on("GET", EVENTS, list("EventList", vec![
        event("a", json!({"lastTimestamp": "2026-10-10T11:00:00Z"})),
        event("b", json!({"involvedObject": {"uid": "uid-older"}, "lastTimestamp": "2026-10-10T10:00:00Z"})),
    ]));
    let client = api.client();
    assert_eq!(pod_events(&client, "shop", "api-1", None).await.len(), 2);
    assert_eq!(pod_events(&client, "shop", "api-1", Some("uid-older")).await.len(), 1);
}

#[test]
fn summarize_with_an_empty_event() {
    let s = summarize(&Event::default(), Some("Pod x".into()));
    assert_eq!(s, EventSummary {
        type_: String::new(),
        reason: String::new(),
        message: String::new(),
        count: 1,
        first_seen: String::new(),
        last_seen: String::new(),
        source: String::new(),
        object: Some("Pod x".into()),
    });
}

#[test]
fn resources_and_ports_of_a_bare_container() {
    let c = Container { name: "app".into(), ..Default::default() };
    let (cpu, memory) = container_resources(&c);
    assert_eq!(cpu, Usage::default());
    assert_eq!(memory, Usage::default());
    assert!(port_names(&c).is_empty());
    assert!(env_vars(&c, &HashMap::new(), &HashMap::new()).is_empty());
}

#[test]
fn env_from_sources_not_read_are_skipped() {
    let c: Container = serde_json::from_value(json!({
        "name": "app",
        "envFrom": [{"configMapRef": {"name": "cfg"}, "secretRef": {"name": "s"}}]
    }))
    .unwrap();
    // Neither source in the maps (e.g. read for another container set): nothing listed.
    assert!(env_vars(&c, &HashMap::new(), &HashMap::new()).is_empty());
    assert!(positive(0.0).is_none());
    assert_eq!(positive(1.5), Some(1.5));
}

#[tokio::test]
async fn reads_only_referenced_sources() {
    let api = MockApi::new();
    fake_sources(&api);
    let c: Container = serde_json::from_value(json!({
        "name": "app",
        "env": [{"name": "A", "value": "1"},
                {"name": "B", "valueFrom": {"fieldRef": {"fieldPath": "metadata.name"}}},
                {"name": "MODE", "valueFrom": {"configMapKeyRef": {"name": "cfg", "key": "mode"}}}],
        "envFrom": [{"secretRef": {"name": "api-keys"}}, {"configMapRef": {"name": "cfg"}}]
    }))
    .unwrap();
    let client = api.client();
    let cms = read_config_maps(&client, "shop", &[&c, &c]).await;
    assert_eq!(cms.len(), 1);
    assert_eq!(cms["cfg"].as_ref().unwrap()["mode"], "fast");
    // Read once though referenced three times.
    assert_eq!(api.calls_to("GET", "/api/v1/namespaces/shop/configmaps/cfg").len(), 1);
    let keys = read_secret_keys(&client, "shop", &[&c]).await;
    assert_eq!(keys["api-keys"].as_ref().unwrap(), &["OTHER", "TOKEN"]);
    assert!(read_config_maps(&client, "shop", &[]).await.is_empty());
}
