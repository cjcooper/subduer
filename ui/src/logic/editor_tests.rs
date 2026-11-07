use super::*;
use crate::app::tests_unit::{kind, row};
use subduer_types::Category;

fn d(old: &str, new: &str) -> Vec<Line> {
    line_diff(old, new)
}

fn session(unlocked: bool) -> EditSession {
    EditSession {
        context: "acme".into(),
        prod_like: false,
        unlocked,
        target: EditTarget { kind: kind("", "ConfigMap"), namespace: Some("shop".into()), name: "cfg".into() },
        title: "cfg · Edit".into(),
        yaml: String::new(),
    }
}

#[test]
fn diffs_lines() {
    use Line::*;
    assert_eq!(d("a\nb\nc", "a\nb\nc"), [Same("a".into()), Same("b".into()), Same("c".into())]);
    assert_eq!(d("a\nb\nc", "a\nx\nc"), [Same("a".into()), Removed("b".into()), Added("x".into()), Same("c".into())]);
    assert_eq!(d("a\nc", "a\nb\nc"), [Same("a".into()), Added("b".into()), Same("c".into())]);
    assert_eq!(d("a\nb\nc", "a\nc"), [Same("a".into()), Removed("b".into()), Same("c".into())]);
    assert_eq!(d("", "a"), [Added("a".into())]);
    assert_eq!(counts(&d("a\nb\nc\nd", "a\nB\nc\nd\ne")), (2, 1));
}

#[test]
fn huge_changes_show_as_replaced() {
    let old: String = (0..2001).map(|i| format!("o{i}\n")).collect();
    let new: String = (0..2001).map(|i| format!("n{i}\n")).collect();
    let lines = d(&old, &new);
    assert_eq!(counts(&lines), (2001, 2001));
    assert_eq!(lines[0], Line::Removed("o0".into()));
    assert_eq!(lines[2001], Line::Added("n0".into()));
}

#[test]
fn summaries_fold_unchanged_runs() {
    let old: String = (0..20).map(|i| format!("l{i}\n")).collect();
    let new = old.replace("l10\n", "L10\n");
    let s = summarize(line_diff(&old, &new), 2);
    assert_eq!(s.first(), Some(&Line::Skipped(8)));
    assert_eq!(s.last(), Some(&Line::Skipped(7)));
    assert_eq!(counts(&s), (1, 1));
    assert_eq!(change_summary(1, 1), "1 line added, 1 removed.");
    assert_eq!(change_summary(2, 0), "2 lines added.");
    assert_eq!(change_summary(0, 1), "1 line removed.");
    assert_eq!(change_summary(2, 3), "2 lines added, 3 removed.");
    assert!(s.contains(&Line::Same("l8".into())) && !s.contains(&Line::Same("l7".into())));
    assert_eq!(summarize(line_diff("a", "a"), 3), [Line::Skipped(1)]);
    assert_eq!(skipped_text(12), "… 12 unchanged lines");
}

#[test]
fn edit_labels_and_keys() {
    assert_eq!(label_in("?edit=edit-3"), Some("edit-3".into()));
    assert_eq!(label_in("?a=1&edit=edit-9"), Some("edit-9".into()));
    assert_eq!(label_in("?logs=x"), None);
    assert_eq!(label_in("?edit="), None);
    assert_eq!(label_in("?kind=cm&edit=1"), None);
    assert_eq!(label_in("?edit=edit-"), None);
    let mut keys = KUBERNETES_KEYS.to_vec();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), KUBERNETES_KEYS.len(), "no duplicate completion keys");
    assert_eq!(ace_theme(true), "ace/theme/cloud_editor_dark");
    assert_eq!(ace_theme(false), "ace/theme/cloud_editor");
}

#[test]
fn edit_button_needs_update_and_no_helm_release() {
    let info = |k: KindRef, verbs: &[&str]| KindInfo {
        kind: k,
        verbs: verbs.iter().map(|v| (*v).to_owned()).collect(),
        category: Category::TopLevel,
    };
    let r = row(Some("shop"), "cfg");
    let kinds = [info(kind("", "ConfigMap"), &["update"]), info(kind("helm.sh", "Release"), &["update"]), info(kind("", "Event"), &["list"])];
    assert_eq!(
        edit_target(Some(kind("", "ConfigMap")), &kinds, &r),
        Some(EditTarget { kind: kind("", "ConfigMap"), namespace: Some("shop".into()), name: "cfg".into() })
    );
    assert_eq!(edit_target(Some(kind("helm.sh", "Release")), &kinds, &r), None);
    assert_eq!(edit_target(Some(kind("helm.sh", "Chart")), &kinds, &r), None, "not listed");
    assert_eq!(edit_target(Some(kind("", "Event")), &kinds, &r), None);
    assert_eq!(edit_target(None, &kinds, &r), None);
}

#[test]
fn save_checks_locally_first() {
    let errors = vec![(4, "bad indent".to_owned()), (9, "x".to_owned())];
    assert_eq!(check(true, false, &[], "a", "b"), Check::Ignore);
    assert_eq!(check(false, true, &[], "a", "b"), Check::Ignore);
    assert_eq!(check(false, false, &errors, "a", "b"), Check::Fix(4, "Fix the YAML first: line 5: bad indent".into()));
    assert_eq!(check(false, false, &[], "a", "a"), Check::NoChanges);
    assert_eq!(check(false, false, &[], "a", "b"), Check::Validate);
}

#[test]
fn after_the_server_check() {
    assert_eq!(after_check(false, "a", "b"), Modal::Unlock);
    assert_eq!(after_check(true, "a", "b"), Modal::Confirm(vec![Line::Removed("a".into()), Line::Added("b".into())]));
    assert_eq!(confirm("a", "a"), Modal::Confirm(vec![Line::Skipped(1)]));
    assert!(unlocked(&Some(Ok(session(true)))));
    assert!(!unlocked(&Some(Ok(session(false)))));
    assert!(!unlocked(&Some(Err("x".into()))));
    assert!(!unlocked(&None));
}

#[test]
fn leaving_with_edits_asks_first() {
    assert_eq!(leave(true, After::Close), Some(Modal::Discard(After::Close)));
    assert_eq!(leave(true, After::Reload), Some(Modal::Discard(After::Reload)));
    assert_eq!(leave(false, After::Close), None);
    assert_eq!(discard_texts(After::Close), ("Discard your edits?", "Discard and close"));
    assert_eq!(discard_texts(After::Reload), ("Reload and discard your edits?", "Discard and reload"));
}

#[test]
fn prompts_and_texts() {
    assert!(can_go(false, false, "", "acme"));
    assert!(!can_go(true, false, "", "acme"));
    assert!(!can_go(false, true, "acm", "acme"));
    assert!(can_go(false, true, " acme ", "acme"));
    assert_eq!(save_heading("ConfigMap"), "Save ConfigMap?");
    assert_eq!(heading(None), "Loading…");
    assert_eq!(heading(Some(Ok(session(true)))), "cfg · Edit");
    assert_eq!(heading(Some(Err("x".into()))), "Edit");
    assert_eq!(first_error(&[]), None);
    assert_eq!(first_error(&[(2, "bad".into())]), Some((2, "Line 3: bad".into())));
    assert_eq!(first_error(&[(2, "bad".into()), (5, "worse".into())]), Some((2, "Line 3: bad (+1 more)".into())));
    assert_eq!(dirty_text(true), "Edited · not saved");
    assert_eq!(dirty_text(false), "No changes");
}

#[test]
fn args_serialize() {
    let target = session(true).target;
    assert_eq!(serde_json::to_value(OpenArgs { target }).unwrap()["target"]["name"], "cfg");
    assert_eq!(serde_json::to_value(DirtyArgs { dirty: true }).unwrap(), serde_json::json!({ "dirty": true }));
    assert_eq!(
        serde_json::to_value(UnlockArgs { context: "acme".into(), unlocked: true }).unwrap(),
        serde_json::json!({ "context": "acme", "unlocked": true })
    );
    assert_eq!(
        serde_json::to_value(SaveArgs { yaml: "a: 1", dry_run: true }).unwrap(),
        serde_json::json!({ "yaml": "a: 1", "dry_run": true })
    );
}
