//! Kubeconfig contexts and per-context clients.

use kube::config::{KubeConfigOptions, Kubeconfig};
use kube::{Client, Config};

use crate::safety::{SafetyGate, is_prod_like};
use crate::types::{ContextInfo, ContextList};
use crate::{Error, Result};

/// Reads the kubeconfig (honours `$KUBECONFIG`).
pub fn load_kubeconfig() -> Result<Kubeconfig> {
    Ok(Kubeconfig::read()?)
}

pub fn list_contexts(kubeconfig: &Kubeconfig, gate: &SafetyGate) -> ContextList {
    let contexts = kubeconfig
        .contexts
        .iter()
        .map(|named| {
            let ctx = named.context.as_ref();
            let cluster = ctx.map(|c| c.cluster.clone());
            ContextInfo {
                prod_like: is_prod_like(&named.name)
                    || cluster.as_deref().is_some_and(is_prod_like),
                unlocked: gate.is_unlocked(&named.name),
                name: named.name.clone(),
                cluster,
                namespace: ctx.and_then(|c| c.namespace.clone()),
            }
        })
        .collect();
    ContextList {
        contexts,
        current: kubeconfig.current_context.clone(),
    }
}

/// Builds a client for `context`. Exec-based auth (e.g. `aws eks get-token`)
/// runs the plugin found on `$PATH`.
pub async fn client_for(kubeconfig: &Kubeconfig, context: &str) -> Result<Client> {
    if !kubeconfig.contexts.iter().any(|c| c.name == context) {
        return Err(Error::UnknownContext(context.to_owned()));
    }
    let options = KubeConfigOptions {
        context: Some(context.to_owned()),
        ..Default::default()
    };
    let config = Config::from_custom_kubeconfig(kubeconfig.clone(), &options).await?;
    Ok(Client::try_from(config)?)
}

#[cfg(test)]
#[path = "contexts_tests.rs"]
mod coverage_tests;
