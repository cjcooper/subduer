//! Coverage tests for the Config section's details.

use super::*;
use crate::testing::{MockApi, kind, list, object, pod};
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

fn cells(s: &InfoSection) -> Vec<Vec<String>> {
    s.table.as_ref().unwrap().rows.iter().map(|r| r.cells.clone()).collect()
}

fn table_row<'a>(s: &'a InfoSection, first: &str) -> &'a InfoTableRow {
    s.table.as_ref().unwrap().rows.iter().find(|r| r.cells[0] == first).unwrap_or_else(|| panic!("no row {first}"))
}

fn strs(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

const ADMISSION: &str = "/apis/admissionregistration.k8s.io/v1";

// ---- object_details ----

#[tokio::test]
async fn kinds_without_details_and_missing_objects() {
    let api = MockApi::new();
    let deployments = kind("apps", "v1", "Deployment", "deployments", true);
    let err = object_details(&api.client(), &deployments, Some("shop"), "api").await.unwrap_err();
    assert!(matches!(err, Error::Message(ref m) if m == "no details view for Deployment"), "{err}");
    assert!(api.calls().is_empty());

    let cms = kind("", "v1", "ConfigMap", "configmaps", true);
    let err = object_details(&api.client(), &cms, Some("shop"), "gone").await.unwrap_err();
    assert!(matches!(err, Error::Kube(kube::Error::Api(ref s)) if s.code == 404), "{err}");
}

#[tokio::test]
async fn helm_releases_are_handed_to_helm() {
    let api = MockApi::new();
    api.on("GET", "/api/v1/namespaces/shop/secrets", list("SecretList", vec![]));
    let release = kind("helm.sh", "v1", "Release", "releases", true);
    let err = object_details(&api.client(), &release, Some("shop"), "shop").await.unwrap_err();
    assert!(matches!(err, Error::NotFound(ref m) if m.contains("shop/shop")), "{err}");
}

#[tokio::test]
async fn config_map_details_with_pods_and_events() {
    let api = MockApi::new();
    api.on("GET", "/api/v1/namespaces/shop/configmaps/app", object("v1", "ConfigMap", Some("shop"), "app", json!({
        "data": {"a": "1"}
    })));
    let web = json!({
        "metadata": {"ownerReferences": [
            {"apiVersion": "apps/v1", "kind": "ReplicaSet", "name": "web-abc", "uid": "u0", "controller": false},
            {"apiVersion": "apps/v1", "kind": "ReplicaSet", "name": "web-7d9f", "uid": "u1", "controller": true}]},
        "spec": {
            "initContainers": [{"name": "init", "image": "acme/init:1", "envFrom": [{"configMapRef": {"name": "app"}}, {"secretRef": {"name": "app"}}]}],
            "volumes": [{"name": "cfg", "configMap": {"name": "app"}}, {"name": "empty", "emptyDir": {}}]
        }
    });
    let api_pod = json!({"spec": {"containers": [{"name": "app", "image": "acme/app:1",
        "env": [{"name": "PLAIN", "value": "1"}, {"name": "A", "valueFrom": {"configMapKeyRef": {"name": "app", "key": "a"}}}]}]}});
    api.on("GET", "/api/v1/namespaces/shop/pods", list("PodList", vec![
        pod("shop", "web-1", "node-1", web),
        pod("shop", "api-1", "node-1", api_pod),
        pod("shop", "other", "node-1", json!({})),
        json!({"apiVersion": "v1", "kind": "Pod", "metadata": {"name": "nospec", "namespace": "shop"}}),
    ]));
    api.on("GET", "/api/v1/namespaces/shop/events", list("EventList", vec![object("v1", "Event", Some("shop"), "e1", json!({
        "type": "Normal", "reason": "Updated", "message": "changed", "count": 1, "lastTimestamp": "2026-10-10T11:00:00Z",
        "involvedObject": {"kind": "ConfigMap", "name": "app", "namespace": "shop"}
    }))]));
    let cms = kind("", "v1", "ConfigMap", "configmaps", true);
    let d = object_details(&api.client(), &cms, Some("shop"), "app").await.unwrap();
    assert_eq!(d.kind, "ConfigMap");
    assert_eq!(d.namespace.as_deref(), Some("shop"));
    let used = section(&d, "used-by");
    assert_eq!(cells(used), [strs(&["api-1", "—", "env"]), strs(&["web-1", "ReplicaSet/web-7d9f", "envFrom, volume"])]);
    assert_eq!(used.summary, "2 pods");
    assert_eq!(used.table.as_ref().unwrap().rows[0].link.as_ref().unwrap().kind, "core/Pod");
    assert_eq!(d.events.len(), 1);
    let events = api.calls_to("GET", "/api/v1/namespaces/shop/events");
    assert!(events[0].query.contains("involvedObject.kind%3DConfigMap%2CinvolvedObject.name%3Dapp"), "{}", events[0].query);
}

#[tokio::test]
async fn secret_details_and_nothing_using_it() {
    let api = MockApi::new();
    api.on("GET", "/api/v1/namespaces/shop/secrets/db", object("v1", "Secret", Some("shop"), "db", json!({
        "type": "Opaque", "data": {"password": STANDARD.encode("hunter2")}
    })));
    let user = json!({"spec": {
        "containers": [{"name": "app", "image": "acme/app:1", "envFrom": [{"configMapRef": {"name": "db"}}]}],
        "volumes": [{"name": "creds", "projected": {"sources": [{"secret": {"name": "db"}}, {"configMap": {"name": "db"}}]}}]
    }});
    api.on("GET", "/api/v1/namespaces/shop/pods", list("PodList", vec![pod("shop", "api-1", "node-1", user)]));
    let secrets = kind("", "v1", "Secret", "secrets", true);
    let d = object_details(&api.client(), &secrets, Some("shop"), "db").await.unwrap();
    assert_eq!(cells(section(&d, "used-by")), [strs(&["api-1", "—", "volume"])]);
    assert!(d.events.is_empty());

    let api = MockApi::new();
    api.on("GET", "/api/v1/namespaces/shop/secrets/db", object("v1", "Secret", Some("shop"), "db", json!({})));
    api.on("GET", "/api/v1/namespaces/shop/pods", list("PodList", vec![]));
    let d = object_details(&api.client(), &secrets, Some("shop"), "db").await.unwrap();
    let used = section(&d, "used-by");
    assert_eq!(used.summary, "none");
    assert_eq!(used.empty.as_deref(), Some("No pods in this namespace use it."));

    // Pods can't be listed.
    let api = MockApi::new();
    api.on("GET", "/api/v1/namespaces/shop/secrets/db", object("v1", "Secret", Some("shop"), "db", json!({})));
    api.on_error("GET", "/api/v1/namespaces/shop/pods", 403, "Forbidden", "pods is forbidden");
    let err = object_details(&api.client(), &secrets, Some("shop"), "db").await.unwrap_err();
    assert_eq!(err.user_message(), "pods is forbidden");
}

#[tokio::test]
async fn pdb_pods_by_selector() {
    let api = MockApi::new();
    let pdbs = kind("policy", "v1", "PodDisruptionBudget", "poddisruptionbudgets", true);
    api.on("GET", "/apis/policy/v1/namespaces/shop/poddisruptionbudgets/api", object("policy/v1", "PodDisruptionBudget", Some("shop"), "api", json!({
        "spec": {"minAvailable": 1, "selector": {"matchLabels": {"app": "api"}}},
        "status": {"currentHealthy": 2, "desiredHealthy": 1, "expectedPods": 2, "disruptionsAllowed": 1}
    })));
    api.on("GET", "/api/v1/namespaces/shop/pods", list("PodList", vec![]));
    api.on_query("GET", "/api/v1/namespaces/shop/pods", "labelSelector=app%3Dapi", list("PodList", vec![
        pod("shop", "api-1", "node-1", json!({})),
        pod("shop", "api-2", "node-2", json!({})),
    ]));
    let d = object_details(&api.client(), &pdbs, Some("shop"), "api").await.unwrap();
    assert_eq!(d.status.as_deref(), Some("Allowing"));
    let pods = section(&d, "pdb-pods");
    assert_eq!(pods.summary, "2");
    assert_eq!(cells(pods), [strs(&["api-1", "Running", "node-1"]), strs(&["api-2", "Running", "node-2"])]);

    // Without a selector: every pod in the namespace (none here).
    let api = MockApi::new();
    api.on("GET", "/apis/policy/v1/namespaces/shop/poddisruptionbudgets/all", object("policy/v1", "PodDisruptionBudget", Some("shop"), "all", json!({
        "spec": {"maxUnavailable": 1}
    })));
    api.on("GET", "/api/v1/namespaces/shop/pods", list("PodList", vec![]));
    let d = object_details(&api.client(), &pdbs, Some("shop"), "all").await.unwrap();
    let pods = section(&d, "pdb-pods");
    assert_eq!(pods.summary, "none");
    assert!(!api.calls_to("GET", "/api/v1/namespaces/shop/pods")[0].query.contains("labelSelector"));
}

#[tokio::test]
async fn admission_policy_bindings() {
    let api = MockApi::new();
    let policies = kind("admissionregistration.k8s.io", "v1", "ValidatingAdmissionPolicy", "validatingadmissionpolicies", false);
    api.on("GET", &format!("{ADMISSION}/validatingadmissionpolicies/replicas"), object("admissionregistration.k8s.io/v1", "ValidatingAdmissionPolicy", None, "replicas", json!({"spec": {}})));
    api.on("GET", &format!("{ADMISSION}/validatingadmissionpolicybindings"), list("ValidatingAdmissionPolicyBindingList", vec![
        object("admissionregistration.k8s.io/v1", "ValidatingAdmissionPolicyBinding", None, "replicas-prd", json!({"spec": {"policyName": "replicas", "validationActions": ["Deny", "Audit"]}})),
        object("admissionregistration.k8s.io/v1", "ValidatingAdmissionPolicyBinding", None, "other", json!({"spec": {"policyName": "other"}})),
    ]));
    let d = object_details(&api.client(), &policies, None, "replicas").await.unwrap();
    let b = section(&d, "bindings");
    assert_eq!(cells(b), [strs(&["replicas-prd", "Deny, Audit"])]);
    let link = b.table.as_ref().unwrap().rows[0].link.as_ref().unwrap();
    assert_eq!(link.kind, "admissionregistration.k8s.io/ValidatingAdmissionPolicyBinding");
    // Cluster-scoped: no events are read.
    assert!(api.calls_to("GET", "/api/v1/events").is_empty());

    let api = MockApi::new();
    api.on("GET", &format!("{ADMISSION}/validatingadmissionpolicies/lonely"), object("admissionregistration.k8s.io/v1", "ValidatingAdmissionPolicy", None, "lonely", json!({"spec": {}})));
    api.on("GET", &format!("{ADMISSION}/validatingadmissionpolicybindings"), list("ValidatingAdmissionPolicyBindingList", vec![]));
    let d = object_details(&api.client(), &policies, None, "lonely").await.unwrap();
    let b = section(&d, "bindings");
    assert_eq!(b.summary, "none · not enforced");
    assert_eq!(b.empty.as_deref(), Some("Not bound: it isn't enforced anywhere."));
}

#[tokio::test]
async fn other_kinds_go_to_their_modules() {
    // A ServiceAccount: Access Control adds its sections.
    let api = MockApi::new();
    api.on("GET", "/api/v1/namespaces/shop/serviceaccounts/api", object("v1", "ServiceAccount", Some("shop"), "api", json!({})));
    api.on("GET", "/apis/rbac.authorization.k8s.io/v1/rolebindings", list("RoleBindingList", vec![]));
    api.on("GET", "/apis/rbac.authorization.k8s.io/v1/clusterrolebindings", list("ClusterRoleBindingList", vec![]));
    api.on("GET", "/api/v1/namespaces/shop/pods", list("PodList", vec![]));
    let accounts = kind("", "v1", "ServiceAccount", "serviceaccounts", true);
    let d = object_details(&api.client(), &accounts, Some("shop"), "api").await.unwrap();
    assert_eq!(row(&d, "Acts as"), Some("system:serviceaccount:shop:api"));
    assert!(d.sections.iter().any(|s| s.id == "sa-bindings"));
    assert_eq!(api.calls_to("GET", "/api/v1/namespaces/shop/events").len(), 1);

    // A Namespace is cluster-scoped: its own details, no event lookup by object.
    let api = MockApi::new();
    api.on("GET", "/api/v1/namespaces/shop", object("v1", "Namespace", None, "shop", json!({"status": {"phase": "Active"}})));
    let namespaces = kind("", "v1", "Namespace", "namespaces", false);
    let d = object_details(&api.client(), &namespaces, None, "shop").await.unwrap();
    assert_eq!(d.status.as_deref(), Some("Active"));
    assert!(d.sections.iter().any(|s| s.id == "ns-contents"));
    let event_calls = api.calls_to("GET", "/api/v1/namespaces/shop/events");
    assert!(event_calls.iter().all(|c| !c.query.contains("involvedObject")));

    // Errors from those modules propagate.
    let api = MockApi::new();
    api.on("GET", "/api/v1/namespaces/shop/serviceaccounts/api", object("v1", "ServiceAccount", Some("shop"), "api", json!({})));
    assert!(object_details(&api.client(), &accounts, Some("shop"), "api").await.is_err());
}

// ---- describe ----

#[test]
fn owners_annotations_and_unknown_kinds() {
    let v = json!({"kind": "Widget", "metadata": {"name": "w", "creationTimestamp": "2026-10-01T00:00:00Z",
        "labels": {"app": "w", "n": 1},
        "ownerReferences": [{"kind": "Thing", "name": "a", "controller": false}, {"kind": "Gadget", "name": "g", "controller": true}]}});
    let d = describe("acme.io/Widget", &v, now());
    assert_eq!(row(&d, "Controlled by"), Some("Gadget/g"));
    assert_eq!(d.created.as_deref(), Some("2026-10-01T00:00:00Z"));
    assert!(d.labels.contains(&("n".to_owned(), String::new())));
    assert!(d.status.is_none() && d.data.is_none() && d.sections.is_empty());

    let bare = describe("acme.io/Widget", &json!({}), now());
    assert_eq!((bare.kind.as_str(), bare.name.as_str(), bare.namespace.clone()), ("", "", None));
    assert!(bare.info.is_empty());
}

#[test]
fn config_map_warnings() {
    let root = json!({"kind": "ConfigMap", "metadata": {"name": "kube-root-ca.crt"}, "data": {"ca.crt": "x"}});
    assert_eq!(describe("core/ConfigMap", &root, now()).data.unwrap().warning.as_deref(), Some("Kept in sync by Kubernetes: edits are overwritten."));

    let owned = json!({"kind": "ConfigMap", "metadata": {"name": "c",
        "ownerReferences": [{"kind": "Thing", "name": "t", "controller": false}, {"kind": "Operator", "name": "op", "controller": true}]}});
    let d = describe("core/ConfigMap", &owned, now());
    assert_eq!(d.data.as_ref().unwrap().warning.as_deref(), Some("Controlled by Operator op, which may overwrite edits."));
    assert_eq!(row(&d, "Keys"), Some("0"));
    assert_eq!(row(&d, "Size"), Some("0 B"));

    let free = json!({"kind": "ConfigMap", "metadata": {"name": "c", "ownerReferences": [{"kind": "Thing", "name": "t"}]}});
    assert_eq!(describe("core/ConfigMap", &free, now()).data.unwrap().warning, None);

    let helm = json!({"kind": "ConfigMap", "metadata": {"name": "c", "labels": {"app.kubernetes.io/managed-by": "Helm"}}});
    assert_eq!(
        describe("core/ConfigMap", &helm, now()).data.unwrap().warning.as_deref(),
        Some("Managed by Helm: the next helm upgrade puts the chart's values back.")
    );

    let immutable = json!({"kind": "ConfigMap", "metadata": {"name": "c"}, "immutable": true});
    let d = describe("core/ConfigMap", &immutable, now());
    assert_eq!(row(&d, "Immutable"), Some("yes"));
}

#[test]
fn secret_kinds() {
    let token = json!({"kind": "Secret", "type": "kubernetes.io/service-account-token",
        "metadata": {"name": "api-token", "namespace": "shop", "annotations": {"kubernetes.io/service-account.name": "api"}},
        "data": {"token": STANDARD.encode("x".repeat(2048))}});
    let d = describe("core/Secret", &token, now());
    let sa = d.info.iter().find(|r| r.key == "Service account").unwrap();
    assert_eq!(sa.value, "api");
    let link = sa.link.as_ref().unwrap();
    assert_eq!((link.kind.as_str(), link.namespace.as_deref()), ("core/ServiceAccount", Some("shop")));
    let data = d.data.clone().unwrap();
    assert!(data.warning.unwrap().starts_with("A service account token"));
    assert_eq!(data.read_only, None);
    assert_eq!(row(&d, "Size"), Some("2.0 KiB"));

    let plain = json!({"kind": "Secret", "metadata": {"name": "s"}, "immutable": true, "data": {"bad": "!!notbase64"}});
    let d = describe("core/Secret", &plain, now());
    assert_eq!(row(&d, "Type"), Some("Opaque"));
    assert_eq!(row(&d, "Immutable"), Some("yes"));
    let data = d.data.clone().unwrap();
    assert!(data.read_only.unwrap().starts_with("Immutable"));
    assert_eq!(data.warning, None);
    // Undecodable values count as empty text.
    assert_eq!((data.entries[0].size, data.entries[0].binary), (0, false));

    // Helm's warning wins over the token one.
    let helm_token = json!({"kind": "Secret", "type": "kubernetes.io/service-account-token",
        "metadata": {"name": "t", "labels": {"app.kubernetes.io/managed-by": "Helm"}, "annotations": {"meta.helm.sh/release-name": "shop"}}});
    assert!(describe("core/Secret", &helm_token, now()).data.unwrap().warning.unwrap().starts_with("Managed by Helm (release shop)"));
}

// ---- HorizontalPodAutoscaler ----

#[test]
fn hpa_every_metric_kind() {
    let v = json!({"kind": "HorizontalPodAutoscaler", "metadata": {"name": "api", "namespace": "shop"},
        "spec": {"maxReplicas": 5, "targetCPUUtilizationPercentage": 80,
            "metrics": [
                {"type": "ContainerResource", "containerResource": {"name": "memory", "container": "app", "target": {"type": "AverageValue", "averageValue": "200Mi"}}},
                {"type": "Object", "object": {"metric": {"name": "requests"}, "describedObject": {"kind": "Ingress", "name": "web"}, "target": {"type": "Value", "value": "10k"}}},
                {"type": "External", "external": {"metric": {"name": "queue"}, "target": {"type": "Bogus"}}},
                {"type": "Pods", "pods": {"metric": {"name": "rps"}, "target": {"type": "AverageValue", "averageValue": "100"}}},
                {"type": "Weird"}],
            "behavior": {
                "scaleUp": {"policies": [{"type": "Pods", "value": 1, "periodSeconds": 15}, {"type": "Pods", "value": 4, "periodSeconds": 60}], "selectPolicy": "Min"},
                "scaleDown": {}}},
        "status": {"currentCPUUtilizationPercentage": 45, "lastScaleTime": "2026-10-10T11:00:00Z",
            "currentMetrics": [
                {"type": "ContainerResource", "containerResource": {"name": "memory", "container": "app", "current": {"averageValue": "150Mi"}}},
                {"type": "Object", "object": {"metric": {"name": "requests"}, "describedObject": {"kind": "Ingress", "name": "web"}, "current": {"value": "5k"}}},
                {"type": "External", "external": {"metric": {"name": "queue"}, "current": {"averageUtilization": 20}}},
                {"type": "Pods", "pods": {"metric": {"name": "rps"}, "current": {}}}],
            "conditions": [{"type": "AbleToScale", "status": "False"}, {"type": "ScalingActive", "status": "False"}]}});
    let d = describe("autoscaling/HorizontalPodAutoscaler", &v, now());
    assert_eq!(d.status.as_deref(), Some("Unable to scale"));
    assert_eq!(row(&d, "Scales"), None);
    assert_eq!(row(&d, "Replicas"), Some("0 now · 0 wanted"));
    assert_eq!(row(&d, "Min replicas"), Some("1"));
    assert_eq!(row(&d, "Max replicas"), Some("5"));
    let last = d.info.iter().find(|r| r.key == "Last scaled").unwrap();
    assert!(last.time);
    let metrics = section(&d, "metrics");
    assert_eq!(metrics.summary, "6");
    assert_eq!(cells(metrics), [
        strs(&["cpu", "45%", "80% of request"]),
        strs(&["memory (app)", "150Mi average", "200Mi average"]),
        strs(&["requests on Ingress/web", "5k", "10k"]),
        strs(&["queue (external)", "20%", "—"]),
        strs(&["rps (per pod)", "—", "100 average"]),
        strs(&["", "—", "—"]),
    ]);
    let behavior = section(&d, "behavior");
    assert_eq!(behavior.rows.len(), 1);
    assert_eq!((behavior.rows[0].key.as_str(), behavior.rows[0].value.as_str()), ("Scale up", "1 pod per 15s, 4 pods per 60s (Min)"));
}

#[test]
fn hpa_states() {
    let hpa = |conditions: Value, extra: Value| {
        let mut v = json!({"kind": "HorizontalPodAutoscaler", "metadata": {"name": "api", "namespace": "shop"},
            "spec": {"scaleTargetRef": {"kind": "ReplicationController", "name": "old"}, "minReplicas": 3},
            "status": {"currentReplicas": 3, "desiredReplicas": 4, "conditions": conditions}});
        crate::testing::merge(&mut v, extra);
        describe("autoscaling/HorizontalPodAutoscaler", &v, now())
    };
    let d = hpa(json!([{"type": "ScalingActive", "status": "False"}]), json!({}));
    assert_eq!(d.status.as_deref(), Some("Inactive"));
    assert_eq!(d.info[0].link.as_ref().unwrap().kind, "core/ReplicationController");
    assert_eq!(row(&d, "Replicas"), Some("3 now · 4 wanted"));
    let metrics = section(&d, "metrics");
    assert_eq!(metrics.summary, "none");
    assert_eq!(metrics.empty.as_deref(), Some("No metrics: it won't scale."));
    assert!(d.sections.iter().all(|s| s.id != "behavior"));

    let d = hpa(json!([{"type": "AbleToScale", "status": "True"}]), json!({}));
    assert_eq!(d.status.as_deref(), Some("Active"));

    // CPU target without a reading yet, and a stabilization window alone.
    let d = hpa(json!([]), json!({"spec": {"targetCPUUtilizationPercentage": 50,
        "behavior": {"scaleUp": {"stabilizationWindowSeconds": 0}}}}));
    assert_eq!(cells(section(&d, "metrics")), [strs(&["cpu", "—", "50% of request"])]);
    assert_eq!(section(&d, "behavior").rows[0].value, "waits 0s");

    // Policies without a type, value or period.
    let d = hpa(json!([]), json!({"spec": {"behavior": {"scaleDown": {"policies": [{}]}}}}));
    assert_eq!(section(&d, "behavior").rows[0].value, "0 pods per 0s (Max)");
}

#[test]
fn metric_values() {
    assert_eq!(metric_value(&json!({"averageUtilization": 50, "averageValue": "1"})), "50% (1)");
    assert_eq!(metric_value(&json!({"averageUtilization": 50})), "50%");
    assert_eq!(metric_value(&json!({"averageValue": "1"})), "1 average");
    assert_eq!(metric_value(&json!({"value": "7"})), "7");
    assert_eq!(metric_value(&json!({})), "—");
    let (label, name, target) = metric(&json!({"resource": {"name": "cpu", "target": {"type": "Utilization"}}}), "Resource");
    assert_eq!((label.as_str(), name.as_str(), target.as_str()), ("cpu", "cpu", "0% of request"));
}

// ---- PodDisruptionBudget ----

#[test]
fn pdb_states() {
    let pdb = |spec: Value, status: Value| {
        describe("policy/PodDisruptionBudget", &json!({"kind": "PodDisruptionBudget", "metadata": {"name": "api"}, "spec": spec, "status": status}), now())
    };
    let d = pdb(json!({"maxUnavailable": "25%", "unhealthyPodEvictionPolicy": "AlwaysAllow"}), json!({}));
    assert_eq!(d.status.as_deref(), Some("No pods"));
    assert_eq!(row(&d, "Max unavailable"), Some("25%"));
    assert_eq!(row(&d, "Min available"), None);
    assert_eq!(row(&d, "Selector"), None);
    assert_eq!(row(&d, "Unhealthy pods"), Some("AlwaysAllow"));
    assert_eq!(row(&d, "Healthy"), Some("0 of 0 pods · 0 needed"));
    assert_eq!(row(&d, "Evictions allowed"), Some("0"));

    let d = pdb(json!({"minAvailable": 2}), json!({"currentHealthy": 1, "desiredHealthy": 2, "expectedPods": 3}));
    assert_eq!(d.status.as_deref(), Some("Unhealthy"));
    let d = pdb(json!({}), json!({"currentHealthy": 3, "desiredHealthy": 2, "expectedPods": 3, "disruptionsAllowed": 1}));
    assert_eq!(d.status.as_deref(), Some("Allowing"));
}

// ---- ResourceQuota and LimitRange ----

#[test]
fn quota_states_and_scopes() {
    let near = json!({"kind": "ResourceQuota", "metadata": {"name": "q"},
        "spec": {"hard": {"pods": "10", "services": "lots"}},
        "status": {"used": {"pods": "9"}}});
    let d = describe("core/ResourceQuota", &near, now());
    assert_eq!(d.status.as_deref(), Some("Near limit"));
    let usage = section(&d, "quota");
    assert_eq!(usage.summary, "2 resources");
    let pods = table_row(usage, "pods");
    assert_eq!((pods.fill, pods.warn), (Some(0.9), true));
    let services = table_row(usage, "services");
    assert_eq!(services.cells, ["services", "0", "lots"]);
    assert_eq!((services.fill, services.warn), (None, false));

    let zero = json!({"kind": "ResourceQuota", "metadata": {"name": "q"},
        "spec": {"hard": {"cpu": "0", "memory": "0"},
                 "scopes": ["BestEffort", "NotTerminating"],
                 "scopeSelector": {"matchExpressions": [
                     {"scopeName": "PriorityClass", "operator": "In", "values": ["high", "low"]},
                     {"scopeName": "Terminating", "operator": "Exists"}]}},
        "status": {"hard": {"cpu": "0", "memory": "0"}, "used": {"cpu": "1", "memory": "0"}}});
    let d = describe("core/ResourceQuota", &zero, now());
    assert_eq!(d.status.as_deref(), Some("At limit"));
    let usage = section(&d, "quota");
    assert_eq!(usage.summary, "2 resources · 1 at limit");
    assert_eq!(table_row(usage, "cpu").fill, Some(1.0));
    assert_eq!(table_row(usage, "memory").fill, Some(0.0));
    assert_eq!(row(&d, "Scopes"), Some("BestEffort, NotTerminating"));
    let selectors: Vec<&str> = d.info.iter().filter(|r| r.key == "Scope selector").map(|r| r.value.as_str()).collect();
    assert_eq!(selectors, ["PriorityClass In high,low", "Terminating Exists"]);

    let fine = json!({"kind": "ResourceQuota", "metadata": {"name": "q"}, "status": {"hard": {"pods": "10"}, "used": {"pods": "1"}}});
    assert_eq!(describe("core/ResourceQuota", &fine, now()).status.as_deref(), Some("Within limits"));

    let empty = json!({"kind": "ResourceQuota", "metadata": {"name": "q"}});
    let d = describe("core/ResourceQuota", &empty, now());
    assert_eq!(d.status.as_deref(), Some("Within limits"));
    assert_eq!(section(&d, "quota").summary, "0 resources");
}

#[test]
fn limit_ranges() {
    let v = json!({"kind": "LimitRange", "metadata": {"name": "defaults"}, "spec": {"limits": [
        {"type": "Container", "max": {"cpu": "2", "memory": "1Gi"}, "default": {"cpu": "500m"}, "defaultRequest": {"cpu": "100m"},
         "maxLimitRequestRatio": {"cpu": "4"}},
        {"type": "PersistentVolumeClaim", "min": {"storage": "1Gi"}}]}});
    let d = describe("core/LimitRange", &v, now());
    assert_eq!(row(&d, "Applies to"), Some("Container, PersistentVolumeClaim"));
    let limits = section(&d, "limits");
    assert_eq!(limits.summary, "3");
    assert_eq!(cells(limits), [
        strs(&["Container", "cpu", "—", "2", "100m", "500m", "4"]),
        strs(&["Container", "memory", "—", "1Gi", "—", "—", "—"]),
        strs(&["PersistentVolumeClaim", "storage", "1Gi", "—", "—", "—", "—"]),
    ]);

    let d = describe("core/LimitRange", &json!({"kind": "LimitRange", "metadata": {"name": "none"}}), now());
    assert_eq!(row(&d, "Applies to"), Some("nothing"));
    assert_eq!(section(&d, "limits").summary, "0");
}

// ---- PriorityClass, RuntimeClass, Lease ----

#[test]
fn priority_classes() {
    let v = json!({"kind": "PriorityClass", "metadata": {"name": "system-cluster-critical"},
        "value": 2000000000, "globalDefault": true, "preemptionPolicy": "Never", "description": "Used for critical pods."});
    let d = describe("scheduling.k8s.io/PriorityClass", &v, now());
    assert_eq!(row(&d, "Value"), Some("2000000000"));
    assert_eq!(row(&d, "Default for pods"), Some("yes"));
    assert_eq!(row(&d, "Preemption"), Some("Never"));
    assert_eq!(row(&d, "Description"), Some("Used for critical pods."));
    assert_eq!(row(&d, "Built in"), Some("yes: reserved for cluster components"));

    let d = describe("scheduling.k8s.io/PriorityClass", &json!({"kind": "PriorityClass", "metadata": {"name": "low"}}), now());
    assert_eq!(row(&d, "Value"), Some("0"));
    assert_eq!(row(&d, "Default for pods"), Some("no"));
    assert_eq!(row(&d, "Preemption"), Some("PreemptLowerPriority"));
    assert_eq!(row(&d, "Description"), None);
    assert_eq!(row(&d, "Built in"), None);
}

#[test]
fn runtime_classes() {
    let v = json!({"kind": "RuntimeClass", "metadata": {"name": "gvisor"}, "handler": "runsc",
        "overhead": {"podFixed": {"cpu": "250m", "memory": "120Mi"}},
        "scheduling": {"nodeSelector": {"sandbox": "true"}, "tolerations": [{"key": "a"}, {"key": "b"}]}});
    let d = describe("node.k8s.io/RuntimeClass", &v, now());
    assert_eq!(row(&d, "Handler"), Some("runsc"));
    assert_eq!(row(&d, "Pod overhead"), Some("cpu 250m · memory 120Mi"));
    assert_eq!(row(&d, "Node selector"), Some("sandbox=true"));
    assert_eq!(row(&d, "Tolerations"), Some("2"));

    let d = describe("node.k8s.io/RuntimeClass", &json!({"kind": "RuntimeClass", "metadata": {"name": "runc"}}), now());
    assert_eq!(row(&d, "Handler"), Some(""));
    assert_eq!(d.info.iter().map(|r| r.key.as_str()).collect::<Vec<_>>(), ["Handler"]);
}

#[test]
fn leases() {
    let v = json!({"kind": "Lease", "metadata": {"name": "l"}, "spec": {"holderIdentity": "",
        "acquireTime": "2026-10-10T10:00:00.000000Z", "leaseTransitions": 3, "strategy": "OldestEmulationVersion",
        "preferredHolder": "node-2"}});
    let d = describe("coordination.k8s.io/Lease", &v, now());
    assert_eq!(d.status.as_deref(), Some("Free"));
    assert_eq!(row(&d, "Holder"), Some("nobody"));
    assert!(d.info.iter().find(|r| r.key == "Acquired").unwrap().time);
    assert_eq!(row(&d, "Changed hands"), Some("3 times"));
    assert_eq!(row(&d, "Strategy"), Some("OldestEmulationVersion"));
    assert_eq!(row(&d, "Preferred holder"), Some("node-2"));
    assert_eq!(row(&d, "Duration"), None);

    // Held, with no duration to expire by.
    let v = json!({"kind": "Lease", "metadata": {"name": "l"}, "spec": {"holderIdentity": "node-1", "renewTime": "2020-01-01T00:00:00Z"}});
    assert_eq!(describe("coordination.k8s.io/Lease", &v, now()).status.as_deref(), Some("Held"));
}

// ---- Admission ----

#[test]
fn mutating_webhooks() {
    let v = json!({"kind": "MutatingWebhookConfiguration", "metadata": {"name": "inject"}, "webhooks": [
        {"name": "inject.acme.io", "clientConfig": {"url": "https://hooks.acme.io/inject", "service": {"name": "ignored"}},
         "reinvocationPolicy": "IfNeeded", "timeoutSeconds": 5, "matchPolicy": "Exact",
         "objectSelector": {"matchLabels": {"inject": "true"}},
         "matchConditions": [{"name": "not-system", "expression": "!object.metadata.namespace.startsWith('kube-')"}],
         "rules": [{"operations": ["CREATE"], "apiGroups": ["apps"], "apiVersions": ["v1"], "resources": ["deployments"], "scope": "Namespaced"}, {}]},
        {"name": "bare.acme.io", "clientConfig": {"service": {"namespace": "acme", "name": "hook", "port": 8443}}},
        {"name": "nothing.acme.io"}]});
    let d = describe("admissionregistration.k8s.io/MutatingWebhookConfiguration", &v, now());
    assert_eq!(row(&d, "Webhooks"), Some("3"));
    let s = section(&d, "webhook:inject.acme.io");
    assert_eq!(s.title, "inject.acme.io");
    assert_eq!(s.summary, "Fail · 2 rules");
    let rows: Vec<(&str, &str)> = s.rows.iter().map(|r| (r.key.as_str(), r.value.as_str())).collect();
    assert_eq!(rows, [
        ("Calls", "https://hooks.acme.io/inject"),
        ("If it fails", "Fail: requests are rejected"),
        ("Timeout", "5s"),
        ("Match policy", "Exact"),
        ("Reinvocation", "IfNeeded"),
        ("Objects", "inject=true"),
        ("Condition not-system", "!object.metadata.namespace.startsWith('kube-')"),
    ]);
    assert!(s.rows[0].link.is_none());
    assert_eq!(cells(s), [
        strs(&["CREATE", "apps", "v1", "deployments", "Namespaced"]),
        strs(&["—", "—", "—", "—", "*"]),
    ]);

    let s = section(&d, "webhook:bare.acme.io");
    assert_eq!(s.rows[0].value, "service acme/hook:8443");
    assert_eq!(s.rows[0].link.as_ref().unwrap().name, "hook");
    assert!(s.rows.iter().any(|r| r.key == "Reinvocation" && r.value == "Never"));

    let s = section(&d, "webhook:nothing.acme.io");
    assert_eq!(s.summary, "Fail · 0 rules");
    assert_eq!(s.rows[0].key, "If it fails");
    assert!(s.rows.iter().any(|r| r.key == "Timeout" && r.value == "10s"));
    assert!(s.rows.iter().any(|r| r.key == "Match policy" && r.value == "Equivalent"));

    // Validating webhooks have no reinvocation.
    let v = json!({"kind": "ValidatingWebhookConfiguration", "metadata": {"name": "v"}, "webhooks": [{"name": "v.acme.io"}]});
    let d = describe("admissionregistration.k8s.io/ValidatingWebhookConfiguration", &v, now());
    assert!(d.sections[0].rows.iter().all(|r| r.key != "Reinvocation"));
    let none = describe("admissionregistration.k8s.io/ValidatingWebhookConfiguration", &json!({"metadata": {"name": "e"}}), now());
    assert_eq!(row(&none, "Webhooks"), Some("0"));
}

#[test]
fn validating_admission_policies() {
    let v = json!({"kind": "ValidatingAdmissionPolicy", "metadata": {"name": "replicas"}, "spec": {
        "failurePolicy": "Ignore",
        "paramKind": {"apiVersion": "v1", "kind": "ConfigMap"},
        "matchConstraints": {"matchPolicy": "Equivalent", "namespaceSelector": {"matchLabels": {"env": "prd"}},
            "resourceRules": [{"operations": ["CREATE", "UPDATE"], "apiGroups": ["apps"], "apiVersions": ["v1"], "resources": ["deployments"]}],
            "excludeResourceRules": [{"operations": ["*"], "apiGroups": [""], "apiVersions": ["*"], "resources": ["pods"]}]},
        "validations": [{"expression": "object.spec.replicas <= 5", "message": "too many"},
                        {"expression": "x", "messageExpression": "'bad ' + x", "reason": "Invalid"}],
        "variables": [{"name": "r", "expression": "object.spec.replicas"}],
        "matchConditions": [{"name": "not-system", "expression": "true"}],
        "auditAnnotations": [{"key": "replicas", "valueExpression": "string(object.spec.replicas)"}]},
        "status": {"typeChecking": {"expressionWarnings": [{"fieldRef": "spec.validations[0].expression", "warning": "unknown field"}]}}});
    let d = describe("admissionregistration.k8s.io/ValidatingAdmissionPolicy", &v, now());
    assert_eq!(d.status.as_deref(), Some("Type warnings"));
    assert_eq!(row(&d, "If it fails"), Some("Ignore: requests go through"));
    assert_eq!(row(&d, "Parameters"), Some("ConfigMap (v1)"));
    assert_eq!(row(&d, "Match policy"), Some("Equivalent"));
    assert_eq!(row(&d, "Namespaces"), Some("env=prd"));
    let ids: Vec<&str> = d.sections.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["resources", "excluded", "validations", "variables", "match-conditions", "audit", "type-warnings"]);
    assert_eq!(cells(section(&d, "excluded")), [strs(&["*", "core", "*", "pods", "*"])]);
    let validations = section(&d, "validations");
    assert_eq!(cells(validations), [
        strs(&["object.spec.replicas <= 5", "too many", ""]),
        strs(&["x", "= 'bad ' + x", "Invalid"]),
    ]);
    assert_eq!(validations.table.as_ref().unwrap().mono, [0]);
    assert_eq!(section(&d, "variables").table.as_ref().unwrap().mono, [0, 1]);
    assert_eq!(cells(section(&d, "audit")), [strs(&["replicas", "string(object.spec.replicas)"])]);
    let warnings = section(&d, "type-warnings");
    assert_eq!(warnings.table.as_ref().unwrap().mono, [0]);
    assert_eq!(cells(warnings), [strs(&["spec.validations[0].expression", "unknown field"])]);

    let bare = json!({"kind": "ValidatingAdmissionPolicy", "metadata": {"name": "p"}, "spec": {}});
    let d = describe("admissionregistration.k8s.io/ValidatingAdmissionPolicy", &bare, now());
    assert_eq!(row(&d, "If it fails"), Some("Fail: requests are rejected"));
    assert_eq!(d.status, None);
    let ids: Vec<&str> = d.sections.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["resources", "validations"]);
    assert_eq!(section(&d, "resources").empty.as_deref(), Some("No resource rules."));
}

#[test]
fn policy_bindings_describe() {
    let v = json!({"kind": "ValidatingAdmissionPolicyBinding", "metadata": {"name": "b"}, "spec": {
        "policyName": "replicas", "validationActions": ["Deny", "Audit"],
        "paramRef": {"name": "limits", "namespace": "shop", "selector": {"matchLabels": {"a": "b"}}, "parameterNotFoundAction": "Deny"},
        "matchResources": {"objectSelector": {},
            "resourceRules": [{"operations": ["CREATE"], "apiGroups": ["apps"], "apiVersions": ["v1"], "resources": ["deployments"], "scope": "Namespaced"}]}}});
    let d = describe("admissionregistration.k8s.io/ValidatingAdmissionPolicyBinding", &v, now());
    let policy = d.info.iter().find(|r| r.key == "Policy").unwrap();
    assert_eq!(policy.value, "replicas");
    assert_eq!(policy.link.as_ref().unwrap().kind, "admissionregistration.k8s.io/ValidatingAdmissionPolicy");
    assert_eq!(row(&d, "On failure"), Some("Deny, Audit"));
    assert_eq!(row(&d, "Parameters"), Some("name limits · namespace shop · selector a=b · if missing: Deny"));
    assert_eq!(row(&d, "Objects"), Some("all"));
    assert_eq!(cells(section(&d, "resources")), [strs(&["CREATE", "apps", "v1", "deployments", "Namespaced"])]);
    assert_eq!(section(&d, "resources").title, "Narrowed to");

    let bare = json!({"kind": "ValidatingAdmissionPolicyBinding", "metadata": {"name": "b"}, "spec": {"paramRef": {}}});
    let d = describe("admissionregistration.k8s.io/ValidatingAdmissionPolicyBinding", &bare, now());
    assert_eq!(row(&d, "Policy"), None);
    assert_eq!(row(&d, "On failure"), Some(""));
    assert_eq!(row(&d, "Parameters"), Some(""));
    assert!(d.sections.is_empty());

    let no_params = json!({"kind": "ValidatingAdmissionPolicyBinding", "metadata": {"name": "b"}, "spec": {"policyName": "p"}});
    let d = describe("admissionregistration.k8s.io/ValidatingAdmissionPolicyBinding", &no_params, now());
    assert_eq!(row(&d, "Parameters"), None);
}

// ---- helpers ----

#[test]
fn helpers() {
    let v = json!({"s": "x", "n": 3, "f": 1.5, "b": false, "o": {}, "z": null});
    assert_eq!(str_at(&v, "s").as_deref(), Some("x"));
    assert_eq!(str_at(&v, "n").as_deref(), Some("3"));
    assert_eq!(str_at(&v, "f").as_deref(), Some("1.5"));
    assert_eq!(str_at(&v, "b").as_deref(), Some("false"));
    assert_eq!(str_at(&v, "o"), None);
    assert_eq!(str_at(&v, "z"), None);
    assert_eq!(str_at(&v, "missing"), None);
    assert_eq!(int_at(&v, "n"), Some(3));
    assert_eq!(int_at(&v, "s"), None);

    assert_eq!(label_selector(&json!({})), None);
    assert_eq!(label_selector(&Value::Null), None);
    assert_eq!(label_selector(&json!("nonsense")), None);
    assert_eq!(label_selector(&json!({"matchLabels": {"app": "api"}})).as_deref(), Some("app=api"));

    assert_eq!(decoded_len(""), 0);
    assert_eq!(decoded_len("YQ=="), 1);
    assert_eq!(decoded_len("YWI="), 2);
    assert_eq!(decoded_len("YWJj"), 3);
    assert_eq!(decoded_len("=="), 0);

    assert_eq!(size_text(0), "0 B");
    assert_eq!(size_text(1023), "1023 B");
    assert_eq!(size_text(1536), "1.5 KiB");
    assert_eq!(size_text(3 * 1024 * 1024), "3.0 MiB");

    assert_eq!(plural(0, "pod"), "0 pods");
    assert_eq!(plural(1, "pod"), "1 pod");
    assert_eq!(plural(2, "pod"), "2 pods");

    assert!(valid_key(&"k".repeat(253)));
    assert!(!valid_key(&"k".repeat(254)));
    assert!(!valid_key("."));
    assert!(valid_key(".env"));

    let s = table_section("t", "T", InfoTable::default(), "");
    assert_eq!((s.summary.as_str(), s.empty.clone()), ("none", None));
    let s = table_section("t", "T", InfoTable { rows: vec![InfoTableRow::new(vec!["a".into()])], ..Default::default() }, "empty");
    assert_eq!((s.summary.as_str(), s.empty.as_deref()), ("1", Some("empty")));
}

#[test]
fn data_patches() {
    assert_eq!(data_patch(false, "1", &[]), json!({"metadata": {"resourceVersion": "1"}, "data": {}}));
    let changes = [crate::types::DataChange { key: "pw".into(), value: Some("hunter2".into()) }];
    assert_eq!(data_patch(true, "2", &changes)["data"]["pw"], STANDARD.encode("hunter2"));
    assert_eq!(data_patch(false, "2", &changes)["data"]["pw"], "hunter2");
}
