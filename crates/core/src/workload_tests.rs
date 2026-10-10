//! Coverage for workload details against the fake API server, and the
//! kind-specific status and info rules.

use super::*;
use crate::testing::{MockApi, kind, list, merge, object, pod};
use crate::types::containers::State;
use crate::types::EnvValue;
use serde_json::{Value, json};

const PODS: &str = "/api/v1/namespaces/shop/pods";
const EVENTS: &str = "/api/v1/namespaces/shop/events";
const METRICS: &str = "/apis/metrics.k8s.io/v1beta1/namespaces/shop/pods";

fn deployment_kind() -> KindRef {
    kind("apps", "v1", "Deployment", "deployments", true)
}
fn statefulset_kind() -> KindRef {
    kind("apps", "v1", "StatefulSet", "statefulsets", true)
}
fn job_kind() -> KindRef {
    kind("batch", "v1", "Job", "jobs", true)
}
fn cron_job_kind() -> KindRef {
    kind("batch", "v1", "CronJob", "cronjobs", true)
}

fn with(mut v: Value, extra: Value) -> Value {
    merge(&mut v, extra);
    v
}

fn owner(kind: &str, name: &str, controller: bool) -> Value {
    json!({"apiVersion": "apps/v1", "kind": kind, "name": name, "uid": format!("uid-{name}"), "controller": controller})
}

fn info<'a>(info: &'a [(String, String)], key: &str) -> Option<&'a str> {
    info.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn app_container() -> Value {
    json!({"name": "app", "image": "acme/app:1",
           "resources": {"requests": {"cpu": "250m", "memory": "128Mi"}, "limits": {"cpu": "500m", "memory": "256Mi"}}})
}

/// A pod of `owner_kind/owner_name`, with the app container's resources.
fn owned_pod(name: &str, owner_kind: &str, owner_name: &str, labels: Value, extra: Value) -> Value {
    let base = pod("shop", name, "n1", json!({
        "metadata": {"labels": labels, "ownerReferences": [owner(owner_kind, owner_name, true)]},
        "spec": {"containers": [app_container()]}
    }));
    with(base, extra)
}

fn event(name: &str, kind: &str, object: &str, last: &str) -> Value {
    crate::testing::object("v1", "Event", Some("shop"), name, json!({
        "involvedObject": {"kind": kind, "name": object, "namespace": "shop"},
        "type": "Normal", "reason": name, "message": "m", "lastTimestamp": last, "source": {"component": "acme"}
    }))
}

fn metrics(name: &str, containers: Value) -> Value {
    object("metrics.k8s.io/v1beta1", "PodMetrics", Some("shop"), name, json!({"containers": containers}))
}

fn deployment() -> Value {
    object("apps/v1", "Deployment", Some("shop"), "api", json!({
        "metadata": {"generation": 4, "labels": {"team": "shop"},
                     "annotations": {"deployment.kubernetes.io/revision": "3"}},
        "spec": {
            "replicas": 3,
            "selector": {"matchLabels": {"app": "api"}},
            "strategy": {"type": "RollingUpdate", "rollingUpdate": {"maxSurge": 1, "maxUnavailable": "10%"}},
            "minReadySeconds": 5,
            "progressDeadlineSeconds": 300,
            "template": {
                "metadata": {"labels": {"app": "api"},
                             "annotations": {"kubectl.kubernetes.io/restartedAt": "2026-10-09T08:00:00Z"}},
                "spec": {
                    "serviceAccountName": "api",
                    "priorityClassName": "high",
                    "initContainers": [{"name": "proxy", "image": "acme/proxy:1", "restartPolicy": "Always",
                        "resources": {"requests": {"cpu": "50m", "memory": "32Mi"}, "limits": {"cpu": "100m", "memory": "64Mi"}}}],
                    "containers": [with(app_container(), json!({
                        "ports": [{"name": "http", "containerPort": 8080}],
                        "env": [{"name": "MODE", "valueFrom": {"configMapKeyRef": {"name": "cfg", "key": "mode"}}}],
                        "envFrom": [{"secretRef": {"name": "api-keys"}}]
                    }))]
                }
            }
        },
        "status": {"observedGeneration": 4, "replicas": 3, "readyReplicas": 2, "updatedReplicas": 2, "availableReplicas": 2,
                   "conditions": [{"type": "Available", "status": "True", "reason": "MinimumReplicasAvailable",
                                   "message": "ok", "lastTransitionTime": "2026-10-09T08:00:00Z"}]}
    }))
}

fn replica_set(name: &str, revision: &str, hash: &str, owner_name: &str) -> Value {
    object("apps/v1", "ReplicaSet", Some("shop"), name, json!({
        "metadata": {"labels": {"app": "api", "pod-template-hash": hash},
                     "annotations": {"deployment.kubernetes.io/revision": revision},
                     "ownerReferences": [owner("Deployment", owner_name, true)]},
        "spec": {"selector": {"matchLabels": {"app": "api"}}}
    }))
}

fn fake_deployment(api: &MockApi) {
    api.on("GET", "/apis/apps/v1/namespaces/shop/deployments/api", deployment());
    api.on("GET", "/apis/apps/v1/namespaces/shop/replicasets", list("ReplicaSetList", vec![
        replica_set("api-new", "3", "new", "api"),
        replica_set("api-old", "2", "old", "api"),
        // Matches the selector but belongs to another Deployment.
        replica_set("api-foreign", "1", "foreign", "api-canary"),
    ]));
    let labels = |hash: &str| json!({"app": "api", "pod-template-hash": hash});
    api.on("GET", PODS, list("PodList", vec![
        owned_pod("api-old-bbbbb", "ReplicaSet", "api-old", labels("old"), json!({
            "status": {"containerStatuses": [{"name": "app", "ready": true, "restartCount": 2, "image": "acme/app:1",
                                              "imageID": "", "state": {"running": {}}}]}})),
        owned_pod("api-new-zzzzz", "ReplicaSet", "api-new", labels("new"), json!({
            "status": {"phase": "Failed", "containerStatuses": [{"name": "app", "ready": false, "restartCount": 0,
                       "image": "acme/app:1", "imageID": "", "state": {"terminated": {"exitCode": 1, "reason": "Error"}}}]}})),
        owned_pod("api-new-aaaaa", "ReplicaSet", "api-new", labels("new"), json!({})),
        owned_pod("api-foreign-ccccc", "ReplicaSet", "api-foreign", labels("foreign"), json!({})),
        // No controller at all.
        pod("shop", "api-stray", "n2", json!({"metadata": {"labels": {"app": "api"}, "ownerReferences": [owner("ReplicaSet", "api-new", false)]}})),
    ]));
    api.on("GET", "/api/v1/namespaces/shop/configmaps/cfg", object("v1", "ConfigMap", Some("shop"), "cfg", json!({"data": {"mode": "fast"}})));
    api.on("GET", "/api/v1/namespaces/shop/secrets/api-keys", object("v1", "Secret", Some("shop"), "api-keys", json!({"data": {"TOKEN": "ZmFrZQ=="}})));
}

#[tokio::test]
async fn deployment_details() {
    let api = MockApi::new();
    fake_deployment(&api);
    api.on("GET", METRICS, list("PodMetricsList", vec![
        metrics("api-new-aaaaa", json!([{"name": "app", "usage": {"cpu": "100m", "memory": "100Mi"}},
                                        {"name": "proxy", "usage": {"cpu": "10m", "memory": "10Mi"}}])),
        metrics("api-old-bbbbb", json!([{"name": "app", "usage": {"cpu": "50m", "memory": "50Mi"}}])),
        // The selector matches it, but it isn't this Deployment's.
        metrics("api-foreign-ccccc", json!([{"name": "app", "usage": {"cpu": "1", "memory": "1Gi"}}])),
    ]));
    api.on("GET", EVENTS, list("EventList", vec![
        event("ScalingReplicaSet", "Deployment", "api", "2026-10-10T10:00:00Z"),
        event("SuccessfulCreate", "ReplicaSet", "api-new", "2026-10-10T11:00:00Z"),
        event("Pulled", "Pod", "api-new-aaaaa", "2026-10-10T11:30:00Z"),
        event("Foreign", "Pod", "api-foreign-ccccc", "2026-10-10T11:40:00Z"),
        event("Other", "Deployment", "web", "2026-10-10T11:50:00Z"),
        object("v1", "Event", Some("shop"), "no-kind", json!({"involvedObject": {"name": "api"}})),
    ]));

    let d = workload_details(&api.client(), &deployment_kind(), "shop", "api").await.unwrap();
    assert_eq!(d.kind, "Deployment");
    assert_eq!((d.name.as_str(), d.namespace.as_str()), ("api", "shop"));
    assert_eq!(d.status, "Rolling out");
    assert_eq!(d.replicas, Some(ReplicaCounts { desired: 3, current: 3, ready: 2, updated: 2, available: 2 }));
    assert_eq!(d.suspended, None);
    assert_eq!(d.created.as_deref(), Some("2026-10-01T00:00:00Z"));
    assert_eq!(d.labels, [("team".to_owned(), "shop".to_owned())]);
    assert_eq!(d.conditions.len(), 1);
    assert_eq!(d.conditions[0].reason, "MinimumReplicasAvailable");
    assert_eq!(d.conditions[0].last_transition.as_deref(), Some("2026-10-09T08:00:00Z"));
    assert_eq!(d.info, [
        ("Strategy", "RollingUpdate · max surge 1 · max unavailable 10%"),
        ("Revision", "3"),
        ("Min ready", "5s"),
        ("Progress deadline", "300s"),
        ("Last restart", "2026-10-09T08:00:00Z"),
        ("Selector", "app=api"),
        ("Service account", "api"),
        ("Priority class", "high"),
    ]
    .map(|(k, v)| (k.to_owned(), v.to_owned())));

    // Only pods of its own ReplicaSets, sorted by name.
    let pods: Vec<_> = d.pods.iter().map(|p| (p.name.as_str(), p.updated, p.finished, p.cpu)).collect();
    assert_eq!(pods, [
        ("api-new-aaaaa", true, false, Some(110.0)),
        ("api-new-zzzzz", true, true, None),
        ("api-old-bbbbb", false, false, Some(50.0)),
    ]);
    let old = &d.pods[2];
    assert_eq!(old.ready, "1/1");
    assert_eq!(old.restarts, 2);
    assert_eq!(old.node.as_deref(), Some("n1"));
    assert_eq!(old.status, "Running");
    assert_eq!(old.memory, Some(50.0 * 1024.0 * 1024.0));
    assert_eq!(old.containers[0].state, State::Running);
    assert_eq!(d.pods[1].status, "Error");
    assert_eq!(d.pods[1].ready, "0/1");

    let containers: Vec<_> = d.containers.iter().map(|c| (c.name.as_str(), c.init, c.sidecar, c.cpu.used)).collect();
    assert_eq!(containers, [("proxy", true, true, Some(10.0)), ("app", false, false, Some(150.0))]);
    let app = &d.containers[1];
    assert_eq!(app.image, "acme/app:1");
    assert_eq!(app.ports, ["http 8080/TCP"]);
    assert_eq!(app.cpu.requests, Some(250.0));
    assert_eq!(app.memory.limits, Some(256.0 * 1024.0 * 1024.0));
    assert_eq!(app.env[0].value, EnvValue::ConfigMap { name: "cfg".into(), key: "mode".into(), value: Some("fast".into()) });
    assert_eq!(app.env[1].value, EnvValue::Secret { name: "api-keys".into(), key: "TOKEN".into() });

    // Totals over the two live pods only; the foreign pod's metrics are left out.
    assert!(d.metrics_available);
    assert_eq!(d.cpu.requests, Some(500.0));
    assert_eq!(d.cpu.limits, Some(1000.0));
    assert_eq!(d.cpu.used, Some(160.0));
    assert_eq!(d.cpu.node, None);
    assert_eq!(d.memory.requests, Some(256.0 * 1024.0 * 1024.0));
    assert_eq!(d.memory.limits, Some(512.0 * 1024.0 * 1024.0));
    assert_eq!(d.memory.used, Some(160.0 * 1024.0 * 1024.0));

    let events: Vec<_> = d.events.iter().map(|e| (e.reason.as_str(), e.object.as_deref())).collect();
    assert_eq!(events, [
        ("Pulled", Some("Pod api-new-aaaaa")),
        ("SuccessfulCreate", Some("ReplicaSet api-new")),
        ("ScalingReplicaSet", None),
    ]);

    let rs_call = &api.calls_to("GET", "/apis/apps/v1/namespaces/shop/replicasets")[0];
    assert!(rs_call.query.contains("labelSelector=app%3Dapi"), "{}", rs_call.query);
    let pod_call = &api.calls_to("GET", PODS)[0];
    assert!(pod_call.query.contains("labelSelector=app%3Dapi"), "{}", pod_call.query);
    assert!(api.calls_to("GET", METRICS)[0].query.contains("labelSelector=app%3Dapi"));
}

#[tokio::test]
async fn deployment_without_pods_or_metrics() {
    let api = MockApi::new();
    api.on("GET", "/apis/apps/v1/namespaces/shop/deployments/api", with(deployment(), json!({
        "spec": {"replicas": 0, "strategy": {"type": "Recreate"}, "minReadySeconds": 0, "progressDeadlineSeconds": 600},
        "status": {"replicas": 0, "readyReplicas": 0, "updatedReplicas": 0, "availableReplicas": 0, "conditions": []}
    })));
    api.on("GET", "/apis/apps/v1/namespaces/shop/replicasets", list("ReplicaSetList", vec![]));

    let d = workload_details(&api.client(), &deployment_kind(), "shop", "api").await.unwrap();
    assert_eq!(d.status, "Scaled to 0");
    assert_eq!(info(&d.info, "Strategy"), Some("Recreate"));
    assert_eq!(info(&d.info, "Min ready"), None);
    assert_eq!(info(&d.info, "Progress deadline"), None);
    assert!(d.pods.is_empty());
    // Without ReplicaSets there's nothing to own pods, so pods aren't listed.
    assert!(api.calls_to("GET", PODS).is_empty());
    assert!(!d.metrics_available);
    assert_eq!(d.cpu, Usage::default());
    assert_eq!(d.memory, Usage::default());
    assert_eq!(d.containers[1].cpu.used, None);
    assert!(d.events.is_empty());
}

#[tokio::test]
async fn deployment_with_kubelet_usage() {
    let api = MockApi::new();
    fake_deployment(&api);
    api.on("GET", "/api/v1/nodes/n1/proxy/stats/summary", json!({"pods": [
        {"podRef": {"name": "api-new-aaaaa", "namespace": "shop"},
         "containers": [{"name": "app", "cpu": {"usageNanoCores": 20e6}, "memory": {"workingSetBytes": 2000.0}}]}
    ]}));
    let d = workload_details(&api.client(), &deployment_kind(), "shop", "api").await.unwrap();
    assert!(d.metrics_available);
    assert_eq!(d.cpu.used, Some(20.0));
    assert_eq!(d.memory.used, Some(2000.0));
    assert_eq!(d.pods[0].cpu, Some(20.0));
    assert_eq!(d.pods[2].cpu, None);
    assert_eq!(d.containers[0].cpu.used, Some(0.0));
}

#[tokio::test]
async fn current_pods_skips_finished_ones() {
    let api = MockApi::new();
    fake_deployment(&api);
    let (selector, pods) = current_pods(&api.client(), &deployment_kind(), "shop", "api").await.unwrap();
    assert_eq!(selector, "app=api");
    assert_eq!(pods, [
        ("api-new-aaaaa".to_owned(), Some("n1".to_owned())),
        ("api-old-bbbbb".to_owned(), Some("n1".to_owned())),
    ]);
}

#[tokio::test]
async fn load_errors() {
    let api = MockApi::new();
    let client = api.client();
    let daemon_set = kind("apps", "v1", "DaemonSet", "daemonsets", true);
    let err = workload_details(&client, &daemon_set, "shop", "agent").await.unwrap_err();
    assert_eq!(err.to_string(), "no details view for DaemonSet");
    assert!(current_pods(&client, &daemon_set, "shop", "agent").await.is_err());
    // Missing workloads of every kind.
    for k in [deployment_kind(), statefulset_kind(), job_kind(), cron_job_kind()] {
        let err = workload_details(&client, &k, "shop", "gone").await.unwrap_err();
        assert!(matches!(err, Error::Kube(kube::Error::Api(ref s)) if s.code == 404), "{}: {err:?}", k.kind);
    }
}

#[tokio::test]
async fn pod_list_errors_are_returned() {
    let api = MockApi::new();
    fake_deployment(&api);
    api.on_error("GET", PODS, 403, "Forbidden", "pods is forbidden");
    let err = workload_details(&api.client(), &deployment_kind(), "shop", "api").await.unwrap_err();
    assert_eq!(err.user_message(), "pods is forbidden");

    let api = MockApi::new();
    api.on("GET", "/apis/apps/v1/namespaces/shop/deployments/api", deployment());
    api.on_error("GET", "/apis/apps/v1/namespaces/shop/replicasets", 403, "Forbidden", "replicasets is forbidden");
    let err = workload_details(&api.client(), &deployment_kind(), "shop", "api").await.unwrap_err();
    assert_eq!(err.user_message(), "replicasets is forbidden");
}

fn db_statefulset() -> Value {
    object("apps/v1", "StatefulSet", Some("shop"), "db", json!({
        "metadata": {"generation": 1, "ownerReferences": [owner("Database", "acme-db", true)]},
        "spec": {
            "replicas": 4,
            "serviceName": "db",
            "podManagementPolicy": "Parallel",
            "selector": {"matchLabels": {"app": "db"}},
            "updateStrategy": {"type": "RollingUpdate", "rollingUpdate": {"partition": 1, "maxUnavailable": 2}},
            "volumeClaimTemplates": [
                {"metadata": {"name": "data"}, "spec": {"storageClassName": "fast", "resources": {"requests": {"storage": "10Gi"}}}},
                {"metadata": {"name": "scratch"}, "spec": {}}
            ],
            "persistentVolumeClaimRetentionPolicy": {"whenDeleted": "Delete"},
            "minReadySeconds": 10,
            "template": {"metadata": {"labels": {"app": "db"}}, "spec": {"containers": [app_container()]}}
        },
        "status": {"observedGeneration": 1, "replicas": 4, "readyReplicas": 4, "updatedReplicas": 4, "availableReplicas": 4,
                   "currentRevision": "db-1", "updateRevision": "db-2"}
    }))
}

#[tokio::test]
async fn statefulset_details() {
    let api = MockApi::new();
    api.on("GET", "/apis/apps/v1/namespaces/shop/statefulsets/db", db_statefulset());
    let pod_of = |n: &str, rev: &str| owned_pod(n, "StatefulSet", "db", json!({"app": "db", "controller-revision-hash": rev}), json!({}));
    api.on("GET", PODS, list("PodList", vec![pod_of("db-10", "db-1"), pod_of("db-2", "db-2"), pod_of("db-0", "db-1"), pod_of("db-1", "db-2")]));

    let d = workload_details(&api.client(), &statefulset_kind(), "shop", "db").await.unwrap();
    assert_eq!(d.kind, "StatefulSet");
    assert_eq!(d.status, "Rolling out");
    assert_eq!(d.info, [
        ("Strategy", "RollingUpdate · partition 1 · max unavailable 2"),
        ("Pod management", "Parallel"),
        ("Service", "db"),
        ("Volume claims", "data (10Gi, fast), scratch"),
        ("Claim retention", "when deleted: Delete · when scaled: Retain"),
        ("Revision", "db-1 → db-2"),
        ("Min ready", "10s"),
        ("Selector", "app=db"),
        ("Controlled by", "Database/acme-db"),
    ]
    .map(|(k, v)| (k.to_owned(), v.to_owned())));
    // By ordinal, not by name.
    let pods: Vec<_> = d.pods.iter().map(|p| (p.name.as_str(), p.updated)).collect();
    assert_eq!(pods, [("db-0", false), ("db-1", true), ("db-2", true), ("db-10", false)]);
    assert!(!d.metrics_available);
    assert_eq!(d.cpu.requests, Some(1000.0));
    assert_eq!(d.cpu.limits, Some(2000.0));
    assert_eq!(d.cpu.used, None);
}

fn sts_with(spec: Value, status: Value) -> Workload {
    let mut v = db_statefulset();
    v["spec"]["updateStrategy"] = json!(null);
    v["spec"]["persistentVolumeClaimRetentionPolicy"] = json!(null);
    v["spec"]["volumeClaimTemplates"] = json!(null);
    v["status"] = json!({"observedGeneration": 1, "replicas": 4, "readyReplicas": 4, "updatedReplicas": 4, "availableReplicas": 4});
    merge(&mut v, json!({"spec": spec, "status": status}));
    statefulset(serde_json::from_value(v).unwrap())
}

#[test]
fn statefulset_states() {
    let w = sts_with(json!({}), json!({}));
    assert_eq!(w.status, "Ready");
    assert_eq!(info(&w.info, "Strategy"), Some("RollingUpdate"));
    assert_eq!(info(&w.info, "Revision"), None);
    assert_eq!(info(&w.info, "Volume claims"), None);
    assert_eq!(info(&w.info, "Claim retention"), None);
    // Without an update revision no pod counts as updated.
    let p: Pod = serde_json::from_value(pod("shop", "db-0", "n1", json!({}))).unwrap();
    assert!(!(w.is_updated)(&p));

    assert_eq!(sts_with(json!({"replicas": 0}), json!({"replicas": 0})).status, "Scaled to 0");
    assert_eq!(sts_with(json!({}), json!({"readyReplicas": 3})).status, "Degraded");
    assert_eq!(sts_with(json!({}), json!({"updatedReplicas": 3})).status, "Rolling out");
    assert_eq!(sts_with(json!({}), json!({"observedGeneration": 0})).status, "Rolling out");

    let w = sts_with(json!({"updateStrategy": {"type": "OnDelete"}, "serviceName": ""}), json!({"currentRevision": "db-1"}));
    assert_eq!(info(&w.info, "Strategy"), Some("OnDelete (pods update only when deleted)"));
    assert_eq!(info(&w.info, "Service"), None);
    assert_eq!(info(&w.info, "Revision"), Some("db-1"));
    let w = sts_with(json!({"updateStrategy": {"rollingUpdate": {"partition": 0}}}), json!({"updateRevision": "db-2", "currentRevision": "db-2"}));
    assert_eq!(w.status, "Ready");
    assert_eq!(info(&w.info, "Strategy"), Some("RollingUpdate"));
    assert_eq!(info(&w.info, "Revision"), Some("db-2"));
    let w = sts_with(json!({"persistentVolumeClaimRetentionPolicy": {"whenScaled": "Delete"}}), json!({"updateRevision": "db-3"}));
    assert_eq!(w.status, "Rolling out");
    assert_eq!(info(&w.info, "Revision"), Some("db-3"));
    assert_eq!(info(&w.info, "Claim retention"), Some("when deleted: Retain · when scaled: Delete"));
}

#[test]
fn deployment_states_and_strategies() {
    let d = |spec: Value, status: Value, extra: Value| -> Workload {
        let mut v = deployment();
        v["status"] = json!({"observedGeneration": 4, "replicas": 3, "readyReplicas": 3, "updatedReplicas": 3, "availableReplicas": 3});
        v["spec"]["strategy"] = json!(null);
        merge(&mut v, json!({"spec": spec, "status": status}));
        merge(&mut v, extra);
        deployment_workload(serde_json::from_value(v).unwrap(), vec![])
    };
    let w = d(json!({}), json!({}), json!({}));
    assert_eq!(w.status, "Available");
    assert_eq!(info(&w.info, "Strategy"), Some("RollingUpdate · max surge 25% · max unavailable 25%"));
    // No ReplicaSets: no pod is known to be on the newest template.
    let p: Pod = serde_json::from_value(pod("shop", "api-x-aaaaa", "n1", json!({"metadata": {"labels": {"pod-template-hash": "new"}}}))).unwrap();
    assert!(!(w.is_updated)(&p));

    let failed = json!({"conditions": [{"type": "Progressing", "status": "False", "reason": "ProgressDeadlineExceeded"}]});
    assert_eq!(d(json!({}), failed, json!({})).status, "Failed");
    assert_eq!(d(json!({}), json!({"observedGeneration": 3}), json!({})).status, "Rolling out");
    assert_eq!(d(json!({}), json!({"replicas": 4}), json!({})).status, "Rolling out");
    assert_eq!(d(json!({}), json!({"availableReplicas": 2}), json!({})).status, "Degraded");

    // Without a revision annotation there's no revision row.
    let w = d(json!({}), json!({}), json!({"metadata": {"annotations": null}}));
    assert_eq!(info(&w.info, "Revision"), None);

    // A bare Deployment: no spec or status at all.
    let bare: Deployment = serde_json::from_value(json!({"metadata": {"name": "api"}})).unwrap();
    let w = deployment_workload(bare, vec![]);
    assert_eq!(w.status, "Rolling out");
    assert_eq!(w.replicas.unwrap().desired, 1);
    assert_eq!(w.pod_selector, "");
}

#[test]
fn job_states() {
    let job = |spec: Value, status: Value| -> Job {
        serde_json::from_value(json!({"metadata": {"name": "migrate"}, "spec": with(json!({"template": {}}), spec), "status": status})).unwrap()
    };
    let cond = |t: &str, s: &str| json!({"conditions": [{"type": t, "status": s}]});
    assert_eq!(job_state(&job(json!({}), cond("SuccessCriteriaMet", "True"))), "Complete");
    assert_eq!(job_state(&job(json!({}), cond("FailureTarget", "True"))), "Failed");
    assert_eq!(job_state(&job(json!({}), cond("Complete", "False"))), "Pending");
    assert_eq!(job_state(&job(json!({"suspend": true}), json!({"active": 1}))), "Suspended");
    assert_eq!(job_state(&job(json!({}), json!({"active": 1}))), "Running");
    let no_status: Job = serde_json::from_value(json!({"metadata": {"name": "migrate"}})).unwrap();
    assert_eq!(job_state(&no_status), "Pending");
    assert_eq!(job_finished(&no_status), None);
    assert_eq!(job_completions(&no_status), "0/1+");
    // A Failed condition without a time.
    assert_eq!(job_finished(&job(json!({}), cond("Failed", "True"))), None);
    assert_eq!(job_completions(&job(json!({"completions": 5}), json!({"succeeded": 2}))), "2/5");

    let now: jiff::Timestamp = "2026-10-10T12:00:00Z".parse().unwrap();
    let w = job_workload(job(json!({"completionMode": "Indexed", "completions": 3, "parallelism": 1}), json!({"active": 1, "completedIndexes": ""})), now);
    assert_eq!(info(&w.info, "Completion mode"), Some("Indexed · done: none"));
    assert_eq!(info(&w.info, "Failed indexes"), None);
    assert_eq!(info(&w.info, "Parallelism"), None);
    assert_eq!(info(&w.info, "Active"), Some("1 pod · 0 ready"));
    assert_eq!(info(&w.info, "Failed pods"), Some("0 · retries allowed: 6"));
    assert_eq!(info(&w.info, "Started"), None);
    assert_eq!(info(&w.info, "Duration"), None);
    assert_eq!(w.pod_selector, "");
    assert!((w.is_updated)(&Pod::default()));
}

#[test]
fn durations() {
    let t = |s: &str| -> jiff::Timestamp { s.parse().unwrap() };
    let from = t("2026-10-10T00:00:00Z");
    assert_eq!(duration(from, t("2026-10-10T00:00:45Z")), "45s");
    assert_eq!(duration(from, t("2026-10-10T02:05:30Z")), "2h 5m");
    assert_eq!(duration(from, t("2026-10-11T03:00:00Z")), "1d 3h");
    // A clock that's behind never gives a negative duration.
    assert_eq!(duration(from, t("2026-10-09T23:00:00Z")), "0s");
}

#[test]
fn selector_operators_and_helpers() {
    let selector: LabelSelector = serde_json::from_value(json!({"matchExpressions": [
        {"key": "tier", "operator": "NotIn", "values": ["db"]},
        {"key": "app", "operator": "Exists"},
        {"key": "x", "operator": "Gt", "values": ["1"]}
    ]}))
    .unwrap();
    assert_eq!(selector_string(&selector), "tier notin (db),app,x Gt (1)");
    assert_eq!(selector_string(&LabelSelector::default()), "");
    assert_eq!(int_or_string(None, "1"), "1");
    assert!(conditions::<Value>(None).is_empty());
    let mut rows = vec![];
    push(&mut rows, "Empty", Some(String::new()));
    push(&mut rows, "None", None);
    assert!(rows.is_empty());
    assert_eq!(summed([(1.0, 2.0), (3.0, 4.0)].into_iter()), (4.0, 6.0));
    assert_eq!(positive(-1.0), None);
}

fn job_object(name: &str, created: &str, spec: Value, status: Value) -> Value {
    object("batch/v1", "Job", Some("shop"), name, json!({
        "metadata": {"creationTimestamp": created},
        "spec": with(json!({"selector": {"matchLabels": {"batch.kubernetes.io/controller-uid": format!("uid-{name}")}},
                            "template": {"spec": {"restartPolicy": "Never", "containers": [app_container()]}}}), spec),
        "status": status
    }))
}

#[tokio::test]
async fn indexed_job_details() {
    let api = MockApi::new();
    api.on("GET", "/apis/batch/v1/namespaces/shop/jobs/migrate", job_object("migrate", "2026-10-10T11:00:00Z",
        json!({"completionMode": "Indexed", "completions": 3, "parallelism": 2, "backoffLimitPerIndex": 1,
               "activeDeadlineSeconds": 600, "ttlSecondsAfterFinished": 100}),
        json!({"active": 1, "ready": 1, "failed": 1, "succeeded": 1, "completedIndexes": "0", "failedIndexes": "2",
               "startTime": "2026-10-10T11:00:00Z"})));
    let job_pod = |n: &str, created: &str, phase: &str| {
        owned_pod(n, "Job", "migrate", json!({"batch.kubernetes.io/controller-uid": "uid-migrate"}),
                  json!({"metadata": {"creationTimestamp": created}, "status": {"phase": phase}}))
    };
    api.on("GET", PODS, list("PodList", vec![
        job_pod("migrate-0-aaaaa", "2026-10-10T11:00:00Z", "Succeeded"),
        job_pod("migrate-1-bbbbb", "2026-10-10T11:05:00Z", "Running"),
    ]));

    let d = workload_details(&api.client(), &job_kind(), "shop", "migrate").await.unwrap();
    assert_eq!(d.kind, "Job");
    assert_eq!(d.status, "Running");
    assert_eq!(d.suspended, Some(false));
    assert_eq!(d.replicas, None);
    assert_eq!(info(&d.info, "Completions"), Some("1/3 succeeded"));
    assert_eq!(info(&d.info, "Active"), Some("1 pod · 1 ready"));
    assert_eq!(info(&d.info, "Failed pods"), Some("1 · retries allowed per index: 1"));
    assert_eq!(info(&d.info, "Parallelism"), Some("2"));
    assert_eq!(info(&d.info, "Completion mode"), Some("Indexed · done: 0"));
    assert_eq!(info(&d.info, "Failed indexes"), Some("2"));
    assert_eq!(info(&d.info, "Started"), Some("2026-10-10T11:00:00Z"));
    assert_eq!(info(&d.info, "Finished"), None);
    assert!(info(&d.info, "Duration").unwrap().starts_with("running for "));
    assert_eq!(info(&d.info, "Active deadline"), Some("600s"));
    assert_eq!(info(&d.info, "Clean up"), Some("deleted 100s after it finishes"));
    assert_eq!(info(&d.info, "Restart policy"), Some("Never"));
    assert_eq!(info(&d.info, "Selector"), None);
    // Newest first for batch workloads.
    let pods: Vec<_> = d.pods.iter().map(|p| (p.name.as_str(), p.finished, p.updated)).collect();
    assert_eq!(pods, [("migrate-1-bbbbb", false, true), ("migrate-0-aaaaa", true, true)]);
    // Totals over the running pod only.
    assert_eq!(d.cpu.requests, Some(250.0));
    assert_eq!(d.cpu.limits, Some(500.0));
    let call = &api.calls_to("GET", PODS)[0];
    assert!(call.query.contains("controller-uid%3Duid-migrate"), "{}", call.query);
}

#[tokio::test]
async fn finished_job_shows_one_pods_resources() {
    let api = MockApi::new();
    api.on("GET", "/apis/batch/v1/namespaces/shop/jobs/migrate", job_object("migrate", "2026-10-10T11:00:00Z",
        json!({"completions": 1}),
        json!({"succeeded": 1, "startTime": "2026-10-10T11:00:00Z", "completionTime": "2026-10-10T11:00:30Z",
               "conditions": [{"type": "Complete", "status": "True"}]})));
    api.on("GET", PODS, list("PodList", vec![owned_pod("migrate-aaaaa", "Job", "migrate",
        json!({"batch.kubernetes.io/controller-uid": "uid-migrate"}), json!({"status": {"phase": "Succeeded"}}))]));

    let d = workload_details(&api.client(), &job_kind(), "shop", "migrate").await.unwrap();
    assert_eq!(d.status, "Complete");
    assert_eq!(info(&d.info, "Duration"), Some("took 30s"));
    assert_eq!(info(&d.info, "Finished"), Some("2026-10-10T11:00:30Z"));
    assert_eq!(info(&d.info, "Active"), None);
    assert_eq!(d.pods.len(), 1);
    // Nothing running: what one pod would get, from the template.
    assert_eq!(d.cpu.requests, Some(250.0));
    assert_eq!(d.cpu.limits, Some(500.0));
    assert_eq!(d.memory.requests, Some(128.0 * 1024.0 * 1024.0));
}

#[tokio::test]
async fn suspended_job() {
    let api = MockApi::new();
    api.on("GET", "/apis/batch/v1/namespaces/shop/jobs/migrate",
        job_object("migrate", "2026-10-10T11:00:00Z", json!({"suspend": true}), json!({})));
    api.on("GET", PODS, list("PodList", vec![]));
    let d = workload_details(&api.client(), &job_kind(), "shop", "migrate").await.unwrap();
    assert_eq!(d.status, "Suspended");
    assert_eq!(d.suspended, Some(true));
    assert_eq!(info(&d.info, "Completions"), Some("0/1+ succeeded"));
}

fn cron_job(spec: Value) -> Value {
    object("batch/v1", "CronJob", Some("shop"), "nightly", json!({
        "spec": with(json!({"schedule": "0 3 * * *",
                            "jobTemplate": {"spec": {"template": {"spec": {"restartPolicy": "OnFailure", "containers": [app_container()]}}}}}), spec),
        "status": {"lastScheduleTime": "2026-10-10T03:00:00Z", "lastSuccessfulTime": "2026-10-10T03:01:00Z"}
    }))
}

fn cron_owned(job: Value, owner_name: &str, manual: bool) -> Value {
    let annotations = if manual { json!({"cronjob.kubernetes.io/instantiate": "manual"}) } else { json!({}) };
    with(job, json!({"metadata": {"annotations": annotations,
        "ownerReferences": [{"apiVersion": "batch/v1", "kind": "CronJob", "name": owner_name, "uid": "u", "controller": true}]}}))
}

#[tokio::test]
async fn cron_job_details() {
    let api = MockApi::new();
    api.on("GET", "/apis/batch/v1/namespaces/shop/cronjobs/nightly", cron_job(json!({
        "timeZone": "Europe/London", "concurrencyPolicy": "Forbid", "startingDeadlineSeconds": 200,
        "successfulJobsHistoryLimit": 5, "failedJobsHistoryLimit": 2
    })));
    api.on("GET", "/apis/batch/v1/namespaces/shop/jobs", list("JobList", vec![
        cron_owned(job_object("nightly-29330000", "2026-10-10T03:00:00Z", json!({"completions": 1}),
            json!({"succeeded": 1, "startTime": "2026-10-10T03:00:00Z", "completionTime": "2026-10-10T03:01:00Z",
                   "conditions": [{"type": "Complete", "status": "True"}]})), "nightly", false),
        cron_owned(job_object("nightly-manual-abc12", "2026-10-10T09:00:00Z", json!({}),
            json!({"active": 1, "startTime": "2026-10-10T09:00:00Z"})), "nightly", true),
        cron_owned(job_object("hourly-29330001", "2026-10-10T10:00:00Z", json!({}), json!({})), "hourly", false),
    ]));
    let job_pod = |n: &str, job: &str, phase: &str| owned_pod(n, "Job", job, json!({"job-name": job}), json!({"status": {"phase": phase}}));
    api.on("GET", PODS, list("PodList", vec![
        job_pod("nightly-29330000-aaaaa", "nightly-29330000", "Succeeded"),
        job_pod("nightly-manual-abc12-bbbbb", "nightly-manual-abc12", "Running"),
        job_pod("hourly-29330001-ccccc", "hourly-29330001", "Running"),
    ]));
    api.on("GET", EVENTS, list("EventList", vec![
        event("SawCompletedJob", "CronJob", "nightly", "2026-10-10T03:01:00Z"),
        event("Completed", "Job", "nightly-29330000", "2026-10-10T03:01:00Z"),
        event("Started", "Pod", "nightly-manual-abc12-bbbbb", "2026-10-10T09:00:05Z"),
        event("Unrelated", "Job", "hourly-29330001", "2026-10-10T10:00:00Z"),
    ]));

    let d = workload_details(&api.client(), &cron_job_kind(), "shop", "nightly").await.unwrap();
    assert_eq!(d.kind, "CronJob");
    assert_eq!(d.status, "Running");
    assert_eq!(d.suspended, Some(false));
    let jobs: Vec<_> = d.jobs.iter().map(|j| (j.name.as_str(), j.status.as_str(), j.manual, j.completions.as_str())).collect();
    assert_eq!(jobs, [("nightly-manual-abc12", "Running", true, "0/1+"), ("nightly-29330000", "Complete", false, "1/1")]);
    assert_eq!(d.jobs[1].started.as_deref(), Some("2026-10-10T03:00:00Z"));
    assert_eq!(d.jobs[1].finished.as_deref(), Some("2026-10-10T03:01:00Z"));
    assert_eq!(d.jobs[0].created.as_deref(), Some("2026-10-10T09:00:00Z"));
    assert_eq!(info(&d.info, "Schedule"), Some("0 3 * * * (Europe/London)"));
    let next = info(&d.info, "Next run").unwrap();
    assert!(next.parse::<jiff::Timestamp>().is_ok(), "{next}");
    assert_eq!(info(&d.info, "Last scheduled"), Some("2026-10-10T03:00:00Z"));
    assert_eq!(info(&d.info, "Last successful"), Some("2026-10-10T03:01:00Z"));
    assert_eq!(info(&d.info, "Concurrency"), Some("Forbid"));
    assert_eq!(info(&d.info, "Starting deadline"), Some("200s"));
    assert_eq!(info(&d.info, "History kept"), Some("5 successful · 2 failed"));
    assert_eq!(info(&d.info, "Restart policy"), Some("OnFailure"));
    assert_eq!(info(&d.info, "Selector"), None);

    let pods: Vec<_> = d.pods.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(pods, ["nightly-29330000-aaaaa", "nightly-manual-abc12-bbbbb"]);
    let events: Vec<_> = d.events.iter().map(|e| (e.reason.as_str(), e.object.as_deref())).collect();
    assert_eq!(events, [
        ("Started", Some("Pod nightly-manual-abc12-bbbbb")),
        ("SawCompletedJob", None),
        ("Completed", Some("Job nightly-29330000")),
    ]);
    let call = &api.calls_to("GET", PODS)[0];
    assert!(call.query.contains("job-name"), "{}", call.query);
}

#[tokio::test]
async fn cron_job_without_jobs() {
    let api = MockApi::new();
    api.on("GET", "/apis/batch/v1/namespaces/shop/cronjobs/nightly", cron_job(json!({"suspend": true})));
    api.on("GET", "/apis/batch/v1/namespaces/shop/jobs", list("JobList", vec![]));
    // metrics-server would answer for every pod in the namespace.
    api.on("GET", METRICS, list("PodMetricsList", vec![
        metrics("web-1", json!([{"name": "app", "usage": {"cpu": "1", "memory": "1Gi"}}])),
    ]));

    let d = workload_details(&api.client(), &cron_job_kind(), "shop", "nightly").await.unwrap();
    assert_eq!(d.status, "Suspended");
    assert_eq!(d.suspended, Some(true));
    assert!(info(&d.info, "Next run").is_none());
    assert_eq!(info(&d.info, "Concurrency"), Some("Allow"));
    assert_eq!(info(&d.info, "History kept"), Some("3 successful · 1 failed"));
    assert!(d.jobs.is_empty() && d.pods.is_empty());
    // An empty selector would list every pod in the namespace.
    assert!(api.calls_to("GET", PODS).is_empty());
    // Another workload's usage doesn't count towards this one.
    assert_eq!(d.cpu.used, Some(0.0));
    assert_eq!(d.containers[0].cpu.used, Some(0.0));
    // What one run's pod would get.
    assert_eq!(d.cpu.requests, Some(250.0));
}

#[tokio::test]
async fn cron_job_jobs_list_error() {
    let api = MockApi::new();
    api.on("GET", "/apis/batch/v1/namespaces/shop/cronjobs/nightly", cron_job(json!({})));
    api.on_error("GET", "/apis/batch/v1/namespaces/shop/jobs", 403, "Forbidden", "jobs is forbidden");
    let err = workload_details(&api.client(), &cron_job_kind(), "shop", "nightly").await.unwrap_err();
    assert_eq!(err.user_message(), "jobs is forbidden");
}

#[test]
fn cron_job_states() {
    let now: jiff::Timestamp = "2026-10-10T12:00:00Z".parse().unwrap();
    let cj = |spec: Value, status: Value| -> CronJob {
        let mut v = cron_job(spec);
        v["status"] = status;
        serde_json::from_value(v).unwrap()
    };
    let w = cron_job_workload(cj(json!({}), json!({})), vec![], now);
    assert_eq!(w.status, "Scheduled");
    assert_eq!(info(&w.info, "Schedule"), Some("0 3 * * * (controller's time zone, usually UTC)"));
    assert_eq!(info(&w.info, "Next run"), Some("2026-10-11T03:00:00Z"));
    assert_eq!(info(&w.info, "Last scheduled"), None);
    // Active Jobs the CronJob reports, even if they weren't listed.
    let w = cron_job_workload(cj(json!({}), json!({"active": [{"kind": "Job", "name": "nightly-1"}]})), vec![], now);
    assert_eq!(w.status, "Running");
    let w = cron_job_workload(cj(json!({}), json!({"active": []})), vec![], now);
    assert_eq!(w.status, "Scheduled");
    let w = cron_job_workload(cj(json!({"schedule": "not a schedule"}), json!({})), vec![], now);
    assert_eq!(info(&w.info, "Next run"), Some("can't work out"));
}

#[test]
fn common_rows_without_template_spec() {
    let w = with_common_rows(Workload {
        kind: "Deployment".into(),
        meta: ObjectMeta::default(),
        pod_selector: "app=api".into(),
        template: PodTemplateSpec::default(),
        status: String::new(),
        replicas: None,
        suspended: None,
        info: vec![],
        conditions: vec![],
        jobs: vec![],
        is_updated: Box::new(|_| false),
        owners: vec![],
    });
    assert_eq!(w.info, [("Selector".to_owned(), "app=api".to_owned())]);
}

#[test]
fn workload_pod_without_spec_or_status() {
    let w = with_common_rows(Workload {
        kind: "Job".into(),
        meta: ObjectMeta::default(),
        pod_selector: String::new(),
        template: PodTemplateSpec::default(),
        status: String::new(),
        replicas: None,
        suspended: None,
        info: vec![],
        conditions: vec![],
        jobs: vec![],
        is_updated: Box::new(|_| true),
        owners: vec![],
    });
    let p: Pod = serde_json::from_value(json!({"metadata": {"name": "x"}})).unwrap();
    let wp = workload_pod(&p, &w, None);
    assert_eq!(wp.ready, "0/0");
    assert_eq!(wp.restarts, 0);
    assert_eq!(wp.node, None);
    assert_eq!(wp.created, None);
    assert!(!wp.finished);
    assert_eq!(wp.cpu, None);
    assert!(controlled_by(&[serde_json::from_value(owner("Job", "x", true)).unwrap()], "Job", "x"));
    assert!(!controlled_by(&[serde_json::from_value(owner("Job", "x", false)).unwrap()], "Job", "x"));
}
