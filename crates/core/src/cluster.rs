//! Discovery and one-shot reads.

use k8s_openapi::api::core::v1::Namespace;
use kube::api::{Api, DynamicObject, ListParams};
use kube::discovery::{ApiResource, Discovery, Scope, verbs};
use kube::{Client, ResourceExt};

use crate::Result;
use crate::types::{KindInfo, KindRef};

/// All resource types the server lets us list, sorted by group then kind.
/// Uses the preferred version of each group.
pub async fn discover_kinds(client: &Client) -> Result<Vec<KindInfo>> {
    let discovery = Discovery::new(client.clone()).run().await?;
    let mut kinds: Vec<KindInfo> = discovery
        .groups()
        .flat_map(|group| group.recommended_resources())
        .filter(|(_, caps)| caps.supports_operation(verbs::LIST))
        .map(|(ar, caps)| KindInfo {
            kind: KindRef {
                group: ar.group,
                version: ar.version,
                kind: ar.kind,
                plural: ar.plural,
                namespaced: caps.scope == Scope::Namespaced,
            },
            verbs: caps.operations,
        })
        .collect();
    kinds.sort_by(|a, b| {
        (a.kind.group.as_str(), a.kind.kind.as_str()).cmp(&(b.kind.group.as_str(), b.kind.kind.as_str()))
    });
    Ok(kinds)
}

pub async fn list_namespaces(client: &Client) -> Result<Vec<String>> {
    let api: Api<Namespace> = Api::all(client.clone());
    let mut names: Vec<String> = api
        .list_metadata(&ListParams::default())
        .await?
        .items
        .iter()
        .map(|ns| ns.name_any())
        .collect();
    names.sort();
    Ok(names)
}

pub fn api_resource(kind: &KindRef) -> ApiResource {
    ApiResource {
        group: kind.group.clone(),
        version: kind.version.clone(),
        api_version: kind.api_version(),
        kind: kind.kind.clone(),
        plural: kind.plural.clone(),
    }
}

/// Dynamic API handle. `namespace` is ignored for cluster-scoped kinds;
/// `None` on a namespaced kind means all namespaces.
pub fn dynamic_api(client: &Client, kind: &KindRef, namespace: Option<&str>) -> Api<DynamicObject> {
    let ar = api_resource(kind);
    match namespace {
        Some(ns) if kind.namespaced => Api::namespaced_with(client.clone(), ns, &ar),
        _ => Api::all_with(client.clone(), &ar),
    }
}

/// Fetches one object as YAML, without `managedFields` noise.
pub async fn get_yaml(client: &Client, kind: &KindRef, namespace: Option<&str>, name: &str) -> Result<String> {
    let mut obj = dynamic_api(client, kind, namespace).get(name).await?;
    obj.metadata.managed_fields = None;
    Ok(serde_norway::to_string(&obj)?)
}
