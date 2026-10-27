use super::*;

fn windows(list: &[(&str, Option<&str>)]) -> Vec<(String, Option<String>)> {
    list.iter().map(|(l, c)| ((*l).to_owned(), c.map(str::to_owned))).collect()
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|n| (*n).to_owned()).collect()
}

#[test]
fn windows_on_a_removed_context_move_to_the_current_one() {
    let w = windows(&[("main", Some("gone")), ("main-2", Some("lab")), ("main-3", None)]);
    let plan = plan(&w, &names(&["lab", "ops"]), Some("ops"), None);
    assert_eq!(plan.switch, [("main".to_owned(), ContextRemoved { removed: "gone".into(), switch_to: "ops".into() })]);
    assert!(plan.close.is_empty() && plan.empty.is_none());
    // kubectl's current context is gone too (or unset): the first one.
    let plan = super::plan(&w, &names(&["lab", "ops"]), Some("gone"), None);
    assert_eq!(plan.switch[0].1.switch_to, "lab");
    // Nothing gone: nothing to do.
    assert_eq!(super::plan(&w, &names(&["gone", "lab"]), None, None), Plan::default());
}

#[test]
fn no_contexts_left_keeps_one_window() {
    let w = windows(&[("main-2", Some("a")), ("main", Some("b")), ("main-3", None)]);
    let plan = plan(&w, &[], None, Some("main-3"));
    assert_eq!(plan.empty.as_deref(), Some("main-3"));
    assert_eq!(plan.close, ["main-2", "main"]);
    assert!(plan.switch.is_empty());
    // Nothing focused (another app is): the first window, `main`.
    let plan = super::plan(&w, &[], None, None);
    assert_eq!((plan.empty.as_deref(), plan.close.len()), (Some("main"), 2));
    // A focused logs window isn't a main window.
    assert_eq!(super::plan(&w, &[], None, Some("logs-1")).empty.as_deref(), Some("main"));
    assert_eq!(super::plan(&[], &[], None, None), Plan::default());
}

#[test]
fn a_change_is_seen_in_the_files() {
    let dir = std::env::temp_dir().join(format!("subduer-kubeconfig-watch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("config");
    let files = [file.clone()];
    let missing = signature(&files);
    assert_eq!(missing[0].1, None);
    std::fs::write(&file, "apiVersion: v1\n").unwrap();
    let written = signature(&files);
    assert_ne!(written, missing);
    std::fs::write(dir.join("config.lock"), "").unwrap();
    assert!(signature(&files)[0].2);
    let _ = std::fs::remove_dir_all(&dir);
}
