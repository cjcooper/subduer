//! Operations that change cluster state. Each one checks the safety gate
//! before touching the cluster.

use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, OwnerReference};
use kube::api::{Api, DeleteParams, Patch, PatchParams, PostParams, PropagationPolicy};
use kube::{Client, ResourceExt};
use serde_json::json;

use crate::cluster::dynamic_api;
use crate::safety::SafetyGate;
use crate::types::{KindRef, Replicas, is_batch_workload, is_scalable_workload};
use crate::{Error, Result};

/// The annotation `kubectl rollout restart` sets on the pod template.
pub(crate) const RESTARTED_AT: &str = "kubectl.kubernetes.io/restartedAt";

/// Deletes one object, letting the garbage collector remove dependents in the
/// background (kubectl's default). A `dry_run` is validated by the server but
/// changes nothing, so it skips the gate.
pub async fn delete(
    client: &Client,
    gate: &SafetyGate,
    context: &str,
    kind: &KindRef,
    namespace: Option<&str>,
    name: &str,
    dry_run: bool,
) -> Result<()> {
    if !dry_run {
        gate.ensure_writable(context)?;
    }
    let params = DeleteParams {
        dry_run,
        propagation_policy: Some(PropagationPolicy::Background),
        ..DeleteParams::default()
    };
    dynamic_api(client, kind, namespace).delete(name, &params).await?;
    tracing::info!(%context, kind = %kind.key(), ?namespace, %name, dry_run, "deleted");
    Ok(())
}

/// Rolls out new pods the way `kubectl rollout restart` does: stamping the pod
/// template with the current time, so the controller replaces its pods
/// following the workload's update strategy.
pub async fn restart(
    client: &Client,
    gate: &SafetyGate,
    context: &str,
    kind: &KindRef,
    namespace: &str,
    name: &str,
    dry_run: bool,
) -> Result<()> {
    ensure_scalable(kind)?;
    if !dry_run {
        gate.ensure_writable(context)?;
    }
    let now = jiff::Timestamp::now().round(jiff::Unit::Second).map_err(|e| Error::Message(e.to_string()))?;
    let patch = json!({
        "spec": { "template": { "metadata": { "annotations": { RESTARTED_AT: now.to_string() } } } }
    });
    let params = PatchParams { dry_run, ..PatchParams::default() };
    dynamic_api(client, kind, Some(namespace))
        .patch(name, &params, &Patch::Merge(&patch))
        .await?;
    tracing::info!(%context, kind = %kind.key(), %namespace, %name, dry_run, "restarted");
    Ok(())
}

/// Reads the scale subresource. A read, so not gated.
pub async fn replicas(client: &Client, kind: &KindRef, namespace: &str, name: &str) -> Result<Replicas> {
    ensure_scalable(kind)?;
    let scale = dynamic_api(client, kind, Some(namespace)).get_scale(name).await?;
    Ok(Replicas {
        desired: scale.spec.and_then(|s| s.replicas).unwrap_or(0),
        current: scale.status.map_or(0, |s| s.replicas),
    })
}

/// Sets the replica count through the scale subresource, like `kubectl scale`.
#[allow(clippy::too_many_arguments)]
pub async fn scale(
    client: &Client,
    gate: &SafetyGate,
    context: &str,
    kind: &KindRef,
    namespace: &str,
    name: &str,
    replicas: i32,
    dry_run: bool,
) -> Result<()> {
    ensure_scalable(kind)?;
    if replicas < 0 {
        return Err(Error::Message("replicas can't be negative".into()));
    }
    if !dry_run {
        gate.ensure_writable(context)?;
    }
    let patch = json!({ "spec": { "replicas": replicas } });
    let params = PatchParams { dry_run, ..PatchParams::default() };
    dynamic_api(client, kind, Some(namespace))
        .patch_scale(name, &params, &Patch::Merge(&patch))
        .await?;
    tracing::info!(%context, kind = %kind.key(), %namespace, %name, replicas, dry_run, "scaled");
    Ok(())
}

/// Suspends or resumes a Job or CronJob through `spec.suspend`. A suspended
/// CronJob starts no new Jobs; a suspended Job stops its running pods.
#[allow(clippy::too_many_arguments)]
pub async fn set_suspended(
    client: &Client,
    gate: &SafetyGate,
    context: &str,
    kind: &KindRef,
    namespace: &str,
    name: &str,
    suspend: bool,
    dry_run: bool,
) -> Result<()> {
    if !is_batch_workload(kind) {
        return Err(Error::Message(format!("{} can't be suspended here", kind.kind)));
    }
    if !dry_run {
        gate.ensure_writable(context)?;
    }
    let patch = json!({ "spec": { "suspend": suspend } });
    let params = PatchParams { dry_run, ..PatchParams::default() };
    dynamic_api(client, kind, Some(namespace))
        .patch(name, &params, &Patch::Merge(&patch))
        .await?;
    tracing::info!(%context, kind = %kind.key(), %namespace, %name, suspend, dry_run, "suspend set");
    Ok(())
}

/// Starts a Job from a CronJob's template now, like `kubectl create job
/// --from=cronjob/<name>`: same labels and annotations, marked as manual, and
/// owned by the CronJob so it shows among its Jobs. Returns the Job's name.
pub async fn run_cron_job(
    client: &Client,
    gate: &SafetyGate,
    context: &str,
    namespace: &str,
    name: &str,
    dry_run: bool,
) -> Result<String> {
    if !dry_run {
        gate.ensure_writable(context)?;
    }
    let cron_job = Api::<CronJob>::namespaced(client.clone(), namespace).get(name).await?;
    let job = manual_job(&cron_job, jiff::Timestamp::now());
    let params = PostParams { dry_run, ..PostParams::default() };
    let created = Api::<Job>::namespaced(client.clone(), namespace).create(&params, &job).await?;
    let job_name = created.name_any();
    tracing::info!(%context, %namespace, cron_job = %name, job = %job_name, dry_run, "cron job run");
    Ok(job_name)
}

fn manual_job(cron_job: &CronJob, now: jiff::Timestamp) -> Job {
    let template = cron_job.spec.job_template.clone();
    let meta = template.metadata.unwrap_or_default();
    let mut annotations = meta.annotations.unwrap_or_default();
    annotations.insert("cronjob.kubernetes.io/instantiate".to_owned(), "manual".to_owned());
    Job {
        metadata: ObjectMeta {
            name: Some(manual_job_name(&cron_job.name_any(), now)),
            namespace: cron_job.namespace(),
            labels: meta.labels,
            annotations: Some(annotations),
            owner_references: Some(vec![OwnerReference {
                api_version: "batch/v1".to_owned(),
                kind: "CronJob".to_owned(),
                name: cron_job.name_any(),
                uid: cron_job.uid().unwrap_or_default(),
                controller: Some(true),
                block_owner_deletion: Some(true),
            }]),
            ..ObjectMeta::default()
        },
        spec: template.spec,
        status: None,
    }
}

/// `<cronjob>-manual-<5 characters from the time>`, kept within the 63
/// characters a Job name can have (its pods carry it as a label).
fn manual_job_name(cron_job: &str, now: jiff::Timestamp) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut n = now.as_millisecond().unsigned_abs();
    let suffix: String = (0..5)
        .map(|_| {
            let c = DIGITS[(n % 36) as usize] as char;
            n /= 36;
            c
        })
        .collect();
    let prefix = &cron_job[..cron_job.len().min(63 - "-manual-".len() - suffix.len())];
    format!("{}-manual-{suffix}", prefix.trim_end_matches(['-', '.']))
}

fn ensure_scalable(kind: &KindRef) -> Result<()> {
    if is_scalable_workload(kind) {
        Ok(())
    } else {
        Err(Error::Message(format!("{} can't be restarted or scaled here", kind.kind)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn locked_context_is_refused_before_any_request() {
        // The client points nowhere: reaching the network would fail differently.
        let config = kube::Config::new("http://127.0.0.1:9".parse().unwrap());
        let client = Client::try_from(config).unwrap();
        let kind = KindRef {
            group: String::new(),
            version: "v1".into(),
            kind: "Pod".into(),
            plural: "pods".into(),
            namespaced: true,
        };
        let err = delete(&client, &SafetyGate::default(), "acme-prd", &kind, Some("apps"), "api", false)
            .await
            .unwrap_err();
        assert!(matches!(err, crate::Error::Locked(ctx) if ctx == "acme-prd"));
    }

    fn nowhere() -> Client {
        Client::try_from(kube::Config::new("http://127.0.0.1:9".parse().unwrap())).unwrap()
    }

    fn deployment() -> KindRef {
        KindRef {
            group: "apps".into(),
            version: "v1".into(),
            kind: "Deployment".into(),
            plural: "deployments".into(),
            namespaced: true,
        }
    }

    #[tokio::test]
    async fn restart_and_scale_are_gated() {
        let gate = SafetyGate::default();
        let err = restart(&nowhere(), &gate, "acme-prd", &deployment(), "apps", "api", false)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Locked(_)));
        let err = scale(&nowhere(), &gate, "acme-prd", &deployment(), "apps", "api", 3, false)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Locked(_)));
    }

    #[tokio::test]
    async fn suspend_and_run_are_gated() {
        let gate = SafetyGate::default();
        let cron = KindRef { group: "batch".into(), kind: "CronJob".into(), plural: "cronjobs".into(), ..deployment() };
        let err = set_suspended(&nowhere(), &gate, "acme-prd", &cron, "apps", "nightly", true, false)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Locked(_)));
        let err = run_cron_job(&nowhere(), &gate, "acme-prd", "apps", "nightly", false)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Locked(_)));
        gate.set_unlocked("acme-uat", true);
        let err = set_suspended(&nowhere(), &gate, "acme-uat", &deployment(), "apps", "api", true, false)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Message(_)));
    }

    #[test]
    fn manual_jobs_copy_the_template() {
        let cron: CronJob = serde_json::from_value(json!({
            "metadata": {"name": "nightly", "namespace": "apps", "uid": "u-1"},
            "spec": {"schedule": "0 3 * * *", "jobTemplate": {
                "metadata": {"labels": {"app": "nightly"}, "annotations": {"team": "data"}},
                "spec": {"backoffLimit": 2, "template": {"spec": {"containers": [{"name": "run", "image": "busybox"}]}}}
            }}
        }))
        .unwrap();
        let now: jiff::Timestamp = "2026-10-10T12:00:00Z".parse().unwrap();
        let job = manual_job(&cron, now);
        let name = job.name_any();
        assert!(name.starts_with("nightly-manual-") && name.len() == "nightly-manual-".len() + 5, "{name}");
        assert_eq!(job.labels().get("app").unwrap(), "nightly");
        assert_eq!(job.annotations().get("cronjob.kubernetes.io/instantiate").unwrap(), "manual");
        assert_eq!(job.annotations().get("team").unwrap(), "data");
        let owner = &job.owner_references()[0];
        assert_eq!((owner.kind.as_str(), owner.uid.as_str(), owner.controller), ("CronJob", "u-1", Some(true)));
        assert_eq!(job.spec.unwrap().backoff_limit, Some(2));

        let long = manual_job_name(&"x".repeat(80), now);
        assert_eq!(long.len(), 63);
    }

    #[tokio::test]
    async fn other_kinds_and_negative_replicas_are_refused() {
        let gate = SafetyGate::default();
        gate.set_unlocked("acme-uat", true);
        let daemonset = KindRef { kind: "DaemonSet".into(), plural: "daemonsets".into(), ..deployment() };
        let err = scale(&nowhere(), &gate, "acme-uat", &daemonset, "apps", "agent", 1, false)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Message(_)));
        let err = scale(&nowhere(), &gate, "acme-uat", &deployment(), "apps", "api", -1, false)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Message(_)));
    }
}
