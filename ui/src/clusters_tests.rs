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
