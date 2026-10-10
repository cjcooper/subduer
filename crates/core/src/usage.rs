//! Live usage from the kubelet's `/stats/summary`, read through the API
//! server's node proxy. Used when metrics-server isn't installed. Figures are
//! per container (CPU in millicores, memory as working set bytes), the same
//! basis as metrics-server and Prometheus' cAdvisor metrics.

use std::collections::HashMap;

use futures::{StreamExt, stream};
use kube::Client;
use serde_json::Value;

use crate::table::text;

/// Container name → (cpu millicores, memory bytes).
pub type ContainerUsage = HashMap<String, (f64, f64)>;

pub(crate) async fn kubelet_summary(client: &Client, node: &str) -> Option<Value> {
    let request = http::Request::get(format!("/api/v1/nodes/{node}/proxy/stats/summary"))
        .body(Vec::new())
        .ok()?;
    client.request(request).await.ok()
}

fn container_usage(c: &Value) -> (f64, f64) {
    (
        c["cpu"]["usageNanoCores"].as_f64().unwrap_or(0.0) / 1e6,
        c["memory"]["workingSetBytes"].as_f64().unwrap_or(0.0),
    )
}

/// One pod's containers from a node summary.
pub(crate) fn pod_usage(summary: &Value, namespace: &str, name: &str) -> Option<ContainerUsage> {
    let pod = summary["pods"]
        .as_array()?
        .iter()
        .find(|p| p["podRef"]["name"] == name && p["podRef"]["namespace"] == namespace)?;
    Some(
        pod["containers"]
            .as_array()?
            .iter()
            .map(|c| (text(&c["name"]), container_usage(c)))
            .collect(),
    )
}

/// Sum of every container on the node.
pub(crate) fn node_total(summary: &Value) -> (f64, f64) {
    summary["pods"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|p| p["containers"].as_array().into_iter().flatten())
        .map(container_usage)
        .fold((0.0, 0.0), |(cpu, mem), (c, m)| (cpu + c, mem + m))
}

/// Node → (cpu, memory) totals for the given nodes, a few requests at a time.
/// Nodes whose stats can't be read are left out.
pub(crate) async fn kubelet_node_usage(client: &Client, nodes: Vec<String>) -> HashMap<String, (f64, f64)> {
    stream::iter(nodes)
        .map(|node| async move {
            let summary = kubelet_summary(client, &node).await?;
            Some((node, node_total(&summary)))
        })
        .buffer_unordered(8)
        .filter_map(|r| async move { r })
        .collect()
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sums_containers_across_pods() {
        let summary = json!({"pods": [
            {"podRef": {"name": "a", "namespace": "x"}, "containers": [
                {"name": "app", "cpu": {"usageNanoCores": 5_000_000.0}, "memory": {"workingSetBytes": 100.0}},
                {"name": "side", "cpu": {"usageNanoCores": 1_000_000.0}, "memory": {"workingSetBytes": 20.0}},
            ]},
            {"podRef": {"name": "b", "namespace": "x"}, "containers": [
                {"name": "app", "cpu": {}, "memory": {"workingSetBytes": 3.0}},
            ]},
        ]});
        assert_eq!(node_total(&summary), (6.0, 123.0));
        assert_eq!(pod_usage(&summary, "x", "a").unwrap()["side"], (1.0, 20.0));
        assert!(pod_usage(&summary, "y", "a").is_none());
    }
}

#[cfg(test)]
#[path = "usage_tests.rs"]
mod coverage_tests;
