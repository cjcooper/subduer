use super::*;
use crate::app::tests_unit::{context, store, store_on};

fn listing(path: &str, writable: bool) -> DirListing {
    DirListing { path: path.into(), writable, entries: vec![], truncated: false, suggestions: vec![] }
}

#[test]
fn dest_hint_needs_an_absolute_path() {
    assert_eq!(dest_hint("tmp/", None, None), "Use an absolute path, e.g. /tmp/");
    assert_eq!(dest_hint("", None, Some("a.txt")), "Use an absolute path, e.g. /tmp/");
    assert_eq!(dest_hint("  ", None, None), "Use an absolute path, e.g. /tmp/");
}

#[test]
fn dest_hint_warns_about_the_read_only_folder_on_screen() {
    let ro = listing("/etc", false);
    let warn = "This folder is read-only for the container user; pick a writable one.";
    assert_eq!(dest_hint("/etc/", Some(&ro), Some("a.txt")), warn);
    assert_eq!(dest_hint("/etc/app.conf", Some(&ro), None), warn);
    // Another folder, or a writable one, is fine.
    assert_eq!(dest_hint("/tmp/", Some(&ro), None), "The file keeps its name in this folder.");
    let rw = listing("/etc/", true);
    assert_eq!(dest_hint("/etc/", Some(&rw), None), "The file keeps its name in this folder.");
}

#[test]
fn dest_hint_explains_where_the_file_goes() {
    assert_eq!(dest_hint(" /tmp/ ", None, Some("report.csv")), "Saved as /tmp/report.csv");
    assert_eq!(
        dest_hint("/tmp/out.csv", None, Some("report.csv")),
        "Saved as /tmp/out.csv, or as /tmp/out.csv/report.csv if /tmp/out.csv is a folder"
    );
    assert_eq!(dest_hint("/tmp/", None, None), "The file keeps its name in this folder.");
    assert_eq!(
        dest_hint("/tmp/out.csv", None, None),
        "A folder keeps the file's name; anything else is the new file's path."
    );
}

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
