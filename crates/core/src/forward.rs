//! Port-forwarding (`kubectl port-forward`): a local TCP listener on
//! 127.0.0.1 whose connections each get their own port-forward stream to a
//! pod. Services resolve to a ready backing pod and its target port, and
//! re-resolve after a failure (e.g. the pod was replaced).

use std::sync::{Arc, Mutex};

use k8s_openapi::api::core::v1::{Pod, Service};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::api::{Api, ListParams};
use kube::{Client, ResourceExt};
use tokio::net::{TcpListener, TcpStream};

use crate::types::{ForwardRequest, ForwardSource, PortOption};
use crate::{Error, Result};

/// Live counters for one forward, shared with the UI layer.
#[derive(Debug, Default)]
pub struct ForwardStats {
    pub active: usize,
    pub total: u64,
    pub last_error: Option<String>,
    /// `pod:port` connections go to.
    pub backend: Option<String>,
}

/// Called whenever stats change, so the UI can refresh.
pub type Notify = Arc<dyn Fn() + Send + Sync>;

/// Ports offered in the start dialog: a pod's declared TCP container ports,
/// or a service's TCP ports.
pub async fn port_options(client: &Client, namespace: &str, source: &ForwardSource) -> Result<Vec<PortOption>> {
    match source {
        ForwardSource::Pod { pod } => {
            let pod = Api::<Pod>::namespaced(client.clone(), namespace).get(pod).await?;
            let spec = pod.spec.unwrap_or_default();
            Ok(spec
                .init_containers
                .iter()
                .flatten()
                .chain(&spec.containers)
                .flat_map(|c| {
                    c.ports.iter().flatten().filter(|p| is_tcp(p.protocol.as_deref())).map(|p| PortOption {
                        port: p.container_port as u16,
                        name: p.name.clone(),
                        detail: Some(c.name.clone()),
                    })
                })
                .collect())
        }
        ForwardSource::Service { service } => {
            let svc = Api::<Service>::namespaced(client.clone(), namespace).get(service).await?;
            Ok(svc
                .spec
                .and_then(|s| s.ports)
                .unwrap_or_default()
                .into_iter()
                .filter(|p| is_tcp(p.protocol.as_deref()))
                .map(|p| PortOption {
                    port: p.port as u16,
                    name: p.name.clone(),
                    detail: p.target_port.map(|t| match t {
                        IntOrString::Int(n) => format!("→ {n}"),
                        IntOrString::String(s) => format!("→ {s}"),
                    }),
                })
                .collect())
        }
    }
}

fn is_tcp(protocol: Option<&str>) -> bool {
    protocol.is_none_or(|p| p.eq_ignore_ascii_case("TCP"))
}

/// Binds the local port (127.0.0.1 only) and checks the target resolves, so
/// mistakes show up when starting rather than on first connection.
pub async fn bind(client: &Client, req: &ForwardRequest) -> Result<(TcpListener, (String, u16))> {
    let backend = resolve(client, req).await?;
    let listener = TcpListener::bind(("127.0.0.1", req.local_port))
        .await
        .map_err(|e| Error::NotFound(format!("can't listen on 127.0.0.1:{}: {e}", req.local_port)))?;
    Ok((listener, backend))
}

/// Accepts connections until the future is dropped.
pub async fn serve(
    client: Client,
    req: ForwardRequest,
    listener: TcpListener,
    backend: (String, u16),
    stats: Arc<Mutex<ForwardStats>>,
    notify: Notify,
) {
    let backend = Arc::new(Mutex::new(Some(backend)));
    {
        let b = backend.lock().unwrap().clone();
        stats.lock().unwrap().backend = b.map(|(pod, port)| format!("{pod}:{port}"));
    }
    while let Ok((socket, _)) = listener.accept().await {
        let (client, req, backend, stats, notify) =
            (client.clone(), req.clone(), backend.clone(), stats.clone(), notify.clone());
        tokio::spawn(async move {
            {
                let mut s = stats.lock().unwrap();
                s.active += 1;
                s.total += 1;
            }
            notify();
            let result = connect(&client, &req, &backend, &stats, socket).await;
            {
                let mut s = stats.lock().unwrap();
                s.active -= 1;
                match result {
                    Ok(()) => {}
                    Err(e) => {
                        s.last_error = Some(e);
                        // Services pick a pod again on the next connection.
                        if matches!(req.source, ForwardSource::Service { .. }) {
                            *backend.lock().unwrap() = None;
                        }
                    }
                }
            }
            notify();
        });
    }
}

async fn connect(
    client: &Client,
    req: &ForwardRequest,
    backend: &Mutex<Option<(String, u16)>>,
    stats: &Mutex<ForwardStats>,
    mut socket: TcpStream,
) -> std::result::Result<(), String> {
    let current = backend.lock().unwrap().clone();
    let (pod, port) = match current {
        Some(b) => b,
        None => {
            let b = resolve(client, req).await.map_err(|e| e.user_message())?;
            *backend.lock().unwrap() = Some(b.clone());
            stats.lock().unwrap().backend = Some(format!("{}:{}", b.0, b.1));
            b
        }
    };
    let pods: Api<Pod> = Api::namespaced(client.clone(), &req.namespace);
    let mut forwarder = pods
        .portforward(&pod, &[port])
        .await
        .map_err(|e| Error::from(e).user_message())?;
    let mut stream = forwarder.take_stream(port).ok_or("port-forward stream wasn't opened")?;
    let errors = forwarder.take_error(port);
    let copied = tokio::io::copy_bidirectional(&mut socket, &mut stream).await;
    drop(stream);
    // The kubelet reports problems (e.g. nothing listening on the port) here.
    if let Some(errors) = errors
        && let Ok(Some(message)) = tokio::time::timeout(std::time::Duration::from_millis(200), errors).await
    {
        return Err(message);
    }
    let _ = forwarder.join().await;
    copied.map(|_| ()).map_err(|e| format!("connection error: {e}"))
}

/// The pod and pod port to connect to.
async fn resolve(client: &Client, req: &ForwardRequest) -> Result<(String, u16)> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), &req.namespace);
    match &req.source {
        ForwardSource::Pod { pod } => {
            let p = pods.get(pod).await?;
            if p.status.as_ref().and_then(|s| s.phase.as_deref()) != Some("Running") {
                return Err(Error::NotFound(format!("pod {pod} isn't running")));
            }
            Ok((pod.clone(), req.remote_port))
        }
        ForwardSource::Service { service } => {
            let svc = Api::<Service>::namespaced(client.clone(), &req.namespace).get(service).await?;
            let spec = svc.spec.unwrap_or_default();
            let port = spec
                .ports
                .iter()
                .flatten()
                .find(|p| p.port as u16 == req.remote_port)
                .ok_or_else(|| Error::NotFound(format!("port {} on service {service}", req.remote_port)))?;
            let selector = spec
                .selector
                .filter(|s| !s.is_empty())
                .ok_or_else(|| Error::NotFound(format!("pod selector on service {service}")))?;
            let labels = selector.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(",");
            let mut candidates: Vec<Pod> = pods.list(&ListParams::default().labels(&labels)).await?.items;
            candidates.retain(is_ready);
            candidates.sort_by_key(|p| p.name_any());
            let pod = candidates
                .into_iter()
                .next()
                .ok_or_else(|| Error::NotFound(format!("ready pod behind service {service}")))?;
            let target = match &port.target_port {
                None => req.remote_port,
                Some(IntOrString::Int(n)) => *n as u16,
                Some(IntOrString::String(name)) => named_port(&pod, name)
                    .ok_or_else(|| Error::NotFound(format!("port named {name} in pod {}", pod.name_any())))?,
            };
            Ok((pod.name_any(), target))
        }
    }
}

fn is_ready(pod: &Pod) -> bool {
    pod.metadata.deletion_timestamp.is_none()
        && pod
            .status
            .as_ref()
            .and_then(|s| s.conditions.as_ref())
            .is_some_and(|c| c.iter().any(|c| c.type_ == "Ready" && c.status == "True"))
}

fn named_port(pod: &Pod, name: &str) -> Option<u16> {
    let spec = pod.spec.as_ref()?;
    spec.init_containers
        .iter()
        .flatten()
        .chain(&spec.containers)
        .flat_map(|c| c.ports.iter().flatten())
        .find(|p| p.name.as_deref() == Some(name))
        .map(|p| p.container_port as u16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resolves_named_ports_and_readiness() {
        let pod: Pod = serde_json::from_value(json!({
            "metadata": {"name": "web-1"},
            "spec": {"containers": [{"name": "app", "ports": [{"name": "http", "containerPort": 8080}]}]},
            "status": {"conditions": [{"type": "Ready", "status": "True"}]},
        }))
        .unwrap();
        assert_eq!(named_port(&pod, "http"), Some(8080));
        assert_eq!(named_port(&pod, "grpc"), None);
        assert!(is_ready(&pod));
        assert!(is_tcp(None) && is_tcp(Some("tcp")) && !is_tcp(Some("UDP")));
    }
}

#[cfg(test)]
#[path = "forward_tests.rs"]
mod coverage_tests;
