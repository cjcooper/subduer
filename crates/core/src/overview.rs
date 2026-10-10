//! Cluster-wide summary: capacity, requests, usage and pod health.

use std::collections::{BTreeMap, HashMap};

use k8s_openapi::api::core::v1::{Event, Node, Pod, PodSpec};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use kube::api::{Api, DynamicObject, ListParams};
use kube::discovery::ApiResource;
use kube::{Client, ResourceExt};

use crate::Result;
use crate::quantity::{bytes, millicores};
use crate::history::HistoryStore;
use crate::types::{ClusterOverview, NamespaceRequests, NodeSummary, PodCounts, ResourceTotals, UsageSource};

/// Requests and limits of one pod: (cpu req, cpu lim, mem req, mem lim).
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub(crate) struct PodResources {
    pub(crate) cpu_requests: f64,
    pub(crate) cpu_limits: f64,
    pub(crate) memory_requests: f64,
    pub(crate) memory_limits: f64,
}

/// `history` supplies the connection's Prometheus (for usage without
/// metrics-server, and the last-hour charts).
pub async fn cluster_overview(client: &Client, history: &HistoryStore) -> Result<ClusterOverview> {
    // resourceVersion=0: served from the API server's cache, cheaper on big clusters.
    let lp = ListParams::default().match_any();
    let nodes_api: Api<Node> = Api::all(client.clone());
    let pods_api: Api<Pod> = Api::all(client.clone());
    let (nodes, pods, version, metrics_server, warnings, cluster_history) = tokio::join!(
        nodes_api.list(&lp),
        pods_api.list(&lp),
        client.apiserver_version(),
        node_usage(client),
        warnings_last_hour(client),
        history.cluster_history(client),
    );
    let (nodes, pods) = (nodes?.items, pods?.items);

    let mut pod_counts = PodCounts::default();
    let mut per_node: HashMap<String, (usize, PodResources)> = HashMap::new();
    let mut per_namespace: BTreeMap<String, NamespaceRequests> = BTreeMap::new();
    for pod in &pods {
        let phase = pod.status.as_ref().and_then(|s| s.phase.as_deref()).unwrap_or("Unknown");
        match phase {
            "Running" => pod_counts.running += 1,
            "Pending" => pod_counts.pending += 1,
            "Succeeded" => pod_counts.succeeded += 1,
            "Failed" => pod_counts.failed += 1,
            _ => pod_counts.unknown += 1,
        }
        // Finished pods no longer hold resources.
        if matches!(phase, "Succeeded" | "Failed") {
            continue;
        }
        let Some(spec) = &pod.spec else { continue };
        let res = pod_resources(spec);
        let ns = per_namespace
            .entry(pod.namespace().unwrap_or_default())
            .or_insert_with_key(|ns| NamespaceRequests {
                namespace: ns.clone(),
                pods: 0,
                cpu: 0.0,
                memory: 0.0,
            });
        ns.pods += 1;
        ns.cpu += res.cpu_requests;
        ns.memory += res.memory_requests;
        if let Some(node) = &spec.node_name {
            let entry = per_node.entry(node.clone()).or_default();
            entry.0 += 1;
            entry.1 = add(entry.1, res);
        }
    }

    let (usage_source, usage) = match metrics_server {
        Some(usage) => (Some(UsageSource::MetricsServer), Some(usage)),
        None => {
            let names: Vec<String> = nodes.iter().map(|n| n.name_any()).collect();
            fallback_usage(client, history, &names, &per_node).await
        }
    };
    let mut node_summaries: Vec<NodeSummary> = nodes
        .iter()
        .map(|node| {
            let name = node.name_any();
            let (pods, res) = per_node.get(&name).copied().unwrap_or_default();
            let used = usage.as_ref().and_then(|u| u.get(&name)).copied();
            node_summary(node, pods, res, used)
        })
        .collect();
    node_summaries.sort_by(|a, b| a.name.cmp(&b.name));

    let metrics_available = usage.is_some();
    let total = |pick: fn(&NodeSummary) -> ResourceTotals| {
        node_summaries.iter().map(pick).fold(
            ResourceTotals {
                used: metrics_available.then_some(0.0),
                ..Default::default()
            },
            |acc, r| ResourceTotals {
                capacity: acc.capacity + r.capacity,
                allocatable: acc.allocatable + r.allocatable,
                requests: acc.requests + r.requests,
                limits: acc.limits + r.limits,
                used: acc.used.map(|u| u + r.used.unwrap_or(0.0)),
            },
        )
    };
    let cpu = total(|n| n.cpu);
    let memory = total(|n| n.memory);

    let mut namespaces: Vec<NamespaceRequests> = per_namespace.into_values().collect();
    namespaces.sort_by(|a, b| b.memory.total_cmp(&a.memory));

    Ok(ClusterOverview {
        server_version: version.ok().map(|v| v.git_version),
        nodes: node_summaries,
        pods: pod_counts,
        cpu,
        memory,
        namespaces,
        warnings_last_hour: warnings,
        metrics_available,
        usage_source,
        history: cluster_history,
    })
}

fn node_summary(node: &Node, pods: usize, res: PodResources, used: Option<(f64, f64)>) -> NodeSummary {
    let status = node.status.as_ref();
    let capacity = status.and_then(|s| s.capacity.as_ref());
    let allocatable = status.and_then(|s| s.allocatable.as_ref());
    let labels = node.labels();

    let mut roles: Vec<String> = labels
        .keys()
        .filter_map(|k| k.strip_prefix("node-role.kubernetes.io/"))
        .filter(|r| !r.is_empty())
        .map(str::to_owned)
        .collect();
    roles.sort();
    let instance_type = if labels.get("eks.amazonaws.com/compute-type").map(String::as_str) == Some("fargate") {
        Some("fargate".to_owned())
    } else {
        labels
            .get("node.kubernetes.io/instance-type")
            .or_else(|| labels.get("beta.kubernetes.io/instance-type"))
            .cloned()
    };

    NodeSummary {
        name: node.name_any(),
        ready: status
            .and_then(|s| s.conditions.as_ref())
            .and_then(|c| c.iter().find(|c| c.type_ == "Ready"))
            .is_some_and(|c| c.status == "True"),
        schedulable: !node.spec.as_ref().and_then(|s| s.unschedulable).unwrap_or(false),
        roles,
        instance_type,
        kubelet_version: status
            .and_then(|s| s.node_info.as_ref())
            .map(|i| i.kubelet_version.clone())
            .unwrap_or_default(),
        created: node.creation_timestamp().map(|t| t.0.to_string()),
        pods,
        pod_capacity: get(allocatable, "pods").and_then(|p| p.parse().ok()).unwrap_or(0),
        cpu: ResourceTotals {
            capacity: get(capacity, "cpu").map_or(0.0, millicores),
            allocatable: get(allocatable, "cpu").map_or(0.0, millicores),
            requests: res.cpu_requests,
            limits: res.cpu_limits,
            used: used.map(|(cpu, _)| cpu),
        },
        memory: ResourceTotals {
            capacity: get(capacity, "memory").map_or(0.0, bytes),
            allocatable: get(allocatable, "memory").map_or(0.0, bytes),
            requests: res.memory_requests,
            limits: res.memory_limits,
            used: used.map(|(_, mem)| mem),
        },
    }
}

/// Effective pod resources, as the scheduler counts them: the larger of the
/// app containers' sum and the biggest init container, plus pod overhead.
pub(crate) fn pod_resources(spec: &PodSpec) -> PodResources {
    let of = |c: &k8s_openapi::api::core::v1::Container| {
        let r = c.resources.as_ref();
        let requests = r.and_then(|r| r.requests.as_ref());
        let limits = r.and_then(|r| r.limits.as_ref());
        PodResources {
            cpu_requests: get(requests, "cpu").map_or(0.0, millicores),
            cpu_limits: get(limits, "cpu").map_or(0.0, millicores),
            memory_requests: get(requests, "memory").map_or(0.0, bytes),
            memory_limits: get(limits, "memory").map_or(0.0, bytes),
        }
    };
    let apps = spec.containers.iter().map(of).fold(PodResources::default(), add);
    let init = spec
        .init_containers
        .iter()
        .flatten()
        .map(of)
        .fold(PodResources::default(), |a, b| PodResources {
            cpu_requests: a.cpu_requests.max(b.cpu_requests),
            cpu_limits: a.cpu_limits.max(b.cpu_limits),
            memory_requests: a.memory_requests.max(b.memory_requests),
            memory_limits: a.memory_limits.max(b.memory_limits),
        });
    let overhead = spec.overhead.as_ref();
    let oh = |key: &str, f: fn(&str) -> f64| overhead.and_then(|o| o.get(key)).map_or(0.0, |q| f(&q.0));
    PodResources {
        cpu_requests: apps.cpu_requests.max(init.cpu_requests) + oh("cpu", millicores),
        cpu_limits: apps.cpu_limits.max(init.cpu_limits) + oh("cpu", millicores),
        memory_requests: apps.memory_requests.max(init.memory_requests) + oh("memory", bytes),
        memory_limits: apps.memory_limits.max(init.memory_limits) + oh("memory", bytes),
    }
}

fn get<'a>(map: Option<&'a BTreeMap<String, Quantity>>, key: &str) -> Option<&'a str> {
    map.and_then(|m| m.get(key)).map(|q| q.0.as_str())
}

fn add(a: PodResources, b: PodResources) -> PodResources {
    PodResources {
        cpu_requests: a.cpu_requests + b.cpu_requests,
        cpu_limits: a.cpu_limits + b.cpu_limits,
        memory_requests: a.memory_requests + b.memory_requests,
        memory_limits: a.memory_limits + b.memory_limits,
    }
}

/// Node name → (cpu millicores, memory bytes) from metrics-server, or `None`
/// if it isn't installed or readable.
async fn node_usage(client: &Client) -> Option<HashMap<String, (f64, f64)>> {
    let ar = ApiResource {
        group: "metrics.k8s.io".into(),
        version: "v1beta1".into(),
        api_version: "metrics.k8s.io/v1beta1".into(),
        kind: "NodeMetrics".into(),
        plural: "nodes".into(),
    };
    let api: Api<DynamicObject> = Api::all_with(client.clone(), &ar);
    let list = api.list(&ListParams::default()).await.ok()?;
    Some(
        list.items
            .iter()
            .map(|m| {
                let usage = &m.data["usage"];
                let cpu = usage["cpu"].as_str().map_or(0.0, millicores);
                let mem = usage["memory"].as_str().map_or(0.0, bytes);
                (m.name_any(), (cpu, mem))
            })
            .collect(),
    )
}

/// Usage without metrics-server: Prometheus per node, with kubelet stats for
/// nodes it doesn't report; or kubelet stats alone. Only nodes running pods
/// are asked (finished pods hold nothing); the rest count as idle.
async fn fallback_usage(
    client: &Client,
    history: &HistoryStore,
    nodes: &[String],
    per_node: &HashMap<String, (usize, PodResources)>,
) -> (Option<UsageSource>, Option<HashMap<String, (f64, f64)>>) {
    let is_busy = |n: &String| per_node.get(n).is_some_and(|(pods, _)| *pods > 0);
    let busy = nodes.iter().filter(|n| is_busy(n)).cloned();
    let (source, mut usage) = match history.node_usage(client).await {
        Some(usage) => (UsageSource::Prometheus, usage),
        None => (UsageSource::Kubelet, HashMap::new()),
    };
    let missing: Vec<String> = busy.filter(|n| !usage.contains_key(n)).collect();
    let asked = missing.len();
    let from_kubelet = crate::usage::kubelet_node_usage(client, missing).await;
    // Kubelet alone and every request failed: there's no usage to show.
    if source == UsageSource::Kubelet && asked > 0 && from_kubelet.is_empty() {
        return (None, None);
    }
    usage.extend(from_kubelet);
    for idle in nodes.iter().filter(|n| !is_busy(n)) {
        usage.entry(idle.clone()).or_insert((0.0, 0.0));
    }
    (Some(source), Some(usage))
}

async fn warnings_last_hour(client: &Client) -> Option<usize> {
    let api: Api<Event> = Api::all(client.clone());
    let list = api.list(&ListParams::default().fields("type=Warning")).await.ok()?;
    let cutoff = jiff::Timestamp::now() - jiff::SignedDuration::from_hours(1);
    Some(
        list.items
            .iter()
            .filter(|e| {
                let last = e
                    .last_timestamp
                    .as_ref()
                    .map(|t| t.0)
                    .or_else(|| e.series.as_ref().and_then(|s| s.last_observed_time.as_ref()).map(|t| t.0))
                    .or_else(|| e.event_time.as_ref().map(|t| t.0));
                last.is_some_and(|t| t >= cutoff)
            })
            .count(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_pod_requests() {
        let spec: PodSpec = serde_json::from_value(serde_json::json!({
            "containers": [
                {"name": "app", "resources": {"requests": {"cpu": "250m", "memory": "256Mi"}, "limits": {"memory": "512Mi"}}},
                {"name": "sidecar", "resources": {"requests": {"cpu": "50m", "memory": "64Mi"}}}
            ],
            "initContainers": [
                {"name": "migrate", "resources": {"requests": {"cpu": "1", "memory": "128Mi"}}}
            ],
            "overhead": {"memory": "10Mi"}
        }))
        .unwrap();
        let r = pod_resources(&spec);
        let mi = 1024.0 * 1024.0;
        assert_eq!(r.cpu_requests, 1000.0); // init container dominates
        assert_eq!(r.memory_requests, (320.0 + 10.0) * mi); // app sum + overhead
        assert_eq!(r.memory_limits, (512.0 + 10.0) * mi);
        assert_eq!(r.cpu_limits, 0.0);
    }
}

#[cfg(test)]
#[path = "overview_tests.rs"]
mod coverage_tests;
