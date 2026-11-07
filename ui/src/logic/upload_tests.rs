use super::*;

fn listing(path: &str, writable: bool) -> DirListing {
    DirListing { path: path.into(), writable, entries: vec![], truncated: false, suggestions: vec![] }
}

fn prompt() -> UploadPrompt {
    UploadPrompt { context: "acme-dev".into(), namespace: "shop".into(), pod: "web-1".into(), container: "app".into() }
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
fn outcomes_lead_to_the_next_step() {
    assert_eq!(
        after_upload(Ok(UploadOutcome::Uploaded { bytes: 3, path: "/tmp/a".into() })),
        (Phase::Done { bytes: 3, path: "/tmp/a".into() }, None)
    );
    assert_eq!(after_upload(Ok(UploadOutcome::Exists { path: "/tmp/a".into() })), (Phase::Exists("/tmp/a".into()), None));
    assert_eq!(after_upload(Err("boom".into())), (Phase::Choosing, Some("boom".into())));
    assert!(cancel_goes_back(&Phase::Exists("/tmp/a".into())));
    assert!(!cancel_goes_back(&Phase::Choosing));
    assert!(!cancel_goes_back(&Phase::Uploading));
}

#[test]
fn size_limit_and_destination() {
    assert_eq!(size_error(MAX_BYTES), None);
    assert_eq!(size_error(MAX_BYTES + 1.0).as_deref(), Some("Files over 512 MiB aren't supported."));
    assert!(dest_ok(" /tmp/"));
    assert!(!dest_ok("tmp/"));
    assert!(can_upload(Some(10.0), "/tmp/"));
    assert!(!can_upload(None, "/tmp/"));
    assert!(!can_upload(Some(MAX_BYTES + 1.0), "/tmp/"));
    assert!(!can_upload(Some(10.0), "tmp/"));
}

#[test]
fn browsing_keeps_a_typed_file_name() {
    assert_eq!(start_dest(), "/tmp/");
    assert_eq!(browsed_dest("/var", "/tmp/report.csv"), "/var/report.csv");
    assert_eq!(browsed_dest("/var/", "/tmp/"), "/var/");
    assert_eq!(browsed_dest("/var", "report"), "/var/report");
    assert!(retry_at_root(true, true));
    assert!(!retry_at_root(false, true));
    assert!(!retry_at_root(true, false));
}

#[test]
fn the_request_and_texts() {
    assert_eq!(
        target(prompt(), " /tmp/ ", "a.txt".into(), true),
        UploadTarget {
            context: "acme-dev".into(),
            namespace: "shop".into(),
            pod: "web-1".into(),
            container: "app".into(),
            path: "/tmp/".into(),
            file_name: "a.txt".into(),
            overwrite: true,
        }
    );
    assert_eq!(target_line(&prompt()), "app · shop/web-1 · acme-dev");
    assert_eq!(file_size(2048.0), " · 2.0 KiB");
}
