//! Interactive shells in containers (`kubectl exec -it`), over the API
//! server's websocket. Callers must check the safety gate first: a shell can
//! change anything the container can.

use base64::Engine;
use futures::SinkExt;
use k8s_openapi::api::core::v1::Pod;
use kube::Client;
use kube::api::{Api, AttachParams, TerminalSize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::Result;
use crate::types::{ExecOutput, ExecTarget};

/// Prefer bash, fall back to sh. Images without `/bin/sh` (distroless) fail
/// with the API server's "executable file not found" message.
const SHELL: &str = "if command -v bash >/dev/null 2>&1; then exec bash; else exec sh; fi";

/// Runs a shell until it exits or the future is dropped. Keystrokes arrive on
/// `input`, terminal size changes (cols, rows) on `resize`. Says how it ended.
pub async fn run<F>(
    client: Client,
    target: &ExecTarget,
    size: (u16, u16),
    mut input: UnboundedReceiver<Vec<u8>>,
    mut resize: UnboundedReceiver<(u16, u16)>,
    emit: F,
) -> Result<Ended>
where
    F: Fn(ExecOutput),
{
    let pods: Api<Pod> = Api::namespaced(client, &target.namespace);
    let params = AttachParams::interactive_tty().container(target.container.clone());
    let mut process = pods.exec(&target.pod, ["/bin/sh", "-c", SHELL], &params).await?;
    let (Some(mut stdin), Some(mut stdout)) = (process.stdin(), process.stdout()) else {
        let message = "The API server didn't open the terminal streams.".to_owned();
        emit(ExecOutput::Exited(message.clone()));
        return Ok(Ended::Failed(message));
    };
    let mut sizer = process.terminal_size();
    let status = process.take_status();
    let to_size = |(width, height): (u16, u16)| TerminalSize { width, height };
    if let Some(s) = sizer.as_mut() {
        let _ = s.send(to_size(size)).await;
    }

    let b64 = base64::engine::general_purpose::STANDARD;
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        tokio::select! {
            read = stdout.read(&mut buf) => match read {
                Ok(0) | Err(_) => break,
                Ok(n) => emit(ExecOutput::Data(b64.encode(&buf[..n]))),
            },
            Some(bytes) = input.recv() => {
                if stdin.write_all(&bytes).await.is_err() {
                    break;
                }
            }
            Some(next) = resize.recv() => {
                if let Some(s) = sizer.as_mut() {
                    let _ = s.send(to_size(next)).await;
                }
            }
        }
    }
    let outcome = match status {
        Some(status) => ended(status.await),
        None => Ended::Failed("Shell ended.".to_owned()),
    };
    if let Ended::Failed(message) = &outcome {
        emit(ExecOutput::Exited(message.clone()));
    }
    Ok(outcome)
}

/// How a session finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    /// The shell process exited (`exit`, Ctrl-D), whatever its exit code.
    Exited,
    /// Anything else: no shell in the image, connection dropped, … The message
    /// has also been emitted as [`ExecOutput::Exited`].
    Failed(String),
}

fn ended(status: Option<k8s_openapi::apimachinery::pkg::apis::meta::v1::Status>) -> Ended {
    let Some(s) = status else { return Ended::Failed("Shell ended.".to_owned()) };
    // A bare `exit` returns the last command's status, so a non-zero exit
    // code is still the user leaving the shell.
    if s.status.as_deref() == Some("Success") || s.reason.as_deref() == Some("NonZeroExitCode") {
        return Ended::Exited;
    }
    Ended::Failed(match s.message.filter(|m| !m.is_empty()) {
        Some(m) if m.contains("/bin/sh") && m.contains("no such file") => {
            "This image has no shell (/bin/sh not found), e.g. a distroless image.".to_owned()
        }
        Some(m) => m,
        None => "Shell ended.".to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::Status;

    fn status(status: &str, reason: &str, message: &str) -> Option<Status> {
        Some(Status {
            status: Some(status.into()),
            reason: Some(reason.into()).filter(|r: &String| !r.is_empty()),
            message: Some(message.into()),
            ..Status::default()
        })
    }

    #[test]
    fn exits_close_and_failures_explain() {
        assert_eq!(ended(status("Success", "", "")), Ended::Exited);
        let nonzero = "command terminated with non-zero exit code: exit status 1";
        assert_eq!(ended(status("Failure", "NonZeroExitCode", nonzero)), Ended::Exited);
        let no_shell = "OCI runtime exec failed: exec: \"/bin/sh\": stat /bin/sh: no such file or directory";
        assert!(matches!(ended(status("Failure", "InternalError", no_shell)), Ended::Failed(m) if m.contains("no shell")));
        assert_eq!(ended(None), Ended::Failed("Shell ended.".into()));
    }
}

#[cfg(test)]
#[path = "exec_tests.rs"]
mod coverage_tests;
