//! Coverage tests for KEDA Details: the describe functions and the related
//! lookups (the HPA, Jobs, users of an authentication, the controller),
//! against the fake API server.

use super::*;
use crate::config::{describe as describe_object, object_details};
use crate::testing::{MockApi, kind, list, object};
use crate::types::{InfoTable, LogSource};
use serde_json::json;

const DEPLOYMENTS: &str = "/apis/apps/v1/deployments";
const HPA: &str = "/apis/autoscaling/v2/namespaces/shop/horizontalpodautoscalers/keda-hpa-web";
const SHOP_SO: &str = "/apis/keda.sh/v1alpha1/namespaces/shop/scaledobjects";
const SHOP_SJ: &str = "/apis/keda.sh/v1alpha1/namespaces/shop/scaledjobs";

fn now() -> jiff::Timestamp {
    "2026-10-27T12:00:00Z".parse().unwrap()
}

fn row<'a>(d: &'a ObjectDetails, key: &str) -> Option<&'a str> {
    d.info.iter().find(|r| r.key == key).map(|r| r.value.as_str())
}

fn info<'a>(d: &'a ObjectDetails, key: &str) -> &'a InfoRow {
    d.info.iter().find(|r| r.key == key).unwrap_or_else(|| panic!("no {key} row"))
}

fn section<'a>(d: &'a ObjectDetails, id: &str) -> &'a InfoSection {
    d.sections.iter().find(|s| s.id == id).unwrap_or_else(|| panic!("no {id} section"))
}

fn table<'a>(d: &'a ObjectDetails, id: &str) -> &'a InfoTable {
    section(d, id).table.as_ref().unwrap()
}

fn cells(t: &InfoTable) -> Vec<Vec<&str>> {
    t.rows.iter().map(|r| r.cells.iter().map(String::as_str).collect()).collect()
}

fn keys(d: &ObjectDetails) -> Vec<&str> {
    d.info.iter().map(|r| r.key.as_str()).collect()
}

fn conditions(list: &[(&str, &str)]) -> Value {
    Value::Array(list.iter().map(|(t, s)| json!({"type": t, "status": s, "reason": "r", "message": "m"})).collect())
}

fn so_kind() -> KindRef {
    kind("keda.sh", "v1alpha1", "ScaledObject", "scaledobjects", true)
}

fn controller_deployment() -> Value {
    json!({"apiVersion": "meta.k8s.io/v1", "kind": "PartialObjectMetadataList", "metadata": {},
           "items": [{"apiVersion": "meta.k8s.io/v1", "kind": "PartialObjectMetadata",
                      "metadata": {"name": "keda-operator", "namespace": "keda"}}]})
}

/// A ScaledObject on a Deployment: a Prometheus trigger with an auth, and a
/// cpu one; scales to zero; the Prometheus reads have been failing.
fn web() -> Value {
    object("keda.sh/v1alpha1", "ScaledObject", Some("shop"), "web", json!({
        "spec": {
            "scaleTargetRef": {"name": "web"},
            "minReplicaCount": 0,
            "maxReplicaCount": 10,
            "cooldownPeriod": 120,
            "fallback": {"failureThreshold": 3, "replicas": 2},
            "advanced": {"restoreToOriginalReplicaCount": true},
            "triggers": [
                {"type": "prometheus", "name": "rps", "authenticationRef": {"name": "prom-auth"},
                 "metadata": {"serverAddress": "http://prom:9090", "threshold": "50", "password": "hunter2", "passwordFromEnv": "PROM_PW"}},
                {"type": "cpu", "metricType": "Utilization", "metadata": {"value": "70"}}
            ]
        },
        "status": {
            "conditions": conditions(&[("Ready", "True"), ("Active", "True"), ("Fallback", "False"), ("Paused", "False")]),
            "externalMetricNames": ["s0-prometheus"],
            "resourceMetricNames": ["cpu"],
            "hpaName": "keda-hpa-web",
            "originalReplicaCount": 3,
            "lastActiveTime": "2026-10-27T11:59:00Z",
            "health": {"s0-prometheus": {"numberOfFailures": 4, "status": "Failing"}}
        }
    }))
}

fn web_hpa() -> Value {
    object("autoscaling/v2", "HorizontalPodAutoscaler", Some("shop"), "keda-hpa-web", json!({
        "metadata": {"ownerReferences": [{"apiVersion": "keda.sh/v1alpha1", "kind": "ScaledObject", "name": "web", "uid": "u", "controller": true}]},
        "spec": {
            "scaleTargetRef": {"apiVersion": "apps/v1", "kind": "Deployment", "name": "web"},
            "minReplicas": 1, "maxReplicas": 10,
            "metrics": [
                {"type": "External", "external": {"metric": {"name": "s0-prometheus"}, "target": {"type": "AverageValue", "averageValue": "50"}}},
                {"type": "Resource", "resource": {"name": "cpu", "target": {"type": "Utilization", "averageUtilization": 70}}}
            ]
        },
        "status": {
            "currentReplicas": 3, "desiredReplicas": 4,
            "currentMetrics": [
                {"type": "External", "external": {"metric": {"name": "s0-prometheus"}, "current": {"averageValue": "61"}}},
                {"type": "Resource", "resource": {"name": "cpu", "current": {"averageUtilization": 55, "averageValue": "110m"}}}
            ]
        }
    }))
}

// ---- ScaledObject ----

#[test]
fn scaled_object_says_what_it_scales_and_how() {
    let d = describe_object("keda.sh/ScaledObject", &web(), now());
    assert_eq!(d.status.as_deref(), Some("Active"));
    let scales = info(&d, "Scales");
    assert_eq!(scales.value, "Deployment web");
    let l = scales.link.as_ref().unwrap();
    assert_eq!((l.kind.as_str(), l.namespace.as_deref()), ("apps/Deployment", Some("shop")), "Deployment is the default target");
    assert_eq!(row(&d, "Range"), Some("0 to 10 replicas"));
    assert_eq!(row(&d, "When idle"), Some("0 replicas after 120s without activity"));
    assert_eq!(row(&d, "Polling"), Some("every 30s"));
    assert_eq!(row(&d, "Fallback"), Some("2 replicas after 3 failed reads in a row"));
    assert_eq!(row(&d, "When deleted"), Some("puts back the 3 replicas it had before"));
    assert!(info(&d, "Last active").time);
    assert_eq!(row(&d, "Paused"), None);

    let t = table(&d, "keda-triggers");
    assert_eq!(t.columns, ["Trigger", "Name", "Health", "Authentication"], "no HPA yet: Now and Target are left out");
    assert_eq!(
        cells(t),
        vec![vec!["prometheus", "rps", "failing (4 in a row)", "TriggerAuthentication/prom-auth"], vec!["cpu", "", "", ""]]
    );
    assert!(t.rows[0].warn && !t.rows[1].warn);
    let auth = t.rows[0].link.as_ref().unwrap();
    assert_eq!((auth.kind.as_str(), auth.namespace.as_deref(), auth.name.as_str()), ("keda.sh/TriggerAuthentication", Some("shop"), "prom-auth"));

    // Each trigger's settings follow the table, a secret-looking one without its value.
    let ids: Vec<&str> = d.sections.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["keda-triggers", "keda-trigger-0", "keda-trigger-1"]);
    let prom = section(&d, "keda-trigger-0");
    assert_eq!((prom.title.as_str(), prom.summary.as_str()), ("Trigger: prometheus (rps)", "4 settings"));
    let pairs: Vec<(&str, &str)> = prom.rows.iter().map(|r| (r.key.as_str(), r.value.as_str())).collect();
    assert_eq!(
        pairs,
        [("password", "(not shown)"), ("passwordFromEnv", "PROM_PW"), ("serverAddress", "http://prom:9090"), ("threshold", "50")]
    );
    assert!(!prom.rows[0].mono && prom.rows[1].mono);
    let cpu = section(&d, "keda-trigger-1");
    assert_eq!(cpu.title, "Trigger: cpu");
    assert_eq!(cpu.rows.iter().map(|r| r.key.as_str()).collect::<Vec<_>>(), ["value", "metricType"]);
}

#[test]
fn scaled_object_options() {
    let v = object("keda.sh/v1alpha1", "ScaledObject", Some("shop"), "q", json!({
        "metadata": {"annotations": {"autoscaling.keda.sh/paused-replicas": "1"}},
        "spec": {
            "scaleTargetRef": {"apiVersion": "argoproj.io/v1alpha1", "kind": "Rollout", "name": "q"},
            "minReplicaCount": 2, "idleReplicaCount": 0, "initialCooldownPeriod": 60, "pollingInterval": 15,
            "fallback": {"failureThreshold": 2, "replicas": 1, "behavior": "currentReplicasIfHigher"},
            "advanced": {
                "scalingModifiers": {"formula": "a + b", "target": "10", "activationTarget": "1"},
                "horizontalPodAutoscalerConfig": {"behavior": {"scaleDown": {}}}
            },
            "triggers": [{"type": "cron", "authenticationRef": {"name": "global", "kind": "ClusterTriggerAuthentication"}}]
        }
    }));
    let d = describe_object("keda.sh/ScaledObject", &v, now());
    assert_eq!(d.status.as_deref(), Some("Paused"), "the annotation pauses it before KEDA says so");
    assert_eq!(info(&d, "Scales").link.as_ref().unwrap().kind, "argoproj.io/Rollout");
    assert_eq!(row(&d, "When idle"), Some("0 replicas after 300s without activity (not in its first 60s)"));
    assert_eq!(row(&d, "Polling"), Some("every 15s"));
    assert_eq!(row(&d, "Paused"), Some("at 1 replicas (autoscaling.keda.sh/paused-replicas)"));
    assert_eq!(row(&d, "Fallback"), Some("1 replica after 2 failed reads in a row (currentReplicasIfHigher)"));
    assert_eq!(row(&d, "Formula"), Some("a + b · target 10 · active above 1"));
    assert_eq!(row(&d, "HPA behavior"), Some("custom (see the HPA)"));
    assert_eq!(row(&d, "When deleted"), Some("leaves the replicas as they are"));
    let t = table(&d, "keda-triggers");
    assert_eq!(t.columns, ["Trigger", "Authentication"]);
    assert_eq!(t.rows[0].link.as_ref().unwrap().namespace, None, "a cluster authentication has no namespace");

    // Above zero with no idle count, KEDA never scales it down itself.
    let busy = object("keda.sh/v1alpha1", "ScaledObject", Some("shop"), "b", json!({"spec": {"minReplicaCount": 1}}));
    let d = describe_object("keda.sh/ScaledObject", &busy, now());
    assert_eq!(row(&d, "When idle"), None);
    assert_eq!(row(&d, "Range"), Some("1 to 100 replicas"));
    assert_eq!(section(&d, "keda-triggers").empty.as_deref(), Some("No triggers: it won't scale."));
}

#[test]
fn scaling_status_from_conditions_and_annotations() {
    let with = |c: &[(&str, &str)]| object("keda.sh/v1alpha1", "ScaledObject", Some("shop"), "x", json!({"status": {"conditions": conditions(c)}}));
    assert_eq!(scaling_status(&with(&[("Ready", "True"), ("Active", "False")])), "Idle");
    assert_eq!(scaling_status(&with(&[("Ready", "False"), ("Active", "True")])), "Not ready");
    assert_eq!(scaling_status(&with(&[("Ready", "True"), ("Active", "True"), ("Fallback", "True")])), "Fallback");
    assert_eq!(scaling_status(&with(&[("Ready", "True"), ("Paused", "True")])), "Paused");
    assert_eq!(scaling_status(&with(&[])), "Pending");
    let mut v = with(&[("Ready", "True"), ("Active", "True")]);
    v["metadata"]["annotations"] = json!({"autoscaling.keda.sh/paused": "true"});
    assert_eq!(scaling_status(&v), "Paused");
    v["metadata"]["annotations"] = json!({"autoscaling.keda.sh/paused": "false"});
    assert_eq!(scaling_status(&v), "Active");
}

#[test]
fn secret_looking_settings_are_hidden() {
    for k in ["password", "connectionString", "apiKey", "awsSecretAccessKey", "token", "sasToken", "credentials"] {
        assert!(is_sensitive(k), "{k}");
    }
    for k in ["passwordFromEnv", "connectionFromEnv", "host", "queueName", "threshold", "sasl", "serverAddress"] {
        assert!(!is_sensitive(k), "{k}");
    }
}

#[test]
fn passwords_are_cut_out_of_addresses() {
    assert_eq!(without_password("amqp://guest:pw@mq:5672/vhost"), "amqp://guest:(not shown)@mq:5672/vhost");
    assert_eq!(without_password("redis://:pw@redis"), "redis://:(not shown)@redis");
    assert_eq!(without_password("https://user@host/a@b"), "https://user@host/a@b", "no password, and @ in the path");
    assert_eq!(without_password("http://prom:9090"), "http://prom:9090");
    assert_eq!(without_password("a@b:c"), "a@b:c");
}

#[tokio::test]
async fn scaled_object_details_read_the_hpa_and_controller() {
    let api = MockApi::new();
    api.on("GET", "/apis/keda.sh/v1alpha1/namespaces/shop/scaledobjects/web", web());
    api.on("GET", HPA, web_hpa());
    api.on("GET", DEPLOYMENTS, controller_deployment());
    api.on("GET", "/api/v1/namespaces/shop/events", list("EventList", vec![json!({
        "metadata": {"name": "e1", "namespace": "shop", "creationTimestamp": "2026-10-27T11:00:00Z"},
        "involvedObject": {"kind": "ScaledObject", "name": "web", "namespace": "shop"},
        "type": "Warning", "reason": "KEDAScalerFailed", "message": "connection refused", "count": 4,
        "lastTimestamp": "2026-10-27T11:59:00Z"
    })]));
    let d = object_details(&api.client(), &so_kind(), Some("shop"), "web").await.unwrap();
    assert_eq!(&keys(&d)[..4], ["Scales", "Replicas", "HPA", "Range"], "the HPA's rows follow what it scales");
    assert_eq!(row(&d, "Replicas"), Some("3 now · 4 wanted"));
    let hpa = info(&d, "HPA");
    assert_eq!(hpa.value, "keda-hpa-web");
    assert_eq!(hpa.link.as_ref().unwrap().kind, "autoscaling/HorizontalPodAutoscaler");
    let t = table(&d, "keda-triggers");
    assert_eq!(t.columns, ["Trigger", "Name", "Now", "Target", "Health", "Authentication"]);
    assert!(d.sections.iter().any(|s| s.id == "keda-trigger-0"), "the settings sections stay after the table is redone");
    assert_eq!(&cells(t)[0][..5], ["prometheus", "rps", "61 average", "50 average", "failing (4 in a row)"]);
    assert_eq!(&cells(t)[1][..4], ["cpu", "", "55% (110m)", "70% of request"]);
    assert_eq!(row(&d, "Controller"), Some("keda/keda-operator"));
    let logs = d.logs.as_ref().unwrap();
    assert_eq!((logs.namespace.as_str(), logs.filter.as_deref()), ("keda", Some("web")));
    assert!(matches!(&logs.source, LogSource::Workload { name, .. } if name == "keda-operator"));
    assert!(api.calls_to("GET", DEPLOYMENTS)[0].query.contains("labelSelector=app.kubernetes.io%2Fname%3Dkeda-operator"));
    assert_eq!(d.events.len(), 1);
}

#[tokio::test]
async fn hpa_missing_paused_or_unreadable() {
    let api = MockApi::new();
    let mut v = web();
    v["metadata"]["annotations"] = json!({"autoscaling.keda.sh/paused": "true"});
    v["status"]["hpaName"] = Value::Null;
    v["status"]["compositeScalerName"] = json!("composite-metric");
    let mut d = describe_object("keda.sh/ScaledObject", &v, now());
    related(&api.client(), &so_kind(), "shop", "web", &v, &mut d).await.unwrap();
    assert_eq!(row(&d, "HPA"), Some("keda-hpa-web (none while paused)"), "the default name, when status hasn't one");
    assert_eq!(row(&d, "Replicas"), None);
    assert_eq!(row(&d, "Controller"), None);

    let mut d = describe_object("keda.sh/ScaledObject", &web(), now());
    related(&api.client(), &so_kind(), "shop", "web", &web(), &mut d).await.unwrap();
    assert_eq!(row(&d, "HPA"), Some("keda-hpa-web (missing)"));

    api.on_error("GET", HPA, 403, "Forbidden", "horizontalpodautoscalers is forbidden");
    let mut d = describe_object("keda.sh/ScaledObject", &web(), now());
    related(&api.client(), &so_kind(), "shop", "web", &web(), &mut d).await.unwrap();
    assert!(row(&d, "Note").unwrap().starts_with("couldn't read the HPA"));
    assert_eq!(row(&d, "HPA"), None);
}

#[tokio::test]
async fn a_formula_shows_its_value_now() {
    let api = MockApi::new();
    let mut v = web();
    v["spec"]["advanced"]["scalingModifiers"] = json!({"formula": "s0 / 2", "target": "5"});
    v["status"]["compositeScalerName"] = json!("composite-metric");
    v["status"]["externalMetricNames"] = json!([]);
    let mut hpa = web_hpa();
    hpa["spec"]["metrics"] = json!([{"type": "External", "external": {"metric": {"name": "composite-metric"}, "target": {"type": "AverageValue", "averageValue": "5"}}}]);
    hpa["status"]["currentMetrics"] = json!([{"type": "External", "external": {"metric": {"name": "composite-metric"}, "current": {"averageValue": "7"}}}]);
    api.on("GET", HPA, hpa);
    let mut d = describe_object("keda.sh/ScaledObject", &v, now());
    related(&api.client(), &so_kind(), "shop", "web", &v, &mut d).await.unwrap();
    assert_eq!(row(&d, "Formula"), Some("s0 / 2 · target 5 · now 7 average"));
    assert!(!table(&d, "keda-triggers").columns.contains(&"Now".to_owned()), "a formula's triggers have no metric of their own");
}

#[test]
fn the_hpa_links_back_to_its_scaled_object() {
    let d = describe_object("autoscaling/HorizontalPodAutoscaler", &web_hpa(), now());
    let owner = info(&d, "Controlled by");
    assert_eq!(owner.value, "ScaledObject/web");
    let l = owner.link.as_ref().unwrap();
    assert_eq!((l.kind.as_str(), l.namespace.as_deref()), ("keda.sh/ScaledObject", Some("shop")));
}

// ---- ScaledJob ----

fn worker() -> Value {
    object("keda.sh/v1alpha1", "ScaledJob", Some("shop"), "worker", json!({
        "spec": {
            "jobTargetRef": {"parallelism": 1, "backoffLimit": 2, "activeDeadlineSeconds": 600,
                             "template": {"spec": {"containers": [{"name": "w", "image": "acme/worker:1"}]}}},
            "maxReplicaCount": 20,
            "successfulJobsHistoryLimit": 5,
            "rollout": {"strategy": "gradual"},
            "scalingStrategy": {"strategy": "custom", "customScalingQueueLengthDeduction": 1, "customScalingRunningJobPercentage": "0.5",
                                "multipleScalersCalculation": "max", "pendingPodConditions": ["Ready"]},
            "triggers": [{"type": "rabbitmq", "metadata": {"queueName": "jobs", "host": "amqp://u:p@mq"}}, {"type": "cron"}]
        },
        "status": {"conditions": conditions(&[("Ready", "True"), ("Active", "False")]), "lastActiveTime": "2026-10-27T10:00:00Z"}
    }))
}

fn job(name: &str, created: &str, status: Value) -> Value {
    let mut v = object("batch/v1", "Job", Some("shop"), name, json!({"spec": {"template": {"spec": {"containers": []}}}, "status": status}));
    v["metadata"]["creationTimestamp"] = json!(created);
    v
}

#[tokio::test]
async fn scaled_job_details_and_its_jobs() {
    let d = describe_object("keda.sh/ScaledJob", &worker(), now());
    assert_eq!(d.status.as_deref(), Some("Idle"));
    assert_eq!(row(&d, "Runs"), Some("w (acme/worker:1)"));
    assert_eq!(row(&d, "Each job"), Some("parallelism 1 · retries 2 · deadline 600s"));
    assert_eq!(row(&d, "Jobs at once"), Some("0 to 20"));
    assert_eq!(row(&d, "Strategy"), Some("custom: waiting items less 1 and 0.5% of running jobs"));
    assert_eq!(row(&d, "Pending until"), Some("Ready"));
    assert_eq!(row(&d, "Several triggers"), Some("max of them"));
    assert!(row(&d, "On change").unwrap().starts_with("gradual"));
    assert_eq!(row(&d, "Keeps"), Some("5 succeeded · 100 failed jobs"));
    assert_eq!(cells(table(&d, "keda-triggers")), vec![vec!["rabbitmq"], vec!["cron"]]);
    assert_eq!(section(&d, "keda-trigger-0").rows[0].value, "amqp://u:(not shown)@mq");
    assert!(!d.sections.iter().any(|s| s.id == "keda-trigger-1"), "no settings, no section");

    let api = MockApi::new();
    api.on("GET", "/apis/batch/v1/namespaces/shop/jobs", list("JobList", vec![
        job("worker-old", "2026-10-27T09:00:00Z", json!({"succeeded": 1, "startTime": "2026-10-27T09:00:00Z"})),
        job("worker-new", "2026-10-27T11:00:00Z", json!({"active": 1, "startTime": "2026-10-27T11:00:00Z"})),
        job("worker-bad", "2026-10-27T10:00:00Z", json!({"failed": 3, "conditions": [{"type": "Failed", "status": "True"}]})),
    ]));
    let mut d = describe_object("keda.sh/ScaledJob", &worker(), now());
    let sj = kind("keda.sh", "v1alpha1", "ScaledJob", "scaledjobs", true);
    related(&api.client(), &sj, "shop", "worker", &worker(), &mut d).await.unwrap();
    let jobs = section(&d, "scaledjob-jobs");
    assert_eq!(jobs.summary, "3 · 1 running");
    let t = jobs.table.as_ref().unwrap();
    assert_eq!(
        cells(t),
        vec![
            vec!["worker-new", "Running", "2026-10-27T11:00:00Z"],
            vec!["worker-bad", "Failed", ""],
            vec!["worker-old", "Succeeded", "2026-10-27T09:00:00Z"],
        ]
    );
    assert!(t.rows[1].warn);
    assert_eq!(t.rows[0].link.as_ref().unwrap().kind, "batch/Job");
    let q = &api.calls_to("GET", "/apis/batch/v1/namespaces/shop/jobs")[0].query;
    assert!(q.contains("labelSelector=scaledjob.keda.sh%2Fname%3Dworker"), "{q}");

    let api = MockApi::new();
    api.on_error("GET", "/apis/batch/v1/namespaces/shop/jobs", 403, "Forbidden", "jobs is forbidden");
    let mut d = describe_object("keda.sh/ScaledJob", &worker(), now());
    related(&api.client(), &sj, "shop", "worker", &worker(), &mut d).await.unwrap();
    assert!(row(&d, "Note").unwrap().starts_with("couldn't list Jobs"));
}

// ---- TriggerAuthentication ----

fn auth(kind_: &str, namespace: Option<&str>) -> Value {
    object("keda.sh/v1alpha1", kind_, namespace, "prom-auth", json!({
        "spec": {
            "podIdentity": {"provider": "aws", "roleArn": "arn:aws:iam::1:role/keda", "identityOwner": "workload"},
            "secretTargetRef": [{"parameter": "password", "name": "prom-creds", "key": "pw"}],
            "configMapTargetRef": [{"parameter": "host", "name": "prom-cfg", "key": "host"}],
            "env": [{"parameter": "user", "name": "PROM_USER", "containerName": "app"}],
            "boundServiceAccountToken": [{"parameter": "bearer", "serviceAccountName": "prom-reader"}],
            "hashiCorpVault": {"address": "https://vault:8200", "authentication": "token", "credential": {"token": "s.secret"},
                               "secrets": [{"parameter": "ca", "path": "secret/data/prom", "key": "ca"}]},
            "azureKeyVault": {"vaultUri": "https://kv.vault.azure.net", "credentials": {"clientId": "cid", "tenantId": "tid",
                              "clientSecret": {"valueFrom": {"secretKeyRef": {"name": "azure-sp", "key": "secret"}}}},
                              "secrets": [{"parameter": "conn", "name": "eh-conn"}]}
        }
    }))
}

#[test]
fn trigger_authentication_says_where_each_parameter_comes_from() {
    let d = describe_object("keda.sh/TriggerAuthentication", &auth("TriggerAuthentication", Some("shop")), now());
    assert_eq!(d.status, None);
    assert_eq!(row(&d, "Pod identity"), Some("AWS (EKS Pod Identity or IRSA) · the scaled workload's"));
    assert_eq!(row(&d, "Role ARN"), Some("arn:aws:iam::1:role/keda"));
    assert_eq!(row(&d, "Vault"), Some("https://vault:8200"));
    assert_eq!(row(&d, "Vault auth"), Some("token"));
    assert_eq!(row(&d, "Vault token"), Some("written in this object (not shown)"));
    assert!(!d.info.iter().any(|r| r.value.contains("s.secret")), "the token itself never goes out");
    assert_eq!(row(&d, "Client ID"), Some("cid"));
    let secret = info(&d, "Client secret");
    assert_eq!(secret.value, "Secret azure-sp · key secret");
    assert_eq!(secret.link.as_ref().unwrap().namespace.as_deref(), Some("shop"));
    let t = table(&d, "keda-auth-params");
    assert_eq!(
        cells(t),
        vec![
            vec!["password", "Secret", "prom-creds", "pw"],
            vec!["host", "ConfigMap", "prom-cfg", "host"],
            vec!["user", "Env var", "PROM_USER", "container app"],
            vec!["bearer", "ServiceAccount token", "prom-reader", ""],
            vec!["ca", "Vault", "secret/data/prom", "ca"],
            vec!["conn", "Azure Key Vault", "eh-conn", ""],
        ]
    );
    let links: Vec<Option<&str>> = t.rows.iter().map(|r| r.link.as_ref().map(|l| l.kind.as_str())).collect();
    assert_eq!(links, [Some("core/Secret"), Some("core/ConfigMap"), None, Some("core/ServiceAccount"), None, None]);
}

#[tokio::test]
async fn trigger_authentication_used_by() {
    let api = MockApi::new();
    let mut other = web();
    other["metadata"]["name"] = json!("other");
    other["spec"]["triggers"] = json!([{"type": "cpu", "authenticationRef": {"name": "prom-auth", "kind": "ClusterTriggerAuthentication"}}]);
    let mut failing = web();
    failing["metadata"]["name"] = json!("api");
    failing["status"]["conditions"] = conditions(&[("Ready", "False")]);
    api.on("GET", SHOP_SO, list("ScaledObjectList", vec![web(), other, failing]));
    api.on_error("GET", SHOP_SJ, 403, "Forbidden", "scaledjobs is forbidden");
    let ta = kind("keda.sh", "v1alpha1", "TriggerAuthentication", "triggerauthentications", true);
    let v = auth("TriggerAuthentication", Some("shop"));
    let mut d = describe_object("keda.sh/TriggerAuthentication", &v, now());
    related(&api.client(), &ta, "shop", "prom-auth", &v, &mut d).await.unwrap();
    let t = table(&d, "keda-auth-users");
    assert_eq!(cells(t), vec![vec!["shop", "ScaledObject", "api", "Not ready"], vec!["shop", "ScaledObject", "web", "Active"]], "not the one naming a cluster auth");
    assert!(t.rows[0].warn);
    assert_eq!(t.rows[1].link.as_ref().unwrap().kind, "keda.sh/ScaledObject");
    assert!(row(&d, "Note").unwrap().contains("couldn't list ScaledJobs"));
}

#[tokio::test]
async fn cluster_trigger_authentication_reads_from_kedas_namespace() {
    let api = MockApi::new();
    let mut user = web();
    user["spec"]["triggers"] = json!([{"type": "kafka", "authenticationRef": {"name": "prom-auth", "kind": "ClusterTriggerAuthentication"}}]);
    api.on("GET", "/apis/keda.sh/v1alpha1/scaledobjects", list("ScaledObjectList", vec![user]));
    api.on("GET", "/apis/keda.sh/v1alpha1/scaledjobs", list("ScaledJobList", vec![]));
    api.on("GET", DEPLOYMENTS, controller_deployment());
    api.on("GET", "/api/v1/events", list("EventList", vec![]));
    let cta = kind("keda.sh", "v1alpha1", "ClusterTriggerAuthentication", "clustertriggerauthentications", false);
    let v = auth("ClusterTriggerAuthentication", None);
    let mut d = describe_object("keda.sh/ClusterTriggerAuthentication", &v, now());
    related(&api.client(), &cta, "", "prom-auth", &v, &mut d).await.unwrap();
    assert_eq!(row(&d, "Secrets from"), Some("keda (KEDA's namespace)"));
    let params = table(&d, "keda-auth-params");
    assert_eq!(params.rows[0].link.as_ref().unwrap().namespace.as_deref(), Some("keda"));
    assert_eq!(info(&d, "Client secret").link.as_ref().unwrap().namespace.as_deref(), Some("keda"));
    assert_eq!(cells(table(&d, "keda-auth-users")), vec![vec!["shop", "ScaledObject", "web", "Active"]]);
    assert_eq!(api.calls_to("GET", "/api/v1/events").len(), 1, "a cluster kind's events come from any namespace");

    // Without the controller the namespace isn't known: no links rather than wrong ones.
    let api = MockApi::new();
    let mut d = describe_object("keda.sh/ClusterTriggerAuthentication", &v, now());
    related(&api.client(), &cta, "", "prom-auth", &v, &mut d).await.unwrap();
    assert!(table(&d, "keda-auth-params").rows[0].link.is_none());
    assert!(info(&d, "Client secret").link.is_none());
    assert_eq!(row(&d, "Secrets from"), None);
}

#[test]
fn identities_and_other_providers() {
    let v = object("keda.sh/v1alpha1", "TriggerAuthentication", Some("shop"), "a", json!({
        "spec": {
            "podIdentity": {"provider": "none"},
            "awsSecretManager": {"region": "eu-west-1", "podIdentity": {"provider": "aws-eks"},
                                 "credentials": {"accessKey": {"valueFrom": {"secretKeyRef": {"name": "aws", "key": "id"}}}},
                                 "secrets": [{"parameter": "p", "name": "prod/db", "versionStage": "AWSCURRENT"}]},
            "gcpSecretManager": {"podIdentity": {"provider": "gcp"}, "secrets": [{"parameter": "g", "id": "db", "version": "3"}]},
            "hashiCorpVault": {"address": "https://v", "namespace": "team", "authentication": "kubernetes", "role": "keda", "mount": "k8s",
                               "credential": {"serviceAccount": "/var/run/secrets/token"}}
        }
    }));
    let d = describe_object("keda.sh/TriggerAuthentication", &v, now());
    assert_eq!(row(&d, "Pod identity"), None, "provider none is no identity");
    assert_eq!(row(&d, "Secrets Manager identity"), Some("AWS IRSA (deprecated)"));
    assert_eq!(row(&d, "Secret Manager identity"), Some("Google Workload Identity"));
    assert_eq!(row(&d, "AWS region"), Some("eu-west-1"));
    assert_eq!(row(&d, "Access key"), Some("Secret aws · key id"));
    assert_eq!(row(&d, "Vault"), Some("https://v · namespace team"));
    assert_eq!(row(&d, "Vault auth"), Some("kubernetes · role keda · mount k8s"));
    assert_eq!(row(&d, "Vault token"), None);
    assert_eq!(row(&d, "Vault token file"), Some("/var/run/secrets/token"));
    assert_eq!(
        cells(table(&d, "keda-auth-params")),
        vec![vec!["p", "AWS Secrets Manager", "prod/db", "AWSCURRENT"], vec!["g", "Google Secret Manager", "db", "3"]]
    );

    let empty = object("keda.sh/v1alpha1", "TriggerAuthentication", Some("shop"), "e", json!({"spec": {"podIdentity": {"provider": "azure-workload", "identityId": "id-1"}}}));
    let d = describe_object("keda.sh/TriggerAuthentication", &empty, now());
    assert_eq!(row(&d, "Pod identity"), Some("Azure Workload Identity"));
    assert_eq!(row(&d, "Identity ID"), Some("id-1"));
    assert_eq!(section(&d, "keda-auth-params").empty.as_deref(), Some("No parameters: only an identity."));
}

// ---- CloudEventSource and the rest ----

#[test]
fn cloud_event_sources() {
    let v = object("eventing.keda.sh/v1alpha1", "CloudEventSource", Some("shop"), "ce", json!({
        "spec": {"clusterName": "uat", "destination": {"http": {"uri": "http://sink"}},
                 "authenticationRef": {"name": "a"},
                 "eventSubscription": {"excludedEventTypes": ["keda.scaledobject.ready.v1"]}},
        "status": {"conditions": conditions(&[("Active", "True")])}
    }));
    let d = describe_object("eventing.keda.sh/CloudEventSource", &v, now());
    assert_eq!(d.status.as_deref(), Some("Active"));
    assert_eq!(row(&d, "Sends to"), Some("http://sink"));
    assert_eq!(row(&d, "Cluster name"), Some("uat"));
    assert_eq!(info(&d, "Authentication").link.as_ref().unwrap().kind, "keda.sh/TriggerAuthentication");
    assert_eq!(row(&d, "Events"), Some("all but keda.scaledobject.ready.v1"));

    let v = object("eventing.keda.sh/v1alpha1", "ClusterCloudEventSource", None, "cce", json!({
        "spec": {"destination": {"azureEventGridTopic": {"endpoint": "https://topic"}},
                 "authenticationRef": {"name": "g", "kind": "ClusterTriggerAuthentication"},
                 "eventSubscription": {"includedEventTypes": ["a", "b"]}}
    }));
    let d = describe_object("eventing.keda.sh/ClusterCloudEventSource", &v, now());
    assert_eq!(d.status.as_deref(), Some("Pending"));
    assert_eq!(row(&d, "Sends to"), Some("Azure Event Grid https://topic"));
    assert_eq!(info(&d, "Authentication").link.as_ref().unwrap().namespace, None);
    assert_eq!(row(&d, "Events"), Some("only a, b"));

    let unknown = object("keda.sh/v1alpha1", "SomethingNew", Some("shop"), "n", json!({"status": {"conditions": conditions(&[("Ready", "False")])}}));
    assert_eq!(describe_object("keda.sh/SomethingNew", &unknown, now()).status.as_deref(), Some("Not ready"));
}

#[test]
fn dropped_columns_keep_numeric_and_time_columns_in_place() {
    let t = InfoTable {
        columns: ["a", "b", "c", "d"].map(String::from).to_vec(),
        numeric: vec![2],
        times: vec![3],
        mono: vec![0],
        rows: vec![InfoTableRow::new(vec!["x".into(), String::new(), "1".into(), "t".into()])],
    };
    let t = drop_empty_columns(t, &[1]);
    assert_eq!(t.columns, ["a", "c", "d"]);
    assert_eq!((t.mono, t.numeric, t.times), (vec![0], vec![1], vec![2]));
}

// ---- Pause/Resume and the replica range ----

const WEB: &str = "/apis/keda.sh/v1alpha1/namespaces/shop/scaledobjects/web";

fn unlocked() -> SafetyGate {
    let gate = SafetyGate::default();
    gate.set_unlocked("acme-uat", true);
    gate
}

#[tokio::test]
async fn pause_and_resume_set_kedas_annotations() {
    let api = MockApi::new();
    api.on("PATCH", WEB, web());
    let (c, gate) = (api.client(), unlocked());
    let annotations = |api: &MockApi| api.calls_to("PATCH", WEB).pop().unwrap().json()["metadata"]["annotations"].clone();

    set_paused(&c, &gate, "acme-uat", &so_kind(), "shop", "web", true, None, false).await.unwrap();
    assert_eq!(annotations(&api), json!({"autoscaling.keda.sh/paused": "true", "autoscaling.keda.sh/paused-replicas": null}));
    set_paused(&c, &gate, "acme-uat", &so_kind(), "shop", "web", true, Some(0), false).await.unwrap();
    assert_eq!(annotations(&api), json!({"autoscaling.keda.sh/paused-replicas": "0", "autoscaling.keda.sh/paused": null}));
    set_paused(&c, &gate, "acme-uat", &so_kind(), "shop", "web", false, None, false).await.unwrap();
    assert_eq!(annotations(&api), json!({"autoscaling.keda.sh/paused": null, "autoscaling.keda.sh/paused-replicas": null}), "resume removes both");
    assert!(!api.calls_to("PATCH", WEB)[0].query.contains("dryRun"));

    let sj = kind("keda.sh", "v1alpha1", "ScaledJob", "scaledjobs", true);
    api.on("PATCH", "/apis/keda.sh/v1alpha1/namespaces/shop/scaledjobs/worker", worker());
    set_paused(&c, &gate, "acme-uat", &sj, "shop", "worker", true, None, false).await.unwrap();
    let err = set_paused(&c, &gate, "acme-uat", &sj, "shop", "worker", true, Some(1), false).await.unwrap_err();
    assert!(err.to_string().contains("only a ScaledObject"), "{err}");
    let err = set_paused(&c, &gate, "acme-uat", &so_kind(), "shop", "web", true, Some(-1), false).await.unwrap_err();
    assert!(err.to_string().contains("negative"), "{err}");
    let ta = kind("keda.sh", "v1alpha1", "TriggerAuthentication", "triggerauthentications", true);
    assert!(set_paused(&c, &gate, "acme-uat", &ta, "shop", "a", true, None, false).await.unwrap_err().to_string().contains("can't be paused"));
}

#[tokio::test]
async fn pause_and_range_are_gated_except_for_dry_runs() {
    let api = MockApi::new();
    api.on("PATCH", WEB, web());
    api.on("GET", WEB, web());
    let locked = SafetyGate::default();
    let err = set_paused(&api.client(), &locked, "acme-uat", &so_kind(), "shop", "web", true, None, false).await.unwrap_err();
    assert!(matches!(err, Error::Locked(_)));
    let err = set_replica_range(&api.client(), &locked, "acme-uat", &so_kind(), "shop", "web", 1, 5, false).await.unwrap_err();
    assert!(matches!(err, Error::Locked(_)));
    assert!(api.calls_to("PATCH", WEB).is_empty());
    set_paused(&api.client(), &locked, "acme-uat", &so_kind(), "shop", "web", true, None, true).await.unwrap();
    set_replica_range(&api.client(), &locked, "acme-uat", &so_kind(), "shop", "web", 1, 5, true).await.unwrap();
    assert!(api.calls_to("PATCH", WEB).iter().all(|c| c.query.contains("dryRun=All")));
}

#[tokio::test]
async fn replica_range_patches_min_and_max() {
    let api = MockApi::new();
    api.on("GET", WEB, web());
    api.on("PATCH", WEB, web());
    let (c, gate) = (api.client(), unlocked());
    set_replica_range(&c, &gate, "acme-uat", &so_kind(), "shop", "web", 0, 20, false).await.unwrap();
    assert_eq!(api.calls_to("PATCH", WEB)[0].json(), json!({"spec": {"minReplicaCount": 0, "maxReplicaCount": 20}}));

    for (min, max) in [(-1, 5), (3, 2), (0, 0)] {
        let err = set_replica_range(&c, &gate, "acme-uat", &so_kind(), "shop", "web", min, max, false).await.unwrap_err();
        assert!(err.to_string().contains("0 ≤ min ≤ max"), "{min}-{max}: {err}");
    }

    // KEDA needs min above the idle count.
    let mut idle = web();
    idle["spec"]["idleReplicaCount"] = json!(0);
    api.on("GET", WEB, idle);
    let err = set_replica_range(&c, &gate, "acme-uat", &so_kind(), "shop", "web", 0, 5, false).await.unwrap_err();
    assert!(err.to_string().contains("above its idle replica count (0)"), "{err}");
    set_replica_range(&c, &gate, "acme-uat", &so_kind(), "shop", "web", 1, 5, false).await.unwrap();

    // A ScaledJob has no idle count, so it isn't read first.
    let sj = kind("keda.sh", "v1alpha1", "ScaledJob", "scaledjobs", true);
    let path = "/apis/keda.sh/v1alpha1/namespaces/shop/scaledjobs/worker";
    api.on("PATCH", path, worker());
    set_replica_range(&c, &gate, "acme-uat", &sj, "shop", "worker", 0, 50, false).await.unwrap();
    assert!(api.calls_to("GET", path).is_empty());
    assert_eq!(api.calls_to("PATCH", path)[0].json()["spec"]["maxReplicaCount"], 50);
}
