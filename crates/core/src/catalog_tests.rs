use super::*;
use crate::testing::kind;

#[test]
fn events_k8s_io_events_are_hidden() {
    assert!(is_hidden(&kind("events.k8s.io", "v1", "Event", "events", true)));
    assert!(!is_hidden(&kind("", "v1", "Event", "events", true)));
    assert!(!is_hidden(&kind("apps", "v1", "Deployment", "deployments", true)));
}

#[test]
fn positions_follow_section_order() {
    let none = HashSet::new();
    let pos = |group: &str, k: &str, plural: &str| classify(&kind(group, "v1", k, plural, true), &none);
    assert_eq!(pos("apps", "Deployment", "deployments"), (Category::Workloads, 1));
    assert_eq!(pos("batch", "CronJob", "cronjobs"), (Category::Workloads, 6));
    assert_eq!(pos("", "Endpoints", "endpoints"), (Category::Network, 5));
    assert_eq!(pos("storage.k8s.io", "CSIStorageCapacity", "csistoragecapacities"), (Category::Storage, 6));
    assert_eq!(pos("rbac.authorization.k8s.io", "ClusterRoleBinding", "clusterrolebindings"), (Category::AccessControl, 4));
    assert_eq!(
        pos("admissionregistration.k8s.io", "ValidatingAdmissionPolicyBinding", "validatingadmissionpolicybindings"),
        (Category::Config, 12)
    );
    // A built-in group with a kind not in any section.
    assert_eq!(pos("apps", "ControllerRevision", "controllerrevisions"), (Category::Other, usize::MAX));
}

#[test]
fn crds_in_cloud_groups_get_cloud_sections() {
    // A CRD named in the list, in a cloud group, lands in that cloud's section.
    let crds = HashSet::from(["targetgroupbindings.elbv2.k8s.aws".to_owned()]);
    let k = kind("elbv2.k8s.aws", "v1beta1", "TargetGroupBinding", "targetgroupbindings", true);
    assert_eq!(classify(&k, &crds), (Category::Aws, usize::MAX));
    // A CRD that shadows a built-in group is still custom (not its built-in section).
    let crds = HashSet::from(["pods.apps".to_owned()]);
    assert_eq!(classify(&kind("apps", "v1", "Pod", "pods", true), &crds), (Category::CustomResources, usize::MAX));
}
