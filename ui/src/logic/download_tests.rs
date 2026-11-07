use super::*;

#[test]
fn a_file_is_an_absolute_path_that_isnt_a_folder() {
    assert!(is_file("/var/log/app.log"));
    assert!(is_file("  /var/log/app.log  "));
    assert!(!is_file("/var/log/"));
    assert!(!is_file("var/log/app.log"));
    assert!(!is_file(""));
}

#[test]
fn enter_downloads_a_file_or_browses_its_folder() {
    assert_eq!(on_enter("/var/log/app.log"), Enter::Download);
    assert_eq!(on_enter("/var/log/"), Enter::Browse("/var/log".into()));
    assert_eq!(on_enter("var"), Enter::Browse("/".into()));
}

#[test]
fn texts() {
    assert_eq!(folder_path("/var/log"), "/var/log/");
    assert_eq!(folder_path("/"), "/");
    assert_eq!(source("app", "shop", "web-1"), "app · shop/web-1");
    assert_eq!(size(2048), "2.0 KiB");
}

#[test]
fn args_serialize() {
    let browse = BrowseArgs { namespace: "shop".into(), pod: "web-1".into(), container: "app".into(), path: ".".into(), suggest: false };
    assert_eq!(
        serde_json::to_value(&browse).unwrap(),
        serde_json::json!({ "namespace": "shop", "pod": "web-1", "container": "app", "path": ".", "suggest": false })
    );
    let download = DownloadArgs { namespace: "shop".into(), pod: "web-1".into(), container: "app".into(), path: "/a".into() };
    assert_eq!(serde_json::to_value(&download).unwrap()["path"], "/a");
    assert_eq!(serde_json::to_value(RevealArgs { path: "/d/a".into() }).unwrap(), serde_json::json!({ "path": "/d/a" }));
}
