//! CPU and memory over the last hour for one pod. Prefers the cluster's
//! Prometheus (cAdvisor metrics); without one, records its own samples each
//! time it's asked, so the line grows from when the pod was first viewed.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use k8s_openapi::api::core::v1::{Pod, Service};
use kube::api::{Api, ListParams};
use kube::{Client, ResourceExt};
use serde_json::Value;
use tokio::sync::OnceCell;

use crate::pod_details::pod_usage;
use crate::types::{ContainerHistory, HistorySource, PodHistory, Series, UsageHistory};
use crate::Result;

pub const WINDOW_SECS: f64 = 3600.0;
/// Matches the UI's refresh interval.
pub const STEP_SECS: f64 = 15.0;
/// Locally recorded pods not asked about for this long are dropped.
const LOCAL_TTL_SECS: f64 = 2.0 * WINDOW_SECS;

/// A Prometheus HTTP API reached through the API server's service proxy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prometheus {
    pub namespace: String,
    pub service: String,
    pub port: i32,
}

impl Prometheus {
    fn api_path(&self, endpoint: &str) -> String {
        format!(
            "/api/v1/namespaces/{}/services/{}:{}/proxy/api/v1/{endpoint}",
            self.namespace, self.service, self.port
        )
    }

    async fn get(&self, client: &Client, endpoint: &str, params: &[(&str, &str)]) -> Option<Value> {
        let query = form_urlencoded::Serializer::new(String::new()).extend_pairs(params).finish();
        let request = http::Request::get(format!("{}?{query}", self.api_path(endpoint)))
            .body(Vec::new())
            .ok()?;
        let mut body: Value = client.request(request).await.ok()?;
        (body["status"] == "success").then(|| body["data"].take())
    }
}

/// Per-connection history state: the Prometheus lookup (done once) and
/// locally recorded samples.
#[derive(Default)]
pub struct HistoryStore {
    prometheus: OnceCell<Option<Prometheus>>,
    local: Mutex<HashMap<String, LocalPod>>,
}

/// Container → (cpu millicores, memory bytes) at one instant.
type Sample = BTreeMap<String, (f64, f64)>;

#[derive(Default)]
struct LocalPod {
    last_asked: f64,
    /// (time, container → (cpu, memory)), oldest first.
    samples: VecDeque<(f64, Sample)>,
}

impl HistoryStore {
    pub async fn prometheus(&self, client: &Client) -> Option<&Prometheus> {
        self.prometheus.get_or_init(|| discover_prometheus(client)).await.as_ref()
    }

    pub async fn pod_history(&self, client: &Client, namespace: &str, name: &str) -> Result<PodHistory> {
        let now = unix_now();
        // Align to the step so points stay put between refreshes.
        let end = (now / STEP_SECS).floor() * STEP_SECS;
        let start = end - WINDOW_SECS;
        if let Some(prom) = self.prometheus(client).await
            && let Some(containers) = prometheus_history(client, prom, namespace, name, start, end).await
            && !containers.is_empty()
        {
            return Ok(assemble(HistorySource::Prometheus, start, end, containers));
        }
        let pod = Api::<Pod>::namespaced(client.clone(), namespace).get(name).await?;
        let node = pod.spec.as_ref().and_then(|s| s.node_name.as_deref());
        let usage = pod_usage(client, namespace, &pod.name_any(), node).await;
        let containers = self.record(&format!("{namespace}/{name}"), now, usage);
        Ok(assemble(HistorySource::Local, start, end, containers))
    }

    /// Appends a sample (if any) and returns the pod's recorded series.
    fn record(&self, key: &str, now: f64, usage: Option<HashMap<String, (f64, f64)>>) -> Vec<ContainerHistory> {
        let mut local = self.local.lock().unwrap();
        local.retain(|_, p| now - p.last_asked < LOCAL_TTL_SECS);
        let pod = local.entry(key.to_owned()).or_default();
        pod.last_asked = now;
        if let Some(usage) = usage {
            pod.samples.push_back((now, usage.into_iter().collect()));
        }
        while pod.samples.front().is_some_and(|(t, _)| now - t > WINDOW_SECS) {
            pod.samples.pop_front();
        }
        let mut by_container: BTreeMap<String, UsageHistory> = BTreeMap::new();
        for (t, sample) in &pod.samples {
            for (container, (cpu, memory)) in sample {
                let h = by_container.entry(container.clone()).or_default();
                h.cpu.push((*t, *cpu));
                h.memory.push((*t, *memory));
            }
        }
        by_container
            .into_iter()
            .map(|(name, usage)| ContainerHistory { name, usage })
            .collect()
    }
}

fn assemble(source: HistorySource, start: f64, end: f64, containers: Vec<ContainerHistory>) -> PodHistory {
    let pod = UsageHistory {
        cpu: sum_series(containers.iter().map(|c| &c.usage.cpu)),
        memory: sum_series(containers.iter().map(|c| &c.usage.memory)),
    };
    PodHistory { source, start, end, step: STEP_SECS, pod, containers }
}

/// Adds series point by point on matching timestamps.
fn sum_series<'a>(series: impl Iterator<Item = &'a Series>) -> Series {
    let mut totals: BTreeMap<i64, f64> = BTreeMap::new();
    for s in series {
        for &(t, v) in s {
            // Millisecond keys: timestamps are shared but f64 isn't Ord.
            *totals.entry((t * 1000.0).round() as i64).or_default() += v;
        }
    }
    totals.into_iter().map(|(t, v)| (t as f64 / 1000.0, v)).collect()
}

async fn prometheus_history(
    client: &Client,
    prom: &Prometheus,
    namespace: &str,
    name: &str,
    start: f64,
    end: f64,
) -> Option<Vec<ContainerHistory>> {
    let selector = format!(r#"namespace="{namespace}",pod="{name}",container!="",container!="POD""#);
    // `max by` rather than `sum by`: some setups scrape cAdvisor twice.
    let cpu = format!("max by (container) (rate(container_cpu_usage_seconds_total{{{selector}}}[2m])) * 1000");
    let memory = format!("max by (container) (container_memory_working_set_bytes{{{selector}}})");
    let (start, end, step) = (start.to_string(), end.to_string(), STEP_SECS.to_string());
    let range = |query: String| {
        let (start, end, step) = (start.clone(), end.clone(), step.clone());
        async move {
            let params = [("query", query.as_str()), ("start", &start), ("end", &end), ("step", &step)];
            prom.get(client, "query_range", &params).await
        }
    };
    let (cpu, memory) = tokio::join!(range(cpu), range(memory));
    let (cpu, memory) = (by_container(&cpu?), by_container(&memory?));
    let mut names: Vec<&String> = cpu.keys().chain(memory.keys()).collect();
    names.sort();
    names.dedup();
    Some(
        names
            .into_iter()
            .map(|n| ContainerHistory {
                name: n.clone(),
                usage: UsageHistory {
                    cpu: cpu.get(n).cloned().unwrap_or_default(),
                    memory: memory.get(n).cloned().unwrap_or_default(),
                },
            })
            .collect(),
    )
}

/// Container → series from a `query_range` matrix result.
fn by_container(data: &Value) -> HashMap<String, Series> {
    data["result"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let name = r["metric"]["container"].as_str()?.to_owned();
            let points = r["values"]
                .as_array()?
                .iter()
                .filter_map(|p| Some((p[0].as_f64()?, p[1].as_str()?.parse::<f64>().ok()?)))
                .filter(|(_, v)| v.is_finite())
                .collect();
            Some((name, points))
        })
        .collect()
}

/// Finds a Prometheus service that answers queries. Needs permission to list
/// services cluster-wide and to use the service proxy; `None` otherwise.
async fn discover_prometheus(client: &Client) -> Option<Prometheus> {
    let services = Api::<Service>::all(client.clone()).list(&ListParams::default()).await.ok()?;
    let mut candidates: Vec<(u8, Prometheus)> = services
        .items
        .iter()
        .filter_map(|svc| {
            let name = svc.name_any();
            let lower = name.to_lowercase();
            let skip = ["operator", "exporter", "kube-state", "alertmanager", "pushgateway", "adapter"];
            if !lower.contains("prometheus") || skip.iter().any(|s| lower.contains(s)) {
                return None;
            }
            let spec = svc.spec.as_ref()?;
            let port = spec.ports.iter().flatten().find(|p| p.port == 9090)?.port;
            let headless = spec.cluster_ip.as_deref() == Some("None");
            let rank = u8::from(headless);
            Some((rank, Prometheus { namespace: svc.namespace()?, service: name, port }))
        })
        .collect();
    candidates.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.service.cmp(&b.1.service)));
    for (_, prom) in candidates {
        if prom.get(client, "query", &[("query", "1")]).await.is_some() {
            tracing::info!(namespace = %prom.namespace, service = %prom.service, "using Prometheus for history");
            return Some(prom);
        }
    }
    None
}

fn unix_now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sums_matching_timestamps() {
        let a = vec![(10.0, 1.0), (25.0, 2.0)];
        let b = vec![(10.0, 3.0), (40.0, 4.0)];
        assert_eq!(sum_series([&a, &b].into_iter()), vec![(10.0, 4.0), (25.0, 2.0), (40.0, 4.0)]);
    }

    #[test]
    fn parses_matrix_by_container() {
        let data = json!({"result": [
            {"metric": {"container": "api"}, "values": [[100, "1.5"], [115, "NaN"], [130, "2"]]},
            {"metric": {}, "values": [[100, "9"]]},
        ]});
        let parsed = by_container(&data);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed["api"], vec![(100.0, 1.5), (130.0, 2.0)]);
    }

    #[test]
    fn local_samples_expire_after_the_window() {
        let store = HistoryStore::default();
        let sample = |cpu: f64| Some(HashMap::from([("api".to_owned(), (cpu, 10.0))]));
        store.record("ns/p", 0.0, sample(1.0));
        store.record("ns/p", 1800.0, sample(2.0));
        let out = store.record("ns/p", 3700.0, sample(3.0));
        assert_eq!(out[0].usage.cpu, vec![(1800.0, 2.0), (3700.0, 3.0)]);
    }
}
