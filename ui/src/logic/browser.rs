//! Paths and labels for the container folder browser.

use subduer_types::DirEntry;

/// `dir` joined with `name`, without doubling the slash.
pub(crate) fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') { format!("{dir}{name}") } else { format!("{dir}/{name}") }
}

/// The folder part of a path: itself if it ends with `/`, else its parent.
pub(crate) fn folder_of(path: &str) -> String {
    match path.trim().rsplit_once('/') {
        Some(("", _)) => "/".into(),
        Some((dir, _)) => dir.into(),
        None => "/".into(),
    }
}

/// What the browser is for, which changes what entries say and allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BrowseMode {
    /// Folders show whether they're writable; files can be replaced.
    Upload,
    /// Files show whether they're readable; unreadable ones can't be picked.
    Download,
}

/// Whether the writable-place shortcuts show: uploading, with some to show.
pub(crate) fn show_chips(mode: BrowseMode, suggestions: &[String]) -> bool {
    mode == BrowseMode::Upload && !suggestions.is_empty()
}

/// Breadcrumbs for a path as (label, path): `/` first, then each folder.
pub(crate) fn crumb_paths(path: &str) -> Vec<(String, String)> {
    let mut acc = String::new();
    let mut out = vec![("/".to_owned(), "/".to_owned())];
    for part in path.split('/').filter(|p| !p.is_empty()) {
        acc.push('/');
        acc.push_str(part);
        out.push((part.to_owned(), acc.clone()));
    }
    out
}

/// An entry's note, whether it's dimmed, and whether it can be clicked.
pub(crate) fn entry_state(mode: BrowseMode, e: &DirEntry) -> (&'static str, bool, bool) {
    match (mode, e.dir) {
        (BrowseMode::Upload, true) => (if e.writable { "writable" } else { "read-only" }, !e.writable, true),
        (BrowseMode::Upload, false) => (if e.writable { "replace" } else { "read-only" }, !e.writable, true),
        (BrowseMode::Download, true) => (if e.readable { "" } else { "no access" }, !e.readable, e.readable),
        (BrowseMode::Download, false) => (if e.readable { "download" } else { "can't read" }, !e.readable, e.readable),
    }
}

pub(crate) fn entry_title(mode: BrowseMode, dir: bool, full: &str) -> String {
    match (mode, dir) {
        (_, true) => format!("Open {full}"),
        (BrowseMode::Upload, false) => format!("Upload as {full} (asks before replacing)"),
        (BrowseMode::Download, false) => format!("Download {full}"),
    }
}

/// The current folder's status note.
pub(crate) fn folder_status(mode: BrowseMode, writable: bool) -> &'static str {
    match (mode, writable) {
        (BrowseMode::Upload, true) => "✓ writable",
        (BrowseMode::Upload, false) => "read-only: files can't be saved here",
        (BrowseMode::Download, _) => "",
    }
}

/// Whether `path` is the root, which has no folder above it.
pub(crate) fn at_root(path: &str) -> bool {
    path == "/"
}

#[cfg(test)]
#[path = "browser_tests.rs"]
mod tests;
