use super::*;
use crate::testing::{Call, MockApi, kind, object, pod};
use serde_json::Value;

fn unlocked() -> SafetyGate {
    let gate = SafetyGate::default();
    gate.set_unlocked("acme-uat", true);
    gate
}

fn deployment() -> KindRef {
    kind("apps", "v1", "Deployment", "deployments", true)
}

const DEPLOY: &str = "/apis/apps/v1/namespaces/shop/deployments/web";

fn only_call(api: &MockApi, method: &str, path: &str) -> Call {
    let calls = api.calls_to(method, path);
    assert_eq!(calls.len(), 1, "{method} {path}: {:?}", api.calls());
    calls.into_iter().next().unwrap()
}

fn is_dry_run(call: &Call) -> bool {
    call.query.contains("dryRun=All")
}

#[tokio::test]
async fn delete_sends_background_propagation() {
    let api = MockApi::new();
    let path = "/api/v1/namespaces/shop/pods/web";
    api.on("DELETE", path, pod("shop", "web", "node-1", json!({})));
    let pods = kind("", "v1", "Pod", "pods", true);
    delete(&api.client(), &unlocked(), "acme-uat", &pods, Some("shop"), "web", false).await.unwrap();
    let call = only_call(&api, "DELETE", path);
    assert!(!is_dry_run(&call));
    assert_eq!(call.json()["propagationPolicy"], "Background");
}

#[tokio::test]
async fn dry_run_delete_skips_the_gate() {
    let api = MockApi::new();
    let path = "/api/v1/namespaces/acme";
    api.on("DELETE", path, object("v1", "Namespace", None, "acme", json!({})));
    let ns = kind("", "v1", "Namespace", "namespaces", false);
    // Cluster-scoped: the namespace argument is ignored.
    delete(&api.client(), &SafetyGate::default(), "acme-prd", &ns, Some("shop"), "acme", true).await.unwrap();
    let call = only_call(&api, "DELETE", path);
    assert!(is_dry_run(&call) || call.json()["dryRun"] == json!(["All"]), "{call:?}");
}

#[tokio::test]
async fn delete_errors_come_back() {
    let api = MockApi::new();
    let pods = kind("", "v1", "Pod", "pods", true);
    let err = delete(&api.client(), &unlocked(), "acme-uat", &pods, Some("shop"), "web", false).await.unwrap_err();
    assert!(matches!(err, Error::Kube(kube::Error::Api(s)) if s.code == 404));
}

#[tokio::test]
async fn restart_stamps_the_pod_template() {
    let api = MockApi::new();
    api.on("PATCH", DEPLOY, object("apps/v1", "Deployment", Some("shop"), "web", json!({})));
    let before = jiff::Timestamp::now().round(jiff::Unit::Second).unwrap();
    restart(&api.client(), &unlocked(), "acme-uat", &deployment(), "shop", "web", false).await.unwrap();
    let call = only_call(&api, "PATCH", DEPLOY);
    assert!(!is_dry_run(&call));
    let stamp = call.json()["spec"]["template"]["metadata"]["annotations"][RESTARTED_AT].as_str().unwrap().to_owned();
    let stamp: jiff::Timestamp = stamp.parse().unwrap();
    assert!(stamp >= before - jiff::SignedDuration::from_secs(1));
    assert_eq!(stamp.subsec_nanosecond(), 0);

    // Dry run: gate skipped, flagged to the server.
    let api = MockApi::new();
    api.on("PATCH", DEPLOY, object("apps/v1", "Deployment", Some("shop"), "web", json!({})));
    restart(&api.client(), &SafetyGate::default(), "acme-prd", &deployment(), "shop", "web", true).await.unwrap();
    assert!(is_dry_run(&only_call(&api, "PATCH", DEPLOY)));
}

#[tokio::test]
async fn restart_refuses_other_kinds() {
    let api = MockApi::new();
    let ds = kind("apps", "v1", "DaemonSet", "daemonsets", true);
    let err = restart(&api.client(), &unlocked(), "acme-uat", &ds, "shop", "agent", false).await.unwrap_err();
    assert!(matches!(&err, Error::Message(m) if m == "DaemonSet can't be restarted or scaled here"), "{err}");
    assert!(api.calls().is_empty());
}

#[tokio::test]
async fn replicas_read_the_scale_subresource() {
    let api = MockApi::new();
    let scale_obj = json!({"apiVersion": "autoscaling/v1", "kind": "Scale",
        "metadata": {"name": "web", "namespace": "shop"}, "spec": {"replicas": 3}, "status": {"replicas": 2}});
    api.on("GET", &format!("{DEPLOY}/scale"), scale_obj);
    let r = replicas(&api.client(), &deployment(), "shop", "web").await.unwrap();
    assert_eq!((r.desired, r.current), (3, 2));

    let api = MockApi::new();
    api.on("GET", &format!("{DEPLOY}/scale"), json!({"apiVersion": "autoscaling/v1", "kind": "Scale", "metadata": {"name": "web"}}));
    let r = replicas(&api.client(), &deployment(), "shop", "web").await.unwrap();
    assert_eq!((r.desired, r.current), (0, 0));

    let job = kind("batch", "v1", "Job", "jobs", true);
    assert!(matches!(replicas(&api.client(), &job, "shop", "web").await, Err(Error::Message(_))));
    let err = replicas(&MockApi::new().client(), &deployment(), "shop", "web").await.unwrap_err();
    assert!(matches!(err, Error::Kube(_)));
}

#[tokio::test]
async fn scale_patches_the_scale_subresource() {
    let api = MockApi::new();
    let path = "/apis/apps/v1/namespaces/shop/statefulsets/db/scale";
    let scale_obj = json!({"apiVersion": "autoscaling/v1", "kind": "Scale", "metadata": {"name": "db"}, "spec": {"replicas": 5}});
    api.on("PATCH", path, scale_obj);
    let sts = kind("apps", "v1", "StatefulSet", "statefulsets", true);
    scale(&api.client(), &unlocked(), "acme-uat", &sts, "shop", "db", 5, false).await.unwrap();
    let call = only_call(&api, "PATCH", path);
    assert_eq!(call.json(), json!({"spec": {"replicas": 5}}));
    assert!(!is_dry_run(&call));
    scale(&api.client(), &SafetyGate::default(), "acme-prd", &sts, "shop", "db", 0, true).await.unwrap();
    let calls = api.calls_to("PATCH", path);
    assert!(is_dry_run(&calls[1]));
    assert_eq!(calls[1].json(), json!({"spec": {"replicas": 0}}));
}

#[tokio::test]
async fn suspend_and_resume() {
    let api = MockApi::new();
    let path = "/apis/batch/v1/namespaces/shop/cronjobs/nightly";
    api.on("PATCH", path, object("batch/v1", "CronJob", Some("shop"), "nightly", json!({})));
    let cron = kind("batch", "v1", "CronJob", "cronjobs", true);
    set_suspended(&api.client(), &unlocked(), "acme-uat", &cron, "shop", "nightly", true, false).await.unwrap();
    set_suspended(&api.client(), &SafetyGate::default(), "acme-prd", &cron, "shop", "nightly", false, true).await.unwrap();
    let calls = api.calls_to("PATCH", path);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].json(), json!({"spec": {"suspend": true}}));
    assert!(!is_dry_run(&calls[0]));
    assert_eq!(calls[1].json(), json!({"spec": {"suspend": false}}));
    assert!(is_dry_run(&calls[1]));
}

const CRON: &str = "/apis/batch/v1/namespaces/shop/cronjobs/nightly";
const JOBS: &str = "/apis/batch/v1/namespaces/shop/jobs";

fn cron_job() -> Value {
    object("batch/v1", "CronJob", Some("shop"), "nightly", json!({"spec": {"schedule": "0 3 * * *", "jobTemplate": {
        "metadata": {"labels": {"app": "nightly"}},
        "spec": {"template": {"spec": {"restartPolicy": "Never", "containers": [{"name": "run", "image": "acme/run:1"}]}}}
    }}}))
}

#[tokio::test]
async fn run_cron_job_creates_an_owned_job() {
    let api = MockApi::new();
    api.on("GET", CRON, cron_job());
    api.on("POST", JOBS, object("batch/v1", "Job", Some("shop"), "nightly-manual-abcde", json!({})));
    let name = run_cron_job(&api.client(), &unlocked(), "acme-uat", "shop", "nightly", false).await.unwrap();
    assert_eq!(name, "nightly-manual-abcde");
    let call = only_call(&api, "POST", JOBS);
    assert!(!is_dry_run(&call));
    let body = call.json();
    assert!(body["metadata"]["name"].as_str().unwrap().starts_with("nightly-manual-"));
    assert_eq!(body["metadata"]["namespace"], "shop");
    assert_eq!(body["metadata"]["labels"]["app"], "nightly");
    assert_eq!(body["metadata"]["annotations"]["cronjob.kubernetes.io/instantiate"], "manual");
    assert_eq!(body["metadata"]["ownerReferences"][0]["uid"], "uid-nightly");
    assert_eq!(body["spec"]["template"]["spec"]["containers"][0]["image"], "acme/run:1");
}

#[tokio::test]
async fn run_cron_job_dry_run_and_errors() {
    let api = MockApi::new();
    api.on("GET", CRON, cron_job());
    api.on("POST", JOBS, object("batch/v1", "Job", Some("shop"), "nightly-manual-x", json!({})));
    run_cron_job(&api.client(), &SafetyGate::default(), "acme-prd", "shop", "nightly", true).await.unwrap();
    assert!(is_dry_run(&only_call(&api, "POST", JOBS)));

    let api = MockApi::new();
    let err = run_cron_job(&api.client(), &unlocked(), "acme-uat", "shop", "nightly", false).await.unwrap_err();
    assert!(matches!(err, Error::Kube(kube::Error::Api(s)) if s.code == 404));
    assert!(api.calls_to("POST", JOBS).is_empty());

    let api = MockApi::new();
    api.on("GET", CRON, cron_job());
    api.on_error("POST", JOBS, 403, "Forbidden", "jobs is forbidden");
    let err = run_cron_job(&api.client(), &unlocked(), "acme-uat", "shop", "nightly", false).await.unwrap_err();
    assert_eq!(err.user_message(), "jobs is forbidden");
}

#[test]
fn manual_job_names_trim_trailing_separators() {
    let now: jiff::Timestamp = "2026-10-10T12:00:00Z".parse().unwrap();
    // 50 characters fit; the cut lands right after a '-'.
    let name = manual_job_name(&format!("{}-tail", "a".repeat(49)), now);
    assert!(name.starts_with(&format!("{}-manual-", "a".repeat(49))), "{name}");
    assert!(name.len() <= 63);
    // The suffix is deterministic for a given time.
    assert_eq!(manual_job_name("x", now), manual_job_name("x", now));
    assert_ne!(manual_job_name("x", now), manual_job_name("x", now + jiff::SignedDuration::from_secs(1)));
}

const CM: &str = "/api/v1/namespaces/shop/configmaps/app";

fn config_map() -> KindRef {
    kind("", "v1", "ConfigMap", "configmaps", true)
}

#[tokio::test]
async fn save_data_patches_with_the_resource_version() {
    let api = MockApi::new();
    api.on("PATCH", CM, object("v1", "ConfigMap", Some("shop"), "app", json!({})));
    let changes = [
        DataChange { key: "a".into(), value: Some("1".into()) },
        DataChange { key: "old".into(), value: None },
    ];
    save_data(&api.client(), &unlocked(), "acme-uat", &config_map(), "shop", "app", "41", &changes, false).await.unwrap();
    let call = only_call(&api, "PATCH", CM);
    assert!(!is_dry_run(&call));
    let body = call.json();
    assert_eq!(body["metadata"]["resourceVersion"], "41");
    assert_eq!(body, crate::config::data_patch(false, "41", &changes));

    // Secrets go through the same path, dry runs skip the gate.
    let api = MockApi::new();
    let path = "/api/v1/namespaces/shop/secrets/app";
    api.on("PATCH", path, object("v1", "Secret", Some("shop"), "app", json!({})));
    let secret = kind("", "v1", "Secret", "secrets", true);
    save_data(&api.client(), &SafetyGate::default(), "acme-prd", &secret, "shop", "app", "7", &changes[..1], true).await.unwrap();
    let call = only_call(&api, "PATCH", path);
    assert!(is_dry_run(&call));
    assert_eq!(call.json(), crate::config::data_patch(true, "7", &changes[..1]));
}

#[tokio::test]
async fn save_data_explains_conflicts() {
    let api = MockApi::new();
    api.on_error("PATCH", CM, 409, "Conflict", "the object has been modified");
    let changes = [DataChange { key: "a".into(), value: Some("1".into()) }];
    let err = save_data(&api.client(), &unlocked(), "acme-uat", &config_map(), "shop", "app", "41", &changes, false)
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::Message(m) if m.starts_with("ConfigMap app was changed by someone else")),
        "{err}"
    );

    let api = MockApi::new();
    api.on_error("PATCH", CM, 403, "Forbidden", "configmaps \"app\" is forbidden");
    let err = save_data(&api.client(), &unlocked(), "acme-uat", &config_map(), "shop", "app", "41", &changes, false)
        .await
        .unwrap_err();
    assert_eq!(err.user_message(), "configmaps \"app\" is forbidden");
}

const HPA: &str = "/apis/autoscaling/v2/namespaces/shop/horizontalpodautoscalers/web";

fn hpa() -> KindRef {
    kind("autoscaling", "v2", "HorizontalPodAutoscaler", "horizontalpodautoscalers", true)
}

#[tokio::test]
async fn hpa_ranges() {
    let api = MockApi::new();
    api.on("PATCH", HPA, object("autoscaling/v2", "HorizontalPodAutoscaler", Some("shop"), "web", json!({})));
    set_hpa_range(&api.client(), &unlocked(), "acme-uat", &hpa(), "shop", "web", 2, 5, false).await.unwrap();
    set_hpa_range(&api.client(), &SafetyGate::default(), "acme-prd", &hpa(), "shop", "web", 1, 1, true).await.unwrap();
    let calls = api.calls_to("PATCH", HPA);
    assert_eq!(calls[0].json(), json!({"spec": {"minReplicas": 2, "maxReplicas": 5}}));
    assert!(!is_dry_run(&calls[0]));
    assert_eq!(calls[1].json(), json!({"spec": {"minReplicas": 1, "maxReplicas": 1}}));
    assert!(is_dry_run(&calls[1]));

    let err = set_hpa_range(&api.client(), &unlocked(), "acme-uat", &hpa(), "shop", "web", 0, 5, false).await.unwrap_err();
    assert!(matches!(&err, Error::Message(m) if m.contains("1 ≤ min ≤ max")), "{err}");
    let err = set_hpa_range(&api.client(), &unlocked(), "acme-uat", &deployment(), "shop", "web", 1, 5, false)
        .await
        .unwrap_err();
    assert!(matches!(&err, Error::Message(m) if m == "Deployment has no replica range"), "{err}");
    assert_eq!(api.calls().len(), 2);
}

const PVC: &str = "/api/v1/namespaces/shop/persistentvolumeclaims/data";

fn claim(size: Option<&str>) -> Value {
    let spec = match size {
        Some(s) => json!({"resources": {"requests": {"storage": s}}}),
        None => json!({}),
    };
    object("v1", "PersistentVolumeClaim", Some("shop"), "data", json!({"spec": spec}))
}

#[tokio::test]
async fn claims_grow() {
    let api = MockApi::new();
    api.on("GET", PVC, claim(Some("10Gi")));
    api.on("PATCH", PVC, claim(Some("20Gi")));
    expand_claim(&api.client(), &unlocked(), "acme-uat", "shop", "data", "20Gi", false).await.unwrap();
    let call = only_call(&api, "PATCH", PVC);
    assert_eq!(call.json(), json!({"spec": {"resources": {"requests": {"storage": "20Gi"}}}}));
    assert!(!is_dry_run(&call));

    expand_claim(&api.client(), &SafetyGate::default(), "acme-prd", "shop", "data", "11Gi", true).await.unwrap();
    assert!(is_dry_run(&api.calls_to("PATCH", PVC)[1]));
}

#[tokio::test]
async fn claims_never_shrink() {
    let api = MockApi::new();
    api.on("GET", PVC, claim(Some("10Gi")));
    for size in ["10Gi", "5Gi"] {
        let err = expand_claim(&api.client(), &unlocked(), "acme-uat", "shop", "data", size, false).await.unwrap_err();
        assert!(matches!(&err, Error::Message(m) if m == "claims can only grow: it's 10Gi now"), "{err}");
    }
    assert!(api.calls_to("PATCH", PVC).is_empty());
    let err = expand_claim(&api.client(), &unlocked(), "acme-uat", "shop", "data", "0", false).await.unwrap_err();
    assert!(matches!(&err, Error::Message(m) if m.contains("isn't a size")), "{err}");
}

#[tokio::test]
async fn claims_without_a_size_can_be_set() {
    let api = MockApi::new();
    api.on("GET", PVC, claim(None));
    api.on("PATCH", PVC, claim(Some("1Gi")));
    expand_claim(&api.client(), &unlocked(), "acme-uat", "shop", "data", "1Gi", false).await.unwrap();
    assert_eq!(api.calls_to("PATCH", PVC).len(), 1);

    let missing = MockApi::new();
    let err = expand_claim(&missing.client(), &unlocked(), "acme-uat", "shop", "data", "1Gi", false).await.unwrap_err();
    assert!(matches!(err, Error::Kube(_)));
}

#[tokio::test]
async fn reclaim_policies() {
    let api = MockApi::new();
    let path = "/api/v1/persistentvolumes/pv-1";
    api.on("PATCH", path, object("v1", "PersistentVolume", None, "pv-1", json!({})));
    set_reclaim_policy(&api.client(), &unlocked(), "acme-uat", "pv-1", "Retain", false).await.unwrap();
    set_reclaim_policy(&api.client(), &SafetyGate::default(), "acme-prd", "pv-1", "Delete", true).await.unwrap();
    let calls = api.calls_to("PATCH", path);
    assert_eq!(calls[0].json(), json!({"spec": {"persistentVolumeReclaimPolicy": "Retain"}}));
    assert!(!is_dry_run(&calls[0]));
    assert_eq!(calls[1].json(), json!({"spec": {"persistentVolumeReclaimPolicy": "Delete"}}));
    assert!(is_dry_run(&calls[1]));
}
