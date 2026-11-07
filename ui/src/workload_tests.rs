use super::*;
use crate::app::tests_unit::{context, kind, row, store, store_on};
use subduer_types::DataChange;

#[test]
fn request_opens_the_dialog_when_unlocked() {
    let s = store_on(true);
    let r = row(Some("shop"), "web");
    request(s, kind("apps", "Deployment"), &r, Op::Restart);
    let target = s.workload_prompt.get_untracked().expect("dialog open");
    assert_eq!(target.context, context("acme-dev", true));
    assert_eq!((target.namespace.as_str(), target.name.as_str(), target.op), ("shop", "web", Op::Restart));
    assert_eq!(target.save, None);
    assert_eq!(s.unlock_prompt.get_untracked(), None);
}

#[test]
fn request_asks_to_unlock_first_when_locked() {
    let s = store_on(false);
    // Cluster-scoped objects have no namespace.
    request(s, kind("", "PersistentVolume"), &row(None, "pv-1"), Op::Reclaim);
    assert_eq!(s.workload_prompt.get_untracked(), None);
    let pending = s.pending_workload.get_untracked().expect("waits for unlock");
    assert_eq!((pending.namespace.as_str(), pending.name.as_str()), ("", "pv-1"));
    assert_eq!(s.unlock_prompt.get_untracked(), Some(context("acme-dev", false)));
}

#[test]
fn request_save_carries_the_edit() {
    let s = store_on(true);
    let save = SaveData {
        resource_version: "42".into(),
        changes: vec![DataChange { key: "a".into(), value: None }],
        added: vec![],
        changed: vec![],
        removed: vec!["a".into()],
    };
    request_save(s, kind("", "ConfigMap"), "shop".into(), "cfg".into(), save.clone());
    let target = s.workload_prompt.get_untracked().unwrap();
    assert_eq!(target.op, Op::Save);
    assert_eq!(target.save, Some(save));
}

#[test]
fn request_does_nothing_without_a_context() {
    let s = store();
    request(s, kind("apps", "Deployment"), &row(Some("shop"), "web"), Op::Scale);
    assert_eq!(s.workload_prompt.get_untracked(), None);
    assert_eq!(s.pending_workload.get_untracked(), None);
}
