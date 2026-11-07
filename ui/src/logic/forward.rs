//! Port-forwarding: the start dialog's choices and request, and the Port
//! Forwards page's rows and sort keys.

use serde::Serialize;
use subduer_types::{ForwardInfo, ForwardRequest, ForwardSource, PortOption};

use crate::sort::Key;

/// The Port Forwards table's headings.
pub(crate) const HEADERS: [&str; 6] = ["Target", "Context", "Local", "Remote", "Connections", "Last error"];

/// What the start dialog is for.
#[derive(Debug, Clone, PartialEq)]
pub struct ForwardPrompt {
    pub namespace: String,
    pub source: ForwardSource,
}

/// `open_forward` and `stop_forward` arguments.
#[derive(Debug, Serialize)]
pub(crate) struct IdArgs {
    pub(crate) id: u64,
}

/// `forward_ports` arguments.
#[derive(Debug, Serialize)]
pub(crate) struct PortsArgs {
    pub(crate) namespace: String,
    pub(crate) source: ForwardSource,
}

/// `start_forward` arguments.
#[derive(Debug, Serialize)]
pub(crate) struct StartArgs {
    pub(crate) request: ForwardRequest,
}

pub(crate) fn ports_args(prompt: ForwardPrompt) -> PortsArgs {
    PortsArgs { namespace: prompt.namespace, source: prompt.source }
}

pub(crate) fn start_args(prompt: ForwardPrompt, remote_port: u16, local_port: u16) -> StartArgs {
    StartArgs { request: ForwardRequest { namespace: prompt.namespace, source: prompt.source, remote_port, local_port } }
}

/// Once the ports load: the first is chosen, with the same local port.
pub(crate) fn first_choice(ports: &Result<Vec<PortOption>, String>) -> Option<(Option<u16>, String)> {
    let first = ports.as_ref().ok()?.first().map(|p| p.port);
    Some((first, first.map(|p| p.to_string()).unwrap_or_default()))
}

/// The typed local port, if it's one.
pub(crate) fn local_port(typed: &str) -> Option<u16> {
    typed.trim().parse::<u16>().ok()
}

/// Whether Start forwarding is enabled.
pub(crate) fn can_start(busy: bool, chosen: Option<u16>, typed: &str) -> bool {
    !busy && chosen.is_some() && local_port(typed).is_some()
}

/// "shop/pod/web-1".
pub(crate) fn target(namespace: &str, source: &ForwardSource) -> String {
    format!("{namespace}/{}", source.label())
}

/// A port choice: "8080 · http · app".
pub(crate) fn port_label(p: &PortOption) -> String {
    let port = p.port;
    match (&p.name, &p.detail) {
        (Some(n), Some(d)) => format!("{port} · {n} · {d}"),
        (Some(n), None) => format!("{port} · {n}"),
        (None, Some(d)) => format!("{port} · {d}"),
        (None, None) => port.to_string(),
    }
}

pub(crate) fn address(f: &ForwardInfo) -> String {
    format!("127.0.0.1:{}", f.local_port)
}

/// The Remote cell: the port, and where connections go.
pub(crate) fn remote(f: &ForwardInfo) -> String {
    match &f.backend {
        Some(b) => format!("{} → {b}", f.remote_port),
        None => f.remote_port.to_string(),
    }
}

pub(crate) fn connections(f: &ForwardInfo) -> String {
    format!("{} open · {} total", f.active, f.total)
}

/// How a forward sorts under column `i`: ports by number, connections by
/// how many are open, then the total.
pub(crate) fn forward_key(f: &ForwardInfo, i: usize) -> Key {
    match i {
        0 => Key::text(&target(&f.namespace, &f.source)),
        1 => Key::text(&f.context),
        2 => Key::num(f64::from(f.local_port)),
        3 => Key::num(f64::from(f.remote_port)),
        4 => Key::num(f.active as f64 * 1e12 + f.total as f64),
        _ => Key::text(f.last_error.as_deref().unwrap_or_default()),
    }
}

#[cfg(test)]
#[path = "forward_tests.rs"]
mod tests;
