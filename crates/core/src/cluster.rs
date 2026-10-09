//! Discovery and one-shot reads.

use std::collections::HashSet;

use k8s_openapi::api::core::v1::Namespace;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::api::{Api, DynamicObject, ListParams};
use kube::discovery::{ApiResource, Discovery, Scope, verbs};
use kube::{Client, ResourceExt};

use crate::{Result, catalog};
use crate::types::{KindInfo, KindRef};

/// All resource types the server lets us list, in sidebar order: by section,
/// then catalog position, then group and kind. Uses each group's preferred version.
pub async fn discover_kinds(client: &Client) -> Result<Vec<KindInfo>> {
    let discovery = Discovery::new(client.clone()).run().await?;
    let crd_names = crd_names(client).await;
    let mut kinds: Vec<(usize, KindInfo)> = discovery
        .groups()
        .flat_map(|group| group.recommended_resources())
        .filter(|(_, caps)| caps.supports_operation(verbs::LIST))
        .map(|(ar, caps)| {
            let kind = KindRef {
                group: ar.group,
                version: ar.version,
                kind: ar.kind,
                plural: ar.plural,
                namespaced: caps.scope == Scope::Namespaced,
            };
            let (category, position) = catalog::classify(&kind, &crd_names);
            let info = KindInfo {
                kind,
                verbs: caps.operations,
                category,
            };
            (position, info)
        })
        .collect();
    kinds.sort_by(|(pa, a), (pb, b)| {
        (a.category, pa, &a.kind.group, &a.kind.kind).cmp(&(b.category, pb, &b.kind.group, &b.kind.kind))
    });
    Ok(kinds.into_iter().map(|(_, info)| info).collect())
}

/// Names (`<plural>.<group>`) of installed CRDs, fetched as metadata only since
/// full CRDs carry large schemas. Empty if CRDs can't be listed; the catalog
/// then falls back to treating non-built-in groups as custom.
async fn crd_names(client: &Client) -> HashSet<String> {
    let api: Api<CustomResourceDefinition> = Api::all(client.clone());
    match api.list_metadata(&ListParams::default()).await {
        Ok(list) => list.items.iter().map(|crd| crd.name_any()).collect(),
        Err(e) => {
            tracing::warn!(error = %e, "cannot list CRDs");
            HashSet::new()
        }
    }
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
