//! Kubeconfig contexts and per-context clients.

use std::time::Duration;

use kube::config::{KubeConfigOptions, Kubeconfig};
use kube::{Client, Config};

use crate::safety::{ProdMarks, SafetyGate};
use crate::types::{ContextInfo, ContextList};
use crate::{Error, Result};

/// How long opening a connection may take (kube's default is 30s). With a
/// VPN down, private API addresses often don't refuse; they just don't answer.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Reads the kubeconfig (honours `$KUBECONFIG`).
pub fn load_kubeconfig() -> Result<Kubeconfig> {
    Ok(Kubeconfig::read()?)
}

pub fn list_contexts(kubeconfig: &Kubeconfig, gate: &SafetyGate, marks: &ProdMarks) -> ContextList {
    let contexts = kubeconfig
        .contexts
        .iter()
        .map(|named| {
            let ctx = named.context.as_ref();
            let cluster = ctx.map(|c| c.cluster.clone());
            let server = kubeconfig
                .clusters
                .iter()
                .find(|c| Some(&c.name) == cluster.as_ref())
                .and_then(|c| c.cluster.as_ref()?.server.clone())
                .unwrap_or_default();
            let user = ctx.and_then(|c| c.user.as_deref());
            let exec = kubeconfig
                .auth_infos
                .iter()
                .find(|u| Some(u.name.as_str()) == user)
                .and_then(|u| u.auth_info.as_ref()?.exec.as_ref()?.command.clone())
                .unwrap_or_default();
            ContextInfo {
                provider: crate::manage_clusters::provider(&named.name, &server, &exec),
                prod_like: marks.is_prod(&named.name, cluster.as_deref()),
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
    let mut config = Config::from_custom_kubeconfig(kubeconfig.clone(), &options).await?;
    config.connect_timeout = Some(CONNECT_TIMEOUT);
    Ok(Client::try_from(config)?)
}

#[cfg(test)]
#[path = "contexts_tests.rs"]
mod coverage_tests;
