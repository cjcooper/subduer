use super::*;
use crate::app::tests_unit::{kind, store};

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
fn on_namespaces_checks_the_selected_kind() {
    let s = store();
    assert!(!on_namespaces(s));
    s.kind.set(Some(kind("", "Namespace")));
    assert!(on_namespaces(s));
    s.kind.set(Some(kind("", "Pod")));
    assert!(!on_namespaces(s));
}
