//! Operations that change cluster state. Each one checks the safety gate
//! before touching the cluster.

use kube::Client;
use kube::api::{DeleteParams, Patch, PatchParams, PropagationPolicy};
use serde_json::json;

use crate::cluster::dynamic_api;
use crate::safety::SafetyGate;
use crate::types::{KindRef, Replicas, is_scalable_workload};
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
