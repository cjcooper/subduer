//! A container's image metadata in the Details tab: the request, when to make
//! it, and the texts of its summary, rows and tables. Read from the registry
//! the first time the section opens, then kept for the selection.

use serde::Serialize;
use subduer_types::ImageMetadata;

use crate::logic::overview::fmt_bytes;

/// `image_metadata` arguments, and the key its result is kept under.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub(crate) struct ImageArgs {
    pub(crate) namespace: String,
    pub(crate) pod: String,
    pub(crate) container: String,
}

/// A request's state: `None` while it runs.
pub(crate) type Loaded = Option<Result<ImageMetadata, String>>;

/// Read it now: the section is open and it hasn't been asked for.
pub(crate) fn needs_load(open: bool, state: Option<&Loaded>) -> bool {
    open && state.is_none()
}

/// The section's summary beside its title.
pub(crate) fn summary(state: Option<&Loaded>) -> String {
    match state {
        None => "read from the registry when opened".into(),
        Some(None) => "reading from the registry…".into(),
        Some(Some(Err(_))) => "couldn't read".into(),
        Some(Some(Ok(m))) => {
            let built = m.created.as_deref().map(|c| format!(" · built {}", c.get(..10).unwrap_or(c))).unwrap_or_default();
            format!("{} · {}{built}", m.platform, fmt_bytes(m.size as f64))
        }
    }
}

/// The rows after Created, without the empty ones.
pub(crate) fn rows(m: &ImageMetadata) -> Vec<(&'static str, String)> {
    let opt = |v: &Option<String>| v.clone().unwrap_or_default();
    [
        ("Reference", m.reference.clone()),
        ("Platform", m.platform.clone()),
        ("Size", format!("{} compressed", fmt_bytes(m.size as f64))),
        ("Image ID", m.id.clone()),
        ("Manifest digest", m.digest.clone()),
        ("Index digest", opt(&m.index_digest)),
        ("Entrypoint", command(&m.entrypoint)),
        ("Command", command(&m.cmd)),
        ("Working dir", opt(&m.working_dir)),
        // Unset means root, which is worth saying.
        ("User", m.user.clone().unwrap_or_else(|| "root (not set)".into())),
        ("Exposed ports", m.exposed_ports.join(", ")),
        ("Volumes", m.volumes.join(", ")),
        ("Stop signal", opt(&m.stop_signal)),
        ("Author", opt(&m.author)),
        ("Docker version", opt(&m.docker_version)),
        ("Read with", m.credentials.clone()),
    ]
    .into_iter()
    .filter(|(_, v)| !v.is_empty())
    .collect()
}

/// An exec-form command as the Dockerfile writes it: `["nginx", "-g", "daemon off;"]`.
pub(crate) fn command(args: &[String]) -> String {
    if args.is_empty() {
        return String::new();
    }
    let quoted: Vec<String> = args.iter().map(|a| serde_json::Value::from(a.as_str()).to_string()).collect();
    format!("[{}]", quoted.join(", "))
}

/// `NAME=value` as (name, value).
pub(crate) fn env_pair(entry: &str) -> (String, String) {
    let (name, value) = entry.split_once('=').unwrap_or((entry, ""));
    (name.to_owned(), value.to_owned())
}

pub(crate) fn env_summary(m: &ImageMetadata) -> String {
    count(m.env.len(), "variable", "variables")
}

pub(crate) fn labels_summary(m: &ImageMetadata) -> String {
    count(m.labels.len(), "label", "labels")
}

/// Steps and how many added a layer.
pub(crate) fn history_summary(m: &ImageMetadata) -> String {
    let layers = m.history.iter().filter(|h| h.size.is_some()).count();
    format!("{} · {}", count(m.history.len(), "step", "steps"), count(layers, "layer", "layers"))
}

/// Each step as (size, step, comment); the size is blank for steps that
/// add no layer.
pub(crate) fn history_rows(m: &ImageMetadata) -> Vec<(String, String, Option<String>)> {
    m.history
        .iter()
        .map(|h| (h.size.map(|s| fmt_bytes(s as f64)).unwrap_or_default(), h.created_by.clone(), h.comment.clone()))
        .collect()
}

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

#[cfg(test)]
#[path = "image_tests.rs"]
mod tests;
