//! The parts of a container's spec the Details tabs don't show elsewhere
//! (image, ports, resources and environment have their own rows), as
//! label/value pairs. Fields without a dedicated row are still listed, so
//! nothing a container sets is hidden.

use serde_json::Value;

/// Shown in their own rows or sections, or as the sidecar marker.
const ELSEWHERE: &[&str] = &["name", "image", "ports", "resources", "env", "envFrom", "restartPolicy"];

/// Server defaults that every container gets; listed only when changed.
const DEFAULTS: &[(&str, &str)] = &[
    ("terminationMessagePath", "/dev/termination-log"),
    ("terminationMessagePolicy", "File"),
];

/// `(label, value)` for every field of a container's spec (as JSON) not
/// shown elsewhere, the common ones first in a fixed order.
pub(crate) fn container_spec(c: &Value) -> Vec<(String, String)> {
    let Some(fields) = c.as_object() else { return vec![] };
    let mut out = vec![];
    let mut push = |label: &str, value: String| out.push((label.to_owned(), value));

    if let Some(v) = fields.get("command") {
        push("Command", words(v));
    }
    if let Some(v) = fields.get("args") {
        push("Args", words(v));
    }
    if let Some(v) = fields.get("workingDir").and_then(Value::as_str) {
        push("Working dir", v.to_owned());
    }
    if let Some(v) = fields.get("imagePullPolicy").and_then(Value::as_str) {
        push("Image pull policy", v.to_owned());
    }
    for (key, label) in [("startupProbe", "Startup probe"), ("livenessProbe", "Liveness probe"), ("readinessProbe", "Readiness probe")] {
        if let Some(v) = fields.get(key) {
            push(label, probe(v));
        }
    }
    if let Some(lifecycle) = fields.get("lifecycle").and_then(Value::as_object) {
        for (key, value) in lifecycle {
            match key.as_str() {
                "postStart" => push("Post-start hook", handler(value)),
                "preStop" => push("Pre-stop hook", handler(value)),
                _ => push(&format!("lifecycle.{key}"), flat(value)),
            }
        }
    }
    if let Some(v) = fields.get("volumeMounts").and_then(Value::as_array) {
        push("Mounts", v.iter().map(mount).collect::<Vec<_>>().join("\n"));
    }
    if let Some(v) = fields.get("volumeDevices").and_then(Value::as_array) {
        let devices = v.iter().map(|d| format!("{} → {}", text(&d["name"]), text(&d["devicePath"])));
        push("Devices", devices.collect::<Vec<_>>().join("\n"));
    }
    if let Some(v) = fields.get("securityContext") {
        push("Security context", flat(v));
    }
    if let Some(v) = fields.get("resizePolicy") {
        push("Resize policy", flat(v));
    }
    if let Some(v) = fields.get("targetContainerName").and_then(Value::as_str) {
        push("Target container", v.to_owned());
    }

    const SHOWN: &[&str] = &[
        "command", "args", "workingDir", "imagePullPolicy", "startupProbe", "livenessProbe", "readinessProbe",
        "lifecycle", "volumeMounts", "volumeDevices", "securityContext", "resizePolicy", "targetContainerName",
    ];
    for (key, value) in fields {
        let default = DEFAULTS.iter().any(|(k, d)| k == key && value.as_str() == Some(d));
        if SHOWN.contains(&key.as_str()) || ELSEWHERE.contains(&key.as_str()) || default || value.is_null() {
            continue;
        }
        push(&label(key), flat(value));
    }
    out
}

/// `terminationMessagePolicy` → `Termination message policy`.
fn label(key: &str) -> String {
    let mut out = String::new();
    for (i, ch) in key.chars().enumerate() {
        if i == 0 {
            out.extend(ch.to_uppercase());
        } else if ch.is_uppercase() {
            out.push(' ');
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// A command or args list as a shell would read it: words with spaces or
/// quotes are single-quoted.
fn words(v: &Value) -> String {
    let quote = |w: &str| {
        if !w.is_empty() && !w.contains(|c: char| c.is_whitespace() || "'\"\\$`;&|<>(){}*?#~".contains(c)) {
            w.to_owned()
        } else {
            format!("'{}'", w.replace('\'', r"'\''"))
        }
    };
    v.as_array()
        .into_iter()
        .flatten()
        .map(|w| quote(&text(w)))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Like `kubectl describe`: `http-get http://:8080/healthz delay=0s
/// timeout=1s period=10s #success=1 #failure=3`, with Kubernetes' defaults
/// for unset timings.
fn probe(p: &Value) -> String {
    let secs = |key: &str, default: i64| format!("{}s", p[key].as_i64().unwrap_or(default));
    let count = |key: &str, default: i64| p[key].as_i64().unwrap_or(default);
    let mut out = format!(
        "{} delay={} timeout={} period={} #success={} #failure={}",
        handler(p),
        secs("initialDelaySeconds", 0),
        secs("timeoutSeconds", 1),
        secs("periodSeconds", 10),
        count("successThreshold", 1),
        count("failureThreshold", 3),
    );
    if let Some(grace) = p["terminationGracePeriodSeconds"].as_i64() {
        out.push_str(&format!(" grace={grace}s"));
    }
    out
}

/// A probe's or lifecycle hook's action.
fn handler(h: &Value) -> String {
    if let Some(exec) = h.get("exec") {
        format!("exec {}", words(&exec["command"]))
    } else if let Some(get) = h.get("httpGet") {
        let scheme = get["scheme"].as_str().unwrap_or("HTTP").to_lowercase();
        let headers: Vec<String> =
            get["httpHeaders"].as_array().into_iter().flatten().map(|h| format!("{}: {}", text(&h["name"]), text(&h["value"]))).collect();
        let mut out =
            format!("http-get {scheme}://{}:{}{}", text(&get["host"]), text(&get["port"]), get["path"].as_str().unwrap_or("/"));
        if !headers.is_empty() {
            out.push_str(&format!(" [{}]", headers.join(", ")));
        }
        out
    } else if let Some(tcp) = h.get("tcpSocket") {
        format!("tcp-socket {}:{}", text(&tcp["host"]), text(&tcp["port"]))
    } else if let Some(grpc) = h.get("grpc") {
        match grpc["service"].as_str() {
            Some(service) => format!("grpc :{} {service}", text(&grpc["port"])),
            None => format!("grpc :{}", text(&grpc["port"])),
        }
    } else if let Some(sleep) = h.get("sleep") {
        format!("sleep {}s", text(&sleep["seconds"]))
    } else {
        flat(h)
    }
}

/// `data → /etc/app (ro, subPath config.yaml)`.
fn mount(m: &Value) -> String {
    let mut notes = vec![];
    if m["readOnly"].as_bool() == Some(true) {
        notes.push("ro".to_owned());
    }
    for (key, label) in [("subPath", "subPath"), ("subPathExpr", "subPathExpr"), ("mountPropagation", "propagation")] {
        if let Some(v) = m[key].as_str() {
            notes.push(format!("{label} {v}"));
        }
    }
    if let Some(v) = m["recursiveReadOnly"].as_str() {
        notes.push(format!("recursiveReadOnly {v}"));
    }
    let base = format!("{} → {}", text(&m["name"]), text(&m["mountPath"]));
    if notes.is_empty() { base } else { format!("{base} ({})", notes.join(", ")) }
}

/// Any value on one line: `runAsUser=1000, capabilities.drop=[ALL]`.
fn flat(v: &Value) -> String {
    fn walk(prefix: &str, v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(map) if !map.is_empty() => {
                for (k, v) in map {
                    let key = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
                    walk(&key, v, out);
                }
            }
            _ if prefix.is_empty() => out.push(scalar(v)),
            _ => out.push(format!("{prefix}={}", scalar(v))),
        }
    }
    let mut out = vec![];
    match v {
        // A list of objects, e.g. a resize policy: one entry each.
        Value::Array(items) if items.iter().any(Value::is_object) => {
            return items.iter().map(flat).collect::<Vec<_>>().join("; ");
        }
        _ => walk("", v, &mut out),
    }
    out.join(", ")
}

fn scalar(v: &Value) -> String {
    match v {
        Value::Array(items) => format!("[{}]", items.iter().map(scalar).collect::<Vec<_>>().join(", ")),
        Value::Object(map) if map.is_empty() => "{}".to_owned(),
        Value::Object(_) => flat(v),
        _ => text(v),
    }
}

/// A string as-is, a number or bool as written, null as nothing.
fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

#[cfg(test)]
#[path = "container_spec_tests.rs"]
mod tests;
