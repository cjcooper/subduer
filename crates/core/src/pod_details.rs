//! Everything about one pod for the Details tab, plus on-demand secret values.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use futures::future::join_all;
use k8s_openapi::api::core::v1::{ConfigMap, Container, EnvFromSource, Event, Node, Pod, Secret};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use kube::api::{Api, DynamicObject, ListParams};
use kube::discovery::ApiResource;
use kube::{Client, ResourceExt};
use serde_json::Value;

use crate::quantity::{bytes, millicores};
use crate::table::{container_states, first_str, int, pod_status, text};
use crate::types::{ContainerDetails, EnvValue, EnvVar, EventSummary, PodDetails, Usage};
use crate::{Error, Result};

pub async fn pod_details(client: &Client, namespace: &str, name: &str) -> Result<PodDetails> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let pod = pods.get(name).await?;
    let spec = pod.spec.clone().unwrap_or_default();
    let all_containers: Vec<&Container> = spec.init_containers.iter().flatten().chain(&spec.containers).collect();

    let node_name = spec.node_name.as_deref();
    let (usage, events, config_maps, secret_keys, node) = tokio::join!(
        pod_usage(client, namespace, name, node_name),
        pod_events(client, namespace, name, pod.metadata.uid.as_deref()),
        read_config_maps(client, namespace, &all_containers),
        read_secret_keys(client, namespace, &all_containers),
        node_allocatable(client, node_name),
    );

    let data = serde_json::to_value(&pod).unwrap_or_default();
    let states = container_states(&data);
    let statuses: Vec<&Value> = ["initContainerStatuses", "containerStatuses"]
        .iter()
        .flat_map(|key| data["status"][key].as_array().into_iter().flatten())
        .collect();

    let containers: Vec<ContainerDetails> = all_containers
        .iter()
        .zip(states)
        .map(|(c, state)| {
            let status = statuses.iter().find(|s| s["name"] == c.name.as_str());
            let used = usage.as_ref().and_then(|u| u.get(&c.name)).copied();
            container_details(c, state, status.copied(), used, node, &config_maps, &secret_keys)
        })
        .collect();

    let status = pod.status.as_ref();
    let app: Vec<&ContainerDetails> = containers.iter().filter(|c| !c.init || c.sidecar).collect();
    let pod_res = crate::overview::pod_resources(&spec);
    let all_limited = |pick: fn(&ContainerDetails) -> Option<f64>| app.iter().all(|c| pick(c).is_some());
    let used_sum = |pick: fn(&ContainerDetails) -> Option<f64>| {
        usage.is_some().then(|| containers.iter().filter_map(pick).sum::<f64>())
    };

    Ok(PodDetails {
        name: pod.name_any(),
        namespace: namespace.to_owned(),
        status: pod_status(&data, pod.metadata.deletion_timestamp.is_some()),
        node: spec.node_name.clone(),
        pod_ip: status.and_then(|s| s.pod_ip.clone()),
        host_ip: status.and_then(|s| s.host_ip.clone()),
        qos_class: status.and_then(|s| s.qos_class.clone()),
        service_account: spec.service_account_name.clone(),
        owner: pod
            .owner_references()
            .iter()
            .find(|o| o.controller == Some(true))
            .or_else(|| pod.owner_references().first())
            .map(|o| format!("{}/{}", o.kind, o.name)),
        restart_policy: spec.restart_policy.clone(),
        priority_class: spec.priority_class_name.clone(),
        created: pod.creation_timestamp().map(|t| t.0.to_string()),
        labels: pod.labels().iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        // A pod limit only exists if every long-running container has one.
        cpu: Usage {
            requests: positive(pod_res.cpu_requests),
            limits: all_limited(|c| c.cpu.limits).then_some(pod_res.cpu_limits),
            used: used_sum(|c| c.cpu.used),
            node: node.map(|(cpu, _)| cpu),
        },
        memory: Usage {
            requests: positive(pod_res.memory_requests),
            limits: all_limited(|c| c.memory.limits).then_some(pod_res.memory_limits),
            used: used_sum(|c| c.memory.used),
            node: node.map(|(_, mem)| mem),
        },
        metrics_available: usage.is_some(),
        containers,
        events,
    })
}

pub(crate) fn port_names(c: &Container) -> Vec<String> {
    c.ports
        .iter()
        .flatten()
        .map(|p| {
            let proto = p.protocol.as_deref().unwrap_or("TCP");
            match &p.name {
                Some(n) => format!("{n} {}/{proto}", p.container_port),
                None => format!("{}/{proto}", p.container_port),
            }
        })
        .collect()
}

fn container_details(
    c: &Container,
    state: crate::types::containers::Container,
    status: Option<&Value>,
    used: Option<(f64, f64)>,
    node: Option<(f64, f64)>,
    config_maps: &HashMap<String, std::result::Result<BTreeMap<String, String>, String>>,
    secret_keys: &HashMap<String, std::result::Result<Vec<String>, String>>,
) -> ContainerDetails {
    let (cpu, memory) = container_resources(c);
    let status = status.unwrap_or(&Value::Null);
    let last = &status["lastState"]["terminated"];

    ContainerDetails {
        name: c.name.clone(),
        init: state.init,
        sidecar: state.init && c.restart_policy.as_deref() == Some("Always"),
        image: c.image.clone().unwrap_or_default(),
        state: state.state,
        detail: state.detail,
        ready: status["ready"].as_bool().unwrap_or(false),
        restarts: int(&status["restartCount"]),
        started: status["state"]["running"]["startedAt"].as_str().map(str::to_owned),
        last_termination: last.is_object().then(|| {
            let reason = last["reason"].as_str().unwrap_or("Terminated");
            let at = text(&last["finishedAt"]);
            format!("{reason} (exit {}) at {at}", int(&last["exitCode"]))
        }),
        ports: port_names(c),
        cpu: Usage {
            used: used.map(|(cpu, _)| cpu),
            node: node.map(|(cpu, _)| cpu),
            ..cpu
        },
        memory: Usage {
            used: used.map(|(_, mem)| mem),
            node: node.map(|(_, mem)| mem),
            ..memory
        },
        env: env_vars(c, config_maps, secret_keys),
    }
}

/// A container's (cpu, memory) requests and limits; `used` and `node` unset.
pub(crate) fn container_resources(c: &Container) -> (Usage, Usage) {
    let resources = c.resources.as_ref();
    let quantity = |map: Option<&BTreeMap<String, Quantity>>, key: &str| map.and_then(|m| m.get(key)).map(|q| q.0.clone());
    let requests = resources.and_then(|r| r.requests.as_ref());
    let limits = resources.and_then(|r| r.limits.as_ref());
    let usage = |key: &str, f: fn(&str) -> f64| Usage {
        requests: quantity(requests, key).map(|q| f(&q)),
        limits: quantity(limits, key).map(|q| f(&q)),
        used: None,
        node: None,
    };
    (usage("cpu", millicores), usage("memory", bytes))
}

/// Explicit `env` first, then each `envFrom` source expanded into its keys.
pub(crate) fn env_vars(
    c: &Container,
    config_maps: &HashMap<String, std::result::Result<BTreeMap<String, String>, String>>,
    secret_keys: &HashMap<String, std::result::Result<Vec<String>, String>>,
) -> Vec<EnvVar> {
    let mut vars: Vec<EnvVar> = c
        .env
        .iter()
        .flatten()
        .map(|e| {
            let from = e.value_from.as_ref();
            let value = if let Some(r) = from.and_then(|f| f.secret_key_ref.as_ref()) {
                EnvValue::Secret {
                    name: r.name.clone(),
                    key: r.key.clone(),
                }
            } else if let Some(r) = from.and_then(|f| f.config_map_key_ref.as_ref()) {
                EnvValue::ConfigMap {
                    name: r.name.clone(),
                    key: r.key.clone(),
                    value: config_maps
                        .get(&r.name)
                        .and_then(|m| m.as_ref().ok())
                        .and_then(|m| m.get(&r.key).cloned()),
                }
            } else if let Some(r) = from.and_then(|f| f.field_ref.as_ref()) {
                EnvValue::Field(r.field_path.clone())
            } else if let Some(r) = from.and_then(|f| f.resource_field_ref.as_ref()) {
                EnvValue::Resource(r.resource.clone())
            } else {
                EnvValue::Literal(e.value.clone().unwrap_or_default())
            };
            EnvVar {
                name: e.name.clone(),
                value,
                from: None,
            }
        })
        .collect();

    for source in c.env_from.iter().flatten() {
        let prefix = source.prefix.clone().unwrap_or_default();
        if let Some(cm) = &source.config_map_ref {
            let label = format!("ConfigMap {}", cm.name);
            match config_maps.get(&cm.name) {
                Some(Ok(data)) => vars.extend(data.iter().map(|(k, v)| EnvVar {
                    name: format!("{prefix}{k}"),
                    value: EnvValue::ConfigMap {
                        name: cm.name.clone(),
                        key: k.clone(),
                        value: Some(v.clone()),
                    },
                    from: Some(label.clone()),
                })),
                Some(Err(e)) => vars.push(unreadable(&label, e)),
                None => {}
            }
        }
        if let Some(secret) = &source.secret_ref {
            let label = format!("Secret {}", secret.name);
            match secret_keys.get(&secret.name) {
                Some(Ok(keys)) => vars.extend(keys.iter().map(|k| EnvVar {
                    name: format!("{prefix}{k}"),
                    value: EnvValue::Secret {
                        name: secret.name.clone(),
                        key: k.clone(),
                    },
                    from: Some(label.clone()),
                })),
                Some(Err(e)) => vars.push(unreadable(&label, e)),
                None => {}
            }
        }
    }
    vars
}

fn unreadable(source: &str, error: &str) -> EnvVar {
    EnvVar {
        name: String::new(),
        value: EnvValue::Unreadable {
            source: source.to_owned(),
            error: error.to_owned(),
        },
        from: Some(source.to_owned()),
    }
}

/// Data of every ConfigMap the containers reference, by name.
pub(crate) async fn read_config_maps(
    client: &Client,
    namespace: &str,
    containers: &[&Container],
) -> HashMap<String, std::result::Result<BTreeMap<String, String>, String>> {
    let names: BTreeSet<String> = containers
        .iter()
        .flat_map(|c| {
            let refs = c
                .env
                .iter()
                .flatten()
                .filter_map(|e| e.value_from.as_ref()?.config_map_key_ref.as_ref().map(|r| r.name.clone()));
            let from = env_from(c).filter_map(|s| s.config_map_ref.as_ref().map(|r| r.name.clone()));
            refs.chain(from).collect::<Vec<_>>()
        })
        .collect();
    let api: Api<ConfigMap> = Api::namespaced(client.clone(), namespace);
    let results = join_all(names.iter().map(|n| api.get(n))).await;
    names
        .into_iter()
        .zip(results)
        .map(|(name, r)| (name, r.map(|cm| cm.data.unwrap_or_default()).map_err(|e| e.to_string())))
        .collect()
}

/// Key names (never values) of Secrets used via `envFrom`.
pub(crate) async fn read_secret_keys(
    client: &Client,
    namespace: &str,
    containers: &[&Container],
) -> HashMap<String, std::result::Result<Vec<String>, String>> {
    let names: BTreeSet<String> = containers
        .iter()
        .flat_map(|c| env_from(c).filter_map(|s| s.secret_ref.as_ref().map(|r| r.name.clone())).collect::<Vec<_>>())
        .collect();
    let api: Api<Secret> = Api::namespaced(client.clone(), namespace);
    let results = join_all(names.iter().map(|n| api.get(n))).await;
    names
        .into_iter()
        .zip(results)
        .map(|(name, r)| {
            let keys = r
                .map(|s| s.data.unwrap_or_default().into_keys().collect())
                .map_err(|e| e.to_string());
            (name, keys)
        })
        .collect()
}

fn env_from(c: &Container) -> impl Iterator<Item = &EnvFromSource> {
    c.env_from.iter().flatten()
}

/// One Secret value, decoded as UTF-8. Called only when the user asks to
/// reveal it; values are never logged or cached.
pub async fn secret_value(client: &Client, namespace: &str, name: &str, key: &str) -> Result<String> {
    let api: Api<Secret> = Api::namespaced(client.clone(), namespace);
    let secret = api.get(name).await?;
    let value = secret
        .data
        .as_ref()
        .and_then(|d| d.get(key))
        .ok_or_else(|| Error::NotFound(format!("key {key:?} in Secret {name}")))?;
    Ok(match String::from_utf8(value.0.clone()) {
        Ok(text) => text,
        Err(_) => format!("<binary, {} bytes>", value.0.len()),
    })
}

/// Every value of a Secret, for editing; `None` for values that aren't
/// UTF-8. Called only when the user starts editing; never logged or cached.
pub async fn secret_values(client: &Client, namespace: &str, name: &str) -> Result<crate::types::SecretValues> {
    let secret = Api::<Secret>::namespaced(client.clone(), namespace).get(name).await?;
    Ok(secret
        .data
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| (k, String::from_utf8(v.0).ok()))
        .collect())
}

/// Container name → (cpu millicores, memory bytes), from metrics-server or,
/// without it, the kubelet's stats on the pod's node. `None` if neither works.
pub(crate) async fn pod_usage(
    client: &Client,
    namespace: &str,
    name: &str,
    node: Option<&str>,
) -> Option<HashMap<String, (f64, f64)>> {
    match metrics_usage(client, namespace, name).await {
        Some(usage) => Some(usage),
        None => kubelet_usage(client, node?, namespace, name).await,
    }
}

/// CPU (millicores) and memory (bytes) in use by every pod in a namespace,
/// from metrics-server. `None` without metrics-server.
pub(crate) async fn namespace_usage(client: &Client, namespace: &str) -> Option<(f64, f64)> {
    let list = pod_metrics_api(client, namespace).list(&ListParams::default()).await.ok()?;
    Some(
        list.items
            .iter()
            .filter_map(metrics_containers)
            .flat_map(|c| c.into_values())
            .fold((0.0, 0.0), |(c, m), (dc, dm)| (c + dc, m + dm)),
    )
}

fn pod_metrics_api(client: &Client, namespace: &str) -> Api<DynamicObject> {
    let ar = ApiResource {
        group: "metrics.k8s.io".into(),
        version: "v1beta1".into(),
        api_version: "metrics.k8s.io/v1beta1".into(),
        kind: "PodMetrics".into(),
        plural: "pods".into(),
    };
    Api::namespaced_with(client.clone(), namespace, &ar)
}

fn metrics_containers(metrics: &DynamicObject) -> Option<HashMap<String, (f64, f64)>> {
    Some(
        metrics.data["containers"]
            .as_array()?
            .iter()
            .map(|c| {
                let usage = &c["usage"];
                (
                    text(&c["name"]),
                    (
                        usage["cpu"].as_str().map_or(0.0, millicores),
                        usage["memory"].as_str().map_or(0.0, bytes),
                    ),
                )
            })
            .collect(),
    )
}

async fn metrics_usage(client: &Client, namespace: &str, name: &str) -> Option<HashMap<String, (f64, f64)>> {
    let metrics = pod_metrics_api(client, namespace).get(name).await.ok()?;
    metrics_containers(&metrics)
}

/// Pod → container → (cpu millicores, memory bytes) for the given pods, from
/// metrics-server or, without it, the kubelet stats of their nodes. `None`
/// if neither works.
pub(crate) async fn pods_usage(
    client: &Client,
    namespace: &str,
    selector: &str,
    pods: &[(String, Option<String>)],
) -> Option<HashMap<String, HashMap<String, (f64, f64)>>> {
    let lp = ListParams::default().labels(selector);
    if let Ok(list) = pod_metrics_api(client, namespace).list(&lp).await {
        return Some(
            list.items
                .iter()
                .filter_map(|m| Some((m.name_any(), metrics_containers(m)?)))
                .collect(),
        );
    }
    let nodes: BTreeSet<&str> = pods.iter().filter_map(|(_, n)| n.as_deref()).collect();
    let summaries = join_all(nodes.iter().map(|n| crate::usage::kubelet_summary(client, n))).await;
    let summaries: HashMap<&str, serde_json::Value> =
        nodes.into_iter().zip(summaries).filter_map(|(n, s)| Some((n, s?))).collect();
    if summaries.is_empty() {
        return None;
    }
    Some(
        pods.iter()
            .filter_map(|(pod, node)| {
                let summary = summaries.get(node.as_deref()?)?;
                Some((pod.clone(), crate::usage::pod_usage(summary, namespace, pod)?))
            })
            .collect(),
    )
}

async fn kubelet_usage(client: &Client, node: &str, namespace: &str, name: &str) -> Option<HashMap<String, (f64, f64)>> {
    let summary = crate::usage::kubelet_summary(client, node).await?;
    crate::usage::pod_usage(&summary, namespace, name)
}

/// The node's allocatable (cpu millicores, memory bytes).
async fn node_allocatable(client: &Client, node: Option<&str>) -> Option<(f64, f64)> {
    let node = Api::<Node>::all(client.clone()).get(node?).await.ok()?;
    let allocatable = node.status?.allocatable?;
    let get = |key: &str| allocatable.get(key).map(|q| q.0.clone());
    Some((millicores(&get("cpu")?), bytes(&get("memory")?)))
}

async fn pod_events(client: &Client, namespace: &str, name: &str, uid: Option<&str>) -> Vec<EventSummary> {
    let fields = format!("involvedObject.name={name},involvedObject.kind=Pod");
    // Same-named pods from earlier rollouts have a different uid.
    list_events(client, namespace, Some(&fields), |e| {
        (uid.is_none() || e.involved_object.uid.as_deref() == uid).then_some(None)
    })
    .await
}

/// A namespace's events (optionally narrowed by a field selector), newest
/// first. `keep` drops an event with `None`, or keeps it with the object
/// label to show beside it.
pub(crate) async fn list_events(
    client: &Client,
    namespace: &str,
    fields: Option<&str>,
    keep: impl Fn(&Event) -> Option<Option<String>>,
) -> Vec<EventSummary> {
    let api: Api<Event> = Api::namespaced(client.clone(), namespace);
    let mut lp = ListParams::default();
    if let Some(fields) = fields {
        lp = lp.fields(fields);
    }
    let Ok(list) = api.list(&lp).await else { return vec![] };
    let mut events: Vec<EventSummary> = list
        .items
        .iter()
        .filter_map(|e| Some(summarize(e, keep(e)?)))
        .collect();
    events.sort_by(|a, b| b.last_seen.cmp(&a.last_seen));
    events
}

fn summarize(e: &Event, object: Option<String>) -> EventSummary {
    let data = serde_json::to_value(e).unwrap_or_default();
    let created = e.metadata.creation_timestamp.as_ref().map(|t| t.0.to_string()).unwrap_or_default();
    let first = first_str(&[&data["firstTimestamp"], &data["eventTime"]]).unwrap_or(&created).to_owned();
    let last = first_str(&[&data["lastTimestamp"], &data["series"]["lastObservedTime"], &data["eventTime"]])
        .map(str::to_owned)
        .unwrap_or_else(|| first.clone());
    EventSummary {
        type_: text(&data["type"]),
        reason: text(&data["reason"]),
        message: text(&data["message"]).trim().to_owned(),
        count: data["count"].as_i64().or_else(|| data["series"]["count"].as_i64()).unwrap_or(1),
        first_seen: first,
        last_seen: last,
        source: first_str(&[&data["source"]["component"], &data["reportingComponent"]])
            .unwrap_or_default()
            .to_owned(),
        object,
    }
}

fn positive(v: f64) -> Option<f64> {
    (v > 0.0).then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_sources() {
        let c: Container = serde_json::from_value(serde_json::json!({
            "name": "app",
            "env": [
                {"name": "PLAIN", "value": "1"},
                {"name": "DB_PASS", "valueFrom": {"secretKeyRef": {"name": "db", "key": "password"}}},
                {"name": "MODE", "valueFrom": {"configMapKeyRef": {"name": "cfg", "key": "mode"}}},
                {"name": "POD", "valueFrom": {"fieldRef": {"fieldPath": "metadata.name"}}}
            ],
            "envFrom": [
                {"configMapRef": {"name": "cfg"}, "prefix": "CFG_"},
                {"secretRef": {"name": "api-keys"}},
                {"secretRef": {"name": "forbidden"}}
            ]
        }))
        .unwrap();
        let cms = HashMap::from([(
            "cfg".to_owned(),
            Ok(BTreeMap::from([("mode".to_owned(), "fast".to_owned())])),
        )]);
        let secrets = HashMap::from([
            ("api-keys".to_owned(), Ok(vec!["TOKEN".to_owned()])),
            ("forbidden".to_owned(), Err("forbidden".to_owned())),
        ]);
        let vars = env_vars(&c, &cms, &secrets);
        let names: Vec<_> = vars.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, ["PLAIN", "DB_PASS", "MODE", "POD", "CFG_mode", "TOKEN", ""]);
        assert_eq!(vars[0].value, EnvValue::Literal("1".into()));
        assert_eq!(vars[1].value, EnvValue::Secret { name: "db".into(), key: "password".into() });
        assert_eq!(
            vars[2].value,
            EnvValue::ConfigMap { name: "cfg".into(), key: "mode".into(), value: Some("fast".into()) }
        );
        assert_eq!(vars[3].value, EnvValue::Field("metadata.name".into()));
        assert_eq!(vars[4].from.as_deref(), Some("ConfigMap cfg"));
        assert_eq!(vars[5].value, EnvValue::Secret { name: "api-keys".into(), key: "TOKEN".into() });
        assert!(matches!(vars[6].value, EnvValue::Unreadable { .. }));
    }
}

#[cfg(test)]
#[path = "pod_details_tests.rs"]
mod coverage_tests;
