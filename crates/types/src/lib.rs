//! Data passed between the Tauri backend and the Leptos UI.
//!
//! Everything here is plain serde data so it compiles for both the native
//! backend and `wasm32-unknown-unknown`.

use serde::{Deserialize, Serialize};

/// A kubeconfig context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextInfo {
    pub name: String,
    pub cluster: Option<String>,
    pub namespace: Option<String>,
    /// Name looks like a production cluster; unlocking needs extra confirmation.
    pub prod_like: bool,
    /// Mutations are allowed against this context.
    pub unlocked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextList {
    pub contexts: Vec<ContextInfo>,
    /// `current-context` from the kubeconfig, if set.
    pub current: Option<String>,
}

/// Identifies a resource type on the API server.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KindRef {
    /// Empty for the core API group.
    pub group: String,
    pub version: String,
    pub kind: String,
    pub plural: String,
    pub namespaced: bool,
}

impl KindRef {
    pub fn api_version(&self) -> String {
        if self.group.is_empty() {
            self.version.clone()
        } else {
            format!("{}/{}", self.group, self.version)
        }
    }

    /// Stable key, e.g. `apps/Deployment` or `core/Pod`.
    pub fn key(&self) -> String {
        let group = if self.group.is_empty() { "core" } else { &self.group };
        format!("{group}/{}", self.kind)
    }
}

/// A listable resource type discovered on the cluster.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KindInfo {
    pub kind: KindRef,
    pub verbs: Vec<String>,
}

/// Result of connecting to a context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterInfo {
    pub context: String,
    pub kinds: Vec<KindInfo>,
    pub namespaces: Vec<String>,
}

/// One object rendered as a table row.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ResourceRow {
    /// `namespace/name`, or `name` for cluster-scoped objects.
    pub id: String,
    pub name: String,
    pub namespace: Option<String>,
    /// RFC 3339 creation timestamp.
    pub created: Option<String>,
    /// Extra kind-specific values, matching `ResourceTable::columns`.
    pub cells: Vec<String>,
}

/// Snapshot of a watched resource list, pushed to the UI on every change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceTable {
    pub watch_id: u64,
    /// Headers for `ResourceRow::cells` (Name / Namespace / Age are implicit).
    pub columns: Vec<String>,
    pub rows: Vec<ResourceRow>,
    /// Set when the watch failed; `rows` holds the last good state.
    pub error: Option<String>,
}

/// Tauri event names.
pub mod events {
    pub const RESOURCE_TABLE: &str = "resource-table";
}
