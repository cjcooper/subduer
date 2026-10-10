//! Sorts discovered kinds into sidebar sections.

use std::collections::HashSet;

use crate::types::{Category, KindRef};

/// Built-in kinds per section, as `group/Kind` keys, in display order.
const SECTIONS: &[(Category, &[&str])] = &[
    (Category::TopLevel, &["core/Node", "core/Event"]),
    (
        Category::Workloads,
        &[
            "core/Pod",
            "apps/Deployment",
            "apps/StatefulSet",
            "apps/DaemonSet",
            "apps/ReplicaSet",
            "batch/Job",
            "batch/CronJob",
            "core/ReplicationController",
        ],
    ),
    (
        Category::Config,
        &[
            "core/ConfigMap",
            "core/Secret",
            "autoscaling/HorizontalPodAutoscaler",
            "policy/PodDisruptionBudget",
            "core/ResourceQuota",
            "core/LimitRange",
            "scheduling.k8s.io/PriorityClass",
            "node.k8s.io/RuntimeClass",
            "coordination.k8s.io/Lease",
            "admissionregistration.k8s.io/MutatingWebhookConfiguration",
            "admissionregistration.k8s.io/ValidatingWebhookConfiguration",
            "admissionregistration.k8s.io/ValidatingAdmissionPolicy",
            "admissionregistration.k8s.io/ValidatingAdmissionPolicyBinding",
        ],
    ),
    (
        Category::Network,
        &[
            "core/Service",
            "networking.k8s.io/Ingress",
            "networking.k8s.io/IngressClass",
            "networking.k8s.io/NetworkPolicy",
            "discovery.k8s.io/EndpointSlice",
            "core/Endpoints",
        ],
    ),
    (
        Category::Storage,
        &[
            "core/PersistentVolumeClaim",
            "core/PersistentVolume",
            "storage.k8s.io/StorageClass",
            "storage.k8s.io/VolumeAttachment",
            "storage.k8s.io/CSIDriver",
            "storage.k8s.io/CSINode",
            "storage.k8s.io/CSIStorageCapacity",
        ],
    ),
    (Category::Namespaces, &["core/Namespace"]),
    (
        Category::AccessControl,
        &[
            "core/ServiceAccount",
            "rbac.authorization.k8s.io/Role",
            "rbac.authorization.k8s.io/RoleBinding",
            "rbac.authorization.k8s.io/ClusterRole",
            "rbac.authorization.k8s.io/ClusterRoleBinding",
        ],
    ),
];

/// Kinds left out of the sidebar because another kind shows the same objects.
const HIDDEN: &[&str] = &[
    // Same objects as core/Event, which has the fields the Events table uses.
    "events.k8s.io/Event",
];

pub fn is_hidden(kind: &KindRef) -> bool {
    HIDDEN.contains(&kind.key().as_str())
}

/// API groups that ship with Kubernetes. Anything else is treated as custom,
/// which covers aggregated APIs (e.g. metrics.k8s.io) as well as CRDs.
const BUILTIN_GROUPS: &[&str] = &[
    "",
    "admissionregistration.k8s.io",
    "apiextensions.k8s.io",
    "apiregistration.k8s.io",
    "apps",
    "authentication.k8s.io",
    "authorization.k8s.io",
    "autoscaling",
    "batch",
    "certificates.k8s.io",
    "coordination.k8s.io",
    "discovery.k8s.io",
    "events.k8s.io",
    "flowcontrol.apiserver.k8s.io",
    "internal.apiserver.k8s.io",
    "networking.k8s.io",
    "node.k8s.io",
    "policy",
    "rbac.authorization.k8s.io",
    "resource.k8s.io",
    "scheduling.k8s.io",
    "storage.k8s.io",
    "storagemigration.k8s.io",
];

/// Section and position within it. `crd_names` holds the cluster's
/// CustomResourceDefinition names (`<plural>.<group>`); it may be empty if
/// CRDs can't be read.
pub fn classify(kind: &KindRef, crd_names: &HashSet<String>) -> (Category, usize) {
    let key = kind.key();
    let crd_name = format!("{}.{}", kind.plural, kind.group);
    if crd_names.contains(&crd_name) || !BUILTIN_GROUPS.contains(&kind.group.as_str()) {
        return (Category::CustomResources, usize::MAX);
    }
    SECTIONS
        .iter()
        .find_map(|(category, keys)| keys.iter().position(|k| *k == key).map(|i| (*category, i)))
        .unwrap_or((Category::Other, usize::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(group: &str, kind: &str) -> KindRef {
        KindRef {
            group: group.into(),
            version: "v1".into(),
            kind: kind.into(),
            plural: format!("{}s", kind.to_lowercase()),
            namespaced: true,
        }
    }

    #[test]
    fn sections() {
        let none = HashSet::new();
        assert_eq!(classify(&kind("", "Pod"), &none), (Category::Workloads, 0));
        assert_eq!(classify(&kind("", "Secret"), &none).0, Category::Config);
        assert_eq!(classify(&kind("networking.k8s.io", "Ingress"), &none).0, Category::Network);
        assert_eq!(classify(&kind("", "PersistentVolume"), &none).0, Category::Storage);
        assert_eq!(classify(&kind("", "Namespace"), &none).0, Category::Namespaces);
        assert_eq!(classify(&kind("rbac.authorization.k8s.io", "Role"), &none).0, Category::AccessControl);
        assert_eq!(classify(&kind("", "Node"), &none), (Category::TopLevel, 0));
        assert_eq!(classify(&kind("", "Event"), &none), (Category::TopLevel, 1));
        assert_eq!(classify(&kind("certificates.k8s.io", "CertificateSigningRequest"), &none).0, Category::Other);
    }

    #[test]
    fn custom_resources() {
        let none = HashSet::new();
        assert_eq!(classify(&kind("keda.sh", "ScaledObject"), &none).0, Category::CustomResources);
        assert_eq!(classify(&kind("metrics.k8s.io", "PodMetrics"), &none).0, Category::CustomResources);
        // The CRD list is honoured regardless of group.
        let gateway = kind("gateway.networking.k8s.io", "Gateway");
        let crds = HashSet::from(["gateways.gateway.networking.k8s.io".to_owned()]);
        assert_eq!(classify(&gateway, &crds).0, Category::CustomResources);
    }
}
