use super::*;
use crate::testing::MockApi;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{StatusCause, StatusDetails};

fn target(path: &str, file_name: &str) -> UploadTarget {
    UploadTarget {
        context: "acme-uat".into(),
        namespace: "shop".into(),
        pod: "web".into(),
        container: "app".into(),
        path: path.into(),
        file_name: file_name.into(),
        overwrite: false,
    }
}

#[tokio::test]
async fn bad_targets_are_refused_before_any_request() {
    let api = MockApi::new();
    let err = upload(api.client(), &target("tmp", "a.txt"), b"x").await.unwrap_err();
    assert!(matches!(&err, Error::Message(m) if m.contains("absolute path")), "{err}");
    let err = upload(api.client(), &target("/tmp", ""), b"x").await.unwrap_err();
    assert!(matches!(&err, Error::Message(m) if m.contains("without slashes")), "{err}");
    let err = upload(api.client(), &target("/tmp", "a/b"), b"x").await.unwrap_err();
    assert!(matches!(&err, Error::Message(m) if m.contains("without slashes")), "{err}");
    let big = vec![0u8; MAX_BYTES + 1];
    let err = upload(api.client(), &target("/tmp", "big.bin"), &big).await.unwrap_err();
    assert!(matches!(&err, Error::Message(m) if m == "Files over 512 MiB aren't supported."), "{err}");
    assert!(api.calls().is_empty());
}

#[tokio::test]
async fn upload_exec_failure_is_an_error() {
    let api = MockApi::new();
    let mut t = target("/tmp/", "a.txt");
    t.overwrite = true;
    let err = upload(api.client(), &t, b"hello").await.unwrap_err();
    assert!(matches!(err, Error::Kube(_)), "{err:?}");
    let calls = api.calls_to("POST", "/api/v1/namespaces/shop/pods/web/exec");
    let calls = if calls.is_empty() { api.calls_to("GET", "/api/v1/namespaces/shop/pods/web/exec") } else { calls };
    assert_eq!(calls.len(), 1);
    let q = &calls[0].query;
    for part in ["stdin=true", "stdout=true", "stderr=true", "container=app", "command=%2Ftmp%2F", "command=a.txt", "command=1"] {
        assert!(q.contains(part), "{q} lacks {part}");
    }
}

#[tokio::test]
async fn browse_exec_failure_is_an_error() {
    let api = MockApi::new();
    let err = browse(api.client(), "shop", "web", "app", "/data", true).await.unwrap_err();
    assert!(matches!(err, Error::Kube(_)), "{err:?}");
    let q = &api.calls()[0].query;
    // The entry limit is spliced into the script; the path is an argument.
    assert!(q.contains("1000") && !q.contains("MAX"), "{q}");
    assert!(q.contains("command=%2Fdata") && !q.contains("stdin=true"), "{q}");
}

#[test]
fn scripts_never_splice_arguments() {
    assert!(UPLOAD_SCRIPT.contains("exit 17") && UPLOAD_SCRIPT.contains("exit 18"));
    assert!(BROWSE_SCRIPT.contains("\"$1\"") && BROWSE_SCRIPT.contains("-gt MAX"));
}

#[test]
fn listing_edge_cases() {
    let l = parse_listing("");
    assert_eq!(l.path, "/");
    assert!(!l.writable && !l.truncated && l.entries.is_empty() && l.suggestions.is_empty());
    let l = parse_listing("/\n-\nE\nEd\nEdw\nunknown\nEfwr x\nT\n");
    assert!(!l.writable && l.truncated);
    assert_eq!(l.entries.len(), 1);
    assert_eq!(l.entries[0].name, "x");
    assert!(!l.entries[0].dir && l.entries[0].writable && l.entries[0].readable);
}

#[tokio::test]
async fn read_all_reads_text_and_drops_bad_utf8() {
    assert_eq!(read_all(&mut &b"hello\nworld"[..]).await, "hello\nworld");
    assert_eq!(read_all(&mut &[0xffu8, 0xfe][..]).await, "");
}

fn failed(message: Option<&str>, causes: Option<Vec<StatusCause>>) -> Status {
    Status {
        status: Some("Failure".into()),
        message: message.map(str::to_owned),
        details: Some(StatusDetails { causes, ..StatusDetails::default() }),
        ..Status::default()
    }
}

#[test]
fn exit_codes_need_an_exit_code_cause() {
    assert_eq!(exit_code(&Status::default()), None);
    assert_eq!(exit_code(&failed(None, None)), None);
    let other = StatusCause { reason: Some("Other".into()), message: Some("1".into()), field: None };
    assert_eq!(exit_code(&failed(None, Some(vec![other]))), None);
}

#[test]
fn friendly_messages() {
    let s = failed(Some("command terminated with non-zero exit code: Permission denied"), None);
    // Empty stderr: the status message is used.
    assert!(friendly("  ", &s).contains("can't write there"));
    assert!(friendly("exec: \"sh\": executable file not found in $PATH", &s).contains("no shell"));
    assert_eq!(friendly("disk full", &s), "disk full");
    assert_eq!(friendly("", &failed(None, None)), "");
}

use crate::coverage_tests::ws_fake;
use ws_fake::{Step, err, exit, out, start, success};

#[tokio::test]
async fn uploads_send_every_byte() {
    let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    let fake = start(vec![Step::Wait(|s| s.stdin_closed), out("/tmp/a.txt\n"), success()]).await;
    let got = upload(fake.client, &target("/tmp/", "a.txt"), &data).await.unwrap();
    assert_eq!(got, UploadOutcome::Uploaded { bytes: 200_000, path: "/tmp/a.txt".into() });
    let seen = fake.seen.lock().unwrap().clone();
    assert_eq!(seen.stdin, data);
    let req = fake.requests.lock().unwrap()[0].clone();
    assert!(req.contains("command=0"), "overwrite off: {req}");
}

#[tokio::test]
async fn upload_outcomes_from_exit_codes() {
    let fake = start(vec![out("/tmp/a.txt\n"), exit(17)]).await;
    let got = upload(fake.client, &target("/tmp", "a.txt"), b"x").await.unwrap();
    assert_eq!(got, UploadOutcome::Exists { path: "/tmp/a.txt".into() });

    let fake = start(vec![out("/nope\n"), exit(18)]).await;
    let e = upload(fake.client, &target("/nope/a.txt", "a.txt"), b"x").await.unwrap_err();
    assert!(matches!(&e, Error::Message(m) if m == "The directory /nope doesn't exist in the container."), "{e}");

    let fake = start(vec![out("/etc/a.txt\n"), err("sh: can't create /etc/a.txt: Read-only file system\n"), exit(2)]).await;
    let e = upload(fake.client, &target("/etc/", "a.txt"), b"x").await.unwrap_err();
    assert!(matches!(&e, Error::Message(m) if m.contains("read-only filesystem")), "{e}");

    let fake = start(vec![]).await;
    let e = upload(fake.client, &target("/tmp/", "a.txt"), b"x").await.unwrap_err();
    assert!(matches!(&e, Error::Message(m) if m.contains("closed before")), "{e}");
}

#[tokio::test]
async fn browsing_lists_a_folder() {
    let fake = start(vec![out("/data\nw\nS /tmp\nEdwr cache\nEf-r a.txt\nT\n"), success()]).await;
    let l = browse(fake.client, "shop", "web", "app", "/data", false).await.unwrap();
    assert_eq!(l.path, "/data");
    assert!(l.writable && l.truncated);
    assert_eq!(l.suggestions, ["/tmp"]);
    assert_eq!(l.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["cache", "a.txt"]);

    let fake = start(vec![err("Can't open /secret\n"), exit(3)]).await;
    let e = browse(fake.client, "shop", "web", "app", "/secret", false).await.unwrap_err();
    assert!(matches!(&e, Error::Message(m) if m == "Can't open /secret"), "{e}");

    let fake = start(vec![]).await;
    let e = browse(fake.client, "shop", "web", "app", "/", true).await.unwrap_err();
    assert!(matches!(&e, Error::Message(m) if m.contains("closed before")), "{e}");
}
