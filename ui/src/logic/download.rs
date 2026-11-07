//! Downloading a file from a container: what Enter does, when Download is
//! enabled, and the requests.

use serde::Serialize;

use crate::browser::{folder_of, join};
use crate::logic::overview::fmt_bytes;

/// `browse_dir` arguments (shared with the upload dialog).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct BrowseArgs {
    pub(crate) namespace: String,
    pub(crate) pod: String,
    pub(crate) container: String,
    pub(crate) path: String,
    pub(crate) suggest: bool,
}

/// `download_file` arguments.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct DownloadArgs {
    pub(crate) namespace: String,
    pub(crate) pod: String,
    pub(crate) container: String,
    pub(crate) path: String,
}

/// `reveal_download` arguments.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct RevealArgs {
    pub(crate) path: String,
}

/// Whether the typed path names a file: absolute, not a folder.
pub(crate) fn is_file(path: &str) -> bool {
    let p = path.trim();
    p.starts_with('/') && !p.ends_with('/')
}

/// What Enter in the path box does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Enter {
    Download,
    /// Browse to this folder.
    Browse(String),
}

pub(crate) fn on_enter(path: &str) -> Enter {
    if is_file(path) { Enter::Download } else { Enter::Browse(folder_of(path)) }
}

/// A browsed folder as the path box shows it, ending in `/`.
pub(crate) fn folder_path(dir: &str) -> String {
    join(dir, "")
}

/// "app · shop/web-1" under the dialog's title.
pub(crate) fn source(container: &str, namespace: &str, pod: &str) -> String {
    format!("{container} · {namespace}/{pod}")
}

/// A transferred size.
pub(crate) fn size(bytes: u64) -> String {
    fmt_bytes(bytes as f64)
}

#[cfg(test)]
#[path = "download_tests.rs"]
mod tests;
