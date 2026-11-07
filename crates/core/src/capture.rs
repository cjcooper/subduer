//! Packet captures of one pod, for proving which TLS versions it uses.
//!
//! An ephemeral container joins the running pod (no restart) and shares its
//! network; the app runs tcpdump in it over exec and receives the packets
//! as a pcap stream, so nothing is written inside the pod. It asks for no
//! extra capabilities, like `kubectl debug --profile=baseline`, which is
//! enough on EKS Fargate. Kubernetes can't remove an ephemeral container:
//! it sleeps for the capture's time limit and then exits, but stays listed
//! in the pod until the pod is replaced.
//!
//! Attaching changes the pod, so it's behind the safety gate.

use std::time::Duration;

use k8s_openapi::api::core::v1::Pod;
use kube::Client;
use kube::api::{Api, AttachParams, Patch, PatchParams};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::safety::SafetyGate;
use crate::{Error, Result};

/// Names of the containers this app attaches start with this.
pub const CONTAINER_PREFIX: &str = "subduer-capture-";
/// The interface captured: the pod's own. Not `any`, which also takes
/// loopback, where an app and its sidecars talk in plain text.
const INTERFACE: &str = "eth0";
/// Longest capture.
pub const MAX_SECS: u32 = 30 * 60;
/// Longest a capture container may live: the capture, and a few minutes to start and finish.
pub const MAX_LIFE_SECS: u32 = MAX_SECS + 5 * 60;

pub use crate::types::{CaptureFilter as Filter, CaptureTraffic as Traffic};

/// The byte after the TCP header is the TLS record type; 22 = handshake.
const HANDSHAKE: &str = "tcp[((tcp[12:1] & 0xf0) >> 2):1] = 0x16";

/// The tcpdump expression for a filter. `pod_ips` (the pod's addresses)
/// make inbound mean connections to the pod itself: without them, any
/// traffic on that port, which also takes the pod's own connections out to
/// other servers' port of that number.
pub fn expression(filter: &Filter, pod_ips: &[String]) -> String {
    let traffic = match &filter.traffic {
        Traffic::All => "tcp".to_owned(),
        Traffic::Inbound(port) if pod_ips.is_empty() => format!("tcp port {port}"),
        Traffic::Inbound(port) => {
            let hosts = |dir: &str| pod_ips.iter().map(|ip| format!("{dir} host {ip}")).collect::<Vec<_>>().join(" or ");
            format!("tcp and ((dst port {port} and ({})) or (src port {port} and ({})))", hosts("dst"), hosts("src"))
        }
        Traffic::Custom(e) => e.trim().to_owned(),
    };
    match (filter.handshakes_only, &filter.traffic) {
        (false, _) => traffic,
        (true, Traffic::All) => format!("tcp and {HANDSHAKE}"),
        (true, Traffic::Custom(_)) if traffic.is_empty() => format!("tcp and {HANDSHAKE}"),
        (true, Traffic::Custom(_)) => format!("({traffic}) and tcp and {HANDSHAKE}"),
        (true, Traffic::Inbound(_)) => format!("{traffic} and {HANDSHAKE}"),
    }
}

/// The ephemeral container: the image, sleeping for `secs` so it ends by
/// itself, with no capabilities asked for.
pub fn container_spec(name: &str, image: &str, secs: u32) -> Value {
    json!({
        "name": name,
        "image": image,
        "command": ["sleep", secs.to_string()],
        "stdin": false,
        "tty": false,
        "terminationMessagePolicy": "File",
    })
}

/// Adds a capture container to the pod and returns its name. `dry_run`
/// asks the API server whether it would accept it (RBAC, admission, the
/// pod existing) without changing anything, and needs no unlocking.
#[allow(clippy::too_many_arguments)]
pub async fn attach(
    client: &Client,
    gate: &SafetyGate,
    context: &str,
    namespace: &str,
    pod: &str,
    image: &str,
    secs: u32,
    dry_run: bool,
) -> Result<String> {
    if !dry_run {
        gate.ensure_writable(context)?;
    }
    if image.trim().is_empty() {
        return Err(Error::Message("choose the capture image first".into()));
    }
    if secs == 0 || secs > MAX_LIFE_SECS {
        return Err(Error::Message(format!("a capture lasts 1 to {} minutes", MAX_SECS / 60)));
    }
    let name = format!("{CONTAINER_PREFIX}{}", jiff::Timestamp::now().as_second());
    let patch = json!({ "spec": { "ephemeralContainers": [container_spec(&name, image.trim(), secs)] } });
    let params = PatchParams { dry_run, ..PatchParams::default() };
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    pods.patch_ephemeral_containers(pod, &params, &Patch::Strategic(&patch)).await?;
    tracing::info!(%context, %namespace, %pod, container = %name, %image, secs, dry_run, "capture container attached");
    Ok(name)
}

/// The pod's addresses (every family), to tell which way each connection went.
pub async fn pod_ips(client: &Client, namespace: &str, pod: &str) -> Result<Vec<String>> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let status = pods.get(pod).await?.status.unwrap_or_default();
    let mut ips: Vec<String> = status.pod_ips.unwrap_or_default().into_iter().map(|i| i.ip).collect();
    if ips.is_empty() {
        ips.extend(status.pod_ip);
    }
    Ok(ips)
}

/// Where a capture container is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Not reported yet, or still being created or pulled.
    Starting,
    /// Running; the image it runs, as the node resolved it (with its digest).
    Running { image_id: String },
    /// It won't run: an image that can't be pulled, an exited container, ….
    Failed(String),
}

/// The container's state from the pod's status.
pub fn state(pod: &Value, container: &str) -> State {
    let Some(status) = pod["status"]["ephemeralContainerStatuses"]
        .as_array()
        .and_then(|all| all.iter().find(|s| s["name"] == container))
    else {
        return State::Starting;
    };
    let s = &status["state"];
    if s["running"].is_object() {
        return State::Running { image_id: status["imageID"].as_str().unwrap_or_default().to_owned() };
    }
    let text = |v: &Value| {
        let reason = v["reason"].as_str().unwrap_or_default();
        match v["message"].as_str().filter(|m| !m.is_empty()) {
            Some(m) => format!("{reason}: {m}"),
            None => reason.to_owned(),
        }
    };
    if s["terminated"].is_object() {
        return State::Failed(format!("the capture container ended ({})", text(&s["terminated"])));
    }
    match s["waiting"]["reason"].as_str() {
        Some("ErrImagePull" | "ImagePullBackOff" | "InvalidImageName" | "CreateContainerError" | "CreateContainerConfigError") => {
            State::Failed(text(&s["waiting"]))
        }
        _ => State::Starting,
    }
}

/// Waits (up to `limit`) for the capture container to run; returns the
/// image it runs.
pub async fn wait_running(client: &Client, namespace: &str, pod: &str, container: &str, limit: Duration) -> Result<String> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let deadline = tokio::time::Instant::now() + limit;
    loop {
        // A `Pod` always converts.
        let current = serde_json::to_value(pods.get(pod).await?).unwrap_or_default();
        match state(&current, container) {
            State::Running { image_id } => return Ok(image_id),
            State::Failed(why) => return Err(Error::Message(why)),
            State::Starting if tokio::time::Instant::now() >= deadline => {
                return Err(Error::Message(format!("the capture container didn't start within {} s", limit.as_secs())));
            }
            State::Starting => tokio::time::sleep(Duration::from_secs(2)).await,
        }
    }
}

/// The command run in the container: tcpdump on the pod's interface,
/// writing pcap to stdout packet by packet, stopped after `secs`.
pub fn command(expression: &str, secs: u32) -> Vec<String> {
    let mut cmd: Vec<String> = ["timeout", "-s", "INT", &secs.to_string(), "tcpdump", "-i", INTERFACE, "-n", "-U", "-w", "-"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    if !expression.trim().is_empty() {
        cmd.push(expression.trim().to_owned());
    }
    cmd
}

/// How a capture went.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Captured {
    /// pcap bytes written to the output.
    pub bytes: u64,
    /// What tcpdump said: "listening on eth0…", packet counts, or errors.
    pub messages: String,
}

/// Runs tcpdump in the capture container for `secs` (or until [`stop`]) and
/// writes the pcap stream to `out`, showing each piece to `seen` as it
/// arrives. A read: it changes nothing in the cluster.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    client: &Client,
    namespace: &str,
    pod: &str,
    container: &str,
    expression: &str,
    secs: u32,
    out: &mut (impl AsyncWrite + Unpin),
    mut seen: impl FnMut(&[u8]),
) -> Result<Captured> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let params = AttachParams::default().container(container).stdin(false).stdout(true).stderr(true).tty(false);
    let mut process = pods.exec(pod, command(expression, secs), &params).await?;
    let (mut stdout, mut stderr) =
        crate::upload::opened(process.stdout().zip(process.stderr()), "the API server didn't open the capture's streams")?;
    // Both streams are read side by side until tcpdump exits.
    let copy = async {
        let mut bytes = 0u64;
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match stdout.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    out.write_all(&buf[..n]).await.map_err(|e| Error::Message(e.to_string()))?;
                    seen(&buf[..n]);
                    bytes += n as u64;
                }
            }
        }
        out.flush().await.map_err(|e| Error::Message(e.to_string()))?;
        Ok::<_, Error>(bytes)
    };
    let messages = async {
        let mut m = Vec::new();
        let _ = stderr.read_to_end(&mut m).await;
        String::from_utf8_lossy(&m).trim().to_owned()
    };
    let (bytes, messages) = tokio::join!(copy, messages);
    Ok(Captured { bytes: bytes?, messages })
}

/// Ends a running capture early: interrupts tcpdump, which finishes the
/// file, so [`run`] returns as if its time was up.
pub async fn stop(client: &Client, namespace: &str, pod: &str, container: &str) -> Result<()> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let params = AttachParams::default().container(container).stdin(false).stdout(false).stderr(true).tty(false);
    let mut process = pods.exec(pod, ["pkill", "-INT", "tcpdump"], &params).await?;
    futures::future::OptionFuture::from(process.take_status()).await;
    Ok(())
}

#[cfg(test)]
#[path = "capture_tests.rs"]
mod tests;
