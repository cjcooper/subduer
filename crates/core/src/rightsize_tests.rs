use serde_json::json;

use super::*;
use crate::testing::{MockApi, object};
use crate::types::ContainerRequests;

const WEB: &str = "/apis/apps/v1/namespaces/shop/deployments/web";

fn deployment(extra: Value) -> Value {
    let mut d = object("apps/v1", "Deployment", Some("shop"), "web", json!({"spec": {"template": {"spec": {"containers": [
        {"name": "app", "image": "acme/web:1", "resources": {
            "requests": {"cpu": "1500m", "memory": "6Gi"}, "limits": {"cpu": "2", "memory": "6Gi"}}},
        {"name": "proxy", "image": "acme/proxy:1"}
    ]}}}}));
    crate::testing::merge(&mut d, extra);
    d
}

fn change(kind: &str, containers: Vec<ContainerRequests>) -> RightsizeChange {
    RightsizeChange { kind: kind.into(), namespace: "shop".into(), name: "web".into(), containers }
}

fn wants(container: &str, cpu: Option<f64>, mem: Option<f64>) -> ContainerRequests {
    ContainerRequests { container: container.into(), cpu_request: cpu, mem_request: mem }
}

fn q(before: Option<&str>, after: Option<&str>) -> QuantityChange {
    QuantityChange { before: before.map(Into::into), after: after.map(Into::into) }
}

#[test]
fn quantities() {
    assert_eq!((cpu_quantity(15.0), cpu_quantity(1350.0), cpu_quantity(0.2)), ("15m".into(), "1350m".into(), "1m".into()));
    assert_eq!(memory_quantity(2944.0 * MIB), "2944Mi");
    assert_eq!(memory_quantity(1000.5), "1001");
}

#[tokio::test]
async fn preview_plans_one_patch_and_dry_runs_it() {
    let api = MockApi::new();
    api.on("GET", WEB, deployment(json!({"metadata": {
        "labels": {"app.kubernetes.io/managed-by": "Helm"}, "annotations": {"meta.helm.sh/release-name": "shop-web"}}})));
    api.on("PATCH", WEB, deployment(json!({})));
    let c = change("Deployment", vec![wants("app", Some(15.0), Some(2944.0 * MIB)), wants("proxy", Some(50.0), Some(64.0 * MIB)), wants("istio-proxy", Some(10.0), None)]);
    let p = preview(&api.client(), &c).await.unwrap();
    assert_eq!((p.helm_release.as_deref(), p.blocked.as_deref(), p.dry_run_error.as_deref()), (Some("shop-web"), None, None));
    assert!(p.effect.starts_with("Rolls out new pods"));
    let app = &p.containers[0];
    assert_eq!(app.cpu_request, q(Some("1500m"), Some("15m")));
    assert_eq!(app.mem_request, q(Some("6Gi"), Some("2944Mi")));
    assert!(!app.cpu_limit.changed() && !app.mem_limit.changed(), "limits stay");
    let proxy = &p.containers[1];
    assert_eq!(proxy.cpu_request, q(None, Some("50m")));
    assert!(p.containers[2].injected);

    let patch = api.calls_to("PATCH", WEB).pop().unwrap();
    assert!(patch.query.contains("dryRun=All"), "{}", patch.query);
    assert_eq!(patch.json(), json!({"spec": {"template": {"spec": {"containers": [
        {"name": "app", "resources": {"requests": {"cpu": "15m", "memory": "2944Mi"}}},
        {"name": "proxy", "resources": {"requests": {"cpu": "50m", "memory": "64Mi"}}}
    ]}}}}));

    let snippet = p.helm_snippet.clone().unwrap();
    assert_eq!(snippet, helm_snippet("shop-web", &p));
    assert!(snippet.starts_with("# Helm release shop-web: Deployment shop/web\n"), "{snippet}");
    assert!(snippet.contains("# container: app\nresources:\n  requests:\n    cpu: 15m\n    memory: 2944Mi\n  limits:\n    cpu: 2\n    memory: 6Gi\n"), "{snippet}");
    assert!(snippet.contains("# container: proxy\nresources:\n  requests:\n    cpu: 50m\n    memory: 64Mi\n"), "{snippet}");
    assert!(!snippet.contains("istio-proxy"));
}

#[tokio::test]
async fn limits_below_a_new_request_rise_with_it() {
    let api = MockApi::new();
    api.on("GET", WEB, deployment(json!({})));
    api.on("PATCH", WEB, deployment(json!({})));
    let p = preview(&api.client(), &change("Deployment", vec![wants("app", Some(2500.0), Some(8192.0 * MIB))])).await.unwrap();
    let app = &p.containers[0];
    assert_eq!(app.cpu_limit, q(Some("2"), Some("2500m")));
    assert_eq!(app.mem_limit, q(Some("6Gi"), Some("8192Mi")));
    let body = api.calls_to("PATCH", WEB).pop().unwrap().json();
    assert_eq!(body["spec"]["template"]["spec"]["containers"][0]["resources"]["limits"], json!({"cpu": "2500m", "memory": "8192Mi"}));
}

#[tokio::test]
async fn cron_jobs_patch_their_job_template() {
    let path = "/apis/batch/v1/namespaces/shop/cronjobs/web";
    let api = MockApi::new();
    let cron = object("batch/v1", "CronJob", Some("shop"), "web", json!({"spec": {"jobTemplate": {"spec": {"template": {"spec": {"containers": [
        {"name": "run", "image": "acme/run:1", "resources": {"requests": {"cpu": "100m"}}}]}}}}}}));
    api.on("GET", path, cron.clone());
    api.on("PATCH", path, cron);
    let p = preview(&api.client(), &change("CronJob", vec![wants("run", Some(80.0), None)])).await.unwrap();
    assert!(p.effect.starts_with("Applies from the next scheduled run"));
    assert_eq!(api.calls_to("PATCH", path)[0].json(), json!({"spec": {"jobTemplate": {"spec": {"template": {"spec": {"containers": [
        {"name": "run", "resources": {"requests": {"cpu": "80m"}}}]}}}}}}));
}

#[tokio::test]
async fn blocked_and_refused() {
    let api = MockApi::new();
    // Kinds whose template can't change: no request is made.
    for (kind, text) in [("Job", "can't be changed once created"), ("Pod", "bare pod"), ("ReplicaSet", "Deployment that owns"), ("Thing", "on a Thing")] {
        let p = preview(&api.client(), &change(kind, vec![wants("app", Some(1.0), None)])).await.unwrap();
        assert!(p.blocked.as_deref().unwrap().contains(text), "{kind}");
    }
    assert!(api.calls().is_empty());

    api.on("GET", WEB, deployment(json!({})));
    let same = preview(&api.client(), &change("Deployment", vec![wants("app", Some(1500.0), None)])).await.unwrap();
    assert_eq!(same.blocked.as_deref(), Some("Nothing to change: the requests already match."));
    assert_eq!(same.helm_snippet, None, "not Helm-managed");
    let injected = preview(&api.client(), &change("Deployment", vec![wants("sidecar", Some(5.0), None)])).await.unwrap();
    assert!(injected.blocked.unwrap().contains("admission webhook"));
    assert!(api.calls_to("PATCH", WEB).is_empty());

    // The server refuses the dry run (a quota, say).
    api.on_error("PATCH", WEB, 403, "Forbidden", "exceeded quota: shop-quota, requested: requests.memory=8Gi");
    let refused = preview(&api.client(), &change("Deployment", vec![wants("app", None, Some(8192.0 * MIB))])).await.unwrap();
    assert_eq!(refused.dry_run_error.as_deref(), Some("exceeded quota: shop-quota, requested: requests.memory=8Gi"));
    assert!(preview(&api.client(), &change("StatefulSet", vec![])).await.is_err(), "a missing workload is an error");
}

#[tokio::test]
async fn apply_is_gated_and_patches_for_real() {
    let api = MockApi::new();
    api.on("GET", WEB, deployment(json!({})));
    api.on("PATCH", WEB, deployment(json!({})));
    let gate = SafetyGate::default();
    let c = change("Deployment", vec![wants("app", Some(15.0), None)]);
    let err = apply(&api.client(), &gate, "acme-uat", &c).await.unwrap_err();
    assert!(matches!(err, Error::Locked(_)));
    assert!(api.calls().is_empty(), "nothing is read before the gate");
    gate.set_unlocked("acme-uat", true);
    apply(&api.client(), &gate, "acme-uat", &c).await.unwrap();
    let patch = api.calls_to("PATCH", WEB).pop().unwrap();
    assert!(!patch.query.contains("dryRun"));
    assert_eq!(patch.json()["spec"]["template"]["spec"]["containers"][0]["resources"], json!({"requests": {"cpu": "15m"}}));
    let none = apply(&api.client(), &gate, "acme-uat", &change("Deployment", vec![wants("app", Some(1500.0), None)])).await.unwrap_err();
    assert_eq!(none.to_string(), "nothing to change");
    assert!(apply(&api.client(), &gate, "acme-uat", &change("Job", vec![])).await.is_err());
}
