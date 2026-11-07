use super::*;
use crate::app::tests_unit::{context, store, store_on};

#[test]
fn request_opens_the_dialog_when_unlocked() {
    let s = store_on(true);
    request(s, "shop".into(), "web-1".into(), "app".into());
    let expected =
        UploadPrompt { context: "acme-dev".into(), namespace: "shop".into(), pod: "web-1".into(), container: "app".into() };
    assert_eq!(s.upload_prompt.get_untracked(), Some(expected));
    assert_eq!(s.pending_upload.get_untracked(), None);
}

#[test]
fn request_asks_to_unlock_first() {
    let s = store_on(false);
    request(s, "shop".into(), "web-1".into(), "app".into());
    assert_eq!(s.upload_prompt.get_untracked(), None);
    assert_eq!(s.pending_upload.get_untracked().map(|p| p.container), Some("app".into()));
    assert_eq!(s.unlock_prompt.get_untracked(), Some(context("acme-dev", false)));
}

#[test]
fn request_needs_a_context() {
    let s = store();
    request(s, "shop".into(), "web-1".into(), "app".into());
    assert_eq!(s.upload_prompt.get_untracked(), None);
    assert_eq!(s.pending_upload.get_untracked(), None);
}
