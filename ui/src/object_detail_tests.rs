use super::*;

fn text(id: u32, key: &str, value: &str) -> Draft {
    Draft { id, key: key.into(), value: value.into(), binary: false, size: value.len() }
}

fn binary(id: u32, key: &str, size: usize) -> Draft {
    Draft { id, key: key.into(), value: String::new(), binary: true, size }
}

fn original(pairs: &[(&str, Option<&str>)]) -> Vec<(String, Option<String>)> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), v.map(str::to_owned))).collect()
}

fn set(key: &str, value: &str) -> DataChange {
    DataChange { key: key.into(), value: Some(value.into()) }
}

fn unset(key: &str) -> DataChange {
    DataChange { key: key.into(), value: None }
}

fn summary(added: &[&str], changed: &[&str], removed: &[&str]) -> SaveSummary {
    let v = |s: &[&str]| s.iter().map(|x| (*x).to_owned()).collect();
    SaveSummary { added: v(added), changed: v(changed), removed: v(removed) }
}

#[test]
fn valid_key_accepts_api_key_names() {
    for k in ["a", "app.properties", "APP_CONFIG", "tls-cert", "x.y-z_1", "..a", ".env", &"k".repeat(253)] {
        assert!(valid_key(k), "{k:?} should be valid");
    }
}

#[test]
fn valid_key_rejects_bad_names() {
    for k in ["", ".", "..", "a b", "a/b", "ä", "key:", "tab\t", &"k".repeat(254)] {
        assert!(!valid_key(k), "{k:?} should be invalid");
    }
}

#[test]
fn diff_without_changes_is_none() {
    let orig = original(&[("a", Some("1")), ("b", Some("2"))]);
    assert_eq!(diff(&orig, &[text(0, "a", "1"), text(1, "b", "2")]), Ok(None));
    assert_eq!(diff(&[], &[]), Ok(None));
}

#[test]
fn diff_reports_added_changed_and_removed_keys() {
    let orig = original(&[("keep", Some("same")), ("edit", Some("old")), ("drop", Some("x"))]);
    let draft = [text(0, "keep", "same"), text(1, "edit", "new"), text(3, "add", "fresh")];
    let (changes, s) = diff(&orig, &draft).unwrap().unwrap();
    assert_eq!(changes, [unset("drop"), set("edit", "new"), set("add", "fresh")]);
    assert_eq!(s, summary(&["add"], &["edit"], &["drop"]));
}

#[test]
fn diff_treats_a_rename_as_remove_and_add() {
    let orig = original(&[("old-name", Some("v"))]);
    let (changes, s) = diff(&orig, &[text(0, "new-name", "v")]).unwrap().unwrap();
    assert_eq!(changes, [unset("old-name"), set("new-name", "v")]);
    assert_eq!(s, summary(&["new-name"], &[], &["old-name"]));
}

#[test]
fn diff_counts_an_empty_value_as_a_change() {
    let orig = original(&[("a", Some("1"))]);
    let (changes, s) = diff(&orig, &[text(0, "a", "")]).unwrap().unwrap();
    assert_eq!(changes, [set("a", "")]);
    assert_eq!(s, summary(&[], &["a"], &[]));
}

#[test]
fn diff_keeps_binary_entries_untouched() {
    // A binary entry kept in the draft is never rewritten.
    let orig = original(&[("blob", None), ("a", Some("1"))]);
    assert_eq!(diff(&orig, &[binary(0, "blob", 300), text(1, "a", "1")]), Ok(None));
    // But it can be removed.
    let (changes, s) = diff(&orig, &[text(1, "a", "1")]).unwrap().unwrap();
    assert_eq!(changes, [unset("blob")]);
    assert_eq!(s, summary(&[], &[], &["blob"]));
}

#[test]
fn diff_writes_text_over_a_key_that_had_no_text_value() {
    // A Secret's original values are unknown (None): a text draft for the
    // same key always counts as a change.
    let orig = original(&[("password", None)]);
    let (changes, s) = diff(&orig, &[text(0, "password", "s3cret")]).unwrap().unwrap();
    assert_eq!(changes, [set("password", "s3cret")]);
    assert_eq!(s, summary(&[], &["password"], &[]));
}

#[test]
fn diff_rejects_empty_and_invalid_keys() {
    let orig = original(&[("a", Some("1"))]);
    assert_eq!(diff(&orig, &[text(0, "", "x")]), Err("Every key needs a name.".to_owned()));
    assert_eq!(
        diff(&orig, &[text(0, "a b", "x")]),
        Err("“a b” isn't a valid key: use letters, digits, '-', '_' and '.'.".to_owned())
    );
    // Binary entries are checked too.
    assert_eq!(
        diff(&orig, &[binary(0, "..", 1)]),
        Err("“..” isn't a valid key: use letters, digits, '-', '_' and '.'.".to_owned())
    );
}

#[test]
fn diff_rejects_duplicate_keys() {
    let orig = original(&[]);
    assert_eq!(diff(&orig, &[text(0, "a", "1"), text(1, "a", "2")]), Err("“a” is used twice.".to_owned()));
    assert_eq!(diff(&orig, &[binary(0, "a", 1), text(1, "a", "2")]), Err("“a” is used twice.".to_owned()));
}

#[test]
fn diff_reports_the_first_problem() {
    let draft = [text(0, "a", "1"), text(1, "", "2"), text(2, "a", "3")];
    assert_eq!(diff(&[], &draft), Err("Every key needs a name.".to_owned()));
}

#[test]
fn data_values_keeps_keys_and_text_values() {
    let entries = vec![
        DataEntry { key: "a".into(), value: Some("1".into()), size: 1, binary: false },
        DataEntry { key: "blob".into(), value: None, size: 9, binary: true },
    ];
    assert_eq!(data_values(&entries), original(&[("a", Some("1")), ("blob", None)]));
    assert!(data_values(&[]).is_empty());
}

#[test]
fn size_text_units() {
    assert_eq!(size_text(0), "0 B");
    assert_eq!(size_text(1023), "1023 B");
    assert_eq!(size_text(1024), "1.0 KiB");
    assert_eq!(size_text(1536), "1.5 KiB");
    assert_eq!(size_text(1024 * 1024 - 1), "1024.0 KiB");
    assert_eq!(size_text(1024 * 1024), "1.0 MiB");
    assert_eq!(size_text(5 * 1024 * 1024 + 512 * 1024), "5.5 MiB");
}

#[test]
fn details_tables_sort_by_column_type() {
    use crate::sort::Key;
    use subduer_types::{InfoTable, InfoTableRow};
    let mut r = InfoTableRow::new(vec!["data-0".into(), "20Gi".into(), "2026-10-16T12:00:00Z".into()]);
    r.fill = Some(0.87);
    let t = InfoTable { columns: vec!["Claim".into(), "Size".into(), "Since".into()], numeric: vec![1], times: vec![2], rows: vec![r.clone()], ..Default::default() };
    assert_eq!(info_key(&t, &r, 0), Key::Text("data-0".into()));
    assert_eq!(info_key(&t, &r, 1), Key::Num(20.0 * 1024f64.powi(3)));
    assert!(matches!(info_key(&t, &r, 2), Key::Num(n) if n < 0.0));
    assert_eq!(info_key(&t, &r, 3), Key::Num(0.87), "the Used bar");
    assert_eq!(info_key(&t, &InfoTableRow::new(vec![]), 1), Key::Missing);
}
