use super::*;
use crate::testing::MockApi;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("subduer-dl-cov-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn relative_paths_are_refused_before_any_request() {
    let api = MockApi::new();
    let dir = scratch("rel");
    let err = download(api.client(), "shop", "web", "app", "etc/hosts", &dir.join("hosts")).await.unwrap_err();
    assert!(matches!(&err, Error::Message(m) if m.contains("absolute path") && m.contains("\"etc/hosts\"")), "{err}");
    assert!(api.calls().is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn exec_failures_are_errors_and_leave_no_files() {
    // The fake server can't upgrade to a websocket, like a proxy that strips it.
    let api = MockApi::new();
    let dir = scratch("exec");
    let dest = dir.join("app.log");
    let err = download(api.client(), "shop", "web", "app", "/var/log/app.log", &dest).await.unwrap_err();
    assert!(matches!(err, Error::Kube(_)), "{err:?}");
    assert!(!err.user_message().is_empty());
    let calls = api.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].path, "/api/v1/namespaces/shop/pods/web/exec");
    for part in ["container=app", "stdout=true", "stderr=true", "command=sh", "command=-c", "command=%2Fvar%2Flog%2Fapp.log"] {
        assert!(calls[0].query.contains(part), "{} lacks {part}", calls[0].query);
    }
    assert!(!calls[0].query.contains("stdin=true"));
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn script_checks_before_reading() {
    for code in ["exit 21", "exit 19", "exit 20", "exec cat \"$1\""] {
        assert!(SCRIPT.contains(code), "{code}");
    }
}

#[test]
fn sanitized_names() {
    assert_eq!(sanitize("a/b\\c"), "a_b_c");
    assert_eq!(sanitize("bad\u{7}name"), "bad_name");
    assert_eq!(sanitize("   "), "download");
    assert_eq!(sanitize("."), "download");
    assert_eq!(sanitize(" app.log "), "app.log");
}

#[test]
fn free_names_without_extensions() {
    let dir = scratch("names");
    std::fs::write(dir.join("Makefile"), b"x").unwrap();
    assert_eq!(unique_path(&dir, "Makefile"), dir.join("Makefile (1)"));
    std::fs::write(dir.join("a_b.tar.gz"), b"x").unwrap();
    assert_eq!(unique_path(&dir, "a/b.tar.gz"), dir.join("a_b.tar (1).gz"));
    std::fs::remove_dir_all(&dir).unwrap();
}

use crate::coverage_tests::ws_fake;
use ws_fake::{err, exit, out, start, success};

#[test]
fn unopened_streams_are_an_error() {
    assert_eq!(opened::<()>(None, EXEC_STREAMS).unwrap_err().to_string(), "The API server didn't open the exec streams.");
}

#[tokio::test]
async fn downloads_stream_to_disk() {
    let fake = start(vec![out("hello "), out("world"), success()]).await;
    let dir = scratch("ok");
    let dest = dir.join("app.log");
    let n = download(fake.client, "shop", "web", "app", "/var/log/app.log", &dest).await.unwrap();
    assert_eq!(n, 11);
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "hello world");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no .part left");
    let req = fake.requests.lock().unwrap()[0].clone();
    assert!(req.contains("/api/v1/namespaces/shop/pods/web/exec?") && req.contains("container=app"), "{req}");

    // No extension: `name.part` while streaming, then renamed.
    let fake = start(vec![out("x"), success()]).await;
    let dest = dir.join("Makefile");
    assert_eq!(download(fake.client, "shop", "web", "app", "/src/Makefile", &dest).await.unwrap(), 1);
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "x");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn download_exit_codes_explain_themselves() {
    let dir = scratch("codes");
    let cases = [
        (21, "/x doesn't exist in the container."),
        (19, "/x isn't a regular file (a folder or device?)."),
        (20, "The container user can't read /x."),
    ];
    for (code, want) in cases {
        let fake = start(vec![exit(code)]).await;
        let err = download(fake.client, "shop", "web", "app", "/x", &dir.join("x")).await.unwrap_err();
        assert!(matches!(&err, Error::Message(m) if m == want), "{code}: {err}");
    }
    let fake = start(vec![err("cat: read error: I/O error\n"), exit(1)]).await;
    let e = download(fake.client, "shop", "web", "app", "/x", &dir.join("x")).await.unwrap_err();
    assert!(matches!(&e, Error::Message(m) if m == "cat: read error: I/O error"), "{e}");
    // Partial data is never left behind.
    let fake = start(vec![out("partial")]).await;
    let e = download(fake.client, "shop", "web", "app", "/x", &dir.join("x")).await.unwrap_err();
    assert!(matches!(&e, Error::Message(m) if m.contains("closed before")), "{e}");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn download_local_file_errors() {
    let dir = scratch("local");
    let fake = start(vec![out("x"), success()]).await;
    let e = download(fake.client, "shop", "web", "app", "/x", &dir.join("missing").join("x")).await.unwrap_err();
    assert!(matches!(&e, Error::Message(m) if m.starts_with("Can't create")), "{e}");
    // The destination is a non-empty folder: the rename fails.
    let busy = dir.join("busy");
    std::fs::create_dir_all(&busy).unwrap();
    std::fs::write(busy.join("f"), b"x").unwrap();
    let fake = start(vec![out("x"), success()]).await;
    let e = download(fake.client, "shop", "web", "app", "/x", &busy).await.unwrap_err();
    assert!(matches!(&e, Error::Message(m) if m.starts_with("Can't save")), "{e}");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Writing the file fails part way: the `.part` file is a pipe whose reader
/// has gone, so every write is refused.
#[cfg(unix)]
#[tokio::test]
async fn a_failed_write_is_an_error_and_leaves_nothing() {
    let dir = scratch("write");
    let dest = dir.join("app.log");
    let part = dir.join("app.log.part");
    let made = std::process::Command::new("mkfifo").arg(&part).status().unwrap();
    assert!(made.success());
    // The reader opens the pipe (once the download opens it to write) and leaves.
    let reader = {
        let part = part.clone();
        std::thread::spawn(move || drop(std::fs::File::open(part).unwrap()))
    };
    let fake = start(vec![ws_fake::Step::Pause(std::time::Duration::from_millis(300)), out("data"), success()]).await;
    let e = download(fake.client, "shop", "web", "app", "/var/log/app.log", &dest).await.unwrap_err();
    reader.join().unwrap();
    assert!(matches!(&e, Error::Message(m) if m.starts_with(&format!("Writing {} failed: ", part.display()))), "{e}");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    std::fs::remove_dir_all(&dir).unwrap();
}
