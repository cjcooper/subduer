use super::*;
use crate::app::tests_unit::{kind, row};
use subduer_types::Category;

fn info(group: &str, k: &str, verbs: &[&str]) -> KindInfo {
    KindInfo { kind: kind(group, k), verbs: verbs.iter().map(|v| (*v).to_owned()).collect(), category: Category::TopLevel }
}

#[test]
fn valid_name_accepts_dns_labels() {
    for n in ["a", "shop", "acme-dev", "team-42", "9lives", &"a".repeat(63)] {
        assert!(valid_name(n), "{n:?} should be valid");
    }
}

#[test]
fn valid_name_rejects_bad_labels() {
    for n in ["", "Shop", "-shop", "shop-", "shop_dev", "shop.dev", "shop dev", "ünï", &"a".repeat(64)] {
        assert!(!valid_name(n), "{n:?} should be invalid");
    }
}

#[test]
fn is_namespaces_checks_the_selected_kind() {
    assert!(!is_namespaces(None));
    assert!(is_namespaces(Some(&kind("", "Namespace"))));
    assert!(!is_namespaces(Some(&kind("", "Pod"))));
}

#[test]
fn may_create_needs_the_create_verb_on_namespaces() {
    assert!(may_create(&[info("", "Pod", &["create"]), info("", "Namespace", &["list", "create"])]));
    assert!(!may_create(&[info("", "Namespace", &["list"])]));
    assert!(!may_create(&[info("", "Pod", &["create"])]));
    assert!(!may_create(&[]));
}

#[test]
fn dialog_texts_and_checks() {
    assert_eq!(new_title(true), "Create a namespace… (asks to allow changes first)");
    assert_eq!(new_title(false), "Create a namespace…");
    let rows = [row(None, "shop"), row(None, "acme")];
    assert!(exists(&rows, " shop "));
    assert!(!exists(&rows, "team"));
    assert!(can_create(" team ", false, false));
    assert!(!can_create("team", true, false));
    assert!(!can_create("team", false, true));
    assert!(!can_create("Team", false, false));
    assert_eq!(hint("  ", false), None);
    assert_eq!(
        hint("Team", false),
        Some("Use lowercase letters, digits and '-', starting and ending with a letter or digit (63 at most).")
    );
    assert_eq!(hint("shop", true), Some("A namespace with that name already exists."));
    assert_eq!(hint("team", false), None);
}

#[test]
fn filter_button_texts() {
    assert_eq!(filter_button("shop", false), ("Show only objects in shop in every list".into(), "Filter to it"));
    assert_eq!(filter_button("shop", true).1, "Filtering to it");
}

#[test]
fn create_args_serialize() {
    let v = serde_json::to_value(CreateArgs { context: "acme".into(), name: "team".into() }).unwrap();
    assert_eq!(v, serde_json::json!({ "context": "acme", "name": "team" }));
}
