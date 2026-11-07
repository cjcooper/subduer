use super::*;

fn ctx(name: &str, file: &str, cluster: &str, user: &str) -> ManagedContext {
    ManagedContext { name: name.into(), cluster: cluster.into(), user: user.into(), server: String::new(), provider: None, file: file.into() }
}

fn list() -> Vec<ManagedContext> {
    vec![ctx("a", "main", "c1", "u1"), ctx("b", "main", "shared", "u2"), ctx("c", "main", "shared", "u3"), ctx("d", "other", "c4", "u1")]
}

#[test]
fn moves_only_within_its_file() {
    assert_eq!(moved(&list(), "c", 0).unwrap(), ["c", "a", "b", "d"]);
    assert_eq!(moved(&list(), "a", 2).unwrap(), ["b", "c", "a", "d"]);
    // Out of its file, nowhere, or not at all.
    assert_eq!(moved(&list(), "c", 3), None);
    assert_eq!(moved(&list(), "d", 2), None);
    assert_eq!(moved(&list(), "a", 0), None);
    assert_eq!(moved(&list(), "zz", 0), None);
}

#[test]
fn where_a_drop_lands() {
    // Row 0 dropped on row 2: before it, or after it.
    assert_eq!(drop_index(0, 2, false), 1);
    assert_eq!(drop_index(0, 2, true), 2);
    // Row 2 dropped on row 0.
    assert_eq!(drop_index(2, 0, false), 0);
    assert_eq!(drop_index(2, 0, true), 1);
    // On itself: stays.
    assert_eq!(drop_index(1, 1, false), 1);
    assert_eq!(drop_index(1, 1, true), 1);
}

#[test]
fn what_goes_with_a_context() {
    assert_eq!(goes_with(&list(), "a"), (Some("c1".into()), None));
    assert_eq!(goes_with(&list(), "b"), (None, Some("u2".into())));
    assert_eq!(goes_with(&[ctx("x", "f", "", "")], "x"), (None, None));
}

#[test]
fn says_what_was_removed() {
    assert_eq!(removed_text(&RemovedContext { context: "a".into(), cluster: None, user: None }), "Removed \"a\".");
    let r = RemovedContext { context: "a".into(), cluster: Some("c1".into()), user: Some("u1".into()) };
    assert_eq!(removed_text(&r), "Removed \"a\", with its cluster \"c1\" and user \"u1\".");
}

#[test]
fn moving_rows_reorders_and_drops_by_half() {
    let mut l = list();
    reorder(&mut l, &["c".into(), "a".into(), "b".into(), "d".into()]);
    assert_eq!(l.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["c", "a", "b", "d"]);
    assert!(drops_after(60.0, 40.0, 30.0));
    assert!(!drops_after(50.0, 40.0, 30.0));
}

#[test]
fn files_bound_where_rows_move() {
    assert_eq!(file_bounds(&list(), 1), (0, 2));
    assert_eq!(file_bounds(&list(), 3), (3, 3));
    assert!(drag_in_file(&list(), Some("b"), "main"));
    assert!(!drag_in_file(&list(), Some("d"), "main"));
    assert!(!drag_in_file(&list(), None, "main"));
    let managed = |files: &[&str]| ManagedContexts { contexts: list(), files: files.iter().map(|f| (*f).to_owned()).collect(), current: None };
    assert!(several_files(&managed(&["main", "other"])));
    assert!(!several_files(&managed(&["main"])));
    assert!(!several_files(&managed(&["main", "empty"])), "a file with no contexts doesn't count");
}

#[test]
fn row_wording() {
    assert_eq!(server_label(""), "No cluster entry");
    assert_eq!(server_label("https://10.0.0.1"), "https://10.0.0.1");
    assert_eq!(prod_title(true), "Production: changes need extra confirmation. Click to unmark.");
    assert_eq!(prod_title(false), "Not production. Click to mark it as production.");
    assert_eq!(prod_label("a"), "Production: a");
    assert_eq!(move_label("a", true), "Move a up");
    assert_eq!(move_label("a", false), "Move a down");
    assert_eq!(remove_title(true, true), "This window is on it: switch to another context first");
    assert_eq!(remove_title(false, true), "Another window is on it: switch that window to another context first");
    assert_eq!(remove_title(false, false), "");
    assert_eq!(remove_label(true), "Removing…");
    assert_eq!(remove_label(false), "Remove");
}

#[test]
fn confirming_says_what_goes() {
    assert_eq!(
        confirm_text("a", None, None),
        "Remove \"a\" from the kubeconfig? Its cluster and user stay: other contexts use them. kubectl and other tools lose it too."
    );
    assert_eq!(
        confirm_text("a", Some("c1"), None),
        "Remove \"a\" from the kubeconfig? Its cluster \"c1\" goes too: nothing else uses it. kubectl and other tools lose it too."
    );
    assert_eq!(
        confirm_text("a", Some("c1"), Some("u1")),
        "Remove \"a\" from the kubeconfig? Its cluster \"c1\" and user \"u1\" go too: nothing else uses them. kubectl and other tools lose it too."
    );
}
