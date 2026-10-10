use super::*;
use crate::app::tests_unit::{context, store, store_on};

fn target() -> ExecTarget {
    ExecTarget { namespace: "shop".into(), pod: "web-1".into(), container: "app".into() }
}

#[test]
fn label_in_finds_the_exec_parameter() {
    assert_eq!(label_in("?exec=shell-3").as_deref(), Some("shell-3"));
    assert_eq!(label_in("?theme=dark&exec=shell-3&x=1").as_deref(), Some("shell-3"));
    assert_eq!(label_in("exec=abc").as_deref(), Some("abc"));
    assert_eq!(label_in("?exec="), None);
    assert_eq!(label_in("?logs=abc"), None);
    assert_eq!(label_in("?myexec=abc"), None);
    assert_eq!(label_in(""), None);
}

#[test]
fn request_asks_to_unlock_first() {
    let s = store_on(false);
    request(s, target());
    assert_eq!(s.pending_exec.get_untracked(), Some(("acme-dev".into(), target())));
    assert_eq!(s.unlock_prompt.get_untracked(), Some(context("acme-dev", false)));
}

#[test]
fn request_needs_a_context() {
    let s = store();
    request(s, target());
    assert_eq!(s.pending_exec.get_untracked(), None);
    assert_eq!(s.unlock_prompt.get_untracked(), None);
}
