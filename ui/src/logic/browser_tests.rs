use super::*;

fn entry(name: &str, dir: bool, writable: bool, readable: bool) -> DirEntry {
    DirEntry { name: name.into(), dir, writable, readable }
}

#[test]
fn join_adds_one_slash() {
    assert_eq!(join("/tmp", "a.txt"), "/tmp/a.txt");
    assert_eq!(join("/tmp/", "a.txt"), "/tmp/a.txt");
    assert_eq!(join("/", "etc"), "/etc");
    assert_eq!(join("/tmp", ""), "/tmp/");
}

#[test]
fn folder_of_is_the_parent_or_itself() {
    assert_eq!(folder_of("/tmp/a.txt"), "/tmp");
    assert_eq!(folder_of("/tmp/"), "/tmp");
    assert_eq!(folder_of("/var/log/app/x.log"), "/var/log/app");
    assert_eq!(folder_of("/a.txt"), "/");
    assert_eq!(folder_of("/"), "/");
    assert_eq!(folder_of("  /tmp/a  "), "/tmp");
    assert_eq!(folder_of("relative"), "/");
    assert_eq!(folder_of(""), "/");
}

#[test]
fn crumb_paths_build_each_level() {
    let owned = |v: &[(&str, &str)]| v.iter().map(|(a, b)| ((*a).to_owned(), (*b).to_owned())).collect::<Vec<_>>();
    assert_eq!(crumb_paths("/"), owned(&[("/", "/")]));
    assert_eq!(crumb_paths(""), owned(&[("/", "/")]));
    assert_eq!(
        crumb_paths("/var/log/app/"),
        owned(&[("/", "/"), ("var", "/var"), ("log", "/var/log"), ("app", "/var/log/app")])
    );
    assert_eq!(crumb_paths("//tmp//x"), owned(&[("/", "/"), ("tmp", "/tmp"), ("x", "/tmp/x")]));
}

#[test]
fn entry_state_for_uploads() {
    use BrowseMode::Upload;
    assert_eq!(entry_state(Upload, &entry("d", true, true, true)), ("writable", false, true));
    assert_eq!(entry_state(Upload, &entry("d", true, false, true)), ("read-only", true, true));
    assert_eq!(entry_state(Upload, &entry("f", false, true, true)), ("replace", false, true));
    assert_eq!(entry_state(Upload, &entry("f", false, false, true)), ("read-only", true, true));
}

#[test]
fn entry_state_for_downloads() {
    use BrowseMode::Download;
    assert_eq!(entry_state(Download, &entry("d", true, false, true)), ("", false, true));
    assert_eq!(entry_state(Download, &entry("d", true, true, false)), ("no access", true, false));
    assert_eq!(entry_state(Download, &entry("f", false, false, true)), ("download", false, true));
    assert_eq!(entry_state(Download, &entry("f", false, true, false)), ("can't read", true, false));
}

#[test]
fn entry_titles() {
    assert_eq!(entry_title(BrowseMode::Upload, true, "/tmp/x"), "Open /tmp/x");
    assert_eq!(entry_title(BrowseMode::Download, true, "/tmp/x"), "Open /tmp/x");
    assert_eq!(entry_title(BrowseMode::Upload, false, "/tmp/a"), "Upload as /tmp/a (asks before replacing)");
    assert_eq!(entry_title(BrowseMode::Download, false, "/tmp/a"), "Download /tmp/a");
}

#[test]
fn folder_status_notes() {
    assert_eq!(folder_status(BrowseMode::Upload, true), "✓ writable");
    assert_eq!(folder_status(BrowseMode::Upload, false), "read-only: files can't be saved here");
    assert_eq!(folder_status(BrowseMode::Download, true), "");
    assert_eq!(folder_status(BrowseMode::Download, false), "");
}

#[test]
fn chips_only_for_uploads_with_suggestions() {
    let some = vec!["/tmp".to_owned()];
    assert!(show_chips(BrowseMode::Upload, &some));
    assert!(!show_chips(BrowseMode::Upload, &[]));
    assert!(!show_chips(BrowseMode::Download, &some));
    assert!(at_root("/"));
    assert!(!at_root("/tmp"));
}
