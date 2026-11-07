//! Uploading a file into a container: size limit, destination checks and
//! hints, the request, and what each outcome leads to.

use subduer_types::{DirListing, UploadOutcome, UploadTarget};

use crate::browser::{folder_of, join};
use crate::logic::overview::fmt_bytes;

/// Matches the backend limit; the file is held in memory.
pub(crate) const MAX_BYTES: f64 = 512.0 * 1024.0 * 1024.0;
pub(crate) const START_DIR: &str = "/tmp";

/// The container the dialog uploads into.
#[derive(Debug, Clone, PartialEq)]
pub struct UploadPrompt {
    pub context: String,
    pub namespace: String,
    pub pod: String,
    pub container: String,
}

/// The destination the dialog starts with.
pub(crate) fn start_dest() -> String {
    format!("{START_DIR}/")
}

/// The dialog's step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Phase {
    Choosing,
    Uploading,
    /// The file exists; offer to replace it.
    Exists(String),
    Done { bytes: u64, path: String },
}

/// The step after an upload, and the error to show.
pub(crate) fn after_upload(result: Result<UploadOutcome, String>) -> (Phase, Option<String>) {
    match result {
        Ok(UploadOutcome::Uploaded { bytes, path }) => (Phase::Done { bytes, path }, None),
        Ok(UploadOutcome::Exists { path }) => (Phase::Exists(path), None),
        Err(e) => (Phase::Choosing, Some(e)),
    }
}

/// What Esc does: from "already exists", back to choosing; otherwise close.
pub(crate) fn cancel_goes_back(phase: &Phase) -> bool {
    matches!(phase, Phase::Exists(_))
}

/// The error for a chosen file that's too big.
pub(crate) fn size_error(size: f64) -> Option<String> {
    (size > MAX_BYTES).then(|| "Files over 512 MiB aren't supported.".into())
}

pub(crate) fn dest_ok(dest: &str) -> bool {
    dest.trim().starts_with('/')
}

/// Whether Upload is enabled: a file within the limit and an absolute destination.
pub(crate) fn can_upload(size: Option<f64>, dest: &str) -> bool {
    size.is_some_and(|s| s <= MAX_BYTES) && dest_ok(dest)
}

/// After browsing to `folder`: the destination points at it, keeping a typed
/// file name.
pub(crate) fn browsed_dest(folder: &str, dest: &str) -> String {
    join(folder, dest.rsplit('/').next().unwrap_or_default())
}

/// Whether to browse `/` instead: no start folder (`suggest`) to show.
pub(crate) fn retry_at_root(suggest: bool, failed: bool) -> bool {
    suggest && failed
}

/// What's uploaded where.
pub(crate) fn target(prompt: UploadPrompt, dest: &str, file_name: String, overwrite: bool) -> UploadTarget {
    UploadTarget {
        context: prompt.context,
        namespace: prompt.namespace,
        pod: prompt.pod,
        container: prompt.container,
        path: dest.trim().to_owned(),
        file_name,
        overwrite,
    }
}

/// "app · shop/web-1 · acme" under the dialog's title.
pub(crate) fn target_line(p: &UploadPrompt) -> String {
    format!("{} · {}/{} · {}", p.container, p.namespace, p.pod, p.context)
}

/// " · 2.0 MiB" after the chosen file's name.
pub(crate) fn file_size(size: f64) -> String {
    format!(" · {}", fmt_bytes(size))
}

/// What the destination `dest` will mean for a file named `name` (if chosen),
/// given the folder on screen.
pub(crate) fn dest_hint(dest: &str, shown: Option<&DirListing>, name: Option<&str>) -> String {
    let d = dest.trim();
    if !d.starts_with('/') {
        return "Use an absolute path, e.g. /tmp/".to_owned();
    }
    // Warn before uploading into the folder on screen when it's read-only.
    let read_only =
        shown.is_some_and(|l| !l.writable && folder_of(d).trim_end_matches('/') == l.path.trim_end_matches('/'));
    if read_only {
        return "This folder is read-only for the container user; pick a writable one.".to_owned();
    }
    match name {
        Some(name) if d.ends_with('/') => format!("Saved as {}", join(d, name)),
        Some(name) => format!("Saved as {d}, or as {} if {d} is a folder", join(d, name)),
        None if d.ends_with('/') => "The file keeps its name in this folder.".into(),
        None => "A folder keeps the file's name; anything else is the new file's path.".into(),
    }
}

#[cfg(test)]
#[path = "upload_tests.rs"]
mod tests;
