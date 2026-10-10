//! Coverage tests for the Namespace details and creating namespaces.

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

fn empty_details() -> ObjectDetails {
    crate::config::describe("core/Namespace", &json!({"kind": "Namespace", "metadata": {"name": "shop"}}), now())
}

fn resources(cpu_req: &str, mem_req: &str, limits: Option<(&str, &str)>) -> Value {
    let mut r = json!({"requests": {"cpu": cpu_req, "memory": mem_req}});
    if let Some((cpu, mem)) = limits {
        r["limits"] = json!({"cpu": cpu, "memory": mem});
    }
    json!({"spec": {"containers": [{"name": "app", "image": "acme/app:1", "resources": r}]}})
}

#[test]
fn other_kinds_are_left_alone() {
    let mut d = empty_details();
    let before = d.info.len();
    describe("core/Pod", &mut d, &json!({"status": {"phase": "Terminating"}}));
    assert_eq!(d.info.len(), before);
}

#[test]
fn terminating_without_conditions_or_finalizers() {
    let v = json!({"kind": "Namespace", "metadata": {"name": "old",
        "labels": {"pod-security.kubernetes.io/audit": "restricted", "pod-security.kubernetes.io/audit-version": "latest"}},
        "status": {"phase": "Terminating"}});
    let d = crate::config::describe("core/Namespace", &v, now());
    assert_eq!(d.status.as_deref(), Some("Terminating"));
    // "latest" isn't worth showing.
    assert_eq!(row(&d, "Pod security"), Some("audit restricted"));
    assert_eq!(row(&d, "Deleting"), Some("removing what's left in it"));
    assert_eq!(row(&d, "Finalizers"), None);
}

#[test]
fn a_namespace_without_a_phase_is_active() {
    let d = empty_details();
    assert_eq!(d.status.as_deref(), Some("Active"));
    assert_eq!(row(&d, "Deleting"), None);
}

#[tokio::test]
async fn related_ignores_other_kinds() {
    let api = MockApi::new();
    let mut d = empty_details();
    related(&api.client(), "core/Pod", "shop", &mut d).await.unwrap();
    assert!(d.sections.is_empty());
    assert!(api.calls().is_empty());
}

#[tokio::test]
async fn related_contents_resources_quotas_and_warnings() {
    let api = MockApi::new();
    let mut running = pod("shop", "api-1", "node-1", resources("250m", "256Mi", Some(("500m", "512Mi"))));
    running["spec"]["serviceAccountName"] = json!("api");
    let mut pending = pod("shop", "api-2", "", resources("750m", "256Mi", None));
    pending["status"] = json!({"phase": "Pending"});
    let mut done = pod("shop", "job-1", "node-1", resources("4", "4Gi", None));
    done["status"] = json!({"phase": "Succeeded"});
    let mut unknown = pod("shop", "odd-1", "node-1", json!({}));
    unknown["status"] = json!({});
    api.on("GET", "/api/v1/namespaces/shop/pods", list("PodList", vec![running, pending, done, unknown]));
    api.on(
        "GET",
        "/apis/apps/v1/namespaces/shop/deployments",
        list("DeploymentList", vec![
            object("apps/v1", "Deployment", Some("shop"), "api", json!({})),
            object("apps/v1", "Deployment", Some("shop"), "web", json!({})),
        ]),
    );
    // An empty list counts as zero and gets no row.
    api.on("GET", "/api/v1/namespaces/shop/services", list("ServiceList", vec![]));
    api.on(
        "GET",
        "/api/v1/namespaces/shop/secrets",
        list("SecretList", vec![object("v1", "Secret", Some("shop"), "tls", json!({}))]),
    );
    api.on_query(
        "GET",
        "/api/v1/namespaces/shop/secrets",
        "owner%3Dhelm",
        list("SecretList", vec![
            object("v1", "Secret", Some("shop"), "sh.helm.release.v1.shop.v2", json!({"metadata": {"labels": {"owner": "helm", "name": "shop"}}})),
            object("v1", "Secret", Some("shop"), "sh.helm.release.v1.shop.v1", json!({"metadata": {"labels": {"owner": "helm", "name": "shop"}}})),
            object("v1", "Secret", Some("shop"), "sh.helm.release.v1.cache.v1", json!({"metadata": {"labels": {"owner": "helm", "name": "cache"}}})),
            object("v1", "Secret", Some("shop"), "unlabelled", json!({"metadata": {"labels": {"owner": "helm"}}})),
        ]),
    );
    api.on(
        "GET",
        "/apis/metrics.k8s.io/v1beta1/namespaces/shop/pods",
        list("PodMetricsList", vec![object("metrics.k8s.io/v1beta1", "PodMetrics", Some("shop"), "api-1", json!({
            "containers": [{"name": "app", "usage": {"cpu": "100m", "memory": "64Mi"}}]
        }))]),
    );
    api.on(
        "GET",
        "/api/v1/namespaces/shop/resourcequotas",
        list("ResourceQuotaList", vec![
            object("v1", "ResourceQuota", Some("shop"), "compute", json!({"status": {"hard": {"requests.cpu": "2", "requests.memory": "512Mi"}}})),
            // The tightest quota wins.
            object("v1", "ResourceQuota", Some("shop"), "loose", json!({"status": {"hard": {"cpu": "8"}}})),
        ]),
    );
    api.on(
        "GET",
        "/api/v1/namespaces/shop/limitranges",
        list("LimitRangeList", vec![object("v1", "LimitRange", Some("shop"), "defaults", json!({}))]),
    );
    api.on_query(
        "GET",
        "/api/v1/namespaces/shop/events",
        "type%3DWarning",
        list("EventList", (0..25).map(|i| object("v1", "Event", Some("shop"), &format!("e{i}"), json!({
            "type": "Warning", "reason": "BackOff", "message": "restarting", "count": 1,
            "lastTimestamp": format!("2026-10-10T11:{:02}:00Z", i),
            "involvedObject": {"kind": "Pod", "name": format!("api-{i}"), "namespace": "shop"}
        }))).collect()),
    );

    let mut d = empty_details();
    related(&api.client(), "core/Namespace", "shop", &mut d).await.unwrap();

    // Contents.
    let contents = section(&d, "ns-contents");
    let rows = &contents.table.as_ref().unwrap().rows;
    assert_eq!(rows[0].cells, ["Pods", "4 (1 pending, 1 running, 1 succeeded, 1 unknown)"]);
    assert!(rows[0].warn);
    assert_eq!(rows[0].link.as_ref().unwrap().kind, "core/Pod");
    assert_eq!(rows[0].link.as_ref().unwrap().name, "");
    let labels: Vec<&str> = rows.iter().map(|r| r.cells[0].as_str()).collect();
    assert_eq!(labels, ["Pods", "Deployments", "Secrets"]);
    assert_eq!(rows[1].cells[1], "2");
    assert_eq!(rows[1].link.as_ref().unwrap().kind, "apps/Deployment");
    assert_eq!(rows[1].link.as_ref().unwrap().namespace.as_deref(), Some("shop"));
    assert_eq!(contents.summary, "7 objects");

    // Resources come first.
    let res = &d.sections[0];
    assert_eq!(res.id, "ns-resources");
    assert_eq!(res.summary, "3 pods running");
    assert!(res.rows.is_empty(), "metrics are known");
    let t = res.table.as_ref().unwrap();
    assert_eq!(t.rows[0].cells, ["CPU", "100m", "1.00 cores", "not set on 2 pods", "2.00 cores"]);
    assert_eq!((t.rows[0].fill, t.rows[0].warn), (Some(0.5), false));
    assert_eq!(t.rows[1].cells, ["Memory", "64.0 MiB", "512.0 MiB", "not set on 2 pods", "512.0 MiB"]);
    assert_eq!((t.rows[1].fill, t.rows[1].warn), (Some(1.0), true));

    // Quotas, limit ranges and releases (deduplicated, sorted).
    let rel = section(&d, "ns-related");
    let cells: Vec<Vec<String>> = rel.table.as_ref().unwrap().rows.iter().map(|r| r.cells.clone()).collect();
    assert_eq!(cells, [
        ["ResourceQuota", "compute"],
        ["ResourceQuota", "loose"],
        ["LimitRange", "defaults"],
        ["Helm release", "cache"],
        ["Helm release", "shop"],
    ]);
    assert_eq!(rel.summary, "5");
    let links: Vec<&str> = rel.table.as_ref().unwrap().rows.iter().map(|r| r.link.as_ref().unwrap().kind.as_str()).collect();
    assert_eq!(links, ["core/ResourceQuota", "core/ResourceQuota", "core/LimitRange", "helm.sh/Release", "helm.sh/Release"]);

    // Warnings, newest first, capped at 20, naming the object.
    assert_eq!(d.events.len(), 20);
    assert_eq!(d.events[0].object.as_deref(), Some("Pod api-24"));
}

#[tokio::test]
async fn related_on_an_empty_namespace_without_metrics() {
    let api = MockApi::new();
    api.on("GET", "/api/v1/namespaces/shop/pods", list("PodList", vec![]));
    let mut d = empty_details();
    related(&api.client(), "core/Namespace", "shop", &mut d).await.unwrap();

    let contents = section(&d, "ns-contents");
    let rows = &contents.table.as_ref().unwrap().rows;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].cells, ["Pods", "0"]);
    assert!(!rows[0].warn);
    assert_eq!(contents.summary, "0 objects");

    let res = &d.sections[0];
    assert_eq!(res.summary, "0 pods running");
    let t = res.table.as_ref().unwrap();
    assert_eq!(t.rows[0].cells, ["CPU", "—", "0m", "—", "none"]);
    assert_eq!(t.rows[1].cells, ["Memory", "—", "0 B", "—", "none"]);
    assert_eq!(t.rows[0].fill, None);
    assert_eq!(res.rows[0].value, "unknown: metrics-server isn't installed");

    assert!(d.sections.iter().all(|s| s.id != "ns-related"));
    assert!(d.events.is_empty());
}

#[tokio::test]
async fn related_with_limits_everywhere_and_no_pods_list() {
    let api = MockApi::new();
    // Pods can't be listed: treated as none. Other pods below are only for limits.
    api.on_error("GET", "/api/v1/namespaces/shop/pods", 403, "Forbidden", "pods is forbidden");
    let mut d = empty_details();
    related(&api.client(), "core/Namespace", "shop", &mut d).await.unwrap();
    assert_eq!(section(&d, "ns-contents").table.as_ref().unwrap().rows[0].cells, ["Pods", "0"]);

    let api = MockApi::new();
    api.on(
        "GET",
        "/api/v1/namespaces/shop/pods",
        list("PodList", vec![pod("shop", "api-1", "node-1", resources("1500m", "1Gi", Some(("2", "2Gi"))))]),
    );
    api.on(
        "GET",
        "/api/v1/namespaces/shop/resourcequotas",
        list("ResourceQuotaList", vec![object("v1", "ResourceQuota", Some("shop"), "zero", json!({"status": {"hard": {"requests.cpu": "0"}}}))]),
    );
    let mut d = empty_details();
    related(&api.client(), "core/Namespace", "shop", &mut d).await.unwrap();
    let t = d.sections[0].table.as_ref().unwrap();
    assert_eq!(t.rows[0].cells, ["CPU", "—", "1.50 cores", "2.00 cores", "0m"]);
    // A zero quota doesn't fill a bar.
    assert_eq!(t.rows[0].fill, None);
    assert_eq!(t.rows[1].cells, ["Memory", "—", "1.0 GiB", "2.0 GiB", "none"]);
    assert_eq!(section(&d, "ns-contents").table.as_ref().unwrap().rows[0].cells, ["Pods", "1 (1 running)"]);
    assert!(!section(&d, "ns-contents").table.as_ref().unwrap().rows[0].warn);
}

#[test]
fn namespace_name_edges() {
    assert!(valid_namespace_name("a"));
    assert!(valid_namespace_name("0"));
    assert!(valid_namespace_name(&"x".repeat(63)));
    assert!(!valid_namespace_name("a--b-"));
    assert!(valid_namespace_name("a--b"));
}

#[tokio::test]
async fn dry_run_needs_no_unlock_and_asks_the_server() {
    let api = MockApi::new();
    api.on("POST", "/api/v1/namespaces", object("v1", "Namespace", None, "acme-uat", json!({})));
    let gate = SafetyGate::default();
    create_namespace(&api.client(), &gate, "acme-prd", "acme-uat", true).await.unwrap();
    let calls = api.calls_to("POST", "/api/v1/namespaces");
    assert_eq!(calls.len(), 1);
    assert!(calls[0].query.contains("dryRun=All"), "{}", calls[0].query);
    assert_eq!(calls[0].json()["metadata"]["name"], "acme-uat");
    assert_eq!(calls[0].json()["kind"], "Namespace");
}

#[tokio::test]
async fn create_when_unlocked() {
    let api = MockApi::new();
    api.on("POST", "/api/v1/namespaces", object("v1", "Namespace", None, "shop", json!({})));
    let gate = SafetyGate::default();
    gate.set_unlocked("acme-uat", true);
    create_namespace(&api.client(), &gate, "acme-uat", "shop", false).await.unwrap();
    let calls = api.calls_to("POST", "/api/v1/namespaces");
    assert_eq!(calls.len(), 1);
    assert!(!calls[0].query.contains("dryRun"));
}

#[tokio::test]
async fn invalid_names_and_server_errors() {
    let api = MockApi::new();
    let gate = SafetyGate::default();
    gate.set_unlocked("acme-uat", true);
    // Validated before anything is sent, even for a dry run.
    let err = create_namespace(&api.client(), &gate, "acme-uat", "-shop", true).await.unwrap_err();
    assert!(matches!(err, Error::Message(m) if m.contains("lowercase")));
    assert!(api.calls().is_empty());

    api.on_error("POST", "/api/v1/namespaces", 409, "AlreadyExists", "namespaces \"shop\" already exists");
    let err = create_namespace(&api.client(), &gate, "acme-uat", "shop", false).await.unwrap_err();
    assert!(err.user_message().contains("already exists"), "{err}");
}
