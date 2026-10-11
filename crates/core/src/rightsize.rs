//! Applying suggested requests from a trace review: one strategic-merge
//! patch per workload, covering every chosen container, so it rolls out
//! once. A limit below a new request is raised to match (the API refuses a
//! request above its limit); limits are otherwise left alone. The preview
//! reads the workload and asks the server for a dry run; applying is gated.

use kube::api::{Patch, PatchParams};
use kube::{Client, ResourceExt};
use serde_json::{Value, json};

use crate::cluster::dynamic_api;
use crate::quantity::{bytes, millicores};
use crate::safety::SafetyGate;
use crate::types::{ContainerPreview, KindRef, QuantityChange, RightsizeChange, RightsizePreview};
use crate::{Error, Result};

const MIB: f64 = 1024.0 * 1024.0;

/// The workload kinds whose pod template can be changed, and where that
/// template lives.
fn kind_ref(kind: &str) -> Option<(KindRef, &'static [&'static str])> {
    const POD: &[&str] = &["spec", "template", "spec", "containers"];
    const CRON: &[&str] = &["spec", "jobTemplate", "spec", "template", "spec", "containers"];
    let (group, plural, path) = match kind {
        "Deployment" => ("apps", "deployments", POD),
        "StatefulSet" => ("apps", "statefulsets", POD),
        "DaemonSet" => ("apps", "daemonsets", POD),
        "CronJob" => ("batch", "cronjobs", CRON),
        _ => return None,
    };
    Some((KindRef { group: group.into(), version: "v1".into(), kind: kind.into(), plural: plural.into(), namespaced: true }, path))
}

/// Why a kind can't take new requests here.
fn unsupported(kind: &str) -> String {
    match kind {
        "Job" => "A Job's pod template can't be changed once created; change it where the Job is defined.".into(),
        "Pod" => "A bare pod's requests can't be changed here; change it where it's created.".into(),
        "ReplicaSet" => "Change the Deployment that owns this ReplicaSet, or the ReplicaSet's definition.".into(),
        other => format!("Requests can't be changed on a {other} here."),
    }
}

/// `15m`, `1500m`.
pub fn cpu_quantity(millicores: f64) -> String {
    format!("{}m", millicores.round().max(1.0) as u64)
}

/// `2944Mi` for whole MiB, else bytes.
pub fn memory_quantity(bytes: f64) -> String {
    let b = bytes.round().max(1.0);
    if (b / MIB).fract() == 0.0 { format!("{}Mi", (b / MIB) as u64) } else { format!("{}", b as u64) }
}

/// The containers list in the workload's template.
fn template_containers<'a>(data: &'a Value, path: &[&str]) -> Option<&'a Vec<Value>> {
    path.iter().try_fold(data, |v, key| v.get(*key))?.as_array()
}

fn str_at(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().try_fold(v, |v, k| v.get(*k))?.as_str().map(str::to_owned)
}

/// What each chosen container's requests and limits become.
fn plan_containers(change: &RightsizeChange, containers: &[Value]) -> Vec<ContainerPreview> {
    change
        .containers
        .iter()
        .map(|want| {
            let Some(c) = containers.iter().find(|c| c["name"] == want.container.as_str()) else {
                return ContainerPreview {
                    container: want.container.clone(),
                    injected: true,
                    cpu_request: QuantityChange::default(),
                    cpu_limit: QuantityChange::default(),
                    mem_request: QuantityChange::default(),
                    mem_limit: QuantityChange::default(),
                };
            };
            let get = |section: &str, key: &str| str_at(c, &["resources", section, key]);
            let (cpu_req, cpu_lim) = (get("requests", "cpu"), get("limits", "cpu"));
            let (mem_req, mem_lim) = (get("requests", "memory"), get("limits", "memory"));
            let new_cpu = want.cpu_request.map(cpu_quantity).or(cpu_req.clone());
            let new_mem = want.mem_request.map(memory_quantity).or(mem_req.clone());
            // A limit below the new request rises to it.
            let raise = |limit: &Option<String>, request: Option<f64>, parse: fn(&str) -> f64, fmt: fn(f64) -> String| {
                match (limit, request) {
                    (Some(l), Some(r)) if parse(l) < r => Some(fmt(r)),
                    _ => limit.clone(),
                }
            };
            let new_cpu_lim = raise(&cpu_lim, want.cpu_request, millicores, cpu_quantity);
            let new_mem_lim = raise(&mem_lim, want.mem_request, bytes, memory_quantity);
            ContainerPreview {
                container: want.container.clone(),
                injected: false,
                cpu_request: QuantityChange { before: cpu_req, after: new_cpu },
                cpu_limit: QuantityChange { before: cpu_lim, after: new_cpu_lim },
                mem_request: QuantityChange { before: mem_req, after: new_mem },
                mem_limit: QuantityChange { before: mem_lim, after: new_mem_lim },
            }
        })
        .collect()
}

/// The strategic-merge patch for the changed containers (merged by name).
fn patch_body(path: &[&str], planned: &[ContainerPreview]) -> Option<Value> {
    let containers: Vec<Value> = planned
        .iter()
        .filter(|c| !c.injected)
        .filter_map(|c| {
            let section = |cpu: &QuantityChange, memory: &QuantityChange| -> serde_json::Map<String, Value> {
                [("cpu", cpu), ("memory", memory)]
                    .into_iter()
                    .filter(|(_, q)| q.changed())
                    .filter_map(|(key, q)| Some((key.to_owned(), json!(q.after.as_ref()?))))
                    .collect()
            };
            let requests = section(&c.cpu_request, &c.mem_request);
            let limits = section(&c.cpu_limit, &c.mem_limit);
            if requests.is_empty() && limits.is_empty() {
                return None;
            }
            let mut resources = serde_json::Map::new();
            if !requests.is_empty() {
                resources.insert("requests".into(), Value::Object(requests));
            }
            if !limits.is_empty() {
                resources.insert("limits".into(), Value::Object(limits));
            }
            Some(json!({"name": c.container, "resources": resources}))
        })
        .collect();
    if containers.is_empty() {
        return None;
    }
    // Build {spec: {template: {spec: {containers: [...]}}}} from the path.
    Some(path.iter().rev().fold(Value::Array(containers), |inner, key| json!({ *key: inner })))
}

/// Reads the workload, plans the change and asks the server for a dry run.
/// Changes nothing, so not gated.
pub async fn preview(client: &Client, change: &RightsizeChange) -> Result<RightsizePreview> {
    let mut out = RightsizePreview {
        change: change.clone(),
        helm_release: None,
        helm_snippet: None,
        containers: vec![],
        blocked: None,
        dry_run_error: None,
        effect: String::new(),
    };
    let Some((kind, path)) = kind_ref(&change.kind) else {
        out.blocked = Some(unsupported(&change.kind));
        return Ok(out);
    };
    out.effect = if change.kind == "CronJob" {
        "Applies from the next scheduled run; running Jobs keep their requests.".into()
    } else {
        "Rolls out new pods, following the workload's update strategy.".into()
    };
    let api = dynamic_api(client, &kind, Some(&change.namespace));
    let obj = api.get(&change.name).await?;
    if obj.labels().get("app.kubernetes.io/managed-by").map(String::as_str) == Some("Helm") {
        out.helm_release = Some(obj.annotations().get("meta.helm.sh/release-name").cloned().unwrap_or_else(|| "unknown".into()));
    }
    let containers = template_containers(&obj.data, path).cloned().unwrap_or_default();
    out.containers = plan_containers(change, &containers);
    out.helm_snippet = out.helm_release.as_deref().map(|r| helm_snippet(r, &out));
    match patch_body(path, &out.containers) {
        None if out.containers.iter().all(|c| c.injected) => {
            out.blocked = Some("These containers aren't in the workload's pod template; an admission webhook adds them.".into());
        }
        None => out.blocked = Some("Nothing to change: the requests already match.".into()),
        Some(body) => {
            let params = PatchParams { dry_run: true, ..PatchParams::default() };
            if let Err(e) = api.patch(&change.name, &params, &Patch::Strategic(&body)).await {
                out.dry_run_error = Some(Error::from(e).user_message());
            }
        }
    }
    Ok(out)
}

/// Applies a change after the same planning as [`preview`]. Gated; the
/// workload is read again so the patch matches its current template.
pub async fn apply(client: &Client, gate: &SafetyGate, context: &str, change: &RightsizeChange) -> Result<()> {
    gate.ensure_writable(context)?;
    let (kind, path) = kind_ref(&change.kind).ok_or_else(|| Error::Message(unsupported(&change.kind)))?;
    let api = dynamic_api(client, &kind, Some(&change.namespace));
    let obj = api.get(&change.name).await?;
    let containers = template_containers(&obj.data, path).cloned().unwrap_or_default();
    let planned = plan_containers(change, &containers);
    let body = patch_body(path, &planned).ok_or_else(|| Error::Message("nothing to change".into()))?;
    api.patch(&change.name, &PatchParams::default(), &Patch::Strategic(&body)).await?;
    tracing::info!(%context, kind = %change.kind, namespace = %change.namespace, name = %change.name, patch = %body, "requests changed");
    Ok(())
}

/// Values to put in a Helm chart so its next upgrade keeps the new requests.
/// Charts name these differently, so it's a `resources:` block per
/// container with a comment saying where it belongs.
pub fn helm_snippet(release: &str, preview: &RightsizePreview) -> String {
    let mut out = format!(
        "# Helm release {release}: {} {}/{}\n# Put each block under that container's resources in the chart's values.\n",
        preview.change.kind, preview.change.namespace, preview.change.name
    );
    for c in preview.containers.iter().filter(|c| !c.injected) {
        out.push_str(&format!("\n# container: {}\nresources:\n", c.container));
        for (section, cpu, mem) in [("requests", &c.cpu_request, &c.mem_request), ("limits", &c.cpu_limit, &c.mem_limit)] {
            let lines: Vec<String> = [("cpu", cpu), ("memory", mem)]
                .into_iter()
                .filter_map(|(k, q)| q.after.as_ref().map(|v| format!("    {k}: {v}")))
                .collect();
            if !lines.is_empty() {
                out.push_str(&format!("  {section}:\n{}\n", lines.join("\n")));
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "rightsize_tests.rs"]
mod tests;
