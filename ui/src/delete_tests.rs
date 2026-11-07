use super::*;
use crate::app::tests_unit::{context, kind, row, store, store_on};

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
