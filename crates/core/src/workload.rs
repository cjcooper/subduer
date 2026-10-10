//! Everything about one Deployment or StatefulSet for the Details tab: rollout
//! status, its pods, the pod template's containers, usage summed over the
//! pods, and the events of the workload, its ReplicaSets and its pods.

use std::collections::{HashMap, HashSet};

use k8s_openapi::api::apps::v1::{Deployment, ReplicaSet, StatefulSet};
use k8s_openapi::api::core::v1::{Container, Pod, PodTemplateSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::api::{Api, ListParams};
use kube::{Client, ResourceExt};

use crate::pod_details::{container_resources, env_vars, list_events, pods_usage, port_names, read_config_maps, read_secret_keys};
use crate::table::{container_states, pod_status};
use crate::types::{
    Condition, KindRef, ReplicaCounts, TemplateContainer, Usage, WorkloadDetails, WorkloadPod, is_scalable_workload,
};
use crate::{Error, Result};

/// Pod → container → (cpu millicores, memory bytes).
type PodsUsage = HashMap<String, HashMap<String, (f64, f64)>>;

/// The parts of a Deployment or StatefulSet that don't depend on the kind.
struct Workload {
    kind: String,
    meta: ObjectMeta,
    selector: LabelSelector,
    template: PodTemplateSpec,
    status: String,
    replicas: ReplicaCounts,
    info: Vec<(String, String)>,
    conditions: Vec<Condition>,
    /// Whether a pod runs the latest template.
    is_updated: Box<dyn Fn(&Pod) -> bool + Send + Sync>,
    /// Owners the pods must be controlled by: the StatefulSet, or the Deployment's ReplicaSets.
    owners: Vec<(String, String)>,
}

pub async fn workload_details(client: &Client, kind: &KindRef, namespace: &str, name: &str) -> Result<WorkloadDetails> {
    let w = load(client, kind, namespace, name).await?;
    let selector = selector_string(&w.selector);
    let pods = owned_pods(client, namespace, &selector, &w.owners).await?;

    let template_spec = w.template.spec.clone().unwrap_or_default();
    let all: Vec<&Container> = template_spec.init_containers.iter().flatten().chain(&template_spec.containers).collect();
    let placements: Vec<(String, Option<String>)> =
        pods.iter().map(|p| (p.name_any(), p.spec.as_ref().and_then(|s| s.node_name.clone()))).collect();

    let object_names: HashMap<(String, String), Option<String>> = w
        .owners
        .iter()
        .filter(|(k, _)| k != &w.kind)
        .map(|(k, n)| ((k.clone(), n.clone()), Some(format!("{k} {n}"))))
        .chain(placements.iter().map(|(p, _)| (("Pod".to_owned(), p.clone()), Some(format!("Pod {p}")))))
        .chain([((w.kind.clone(), name.to_owned()), None)])
        .collect();
    let (usage, config_maps, secret_keys, events) = tokio::join!(
        pods_usage(client, namespace, &selector, &placements),
        read_config_maps(client, namespace, &all),
        read_secret_keys(client, namespace, &all),
        list_events(client, namespace, None, |e| {
            let key = (e.involved_object.kind.clone()?, e.involved_object.name.clone()?);
            object_names.get(&key).cloned()
        }),
    );

    let workload_pods: Vec<WorkloadPod> = pods.iter().map(|p| workload_pod(p, &w, usage.as_ref())).collect();
    let containers: Vec<TemplateContainer> = template_spec
        .init_containers
        .iter()
        .flatten()
        .map(|c| (c, true))
        .chain(template_spec.containers.iter().map(|c| (c, false)))
        .map(|(c, init)| {
            let (cpu, memory) = container_resources(c);
            let used = usage.as_ref().map(|u| summed(u.values().filter_map(|pod| pod.get(&c.name).copied())));
            TemplateContainer {
                name: c.name.clone(),
                init,
                sidecar: init && c.restart_policy.as_deref() == Some("Always"),
                image: c.image.clone().unwrap_or_default(),
                ports: port_names(c),
                cpu: Usage { used: used.map(|u| u.0), ..cpu },
                memory: Usage { used: used.map(|u| u.1), ..memory },
                env: env_vars(c, &config_maps, &secret_keys),
            }
        })
        .collect();

    // Totals over the pods still running; limits only if every long-running
    // container has one (the same rule as a pod's).
    let live: Vec<&Pod> = pods.iter().filter(|p| !finished(p)).collect();
    let resources = live
        .iter()
        .filter_map(|p| p.spec.as_ref())
        .map(crate::overview::pod_resources)
        .fold((0.0, 0.0, 0.0, 0.0), |a, r| {
            (a.0 + r.cpu_requests, a.1 + r.cpu_limits, a.2 + r.memory_requests, a.3 + r.memory_limits)
        });
    let long_running: Vec<&TemplateContainer> = containers.iter().filter(|c| !c.init || c.sidecar).collect();
    let all_limited = |pick: fn(&TemplateContainer) -> Option<f64>| long_running.iter().all(|c| pick(c).is_some());
    let used = usage.as_ref().map(|u| summed(u.values().flat_map(|pod| pod.values().copied())));

    Ok(WorkloadDetails {
        kind: w.kind.clone(),
        name: name.to_owned(),
        namespace: namespace.to_owned(),
        status: w.status.clone(),
        replicas: w.replicas,
        info: w.info.clone(),
        created: w.meta.creation_timestamp.as_ref().map(|t| t.0.to_string()),
        labels: w.meta.labels.clone().unwrap_or_default().into_iter().collect(),
        conditions: w.conditions.clone(),
        cpu: Usage {
            requests: positive(resources.0),
            limits: all_limited(|c| c.cpu.limits).then_some(resources.1).filter(|_| !live.is_empty()),
            used: used.map(|u| u.0),
            node: None,
        },
        memory: Usage {
            requests: positive(resources.2),
            limits: all_limited(|c| c.memory.limits).then_some(resources.3).filter(|_| !live.is_empty()),
            used: used.map(|u| u.1),
            node: None,
        },
        metrics_available: usage.is_some(),
        pods: workload_pods,
        containers,
        events,
    })
}

/// The workload's current pods with their nodes, for usage history.
pub async fn current_pods(
    client: &Client,
    kind: &KindRef,
    namespace: &str,
    name: &str,
) -> Result<(String, Vec<(String, Option<String>)>)> {
    let w = load(client, kind, namespace, name).await?;
    let selector = selector_string(&w.selector);
    let pods = owned_pods(client, namespace, &selector, &w.owners).await?;
    let placements = pods
        .iter()
        .filter(|p| !finished(p))
        .map(|p| (p.name_any(), p.spec.as_ref().and_then(|s| s.node_name.clone())))
        .collect();
    Ok((selector, placements))
}

/// A Prometheus regex matching every pod the workload has ever created,
/// including ones replaced during the last hour.
pub fn pod_name_regex(kind: &KindRef, name: &str) -> String {
    let name = name.replace('.', r"\\.");
    if kind.kind == "StatefulSet" {
        format!("{name}-[0-9]+")
    } else {
        // <deployment>-<pod-template-hash>-<5 random characters>
        format!("{name}-[a-z0-9]{{1,10}}-[a-z0-9]{{5}}")
    }
}

async fn load(client: &Client, kind: &KindRef, namespace: &str, name: &str) -> Result<Workload> {
    if !is_scalable_workload(kind) {
        return Err(Error::Message(format!("no details view for {}", kind.kind)));
    }
    if kind.kind == "StatefulSet" {
        let sts = Api::<StatefulSet>::namespaced(client.clone(), namespace).get(name).await?;
        Ok(statefulset(sts))
    } else {
        let deployment = Api::<Deployment>::namespaced(client.clone(), namespace).get(name).await?;
        let selector = selector_string(&deployment.spec.as_ref().map(|s| s.selector.clone()).unwrap_or_default());
        let replica_sets = Api::<ReplicaSet>::namespaced(client.clone(), namespace)
            .list(&ListParams::default().labels(&selector))
            .await?
            .items
            .into_iter()
            .filter(|rs| controlled_by(rs.owner_references(), "Deployment", name))
            .collect();
        Ok(deployment_workload(deployment, replica_sets))
    }
}

fn deployment_workload(d: Deployment, replica_sets: Vec<ReplicaSet>) -> Workload {
    let spec = d.spec.clone().unwrap_or_default();
    let status = d.status.clone().unwrap_or_default();
    let revision = d.annotations().get("deployment.kubernetes.io/revision").cloned();
    let replicas = ReplicaCounts {
        desired: spec.replicas.unwrap_or(1),
        current: status.replicas.unwrap_or(0),
        ready: status.ready_replicas.unwrap_or(0),
        updated: status.updated_replicas.unwrap_or(0),
        available: status.available_replicas.unwrap_or(0),
    };
    let conditions: Vec<Condition> = status
        .conditions
        .iter()
        .flatten()
        .map(|c| Condition {
            type_: c.type_.clone(),
            status: c.status.clone(),
            reason: c.reason.clone().unwrap_or_default(),
            message: c.message.clone().unwrap_or_default(),
            last_transition: c.last_transition_time.as_ref().map(|t| t.0.to_string()),
        })
        .collect();
    let failed = conditions
        .iter()
        .any(|c| c.type_ == "Progressing" && c.reason == "ProgressDeadlineExceeded");
    let stale = status.observed_generation.unwrap_or(0) < d.metadata.generation.unwrap_or(0);
    let state = if spec.paused == Some(true) {
        "Paused"
    } else if failed {
        "Failed"
    } else if replicas.desired == 0 && replicas.current == 0 {
        "Scaled to 0"
    } else if stale || replicas.updated < replicas.desired || replicas.current > replicas.updated {
        "Rolling out"
    } else if replicas.available < replicas.desired {
        "Degraded"
    } else {
        "Available"
    };

    let strategy = spec.strategy.clone().unwrap_or_default();
    let strategy = match strategy.type_.as_deref() {
        Some("Recreate") => "Recreate".to_owned(),
        _ => {
            let ru = strategy.rolling_update.unwrap_or_default();
            format!(
                "RollingUpdate · max surge {} · max unavailable {}",
                int_or_string(ru.max_surge.as_ref(), "25%"),
                int_or_string(ru.max_unavailable.as_ref(), "25%"),
            )
        }
    };
    // The newest ReplicaSet carries the Deployment's revision.
    let newest_hash = replica_sets
        .iter()
        .find(|rs| rs.annotations().get("deployment.kubernetes.io/revision") == revision.as_ref())
        .and_then(|rs| rs.labels().get("pod-template-hash").cloned());
    let mut info = vec![("Strategy".to_owned(), strategy)];
    push(&mut info, "Revision", revision);
    push(&mut info, "Min ready", spec.min_ready_seconds.filter(|s| *s > 0).map(|s| format!("{s}s")));
    push(&mut info, "Progress deadline", spec.progress_deadline_seconds.filter(|s| *s != 600).map(|s| format!("{s}s")));
    let owners = replica_sets.iter().map(|rs| ("ReplicaSet".to_owned(), rs.name_any())).collect();
    common(
        "Deployment",
        d.metadata,
        spec.selector,
        spec.template,
        state,
        replicas,
        info,
        conditions,
        owners,
        Box::new(move |pod: &Pod| newest_hash.is_some() && pod.labels().get("pod-template-hash") == newest_hash.as_ref()),
    )
}

fn statefulset(sts: StatefulSet) -> Workload {
    let spec = sts.spec.clone().unwrap_or_default();
    let status = sts.status.clone().unwrap_or_default();
    let replicas = ReplicaCounts {
        desired: spec.replicas.unwrap_or(1),
        current: status.replicas,
        ready: status.ready_replicas.unwrap_or(0),
        updated: status.updated_replicas.unwrap_or(0),
        available: status.available_replicas.unwrap_or(0),
    };
    let conditions = status
        .conditions
        .iter()
        .flatten()
        .map(|c| Condition {
            type_: c.type_.clone(),
            status: c.status.clone(),
            reason: c.reason.clone().unwrap_or_default(),
            message: c.message.clone().unwrap_or_default(),
            last_transition: c.last_transition_time.as_ref().map(|t| t.0.to_string()),
        })
        .collect();
    let (current_rev, update_rev) = (status.current_revision.clone(), status.update_revision.clone());
    let stale = status.observed_generation.unwrap_or(0) < sts.metadata.generation.unwrap_or(0);
    let state = if replicas.desired == 0 && replicas.current == 0 {
        "Scaled to 0"
    } else if stale || (update_rev.is_some() && current_rev != update_rev) || replicas.updated < replicas.desired {
        "Rolling out"
    } else if replicas.ready < replicas.desired {
        "Degraded"
    } else {
        "Ready"
    };

    let strategy = spec.update_strategy.clone().unwrap_or_default();
    let strategy = match strategy.type_.as_deref() {
        Some("OnDelete") => "OnDelete (pods update only when deleted)".to_owned(),
        _ => {
            let ru = strategy.rolling_update.unwrap_or_default();
            let mut s = "RollingUpdate".to_owned();
            if let Some(p) = ru.partition.filter(|p| *p > 0) {
                s += &format!(" · partition {p}");
            }
            if let Some(m) = ru.max_unavailable.as_ref() {
                s += &format!(" · max unavailable {}", int_or_string(Some(m), "1"));
            }
            s
        }
    };
    let claims: Vec<String> = spec
        .volume_claim_templates
        .iter()
        .flatten()
        .map(|t| {
            let pvc = t.spec.clone().unwrap_or_default();
            let size = pvc
                .resources
                .and_then(|r| r.requests)
                .and_then(|r| r.get("storage").map(|q| q.0.clone()));
            let details: Vec<String> = size.into_iter().chain(pvc.storage_class_name).collect();
            if details.is_empty() {
                t.name_any()
            } else {
                format!("{} ({})", t.name_any(), details.join(", "))
            }
        })
        .collect();
    let retention = spec.persistent_volume_claim_retention_policy.as_ref().map(|r| {
        format!(
            "when deleted: {} · when scaled: {}",
            r.when_deleted.as_deref().unwrap_or("Retain"),
            r.when_scaled.as_deref().unwrap_or("Retain"),
        )
    });
    let revision = match (&current_rev, &update_rev) {
        (Some(c), Some(u)) if c != u => Some(format!("{c} → {u}")),
        (Some(c), _) => Some(c.clone()),
        (None, u) => u.clone(),
    };

    let mut info = vec![("Strategy".to_owned(), strategy)];
    push(&mut info, "Pod management", spec.pod_management_policy.clone());
    push(&mut info, "Service", spec.service_name.clone().filter(|s| !s.is_empty()));
    push(&mut info, "Volume claims", (!claims.is_empty()).then(|| claims.join(", ")));
    push(&mut info, "Claim retention", retention);
    push(&mut info, "Revision", revision);
    push(&mut info, "Min ready", spec.min_ready_seconds.filter(|s| *s > 0).map(|s| format!("{s}s")));
    let name = sts.name_any();
    common(
        "StatefulSet",
        sts.metadata,
        spec.selector,
        spec.template,
        state,
        replicas,
        info,
        conditions,
        vec![("StatefulSet".to_owned(), name)],
        Box::new(move |pod: &Pod| {
            update_rev.is_some() && pod.labels().get("controller-revision-hash") == update_rev.as_ref()
        }),
    )
}

/// Adds the rows every workload has, after the kind-specific ones.
#[allow(clippy::too_many_arguments)]
fn common(
    kind: &str,
    meta: ObjectMeta,
    selector: LabelSelector,
    template: PodTemplateSpec,
    status: &str,
    replicas: ReplicaCounts,
    mut info: Vec<(String, String)>,
    conditions: Vec<Condition>,
    owners: Vec<(String, String)>,
    is_updated: Box<dyn Fn(&Pod) -> bool + Send + Sync>,
) -> Workload {
    let annotations = template.metadata.as_ref().and_then(|m| m.annotations.as_ref());
    push(&mut info, "Last restart", annotations.and_then(|a| a.get(crate::actions::RESTARTED_AT).cloned()));
    push(&mut info, "Selector", Some(selector_string(&selector)).filter(|s| !s.is_empty()));
    let spec = template.spec.as_ref();
    push(&mut info, "Service account", spec.and_then(|s| s.service_account_name.clone()));
    push(&mut info, "Priority class", spec.and_then(|s| s.priority_class_name.clone()));
    let owner = meta.owner_references.iter().flatten().find(|o| o.controller == Some(true));
    push(&mut info, "Controlled by", owner.map(|o| format!("{}/{}", o.kind, o.name)));
    Workload {
        kind: kind.to_owned(),
        meta,
        selector,
        template,
        status: status.to_owned(),
        replicas,
        info,
        conditions,
        is_updated,
        owners,
    }
}

/// Pods matching the selector that one of `owners` controls.
async fn owned_pods(client: &Client, namespace: &str, selector: &str, owners: &[(String, String)]) -> Result<Vec<Pod>> {
    let owners: HashSet<(&str, &str)> = owners.iter().map(|(k, n)| (k.as_str(), n.as_str())).collect();
    let mut pods: Vec<Pod> = Api::<Pod>::namespaced(client.clone(), namespace)
        .list(&ListParams::default().labels(selector))
        .await?
        .items
        .into_iter()
        .filter(|p| {
            p.owner_references()
                .iter()
                .any(|o| o.controller == Some(true) && owners.contains(&(o.kind.as_str(), o.name.as_str())))
        })
        .collect();
    // StatefulSet pods by ordinal; others by name.
    pods.sort_by_key(|p| {
        let name = p.name_any();
        let ordinal = name.rsplit('-').next().and_then(|n| n.parse::<u64>().ok());
        (ordinal, name)
    });
    Ok(pods)
}

fn workload_pod(pod: &Pod, w: &Workload, usage: Option<&PodsUsage>) -> WorkloadPod {
    let data = serde_json::to_value(pod).unwrap_or_default();
    let containers = container_states(&data);
    let statuses = pod.status.as_ref().and_then(|s| s.container_statuses.as_ref());
    let ready = statuses.map_or(0, |s| s.iter().filter(|c| c.ready).count());
    let total = pod.spec.as_ref().map_or(0, |s| s.containers.len());
    let name = pod.name_any();
    let used = usage.and_then(|u| u.get(&name)).map(|c| summed(c.values().copied()));
    WorkloadPod {
        status: pod_status(&data, pod.metadata.deletion_timestamp.is_some()),
        containers,
        ready: format!("{ready}/{total}"),
        restarts: statuses.map_or(0, |s| s.iter().map(|c| i64::from(c.restart_count)).sum()),
        node: pod.spec.as_ref().and_then(|s| s.node_name.clone()),
        created: pod.creation_timestamp().map(|t| t.0.to_string()),
        updated: (w.is_updated)(pod),
        cpu: used.map(|u| u.0),
        memory: used.map(|u| u.1),
        name,
    }
}

fn controlled_by(owners: &[k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference], kind: &str, name: &str) -> bool {
    owners.iter().any(|o| o.controller == Some(true) && o.kind == kind && o.name == name)
}

fn finished(pod: &Pod) -> bool {
    matches!(pod.status.as_ref().and_then(|s| s.phase.as_deref()), Some("Succeeded" | "Failed"))
}

/// kubectl's selector syntax: `app=api,tier in (web,edge),!canary`.
pub(crate) fn selector_string(selector: &LabelSelector) -> String {
    let labels = selector.match_labels.iter().flatten().map(|(k, v)| format!("{k}={v}"));
    let expressions = selector.match_expressions.iter().flatten().map(|e| {
        let values = e.values.clone().unwrap_or_default().join(",");
        match e.operator.as_str() {
            "In" => format!("{} in ({values})", e.key),
            "NotIn" => format!("{} notin ({values})", e.key),
            "Exists" => e.key.clone(),
            "DoesNotExist" => format!("!{}", e.key),
            op => format!("{} {op} ({values})", e.key),
        }
    });
    labels.chain(expressions).collect::<Vec<_>>().join(",")
}

fn int_or_string(v: Option<&IntOrString>, default: &str) -> String {
    match v {
        Some(IntOrString::Int(i)) => i.to_string(),
        Some(IntOrString::String(s)) => s.clone(),
        None => default.to_owned(),
    }
}

fn push(info: &mut Vec<(String, String)>, key: &str, value: Option<String>) {
    if let Some(v) = value.filter(|v| !v.is_empty()) {
        info.push((key.to_owned(), v));
    }
}

fn summed(values: impl Iterator<Item = (f64, f64)>) -> (f64, f64) {
    values.fold((0.0, 0.0), |(c, m), (dc, dm)| (c + dc, m + dm))
}

fn positive(v: f64) -> Option<f64> {
    (v > 0.0).then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn selector_syntax() {
        let selector: LabelSelector = serde_json::from_value(json!({
            "matchLabels": {"app": "api"},
            "matchExpressions": [
                {"key": "tier", "operator": "In", "values": ["web", "edge"]},
                {"key": "canary", "operator": "DoesNotExist"}
            ]
        }))
        .unwrap();
        assert_eq!(selector_string(&selector), "app=api,tier in (web,edge),!canary");
    }

    #[test]
    fn deployment_status() {
        let d = |spec: serde_json::Value, status: serde_json::Value| -> String {
            let d: Deployment = serde_json::from_value(json!({
                "metadata": {"name": "api", "generation": 2},
                "spec": spec,
                "status": status,
            }))
            .unwrap();
            deployment_workload(d, vec![]).status
        };
        let sel = json!({"matchLabels": {"app": "api"}});
        let steady = json!({"observedGeneration": 2, "replicas": 3, "readyReplicas": 3, "updatedReplicas": 3, "availableReplicas": 3});
        assert_eq!(d(json!({"replicas": 3, "selector": sel, "template": {}}), steady.clone()), "Available");
        assert_eq!(d(json!({"replicas": 3, "selector": sel, "template": {}, "paused": true}), steady.clone()), "Paused");
        let rolling = json!({"observedGeneration": 2, "replicas": 4, "readyReplicas": 3, "updatedReplicas": 1, "availableReplicas": 3});
        assert_eq!(d(json!({"replicas": 3, "selector": sel, "template": {}}), rolling), "Rolling out");
        let degraded = json!({"observedGeneration": 2, "replicas": 3, "readyReplicas": 2, "updatedReplicas": 3, "availableReplicas": 2});
        assert_eq!(d(json!({"replicas": 3, "selector": sel, "template": {}}), degraded), "Degraded");
        assert_eq!(d(json!({"replicas": 0, "selector": sel, "template": {}}), json!({"observedGeneration": 2})), "Scaled to 0");
    }

    #[test]
    fn pod_name_patterns() {
        let kind = |k: &str| KindRef {
            group: "apps".into(),
            version: "v1".into(),
            kind: k.into(),
            plural: String::new(),
            namespaced: true,
        };
        assert_eq!(pod_name_regex(&kind("StatefulSet"), "db"), "db-[0-9]+");
        assert_eq!(pod_name_regex(&kind("Deployment"), "a.b"), r"a\\.b-[a-z0-9]{1,10}-[a-z0-9]{5}");
    }
}
