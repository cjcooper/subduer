use super::*;
use crate::app::tests_unit::{context, kind, row, store, store_on};

#[test]
fn cascades_for_kinds_that_take_much_more_with_them() {
    assert!(cascades(&kind("", "Namespace")));
    assert!(cascades(&kind("", "Node")));
    assert!(cascades(&kind("", "PersistentVolume")));
    assert!(cascades(&kind("apiextensions.k8s.io", "CustomResourceDefinition")));
    assert!(!cascades(&kind("", "Pod")));
    assert!(!cascades(&kind("apps", "Deployment")));
    assert!(!cascades(&kind("", "PersistentVolumeClaim")));
    // The group matters: a custom kind named Namespace doesn't cascade.
    assert!(!cascades(&kind("acme.io", "Namespace")));
}

#[test]
fn consequence_explains_what_else_goes() {
    assert_eq!(consequence(&kind("", "Namespace")), Some("Everything inside the namespace is deleted too."));
    assert_eq!(
        consequence(&kind("apiextensions.k8s.io", "CustomResourceDefinition")),
        Some("Every object of this custom type is deleted, across all namespaces.")
    );
    assert_eq!(
        consequence(&kind("", "Node")),
        Some("The node is removed from the cluster without being drained first.")
    );
    let storage = Some("The stored data may be deleted too, depending on the reclaim policy.");
    assert_eq!(consequence(&kind("", "PersistentVolume")), storage);
    assert_eq!(consequence(&kind("", "PersistentVolumeClaim")), storage);
    for (g, k) in [
        ("apps", "Deployment"),
        ("apps", "StatefulSet"),
        ("apps", "DaemonSet"),
        ("apps", "ReplicaSet"),
        ("batch", "Job"),
        ("batch", "CronJob"),
    ] {
        assert_eq!(consequence(&kind(g, k)), Some("Its pods are deleted too."), "{g}/{k}");
    }
    assert_eq!(
        consequence(&kind("", "Pod")),
        Some("If a controller owns this pod, it will start a replacement.")
    );
    assert_eq!(consequence(&kind("", "ConfigMap")), None);
    assert_eq!(consequence(&kind("acme.io", "Job")), None);
}

#[test]
fn request_delete_opens_the_dialog_when_unlocked() {
    let s = store_on(true);
    request_delete(s, kind("", "Pod"), row(Some("shop"), "web-1"));
    let t = s.delete_prompt.get_untracked().expect("dialog open");
    assert_eq!(t.context, context("acme-dev", true));
    assert_eq!(t.row, row(Some("shop"), "web-1"));
    assert_eq!(s.pending_delete.get_untracked(), None);
    assert_eq!(s.unlock_prompt.get_untracked(), None);
}

#[test]
fn request_delete_asks_to_unlock_first() {
    let s = store_on(false);
    request_delete(s, kind("", "Pod"), row(Some("shop"), "web-1"));
    assert_eq!(s.delete_prompt.get_untracked(), None);
    assert_eq!(s.pending_delete.get_untracked().map(|t| t.kind), Some(kind("", "Pod")));
    assert_eq!(s.unlock_prompt.get_untracked(), Some(context("acme-dev", false)));
}

#[test]
fn request_delete_needs_a_context() {
    let s = store();
    request_delete(s, kind("", "Pod"), row(Some("shop"), "web-1"));
    assert_eq!(s.delete_prompt.get_untracked(), None);
    assert_eq!(s.pending_delete.get_untracked(), None);
}
