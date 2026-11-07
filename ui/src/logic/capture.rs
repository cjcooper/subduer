//! Packet captures' logic: the dialog's filter, ports, file name and
//! request, and the capture window's texts, counts and table order.

use subduer_types::{
    CaptureFilter, CapturePhase, CaptureRequest, CaptureTraffic, CaptureUpdate, PodDetails, TlsConnection, TlsDirection, TlsScan,
};

/// The dialog's choices of how long to capture, in seconds.
pub(crate) const DURATIONS: [u32; 4] = [60, 5 * 60, 10 * 60, 30 * 60];
/// The version auditors expect; anything else is flagged.
pub(crate) const GOOD: &str = "TLS 1.3";
/// The capture window's table and its headings.
pub(crate) const TABLE: &str = "capture";
pub(crate) const HEADERS: [&str; 7] = ["Time", "Way", "Client", "Server", "Server name", "Version", "Cipher"];

/// The filter from the dialog: its traffic choice ("all", "port:<n>" or
/// "custom"), the custom text, and the handshakes box.
pub(crate) fn filter_from(choice: &str, custom: &str, handshakes_only: bool) -> CaptureFilter {
    let traffic = match choice.strip_prefix("port:").and_then(|p| p.parse().ok()) {
        Some(port) => CaptureTraffic::Inbound(port),
        None if choice == "custom" => CaptureTraffic::Custom(custom.trim().to_owned()),
        None => CaptureTraffic::All,
    };
    CaptureFilter { traffic, handshakes_only }
}

/// The TCP ports the pod's containers declare, each once, in order, with a
/// name if one container gives it one: "http 8080/TCP" → (8080, "http").
/// Ephemeral containers declare none.
pub(crate) fn inbound_ports(d: &PodDetails) -> Vec<(u16, Option<String>)> {
    let mut ports: Vec<(u16, Option<String>)> = vec![];
    for spec in d.containers.iter().filter(|c| !c.ephemeral).flat_map(|c| &c.ports) {
        let (name, port) = match spec.rsplit_once(' ') {
            Some((name, port)) => (Some(name.to_owned()), port),
            None => (None, spec.as_str()),
        };
        let Some((number, proto)) = port.split_once('/') else { continue };
        let Ok(number) = number.parse::<u16>() else { continue };
        if proto != "TCP" {
            continue;
        }
        match ports.iter_mut().find(|p| p.0 == number) {
            Some(known) if known.1.is_none() => known.1 = name,
            Some(_) => {}
            None => ports.push((number, name)),
        }
    }
    ports.sort_by_key(|p| p.0);
    ports
}

/// The ports of `pod` in `namespace`, when `details` are that pod's.
pub(crate) fn pod_ports(details: Option<&Result<PodDetails, String>>, namespace: &str, pod: &str) -> Vec<(u16, Option<String>)> {
    match details {
        Some(Ok(d)) if d.name == pod && d.namespace == namespace => inbound_ports(d),
        _ => vec![],
    }
}

/// A port's choice: its value and label.
pub(crate) fn port_choice(port: u16, name: Option<&str>) -> (String, String) {
    let label = match name {
        Some(n) => format!("Inbound to port {port} ({n})"),
        None => format!("Inbound to port {port}"),
    };
    (format!("port:{port}"), label)
}

/// The Save dialog's suggestion: `acme-uat-shop-web-1-2026-10-12-1430.pcap`.
pub(crate) fn file_name(context: &str, namespace: &str, pod: &str, stamp: &str) -> String {
    let safe = |s: &str| s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '-' }).collect::<String>();
    format!("{}-{}-{}-{stamp}.pcap", safe(context), safe(namespace), safe(pod))
}

/// "1 min", "30 min".
pub(crate) fn minutes(secs: u32) -> String {
    format!("{} min", secs / 60)
}

/// What the dialog has chosen.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Choices {
    pub(crate) image: String,
    pub(crate) secs: u32,
    /// "all", "port:<n>" or "custom".
    pub(crate) choice: String,
    pub(crate) custom: String,
    pub(crate) handshakes: bool,
}

impl Choices {
    /// Whether Start can be pressed: not busy, changes allowed, an image, a
    /// custom filter if chosen.
    pub(crate) fn ready(&self, busy: bool, locked: bool) -> bool {
        !busy && !locked && !self.image.trim().is_empty() && !(self.choice == "custom" && self.custom.trim().is_empty())
    }

    /// The capture to start, before a file is chosen.
    pub(crate) fn request(&self, namespace: &str, pod: &str) -> CaptureRequest {
        CaptureRequest {
            namespace: namespace.to_owned(),
            pod: pod.to_owned(),
            image: self.image.trim().to_owned(),
            filter: filter_from(&self.choice, &self.custom, self.handshakes),
            secs: self.secs,
            path: String::new(),
        }
    }
}

/// The window label from a capture window's query string (`?capture=<label>`):
/// only labels the backend gives capture windows (`capture-<n>`).
pub(crate) fn label_in(search: &str) -> Option<String> {
    search
        .trim_start_matches('?')
        .split('&')
        .find_map(|kv| kv.strip_prefix("capture="))
        .filter(|l| l.starts_with("capture-"))
        .map(str::to_owned)
}

/// Connections by negotiated version, TLS 1.3 first, then the rest by
/// count; "no answer" for connections whose server answer wasn't captured.
pub(crate) fn version_counts(scan: &TlsScan) -> Vec<(String, usize)> {
    let mut counts: Vec<(String, usize)> = vec![];
    for c in &scan.connections {
        let v = c.version.clone().unwrap_or_else(|| "no answer".into());
        match counts.iter_mut().find(|(k, _)| *k == v) {
            Some((_, n)) => *n += 1,
            None => counts.push((v, 1)),
        }
    }
    counts.sort_by(|a, b| (b.0 == GOOD).cmp(&(a.0 == GOOD)).then(b.1.cmp(&a.1)).then(a.0.cmp(&b.0)));
    counts
}

/// A count's badge: (classes, text).
pub(crate) fn count_badge(version: &str, n: usize) -> (&'static str, String) {
    let class = if version == GOOD {
        "flag good"
    } else if version == "no answer" {
        "flag info"
    } else {
        "flag serious"
    };
    (class, format!("{version} · {n}"))
}

/// Not the expected version: an older one was chosen.
pub(crate) fn flagged(c: &TlsConnection) -> bool {
    c.version.as_deref().is_some_and(|v| v != GOOD)
}

/// "12.3 KiB".
pub(crate) fn size_text(bytes: u64) -> String {
    crate::overview::fmt_bytes(bytes as f64)
}

/// "4:05" left.
pub(crate) fn clock_text(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

pub(crate) fn phase_text(p: CapturePhase) -> &'static str {
    match p {
        CapturePhase::Starting => "Starting the capture container…",
        CapturePhase::Capturing => "Capturing",
        CapturePhase::Stopping => "Stopping…",
        CapturePhase::Done => "Finished",
        CapturePhase::Failed => "Failed",
    }
}

pub(crate) fn phase_class(p: CapturePhase) -> &'static str {
    match p {
        CapturePhase::Failed => "capture-phase failed",
        CapturePhase::Done => "capture-phase done",
        _ => "capture-phase live",
    }
}

/// The phase, with the time left while capturing; `now` in unix seconds.
pub(crate) fn status_text(u: &CaptureUpdate, now: f64) -> String {
    let left = match (u.phase, u.ends) {
        (CapturePhase::Capturing, Some(end)) => format!(" · {} left", clock_text(end - now)),
        _ => String::new(),
    };
    format!("{}{left}", phase_text(u.phase))
}

/// Packets, bytes and connections so far.
pub(crate) fn totals_text(u: &CaptureUpdate) -> String {
    format!("{} packets · {} · {} TLS connections", u.scan.packets, size_text(u.bytes), u.scan.connections.len())
}

/// How far along the capture is (0–1), once it has a start and an end.
pub(crate) fn progress(u: &CaptureUpdate, now: f64) -> Option<f64> {
    let (Some(a), Some(b)) = (u.started, u.ends) else { return None };
    Some(if u.phase == CapturePhase::Capturing { ((now - a) / (b - a)).clamp(0.0, 1.0) } else { 1.0 })
}

/// The progress bar: (aria value, width style).
pub(crate) fn progress_attrs(done: f64) -> (String, String) {
    ((done * 100.0).round().to_string(), format!("width: {:.1}%", done * 100.0))
}

/// Still going: Stop is offered.
pub(crate) fn running(p: CapturePhase) -> bool {
    matches!(p, CapturePhase::Starting | CapturePhase::Capturing)
}

/// The Stop button: (disabled, label).
pub(crate) fn stop_button(stopping: bool, p: CapturePhase) -> (bool, &'static str) {
    (stopping || p != CapturePhase::Capturing, if stopping || p == CapturePhase::Stopping { "Stopping…" } else { "Stop" })
}

/// The image the container runs, once known, else the one asked for.
pub(crate) fn image_text(u: &CaptureUpdate, asked: &str) -> String {
    u.image_id.clone().unwrap_or_else(|| asked.to_owned())
}

/// Below the window's title: `shop · acme-uat`.
pub(crate) fn where_text(namespace: &str, context: &str) -> String {
    format!("{namespace} · {context}")
}

/// Which way a connection went, in a word.
pub(crate) fn way(d: TlsDirection) -> &'static str {
    match d {
        TlsDirection::Inbound => "in",
        TlsDirection::Outbound => "out",
        TlsDirection::Other => "",
    }
}

/// How a handshake sorts under column `i`; addresses in natural order.
pub(crate) fn connection_key(c: &TlsConnection, i: usize) -> crate::sort::Key {
    use crate::sort::Key;
    match i {
        0 => Key::num(c.first_seen),
        1 => Key::text(way(c.direction)),
        2 => Key::text(&c.client),
        3 => Key::text(&c.server),
        4 => Key::text(c.server_name.as_deref().unwrap_or_default()),
        5 => Key::text(c.version.as_deref().unwrap_or_default()),
        _ => Key::text(c.cipher.as_deref().unwrap_or_default()),
    }
}

/// The table's rows: newest first (what just happened is on top), unless sorted.
pub(crate) fn rows(mut list: Vec<TlsConnection>, by: Option<(usize, bool)>) -> Vec<TlsConnection> {
    list.reverse();
    crate::sort::sort(&mut list, by, connection_key);
    list
}

#[cfg(test)]
#[path = "capture_tests.rs"]
mod tests;
