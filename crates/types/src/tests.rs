//! Tests for the shared types: kind keys and predicates, sidebar categories,
//! the container cell encoding, window titles and the info-row builders.

use super::*;
use serde_json::json;

fn kind(group: &str, kind: &str) -> KindRef {
    KindRef {
        group: group.into(),
        version: "v1".into(),
        kind: kind.into(),
        plural: format!("{}s", kind.to_lowercase()),
        namespaced: true,
    }
}

#[test]
fn kind_keys_and_api_versions() {
    let pod = kind("", "Pod");
    assert_eq!(pod.key(), "core/Pod");
    assert_eq!(pod.api_version(), "v1");
    let deploy = kind("apps", "Deployment");
    assert_eq!(deploy.key(), "apps/Deployment");
    assert_eq!(deploy.api_version(), "apps/v1");
}

#[test]
fn categories_have_labels_in_sidebar_order() {
    let labels: Vec<&str> = Category::SECTIONS.iter().map(|c| c.label()).collect();
    assert_eq!(
        labels,
        [
            "Workloads", "Config", "Network", "Storage", "Namespaces", "Access Control", "Helm", "AWS",
            "Google Cloud", "Azure", "Custom Resources", "Other"
        ]
    );
    // Top level isn't a section, but has a label too.
    assert_eq!(Category::TopLevel.label(), "Top level");
    assert!(Category::CustomResources.groups_by_api_group());
    assert!(Category::Aws.groups_by_api_group());
    assert!(Category::GoogleCloud.groups_by_api_group());
    assert!(Category::Azure.groups_by_api_group());
    assert!(!Category::Workloads.groups_by_api_group());
}

#[test]
fn cloud_groups_match_whole_domain_labels() {
    assert_eq!(Category::cloud_for_group("elbv2.k8s.aws"), Some(Category::Aws));
    assert_eq!(Category::cloud_for_group("k8s.aws"), Some(Category::Aws));
    assert_eq!(Category::cloud_for_group("vpcresources.k8s.amazonaws.com"), Some(Category::Aws));
    assert_eq!(Category::cloud_for_group("networking.gke.io"), Some(Category::GoogleCloud));
    assert_eq!(Category::cloud_for_group("iam.cnrm.cloud.google.com"), Some(Category::GoogleCloud));
    assert_eq!(Category::cloud_for_group("monitoring.googleapis.com"), Some(Category::GoogleCloud));
    assert_eq!(Category::cloud_for_group("containerservice.azure.com"), Some(Category::Azure));
    assert_eq!(Category::cloud_for_group("azure.microsoft.com"), Some(Category::Azure));
    assert_eq!(Category::cloud_for_group("aadpodidentity.k8s.io"), Some(Category::Azure));
    // Not a whole label: someone else's domain.
    assert_eq!(Category::cloud_for_group("myazure.com"), None);
    assert_eq!(Category::cloud_for_group("apps"), None);
}

#[test]
fn every_container_state_round_trips() {
    use containers::{Container, State, decode, encode};
    let all: Vec<Container> = [State::Running, State::Starting, State::Stopped, State::Failed]
        .into_iter()
        .enumerate()
        .map(|(i, state)| Container { name: format!("c{i}"), state, detail: format!("d{i}"), init: i == 0 })
        .collect();
    assert_eq!(decode(&encode(&all)), all);
    // Unknown codes are skipped, missing fields default.
    assert!(decode("X\u{1e}x").is_empty());
    let partial = decode("R\u{1e}app");
    assert_eq!((partial[0].name.as_str(), partial[0].detail.as_str(), partial[0].init), ("app", "", false));
}

#[test]
fn pod_counts_total() {
    let c = PodCounts { running: 3, pending: 1, succeeded: 2, failed: 1, unknown: 1 };
    assert_eq!(c.total(), 8);
    assert_eq!(PodCounts::default().total(), 0);
}

#[test]
fn info_row_and_table_row_builders() {
    let link = ObjectLink { kind: "core/Pod".into(), namespace: Some("shop".into()), name: "api-1".into() };
    let row = InfoRow::new("Started", "2026-10-10T00:00:00Z").mono().time().link(link.clone()).url("https://acme.io");
    assert_eq!((row.key.as_str(), row.value.as_str(), row.mono, row.time), ("Started", "2026-10-10T00:00:00Z", true, true));
    assert_eq!(row.link, Some(link));
    assert_eq!(row.url.as_deref(), Some("https://acme.io"));
    let plain = InfoRow::new("A", "b");
    assert!(!plain.mono && !plain.time && plain.link.is_none() && plain.url.is_none());

    let t = InfoTableRow::new(vec!["x".into()]);
    assert_eq!((t.cells.len(), t.link.is_none(), t.fill, t.warn), (1, true, None, false));
}

#[test]
fn info_rows_and_tables_accept_older_payloads() {
    // Fields added later default when absent.
    let row: InfoRow = serde_json::from_value(json!({"key": "k", "value": "v"})).unwrap();
    assert!(!row.mono && !row.time && row.link.is_none() && row.url.is_none());
    let table: InfoTable = serde_json::from_value(json!({"columns": ["A"], "rows": [{"cells": ["1"]}]})).unwrap();
    assert!(table.numeric.is_empty() && table.mono.is_empty() && table.times.is_empty());
    assert!(!table.rows[0].warn);
}

#[test]
fn kind_predicates() {
    let deploy = kind("apps", "Deployment");
    let sts = kind("apps", "StatefulSet");
    let ds = kind("apps", "DaemonSet");
    let rs = kind("apps", "ReplicaSet");
    let job = kind("batch", "Job");
    let cron = kind("batch", "CronJob");
    let rc = kind("", "ReplicationController");
    let cm = kind("", "ConfigMap");
    let secret = kind("", "Secret");
    let pod = kind("", "Pod");

    for k in [&deploy, &sts] {
        assert!(is_scalable_workload(k) && has_workload_details(k) && has_workload_logs(k));
    }
    for k in [&job, &cron] {
        assert!(is_batch_workload(k) && has_workload_details(k) && !is_scalable_workload(k));
    }
    assert!(has_workload_logs(&job) && !has_workload_logs(&cron));
    assert!(has_workload_logs(&ds) && has_workload_logs(&rs) && has_workload_logs(&rc));
    assert!(!has_workload_details(&ds) && !has_workload_logs(&pod));

    assert!(has_editable_data(&cm) && has_editable_data(&secret) && !has_editable_data(&pod));
    for key in [
        ("", "ConfigMap"),
        ("autoscaling", "HorizontalPodAutoscaler"),
        ("networking.k8s.io", "Ingress"),
        ("storage.k8s.io", "StorageClass"),
        ("rbac.authorization.k8s.io", "ClusterRoleBinding"),
        ("helm.sh", "Release"),
        ("", "Namespace"),
    ] {
        assert!(has_object_details(&kind(key.0, key.1)), "{key:?}");
    }
    assert!(!has_object_details(&pod) && !has_object_details(&deploy));
}

#[test]
fn window_titles_and_labels() {
    let logs = LogTarget {
        namespace: "shop".into(),
        source: LogSource::Container { pod: "api-1".into(), container: "app".into() },
    };
    assert_eq!(logs.title("acme-uat"), "app · shop/api-1 · acme-uat · Logs");
    let merged = LogTarget {
        namespace: "shop".into(),
        source: LogSource::Workload { kind: kind("apps", "Deployment"), name: "api".into() },
    };
    assert_eq!(merged.title("acme-uat"), "Deployment shop/api · acme-uat · Logs");

    let exec = ExecTarget { namespace: "shop".into(), pod: "api-1".into(), container: "app".into() };
    assert_eq!(exec.title("acme-prd"), "app · shop/api-1 · acme-prd · Shell");

    assert_eq!(ForwardSource::Pod { pod: "api-1".into() }.label(), "pod/api-1");
    assert_eq!(ForwardSource::Service { service: "web".into() }.label(), "service/web");
}

#[test]
fn details_payloads_round_trip() {
    let details = WorkloadDetails {
        kind: "Job".into(),
        name: "migrate".into(),
        namespace: "shop".into(),
        status: "Complete".into(),
        replicas: None,
        suspended: Some(false),
        info: vec![("Completions".into(), "1/1".into())],
        created: None,
        labels: vec![],
        conditions: vec![],
        cpu: Usage::default(),
        memory: Usage::default(),
        metrics_available: false,
        pods: vec![],
        containers: vec![],
        jobs: vec![JobRun {
            name: "migrate".into(),
            status: "Complete".into(),
            completions: "1/1".into(),
            manual: true,
            created: None,
            started: None,
            finished: None,
        }],
        events: vec![],
    };
    let back: WorkloadDetails = serde_json::from_value(serde_json::to_value(&details).unwrap()).unwrap();
    assert_eq!(back, details);
    // Older payloads without `suspended`, `jobs` or event `object` still parse.
    let mut v = serde_json::to_value(&details).unwrap();
    v.as_object_mut().unwrap().remove("suspended");
    v.as_object_mut().unwrap().remove("jobs");
    let old: WorkloadDetails = serde_json::from_value(v).unwrap();
    assert!(old.jobs.is_empty() && old.suspended.is_none());
}
