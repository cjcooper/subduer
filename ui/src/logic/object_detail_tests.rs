use super::*;
use subduer_types::{InfoTable, InfoTableRow};

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

    let mut r = InfoTableRow::new(vec!["data-0".into(), "20Gi".into(), "2026-10-16T12:00:00Z".into()]);
    r.fill = Some(0.87);
    let t = InfoTable { columns: vec!["Claim".into(), "Size".into(), "Since".into()], numeric: vec![1], times: vec![2], rows: vec![r.clone()], ..Default::default() };
    assert_eq!(info_key(&t, &r, 0), Key::Text("data-0".into()));
    assert_eq!(info_key(&t, &r, 1), Key::Num(20.0 * 1024f64.powi(3)));
    assert!(matches!(info_key(&t, &r, 2), Key::Num(n) if n < 0.0));
    assert_eq!(info_key(&t, &r, 3), Key::Num(0.87), "the Used bar");
    assert_eq!(info_key(&t, &InfoTableRow::new(vec![]), 1), Key::Missing);
}

fn event(type_: &str) -> EventSummary {
    EventSummary {
        type_: type_.into(),
        reason: String::new(),
        message: String::new(),
        count: 1,
        first_seen: String::new(),
        last_seen: String::new(),
        source: String::new(),
        object: None,
    }
}

fn condition(type_: &str, status: &str) -> Condition {
    Condition { type_: type_.into(), status: status.into(), reason: String::new(), message: String::new(), last_transition: None }
}

fn info_row(value: &str) -> InfoRow {
    InfoRow { key: "k".into(), value: value.into(), mono: false, time: false, link: None, url: None }
}

fn link(kind: &str, name: &str) -> ObjectLink {
    ObjectLink { kind: kind.into(), namespace: Some("shop".into()), name: name.into() }
}

fn entry(key: &str, value: Option<&str>, size: usize) -> DataEntry {
    DataEntry { key: key.into(), value: value.map(str::to_owned), size, binary: value.is_none() }
}

#[test]
fn loading_decisions() {
    use crate::app::tests_unit::kind;
    assert!(skip_load(true, true));
    assert!(!skip_load(false, true) && !skip_load(true, false));
    assert!(wants_node_history(&kind("", "Node")));
    assert!(!wants_node_history(&kind("", "ConfigMap")));
    // Only the object still selected.
    assert!(!keep_result(false, false, false, false, false));
    assert!(keep_result(false, true, true, true, true), "a foreground load always lands");
    assert!(!keep_result(true, true, true, false, false), "editing started meanwhile");
    assert!(!keep_result(true, true, false, true, true), "a failed refresh keeps the good render");
    assert!(keep_result(true, true, false, true, false));
    assert!(keep_result(true, true, false, false, true));
    let args = DetailsArgs { kind: kind("", "Secret"), namespace: None, name: "db".into() };
    assert_eq!(serde_json::to_value(&args).unwrap()["name"], "db");
}

#[test]
fn sections_and_summaries() {
    assert_eq!(kind_section_id("ConfigMap"), "configmap");
    assert_eq!(events_section("ClusterRole", None, &[]), None);
    let ns = events_section("Namespace", None, &[]).unwrap();
    assert_eq!((ns.id, ns.title, ns.empty, ns.summary.as_str()), ("ns-warnings", "Recent warnings", "No warnings in it.", "none"));
    let node = events_section("Node", None, &[event("Normal"), event("Normal")]).unwrap();
    assert_eq!((node.id, node.title, node.empty, node.summary.as_str()), ("events", "Events", "No events for it.", "2"));
    let cm = events_section("ConfigMap", Some("shop"), &[event("Warning"), event("Normal")]).unwrap();
    assert_eq!(cm.summary, "2 · ⚠ 1");
    assert!(checks_access("ServiceAccount") && !checks_access("Secret"));
    let n = NodeResources { pods: 12, pod_capacity: 29, ..Default::default() };
    assert_eq!(node_summary(&n), "12 of 29 pods");
    let fargate = NodeResources { one_pod_hosting: Some("Fargate".into()), ..n };
    assert_eq!(node_summary(&fargate), "Fargate");
    assert_eq!(hosting_note("Fargate"), "Pod count not shown: Fargate runs one pod per node, so the node is always full by design.");
    assert_eq!(fmt_count(12.4), "12");
    assert_eq!(ago_text("5m", "2026-10-16T12:00:00Z"), "5m ago · 2026-10-16T12:00:00Z");
    assert_eq!(label_chip("app", "web"), "app=web");
    assert_eq!(conditions_summary(&[]), "0");
    assert_eq!(conditions_summary(&[condition("Ready", "True")]), "1");
    assert_eq!(conditions_summary(&[condition("Ready", "True"), condition("Ready", "False")]), "2 · 1 unhealthy");
}

#[test]
fn info_rows_pick_how_to_show_the_value() {
    let time = InfoRow { time: true, ..info_row("2026-10-16T12:00:00Z") };
    assert_eq!(row_value(&time, true), RowValue::Time);
    assert_eq!(row_value(&time, false), RowValue::Plain, "not a time after all");
    assert_eq!(row_value(&info_row("a\nb"), false), RowValue::Block);
    let url = InfoRow { url: Some("https://x".into()), link: Some(link("apps/Deployment", "web")), ..info_row("x") };
    assert_eq!(row_value(&url, false), RowValue::Url("https://x".into()));
    let linked = InfoRow { link: Some(link("apps/Deployment", "web")), ..info_row("web") };
    assert_eq!(row_value(&linked, false), RowValue::Link(link("apps/Deployment", "web")));
    assert_eq!(time_text("5m ago", "T"), "5m ago · T");
    assert_eq!(link_title(&link("apps/Deployment", "web")), "Open Deployment web");
    assert_eq!(link_title(&link("", "web")), "Open  web");
}

#[test]
fn info_sections_and_tables() {
    let section = |rows: Vec<InfoRow>, table: Option<InfoTable>| InfoSection {
        id: "s".into(),
        title: "S".into(),
        summary: String::new(),
        rows,
        table,
        empty: None,
    };
    assert!(section_empty(&section(vec![], None)));
    assert!(section_empty(&section(vec![], Some(InfoTable::default()))));
    assert!(!section_empty(&section(vec![info_row("x")], None)));
    let mut full = InfoTableRow::new(vec!["a".into()]);
    full.fill = Some(1.0);
    full.link = Some(link("core/PersistentVolumeClaim", "data-0"));
    let mut warn = InfoTableRow::new(vec!["b".into()]);
    warn.fill = Some(0.874);
    warn.warn = true;
    let t = InfoTable { columns: vec!["A".into(), "B".into()], numeric: vec![1], times: vec![0], rows: vec![full.clone()], ..Default::default() };
    assert_eq!(sort_table("quota"), "detail:quota");
    assert!(has_fill(&t));
    assert!(!has_fill(&InfoTable::default()));
    assert_eq!(sort_headers(&t), ["A", "B", FILL_HEADER]);
    assert_eq!((head_class(&t, 0), head_class(&t, 1)), ("", "num"));
    assert!(time_cell(&t, 0, true));
    assert!(!time_cell(&t, 0, false) && !time_cell(&t, 1, true));
    assert_eq!(
        fill(&full),
        Some(Fill { warning: false, critical: true, title: "100% used".into(), style: "width:100%".into() })
    );
    assert_eq!(
        fill(&warn),
        Some(Fill { warning: true, critical: false, title: "87% used".into(), style: "width:87%".into() })
    );
    assert_eq!(fill(&InfoTableRow::new(vec![])), None);
    assert_eq!(row_title(&full).as_deref(), Some("Open data-0"));
    assert_eq!(row_title(&warn), None);
}

#[test]
fn access_checks() {
    assert!(can_ask("pods") && !can_ask("  "));
    assert_eq!(VERBS[0], "get");
    let allowed = AccessCheck { allowed: true, reason: String::new() };
    assert_eq!(access_answer(&allowed), ("badge badge-ready", "Allowed", String::new()));
    let denied = AccessCheck { allowed: false, reason: String::new() };
    assert_eq!(access_answer(&denied), ("badge badge-warning", "Denied", "No rule allows it.".into()));
    let why = AccessCheck { allowed: false, reason: "RBAC: nope".into() };
    assert_eq!(access_answer(&why).2, "RBAC: nope");
    let args = AccessArgs {
        account_namespace: "shop".into(),
        account: "web".into(),
        verb: "get".into(),
        resource: "pods".into(),
        namespace: String::new(),
        name: String::new(),
    };
    assert_eq!(serde_json::to_value(&args).unwrap()["account"], "web");
    assert_eq!(serde_json::to_value(UrlArgs { url: "u".into() }).unwrap()["url"], "u");
    assert_eq!(serde_json::to_value(SecretArgs { namespace: "n".into(), name: "s".into() }).unwrap()["name"], "s");
}

#[test]
fn drafts_start_from_the_values() {
    let entries = [entry("a", Some("1"), 1), entry("blob", None, 9)];
    let values = data_values(&entries);
    assert_eq!(drafts(&entries, &values), [text(0, "a", "1"), binary(1, "blob", 9)]);
    // A Secret's values come separately; a missing one counts as binary.
    let secret = [entry("password", None, 6)];
    let values = original(&[("password", Some("s3cret"))]);
    assert_eq!(drafts(&secret, &values), [Draft { size: 6, ..text(0, "password", "s3cret") }]);
    assert_eq!(drafts(&secret, &[]), [binary(0, "password", 6)]);
    assert_eq!(Draft::new(4), Draft { id: 4, key: String::new(), value: String::new(), binary: false, size: 0 });
}

#[test]
fn drafts_are_edited_by_id() {
    let mut d = vec![text(0, "a", "1"), text(1, "b", "2")];
    assert_eq!(draft_field(&d, 1, |x| x.key.clone()), "b");
    assert_eq!(draft_field(&d, 9, |x| x.key.clone()), "");
    update_draft(&mut d, 1, |x| x.value = "3".into());
    update_draft(&mut d, 9, |x| x.value = "never".into());
    assert_eq!(d[1].value, "3");
    remove_draft(&mut d, 0);
    assert_eq!(d, [text(1, "b", "3")]);
    assert!(bad_key("a b") && !bad_key("") && !bad_key("ok"));
    assert_eq!(value_rows(""), "2");
    assert_eq!(value_rows("a\nb\nc"), "4");
    assert_eq!(value_rows(&"x\n".repeat(40)), "16");
    assert_eq!(size_line(12, false), "12 B");
    assert_eq!(size_line(3072, true), "3.0 KiB · binary");
    assert_eq!(binary_note(300), "Binary value, 300 B: kept as is.");
}

#[test]
fn editing_and_saving() {
    use crate::app::tests_unit::kind;
    use subduer_types::{Category, ClusterInfo, KindInfo};
    let cm = kind("", "ConfigMap");
    let cluster = |verbs: &[&str]| ClusterInfo {
        context: "acme-dev".into(),
        kinds: vec![KindInfo { kind: cm.clone(), verbs: verbs.iter().map(|v| (*v).to_owned()).collect(), category: Category::Config }],
        namespaces: vec![],
    };
    assert!(can_patch(Some(&cm), Some(&cluster(&["get", "patch"]))));
    assert!(!can_patch(Some(&cm), Some(&cluster(&["get"]))));
    assert!(!can_patch(Some(&kind("", "Secret")), Some(&cluster(&["patch"]))));
    assert!(!can_patch(None, Some(&cluster(&["patch"]))));
    assert!(!can_patch(Some(&cm), None));
    assert!(editable(None, true));
    assert!(!editable(Some("immutable"), true) && !editable(None, false));
    assert!(no_access(None, false));
    assert!(!no_access(Some("immutable"), false) && !no_access(None, true));
    assert_eq!((edit_label(true), edit_label(false)), ("Edit (shows values)", "Edit"));
    assert_eq!(dirty_label(true, "db"), "Secret db");
    assert_eq!(dirty_label(false, "cfg"), "ConfigMap cfg");
    let some: Pending = Ok(Some((vec![set("a", "1")], summary(&["a"], &[], &[]))));
    let bad: Pending = Err("Every key needs a name.".into());
    assert_eq!(dirty(true, &some, "ConfigMap cfg").as_deref(), Some("ConfigMap cfg"));
    assert_eq!(dirty(true, &bad, "x").as_deref(), Some("x"), "an unsaveable draft is unsaved too");
    assert_eq!(dirty(true, &Ok(None), "x"), None);
    assert_eq!(dirty(false, &some, "x"), None);
    assert!(can_save(&some) && !can_save(&bad) && !can_save(&Ok(None)));
    let (k, save) = save_request(Some(cm.clone()), some.clone(), "42").unwrap();
    assert_eq!(k, cm);
    assert_eq!(
        save,
        SaveData { resource_version: "42".into(), changes: vec![set("a", "1")], added: vec!["a".into()], changed: vec![], removed: vec![] }
    );
    assert_eq!(save_request(None, some.clone(), "42"), None);
    assert_eq!(save_request(Some(cm), Ok(None), "42"), None);
    assert_eq!((data_summary(0), data_summary(1), data_summary(3)), ("empty".into(), "1 key".into(), "3 keys".into()));
    assert_eq!(pending_text(&bad), Err("Every key needs a name.".into()));
    assert_eq!(pending_text(&Ok(None)), Ok("No changes yet.".into()));
    assert_eq!(pending_text(&some), Ok("1 change to save.".into()));
    let two: Pending = Ok(Some((vec![], summary(&["a"], &[], &["b"]))));
    assert_eq!(pending_text(&two), Ok("2 changes to save.".into()));
}
