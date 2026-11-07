use super::*;
use crate::app::tests_unit::{context, store, store_on};

fn target() -> ExecTarget {
    ExecTarget { namespace: "shop".into(), pod: "web-1".into(), container: "app".into() }
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
