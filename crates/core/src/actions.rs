//! Operations that change cluster state. Each one checks the safety gate
//! before touching the cluster.

use kube::Client;
use kube::api::{DeleteParams, PropagationPolicy};

use crate::Result;
use crate::cluster::dynamic_api;
use crate::safety::SafetyGate;
use crate::types::KindRef;

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
}
