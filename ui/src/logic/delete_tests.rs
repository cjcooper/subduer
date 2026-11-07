use super::*;
use crate::app::tests_unit::{context, kind, row};
use subduer_types::Category;

fn target(prod_like: bool, k: KindRef) -> DeleteTarget {
    DeleteTarget { context: ContextInfo { prod_like, ..context("acme-dev", true) }, kind: k, row: row(Some("shop"), "web-1") }
}

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
fn allows_needs_the_verb_on_that_kind() {
    let kinds = [
        KindInfo { kind: kind("", "Pod"), verbs: vec!["list".into(), "delete".into()], category: Category::TopLevel },
        KindInfo { kind: kind("", "Node"), verbs: vec!["list".into()], category: Category::TopLevel },
    ];
    assert!(allows(&kinds, &kind("", "Pod"), "delete"));
    assert!(!allows(&kinds, &kind("", "Node"), "delete"));
    assert!(!allows(&kinds, &kind("apps", "Deployment"), "delete"));
}

#[test]
fn typing_the_name_for_production_and_cascades() {
    assert!(!must_type(&target(false, kind("", "Pod"))));
    assert!(must_type(&target(true, kind("", "Pod"))));
    assert!(must_type(&target(false, kind("", "Namespace"))));
    assert!(can_confirm(false, false, "", "web-1"));
    assert!(!can_confirm(true, false, "", "web-1"), "busy");
    assert!(!can_confirm(false, true, "web", "web-1"));
    assert!(can_confirm(false, true, " web-1 ", "web-1"));
}

#[test]
fn texts_and_args() {
    assert_eq!(button_title(true), "Delete… (asks to allow changes first: the context is read-only)");
    assert_eq!(button_title(false), "Delete…");
    assert_eq!(heading(&kind("apps", "Deployment")), "Delete Deployment?");
    let args = DeleteArgs::new(&target(false, kind("", "Pod")));
    assert_eq!(
        args,
        DeleteArgs { context: "acme-dev".into(), kind: kind("", "Pod"), namespace: Some("shop".into()), name: "web-1".into() }
    );
    let v = serde_json::to_value(&args).unwrap();
    assert_eq!((v["context"].as_str(), v["namespace"].as_str(), v["name"].as_str()), (Some("acme-dev"), Some("shop"), Some("web-1")));
}
