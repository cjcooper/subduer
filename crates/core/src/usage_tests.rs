//! Coverage for the kubelet `/stats/summary` fallback.

use super::*;
use crate::testing::MockApi;
use serde_json::json;

fn summary(pods: Value) -> Value {
    json!({"node": {"nodeName": "n1"}, "pods": pods})
}

fn container(name: &str, nano_cores: f64, bytes: f64) -> Value {
    json!({"name": name, "cpu": {"usageNanoCores": nano_cores}, "memory": {"workingSetBytes": bytes}})
}

#[tokio::test]
async fn kubelet_summary_reads_the_node_proxy() {
    let api = MockApi::new();
    let body = summary(json!([{"podRef": {"name": "api-1", "namespace": "shop"}, "containers": [container("app", 2e6, 10.0)]}]));
    api.on("GET", "/api/v1/nodes/n1/proxy/stats/summary", body.clone());
    let client = api.client();

    assert_eq!(kubelet_summary(&client, "n1").await, Some(body));
    assert_eq!(api.calls_to("GET", "/api/v1/nodes/n1/proxy/stats/summary").len(), 1);
    // A node the proxy can't reach (404 here) gives nothing rather than an error.
    assert_eq!(kubelet_summary(&client, "gone").await, None);
}

#[tokio::test]
async fn kubelet_summary_rejects_non_json() {
    let api = MockApi::new();
    api.on_text("GET", "/api/v1/nodes/n1/proxy/stats/summary", 200, "not json");
    assert_eq!(kubelet_summary(&api.client(), "n1").await, None);
}

#[tokio::test]
async fn node_usage_skips_unreadable_nodes() {
    let api = MockApi::new();
    api.on(
        "GET",
        "/api/v1/nodes/n1/proxy/stats/summary",
        summary(json!([
            {"podRef": {"name": "a", "namespace": "shop"}, "containers": [container("app", 250e6, 1024.0)]},
            {"podRef": {"name": "b", "namespace": "acme"}, "containers": [container("app", 750e6, 1024.0)]}
        ])),
    );
    api.on("GET", "/api/v1/nodes/n2/proxy/stats/summary", summary(json!([])));
    api.on_error("GET", "/api/v1/nodes/n3/proxy/stats/summary", 503, "ServiceUnavailable", "kubelet down");

    let usage = kubelet_node_usage(&api.client(), vec!["n1".into(), "n2".into(), "n3".into()]).await;
    assert_eq!(usage.len(), 2);
    assert_eq!(usage["n1"], (1000.0, 2048.0));
    assert_eq!(usage["n2"], (0.0, 0.0));
    assert!(!usage.contains_key("n3"));
}

#[tokio::test]
async fn node_usage_with_no_nodes_is_empty() {
    let api = MockApi::new();
    assert!(kubelet_node_usage(&api.client(), vec![]).await.is_empty());
    assert!(api.calls().is_empty());
}

#[test]
fn pod_usage_edge_cases() {
    // No pods array at all.
    assert!(pod_usage(&json!({}), "shop", "a").is_none());
    // Matching name in another namespace doesn't count.
    let s = summary(json!([{"podRef": {"name": "a", "namespace": "acme"}, "containers": [container("app", 1e6, 1.0)]}]));
    assert!(pod_usage(&s, "shop", "a").is_none());
    // A pod the kubelet lists without containers (e.g. just started).
    let s = summary(json!([{"podRef": {"name": "a", "namespace": "shop"}}]));
    assert!(pod_usage(&s, "shop", "a").is_none());
    // Missing figures count as zero.
    let s = summary(json!([{"podRef": {"name": "a", "namespace": "shop"}, "containers": [{"name": "app"}]}]));
    assert_eq!(pod_usage(&s, "shop", "a").unwrap()["app"], (0.0, 0.0));
}

#[test]
fn node_total_edge_cases() {
    assert_eq!(node_total(&json!({})), (0.0, 0.0));
    assert_eq!(node_total(&summary(json!([{"podRef": {"name": "a", "namespace": "shop"}}]))), (0.0, 0.0));
    let s = summary(json!([{"podRef": {"name": "a", "namespace": "shop"}, "containers": [container("app", 1.5e6, 7.0)]}]));
    assert_eq!(node_total(&s), (1.5, 7.0));
}
