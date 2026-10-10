use super::*;
use crate::testing::MockApi;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Status;
use std::cell::RefCell;

#[tokio::test]
async fn failed_upgrade_is_an_error_not_a_panic() {
    let api = MockApi::new();
    let target = ExecTarget { namespace: "shop".into(), pod: "web".into(), container: "app".into() };
    let (_in_tx, input) = tokio::sync::mpsc::unbounded_channel();
    let (_size_tx, resize) = tokio::sync::mpsc::unbounded_channel();
    let emitted = RefCell::new(vec![]);
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        run(api.client(), &target, (80, 24), input, resize, |o| emitted.borrow_mut().push(o)),
    )
    .await
    .expect("exec didn't return");
    let err = result.unwrap_err();
    assert!(matches!(err, crate::Error::Kube(_)), "{err:?}");
    assert!(emitted.borrow().is_empty());
    let calls = api.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].path, "/api/v1/namespaces/shop/pods/web/exec");
    for part in ["container=app", "stdin=true", "stdout=true", "tty=true", "command=%2Fbin%2Fsh", "command=-c"] {
        assert!(calls[0].query.contains(part), "{} lacks {part}", calls[0].query);
    }
}

#[test]
fn shell_prefers_bash() {
    assert!(SHELL.contains("exec bash") && SHELL.contains("exec sh"));
}

fn failure(message: Option<&str>) -> Option<Status> {
    Some(Status { status: Some("Failure".into()), message: message.map(str::to_owned), ..Status::default() })
}

#[test]
fn other_failures_keep_their_message() {
    assert_eq!(ended(failure(Some("error dialing backend: EOF"))), Ended::Failed("error dialing backend: EOF".into()));
    assert_eq!(ended(failure(Some(""))), Ended::Failed("Shell ended.".into()));
    assert_eq!(ended(failure(None)), Ended::Failed("Shell ended.".into()));
    // Mentions /bin/sh, but isn't a missing shell.
    let m = "/bin/sh: permission denied";
    assert_eq!(ended(failure(Some(m))), Ended::Failed(m.into()));
}

use crate::coverage_tests::ws_fake;
use ws_fake::{Step, out, start, success};

async fn session(steps: Vec<Step>, keys: &[&[u8]], resizes: &[(u16, u16)]) -> (ws_fake::Fake, crate::Result<Ended>, Vec<ExecOutput>) {
    let fake = start(steps).await;
    let target = ExecTarget { namespace: "shop".into(), pod: "web".into(), container: "app".into() };
    let (in_tx, input) = tokio::sync::mpsc::unbounded_channel();
    let (size_tx, resize) = tokio::sync::mpsc::unbounded_channel();
    for k in keys {
        in_tx.send(k.to_vec()).unwrap();
    }
    for r in resizes {
        size_tx.send(*r).unwrap();
    }
    let emitted = RefCell::new(vec![]);
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        run(fake.client.clone(), &target, (80, 24), input, resize, |o| emitted.borrow_mut().push(o)),
    )
    .await
    .expect("shell didn't end");
    (fake, result, emitted.into_inner())
}

#[tokio::test]
async fn shell_session_round_trip() {
    let steps = vec![
        out("$ "),
        Step::Wait(|s| s.stdin.ends_with(b"exit\n") && s.resizes.len() >= 2),
        success(),
    ];
    let (fake, result, emitted) = session(steps, &[b"ls\n", b"exit\n"], &[(120, 40)]).await;
    assert_eq!(result.unwrap(), Ended::Exited);
    let b64 = base64::engine::general_purpose::STANDARD;
    assert_eq!(emitted, [ExecOutput::Data(b64.encode("$ "))]);
    let seen = fake.seen.lock().unwrap().clone();
    assert_eq!(seen.stdin, b"ls\nexit\n");
    assert!(seen.resizes[0].contains("80") && seen.resizes[0].contains("24"), "{:?}", seen.resizes);
    assert!(seen.resizes.iter().any(|r| r.contains("120") && r.contains("40")), "{:?}", seen.resizes);
}

#[tokio::test]
async fn shell_failures_are_emitted() {
    let no_shell = Step::Status(serde_json::json!({"metadata": {}, "status": "Failure", "reason": "InternalError",
        "message": "OCI runtime exec failed: exec: \"/bin/sh\": stat /bin/sh: no such file or directory"}));
    let (_, result, emitted) = session(vec![no_shell], &[], &[]).await;
    let Ended::Failed(message) = result.unwrap() else { panic!("expected a failure") };
    assert!(message.contains("no shell"));
    assert_eq!(emitted, [ExecOutput::Exited(message)]);

    // Dropped without a status.
    let (_, result, emitted) = session(vec![], &[], &[]).await;
    assert_eq!(result.unwrap(), Ended::Failed("Shell ended.".into()));
    assert_eq!(emitted, [ExecOutput::Exited("Shell ended.".into())]);
}
