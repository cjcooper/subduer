//! The Details tab of a Namespace: its state (and what a stuck deletion is
//! waiting for), pod security, what it contains, the resources its pods ask
//! for and use against its quota, its quotas, limit ranges and Helm releases,
//! and recent warnings from anything in it. Plus creating a namespace.

use std::collections::BTreeMap;

use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, StatefulSet};
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{
    ConfigMap, LimitRange, Namespace, PersistentVolumeClaim, Pod, ResourceQuota, Secret, Service, ServiceAccount,
};
use k8s_openapi::api::networking::v1::{Ingress, NetworkPolicy};
use k8s_openapi::api::rbac::v1::{Role, RoleBinding};
use kube::api::{Api, ListParams, ObjectMeta, PostParams};
use kube::{Client, Resource, ResourceExt};
use serde_json::Value;

use crate::config::{plural, str_at, table_section};
use crate::safety::SafetyGate;
use crate::types::{InfoRow, InfoTable, InfoTableRow, ObjectDetails, ObjectLink};
use crate::{Error, Result};

/// Fills in what comes from the Namespace object itself.
pub(crate) fn describe(key: &str, d: &mut ObjectDetails, v: &Value) {
    if key != "core/Namespace" {
        return;
    }
    let phase = str_at(&v["status"], "phase").unwrap_or_else(|| "Active".into());
    let labels = &v["metadata"]["labels"];
    let mut security = Vec::new();
    for mode in ["enforce", "warn", "audit"] {
        if let Some(level) = str_at(labels, &format!("pod-security.kubernetes.io/{mode}")) {
            let version = str_at(labels, &format!("pod-security.kubernetes.io/{mode}-version"))
                .filter(|v| v != "latest")
                .map(|v| format!(" ({v})"))
                .unwrap_or_default();
            security.push(format!("{mode} {level}{version}"));
        }
    }
    d.info.push(InfoRow::new(
        "Pod security",
        if security.is_empty() { "not set: the cluster default applies".to_owned() } else { security.join(" · ") },
    ));
    if phase == "Terminating" {
        let finalizers: Vec<String> = v["spec"]["finalizers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| f.as_str().map(str::to_owned))
            .collect();
        let waiting: Vec<String> = d
            .conditions
            .iter()
            .filter(|c| c.status == "True" && !c.message.is_empty())
            .map(|c| c.message.clone())
            .collect();
        d.info.push(InfoRow::new(
            "Deleting",
            if waiting.is_empty() {
                "removing what's left in it".to_owned()
            } else {
                format!("waiting: {}", waiting.join("; "))
            },
        ));
        if !finalizers.is_empty() {
            d.info.push(InfoRow::new("Finalizers", finalizers.join(", ")).mono());
        }
    }
    d.status = Some(phase);
}

fn link_list(kind: &str, namespace: &str) -> ObjectLink {
    // An empty name opens the kind's list, filtered to the namespace.
    ObjectLink { kind: kind.into(), namespace: Some(namespace.into()), name: String::new() }
}

/// How many objects of a kind the namespace holds; `None` if it can't be listed.
async fn count<K>(client: &Client, namespace: &str) -> Option<usize>
where
    K: Resource<Scope = k8s_openapi::NamespaceResourceScope> + Clone + serde::de::DeserializeOwned + std::fmt::Debug,
    K::DynamicType: Default,
{
    let api: Api<K> = Api::namespaced(client.clone(), namespace);
    api.list_metadata(&ListParams::default()).await.ok().map(|l| l.items.len())
}

/// Adds what needs other objects.
pub(crate) async fn related(client: &Client, key: &str, name: &str, d: &mut ObjectDetails) -> Result<()> {
    if key != "core/Namespace" {
        return Ok(());
    }
    let ns = name;
    let pods = Api::<Pod>::namespaced(client.clone(), ns);
    let all = ListParams::default();
    let (pods, deployments, statefulsets, daemonsets, jobs, cronjobs, services, ingresses) = tokio::join!(
        pods.list(&all),
        count::<Deployment>(client, ns),
        count::<StatefulSet>(client, ns),
        count::<DaemonSet>(client, ns),
        count::<Job>(client, ns),
        count::<CronJob>(client, ns),
        count::<Service>(client, ns),
        count::<Ingress>(client, ns),
    );
    let (configmaps, secrets, claims, accounts, roles, bindings, policies) = tokio::join!(
        count::<ConfigMap>(client, ns),
        // Metadata only: no Secret data is read.
        count::<Secret>(client, ns),
        count::<PersistentVolumeClaim>(client, ns),
        count::<ServiceAccount>(client, ns),
        count::<Role>(client, ns),
        count::<RoleBinding>(client, ns),
        count::<NetworkPolicy>(client, ns),
    );
    let pods = pods.map(|l| l.items).unwrap_or_default();

    // Contents.
    let mut phases: BTreeMap<String, usize> = BTreeMap::new();
    for p in &pods {
        let phase = p.status.as_ref().and_then(|s| s.phase.clone()).unwrap_or_else(|| "Unknown".into());
        *phases.entry(phase.to_lowercase()).or_default() += 1;
    }
    let pod_text = if pods.is_empty() {
        "0".to_owned()
    } else {
        let parts: Vec<String> = phases.iter().map(|(p, n)| format!("{n} {p}")).collect();
        format!("{} ({})", pods.len(), parts.join(", "))
    };
    let mut rows = vec![{
        let mut r = InfoTableRow::new(vec!["Pods".into(), pod_text]);
        r.link = Some(link_list("core/Pod", ns));
        r.warn = phases.get("failed").is_some_and(|n| *n > 0) || phases.get("pending").is_some_and(|n| *n > 0);
        r
    }];
    let kinds: [(&str, &str, Option<usize>); 14] = [
        ("Deployments", "apps/Deployment", deployments),
        ("StatefulSets", "apps/StatefulSet", statefulsets),
        ("DaemonSets", "apps/DaemonSet", daemonsets),
        ("Jobs", "batch/Job", jobs),
        ("CronJobs", "batch/CronJob", cronjobs),
        ("Services", "core/Service", services),
        ("Ingresses", "networking.k8s.io/Ingress", ingresses),
        ("NetworkPolicies", "networking.k8s.io/NetworkPolicy", policies),
        ("ConfigMaps", "core/ConfigMap", configmaps),
        ("Secrets", "core/Secret", secrets),
        ("PersistentVolumeClaims", "core/PersistentVolumeClaim", claims),
        ("ServiceAccounts", "core/ServiceAccount", accounts),
        ("Roles", "rbac.authorization.k8s.io/Role", roles),
        ("RoleBindings", "rbac.authorization.k8s.io/RoleBinding", bindings),
    ];
    for (label, kind, n) in kinds {
        let Some(n) = n.filter(|n| *n > 0) else { continue };
        let mut r = InfoTableRow::new(vec![label.into(), n.to_string()]);
        r.link = Some(link_list(kind, ns));
        rows.push(r);
    }
    let objects: usize = pods.len() + kinds.iter().filter_map(|k| k.2).sum::<usize>();
    let mut contents = table_section(
        "ns-contents",
        "Contents",
        InfoTable { columns: vec!["Kind".into(), "How many".into()], numeric: vec![1], ..Default::default() },
        "",
    );
    contents.table.as_mut().unwrap().rows = rows;
    contents.summary = plural(objects, "object");
    d.sections.push(contents);

    // Resources: what running pods ask for and use, against the quota.
    let running: Vec<&Pod> = pods
        .iter()
        .filter(|p| !matches!(p.status.as_ref().and_then(|s| s.phase.as_deref()), Some("Succeeded" | "Failed")))
        .collect();
    let asked = running
        .iter()
        .filter_map(|p| p.spec.as_ref())
        .map(crate::overview::pod_resources)
        .fold((0.0, 0.0, 0.0, 0.0), |a, r| {
            (a.0 + r.cpu_requests, a.1 + r.cpu_limits, a.2 + r.memory_requests, a.3 + r.memory_limits)
        });
    let (quota_api, range_api) = (
        Api::<ResourceQuota>::namespaced(client.clone(), ns),
        Api::<LimitRange>::namespaced(client.clone(), ns),
    );
    let (used, quotas, limit_ranges) = tokio::join!(
        crate::pod_details::namespace_usage(client, ns),
        quota_api.list(&all),
        range_api.list_metadata(&all),
    );
    let quotas = quotas.map(|l| l.items).unwrap_or_default();
    // The tightest quota on requests, as a fraction to fill the bar.
    let hard = |names: &[&str]| -> Option<f64> {
        quotas
            .iter()
            .filter_map(|q| q.status.as_ref().and_then(|s| s.hard.as_ref()))
            .filter_map(|h| names.iter().find_map(|n| h.get(*n)).and_then(|q| crate::quantity::parse(&q.0)))
            .reduce(f64::min)
    };
    // A total limit means something only if every container has one.
    let unlimited = |resource: &str| {
        running
            .iter()
            .filter(|p| {
                p.spec.as_ref().is_some_and(|s| {
                    s.containers.iter().any(|c| {
                        c.resources.as_ref().and_then(|r| r.limits.as_ref()).is_none_or(|l| !l.contains_key(resource))
                    })
                })
            })
            .count()
    };
    let (cpu_open, mem_open) = (unlimited("cpu"), unlimited("memory"));
    let cpu = |m: f64| if m >= 1000.0 { format!("{:.2} cores", m / 1000.0) } else { format!("{m:.0}m") };
    let mem = |b: f64| crate::storage::bytes_text(b);
    #[allow(clippy::too_many_arguments)]
    let resource_row = |label: &str, used: Option<f64>, req: f64, lim: f64, open: usize, quota: Option<f64>, fmt: &dyn Fn(f64) -> String, scale: f64| {
        let limits = match open {
            0 if lim > 0.0 => fmt(lim),
            0 => "—".into(),
            n => format!("not set on {}", plural(n, "pod")),
        };
        let mut r = InfoTableRow::new(vec![
            label.into(),
            used.map_or_else(|| "—".into(), fmt),
            fmt(req),
            limits,
            quota.map_or_else(|| "none".into(), |q| fmt(q * scale)),
        ]);
        r.fill = quota.filter(|q| *q > 0.0).map(|q| (req / (q * scale)).min(1.0));
        r.warn = r.fill.is_some_and(|f| f >= 0.9);
        r
    };
    let resources = InfoTable {
        columns: ["", "Used now", "Requested", "Limits", "Quota on requests"].map(String::from).to_vec(),
        numeric: vec![1, 2, 3, 4],
        rows: vec![
            resource_row("CPU", used.map(|u| u.0), asked.0, asked.1, cpu_open, hard(&["requests.cpu", "cpu"]), &cpu, 1000.0),
            resource_row("Memory", used.map(|u| u.1), asked.2, asked.3, mem_open, hard(&["requests.memory", "memory"]), &mem, 1.0),
        ],
        ..Default::default()
    };
    let mut section = table_section("ns-resources", "Resources", resources, "");
    section.summary = format!("{} running", plural(running.len(), "pod"));
    if used.is_none() {
        section.rows.push(InfoRow::new("Used now", "unknown: metrics-server isn't installed"));
    }
    d.sections.insert(0, section);

    // Quotas, limit ranges and Helm releases.
    let mut rows: Vec<InfoTableRow> = Vec::new();
    for q in &quotas {
        let mut r = InfoTableRow::new(vec!["ResourceQuota".into(), q.name_any()]);
        r.link = Some(ObjectLink { kind: "core/ResourceQuota".into(), namespace: Some(ns.into()), name: q.name_any() });
        rows.push(r);
    }
    for l in limit_ranges.map(|l| l.items).unwrap_or_default() {
        let mut r = InfoTableRow::new(vec!["LimitRange".into(), l.name_any()]);
        r.link = Some(ObjectLink { kind: "core/LimitRange".into(), namespace: Some(ns.into()), name: l.name_any() });
        rows.push(r);
    }
    let helm = Api::<Secret>::namespaced(client.clone(), ns)
        .list_metadata(&ListParams::default().labels("owner=helm"))
        .await
        .map(|l| l.items)
        .unwrap_or_default();
    let mut releases: Vec<String> = helm.iter().filter_map(|s| s.labels().get("name").cloned()).collect();
    releases.sort();
    releases.dedup();
    for r in releases {
        let mut row = InfoTableRow::new(vec!["Helm release".into(), r.clone()]);
        row.link = Some(ObjectLink { kind: "helm.sh/Release".into(), namespace: Some(ns.into()), name: r });
        rows.push(row);
    }
    if !rows.is_empty() {
        let n = rows.len();
        let mut s = table_section(
            "ns-related",
            "Quotas, limits and releases",
            InfoTable { columns: vec!["Kind".into(), "Name".into()], rows, ..Default::default() },
            "",
        );
        s.summary = n.to_string();
        d.sections.push(s);
    }

    // Recent warnings from anything in it.
    let mut warnings = crate::pod_details::list_events(client, ns, Some("type=Warning"), |e| {
        let o = &e.involved_object;
        Some(Some(format!("{} {}", o.kind.clone().unwrap_or_default(), o.name.clone().unwrap_or_default())))
    })
    .await;
    warnings.truncate(20);
    d.events = warnings;
    Ok(())
}

/// DNS-1123 label: lowercase letters, digits and `-`, starting and ending
/// with a letter or digit, at most 63 characters.
pub fn valid_namespace_name(name: &str) -> bool {
    let ok_char = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    !name.is_empty()
        && name.len() <= 63
        && name.starts_with(ok_char)
        && name.ends_with(ok_char)
        && name.chars().all(|c| ok_char(c) || c == '-')
}

/// Creates a namespace. A `dry_run` is validated by the server only.
pub async fn create_namespace(client: &Client, gate: &SafetyGate, context: &str, name: &str, dry_run: bool) -> Result<()> {
    if !valid_namespace_name(name) {
        return Err(Error::Message(
            "use lowercase letters, digits and '-', starting and ending with a letter or digit (63 at most)".into(),
        ));
    }
    if !dry_run {
        gate.ensure_writable(context)?;
    }
    let ns = Namespace { metadata: ObjectMeta { name: Some(name.into()), ..ObjectMeta::default() }, ..Namespace::default() };
    let params = PostParams { dry_run, ..PostParams::default() };
    Api::<Namespace>::all(client.clone()).create(&params, &ns).await?;
    tracing::info!(%context, %name, dry_run, "namespace created");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::describe;
    use serde_json::json;

    fn now() -> jiff::Timestamp {
        "2026-10-10T12:00:00Z".parse().unwrap()
    }

    fn row<'a>(d: &'a ObjectDetails, key: &str) -> Option<&'a str> {
        d.info.iter().find(|r| r.key == key).map(|r| r.value.as_str())
    }

    #[test]
    fn active_and_stuck_namespaces() {
        let active = json!({"kind": "Namespace", "metadata": {"name": "shop",
            "labels": {"pod-security.kubernetes.io/enforce": "baseline", "pod-security.kubernetes.io/warn": "restricted",
                       "pod-security.kubernetes.io/warn-version": "v1.30"}},
            "status": {"phase": "Active"}});
        let d = describe("core/Namespace", &active, now());
        assert_eq!(d.status.as_deref(), Some("Active"));
        assert_eq!(row(&d, "Pod security"), Some("enforce baseline · warn restricted (v1.30)"));

        let stuck = json!({"kind": "Namespace", "metadata": {"name": "old"},
            "spec": {"finalizers": ["kubernetes"]},
            "status": {"phase": "Terminating", "conditions": [
                {"type": "NamespaceContentRemaining", "status": "True", "message": "Some resources are remaining: widgets.acme.io has 1 resource instances"},
                {"type": "NamespaceDeletionDiscoveryFailure", "status": "False", "message": "All resources successfully discovered"}]}});
        let d = describe("core/Namespace", &stuck, now());
        assert_eq!(d.status.as_deref(), Some("Terminating"));
        assert_eq!(row(&d, "Deleting"), Some("waiting: Some resources are remaining: widgets.acme.io has 1 resource instances"));
        assert_eq!(row(&d, "Finalizers"), Some("kubernetes"));
        assert_eq!(row(&d, "Pod security"), Some("not set: the cluster default applies"));
    }

    #[test]
    fn namespace_names() {
        assert!(valid_namespace_name("shop-2"));
        for bad in ["", "Shop", "-shop", "shop-", "a_b", "a.b", &"x".repeat(64)] {
            assert!(!valid_namespace_name(bad), "{bad}");
        }
    }

    #[tokio::test]
    async fn creating_is_gated() {
        let client = Client::try_from(kube::Config::new("http://127.0.0.1:9".parse().unwrap())).unwrap();
        let gate = SafetyGate::default();
        let err = create_namespace(&client, &gate, "acme-prd", "shop", false).await.unwrap_err();
        assert!(matches!(err, Error::Locked(_)));
        let err = create_namespace(&client, &gate, "acme-prd", "Bad_Name", false).await.unwrap_err();
        assert!(matches!(err, Error::Message(_)));
    }
}
