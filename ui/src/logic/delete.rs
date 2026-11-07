//! Deleting an object: when the button shows, what the dialog warns about,
//! when the name must be typed, and the request.

use serde::Serialize;
use subduer_types::{ContextInfo, KindInfo, KindRef, ResourceRow};

/// What the dialog is about to delete, captured when it opens.
#[derive(Debug, Clone, PartialEq)]
pub struct DeleteTarget {
    pub context: ContextInfo,
    pub kind: KindRef,
    pub row: ResourceRow,
}

/// `delete_resource` arguments.
#[derive(Debug, PartialEq, Serialize)]
pub(crate) struct DeleteArgs {
    pub(crate) context: String,
    pub(crate) kind: KindRef,
    pub(crate) namespace: Option<String>,
    pub(crate) name: String,
}

impl DeleteArgs {
    pub(crate) fn new(target: &DeleteTarget) -> Self {
        Self {
            context: target.context.name.clone(),
            kind: target.kind.clone(),
            namespace: target.row.namespace.clone(),
            name: target.row.name.clone(),
        }
    }
}

/// Whether the API allows `verb` on `kind`.
pub(crate) fn allows(kinds: &[KindInfo], kind: &KindRef, verb: &str) -> bool {
    kinds.iter().any(|k| &k.kind == kind && k.verbs.iter().any(|v| v == verb))
}

/// The Delete button's tooltip.
pub(crate) fn button_title(locked: bool) -> &'static str {
    if locked { "Delete… (asks to allow changes first: the context is read-only)" } else { "Delete…" }
}

/// Kinds whose deletion takes much more with it.
pub(crate) fn cascades(kind: &KindRef) -> bool {
    matches!(
        (kind.group.as_str(), kind.kind.as_str()),
        ("", "Namespace" | "Node" | "PersistentVolume") | ("apiextensions.k8s.io", "CustomResourceDefinition")
    )
}

pub(crate) fn consequence(kind: &KindRef) -> Option<&'static str> {
    Some(match (kind.group.as_str(), kind.kind.as_str()) {
        ("", "Namespace") => "Everything inside the namespace is deleted too.",
        ("apiextensions.k8s.io", "CustomResourceDefinition") => {
            "Every object of this custom type is deleted, across all namespaces."
        }
        ("", "Node") => "The node is removed from the cluster without being drained first.",
        ("", "PersistentVolume" | "PersistentVolumeClaim") => {
            "The stored data may be deleted too, depending on the reclaim policy."
        }
        ("apps", "Deployment" | "StatefulSet" | "DaemonSet" | "ReplicaSet") | ("batch", "Job" | "CronJob") => {
            "Its pods are deleted too."
        }
        ("", "Pod") => "If a controller owns this pod, it will start a replacement.",
        _ => return None,
    })
}

/// Cascading kinds need the name typed out. Production-like contexts don't:
/// their name was typed when changes were allowed.
pub(crate) fn must_type(target: &DeleteTarget) -> bool {
    cascades(&target.kind)
}

/// Whether Delete is enabled.
pub(crate) fn can_confirm(busy: bool, must_type: bool, typed: &str, name: &str) -> bool {
    !busy && (!must_type || typed.trim() == name)
}

pub(crate) fn heading(kind: &KindRef) -> String {
    format!("Delete {}?", kind.kind)
}

#[cfg(test)]
#[path = "delete_tests.rs"]
mod tests;
